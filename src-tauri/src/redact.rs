//! 日志和错误信息里的密钥脱敏。
//!
//! `scrub` 把 key / token / 密码的**值**换成 [`REDACTED`]，保留名字（`api_key=«redacted»`），
//! 这样日志仍能看出「这里有个 key」，却不会泄露它。
//!
//! 不按名字豁免：`max_tokens: 4096` 也会被抹掉。名字相近的值多抹一点只是少一点诊断信息，
//! 漏一个 key 却是事故；真要豁免，只能按「值出现的位置」，不能按 key 的名字。

use std::borrow::Cow;

use once_cell::sync::Lazy;
use regex::{Captures, Regex};

/// 密钥被换成的占位文字。
pub const REDACTED: &str = "«redacted»";

/// OFox（`sk-of-`）、Anthropic（`sk-ant-`）和通用 `sk-` key。前面不能紧贴字母数字，
/// 免得把 `task-…`、`disk-…` 这类词里的 `sk-` 当成 key；前面的那个字符原样保留。
static SK_KEY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(^|[^A-Za-z0-9])(?:sk-of-[A-Za-z0-9_-]{8,}|sk-ant-[A-Za-z0-9_-]{8,}|sk-[A-Za-z0-9]{16,})",
    )
    .expect("sk- key pattern")
});

static JWT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").expect("JWT pattern")
});

static BEARER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(bearer[ \t]+)[A-Za-z0-9._~+/=-]{8,}").expect("Bearer pattern")
});

/// 名字里带 api_key / token / secret / password / authorization 的键值对，
/// 写法有 `k=v`、`k: v`、`"k": "v"`（含 JSON 被转义进字符串里的 `\"k\": \"v\"`）。
/// 值是带引号的整段，或不带引号时到空白、引号、`& , ; ) } ]` 为止；不带引号的值前面
/// 可以有 `Bearer ` / `Basic ` 这样的认证方案，方案保留、后面的凭据抹掉。
static KEY_VALUE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r#"(?i)(?P<name>(?:api[_-]?key|token|secret|password|authorization)[A-Za-z0-9_.-]*"#,
        r#"(?:\\?["'])?[ \t]*[:=][ \t]*)"#,
        r#"(?:"(?P<dq>(?:[^"\\\n]|\\.)+)""#,
        r#"|\\"(?P<edq>(?:[^"\\\n]|\\[^"])+)\\""#,
        r#"|'(?P<sq>[^'\n]+)'"#,
        r#"|(?P<scheme>(?:bearer|basic)[ \t]+)?(?P<bare>[^\s"',;&)}\]\\]+))"#,
    ))
    .expect("key-value pattern")
});

/// 抹掉 `s` 里的 OFox / Anthropic / 通用 `sk-` key、Bearer 令牌、JWT，以及名字像
/// `api_key` / `token` / `secret` / `password` / `authorization` 的键值对里的值。
/// 没有需要抹的内容时借用原字符串，不分配。
pub fn scrub(s: &str) -> Cow<'_, str> {
    let mut text = Cow::Borrowed(s);
    replace(&mut text, &JWT, |_| REDACTED.to_string());
    replace(&mut text, &SK_KEY, |caps| format!("{}{REDACTED}", &caps[1]));
    replace(&mut text, &BEARER, |caps| format!("{}{REDACTED}", &caps[1]));
    replace(&mut text, &KEY_VALUE, redact_key_value);
    text
}

fn replace(text: &mut Cow<'_, str>, pattern: &Regex, with: impl Fn(&Captures) -> String) {
    // 没有匹配时 `replace_all` 返回 `Borrowed`，原字符串不被复制。
    if let Cow::Owned(replaced) = pattern.replace_all(text, with) {
        *text = Cow::Owned(replaced);
    }
}

fn redact_key_value(caps: &Captures) -> String {
    let name = &caps["name"];
    let (value, quote) = if let Some(value) = caps.name("dq") {
        (value.as_str(), r#"""#)
    } else if let Some(value) = caps.name("edq") {
        (value.as_str(), r#"\""#)
    } else if let Some(value) = caps.name("sq") {
        (value.as_str(), "'")
    } else {
        let scheme = caps.name("scheme").map_or("", |m| m.as_str());
        return redact_value(caps, &caps["bare"], &format!("{name}{scheme}"), "");
    };
    redact_value(caps, value, name, quote)
}

/// `{prefix}{quote}«redacted»{quote}`；值已经是占位文字时原样留下（再抹一遍结果一样，不必改写）。
fn redact_value(caps: &Captures, value: &str, prefix: &str, quote: &str) -> String {
    if value == REDACTED {
        caps[0].to_string()
    } else {
        format!("{prefix}{quote}{REDACTED}{quote}")
    }
}

/// 日志文件和 stdout 共用的行格式：`[日期][时间][级别][来源] 内容`，内容先过 [`scrub`]。
/// 在 `tauri_plugin_log::Builder` 里 `.timezone_strategy(..)` 之后用 `.format(log_format)`
/// 挂上；前端经 `plugin:log` 命令写进来的日志也走这条路。
pub fn log_format(
    out: tauri_plugin_log::fern::FormatCallback,
    message: &std::fmt::Arguments,
    record: &log::Record,
) {
    let now = chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]");
    let message = message.to_string();
    out.finish(format_args!(
        "{now}[{}][{}] {}",
        record.level(),
        record.target(),
        scrub(&message)
    ));
}

/// 配置文件解析失败时给用户看的描述：只留行列和错误类型，**不带出错那一行的原文**。
/// `toml_edit` / `serde_json` 自带的 `Display` 会把出错的源码行（里面可能就是 key）
/// 原样打出来，错误字符串经 UI 和日志流出去之前必须走这里。
pub fn describe_toml_error(source: &str, error: &toml_edit::TomlError) -> String {
    describe_toml(source, error.message(), error.span())
}

/// 同 [`describe_toml_error`]，给 `toml` crate 的解析错误。
pub fn describe_toml_de_error(source: &str, error: &toml::de::Error) -> String {
    describe_toml(source, error.message(), error.span())
}

fn describe_toml(source: &str, message: &str, span: Option<std::ops::Range<usize>>) -> String {
    let position = span.and_then(|span| line_column(source, span.start));
    describe(position, message)
}

/// 同 [`describe_toml_error`]，给 `serde_json` 的解析错误。
pub fn describe_json_error(error: &serde_json::Error) -> String {
    let text = error.to_string();
    // `Display` 在末尾附 ` at line L column C`，位置另行给出。
    let message = text.rfind(" at line ").map_or(&*text, |at| &text[..at]);
    let message = match error.classify() {
        serde_json::error::Category::Io => "I/O error",
        _ => message,
    };
    let position = (error.line() != 0).then(|| (error.line(), error.column()));
    describe(position, message)
}

fn describe(position: Option<(usize, usize)>, message: &str) -> String {
    let message = one_line(without_values(message));
    match position {
        Some((line, column)) => format!("line {line}, column {column}: {message}"),
        None => message,
    }
}

/// serde 的类型错误形如 `invalid type: string "sk-…", expected a boolean`：冒号后面是
/// 出错的值，丢掉。语法错误和 `missing field `x`` 这类不带值的信息原样保留。
fn without_values(message: &str) -> &str {
    ["invalid type", "invalid value"]
        .into_iter()
        .find(|prefix| message.starts_with(&format!("{prefix}:")))
        .unwrap_or(message)
}

/// 多行的错误信息合成一行，并保险起见再过一遍 [`scrub`]。
fn one_line(message: &str) -> String {
    let joined = message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    scrub(&joined).into_owned()
}

/// 字节偏移 `offset` 在 `source` 里的行号和列号（都从 1 起，列按字符数）。
fn line_column(source: &str, offset: usize) -> Option<(usize, usize)> {
    let before = source.get(..offset)?;
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    Some((line, before[line_start..].chars().count() + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_scrubs(input: &str, expected: &str) {
        let out = scrub(input);
        assert_eq!(out, expected, "scrub({input:?})");
        assert_eq!(scrub(&out), expected, "scrub is idempotent for {input:?}");
    }

    #[test]
    fn ofox_keys_are_redacted() {
        assert_scrubs(
            "request failed for sk-of-abc123456789 today",
            "request failed for «redacted» today",
        );
        assert_scrubs("sk-of-A1_b2-C3d4E5", "«redacted»");
    }

    #[test]
    fn anthropic_keys_are_redacted() {
        assert_scrubs(
            "key sk-ant-api03-AbCdEf_123-xyz rejected",
            "key «redacted» rejected",
        );
    }

    #[test]
    fn generic_sk_keys_are_redacted() {
        assert_scrubs(
            "using sk-0123456789abcdefABCDEF now",
            "using «redacted» now",
        );
    }

    #[test]
    fn sk_inside_a_longer_word_is_left_alone() {
        assert_scrubs("task-0123456789abcdefABCDEF", "task-0123456789abcdefABCDEF");
        assert_scrubs("disk-of-abcdefghijkl", "disk-of-abcdefghijkl");
    }

    #[test]
    fn short_sk_strings_are_left_alone() {
        assert_scrubs(
            "sk-short sk-of-abc sk-ant-1234",
            "sk-short sk-of-abc sk-ant-1234",
        );
    }

    #[test]
    fn bearer_tokens_are_redacted_and_the_word_kept() {
        assert_scrubs(
            "sent Bearer abcdefgh12345678 to server",
            "sent Bearer «redacted» to server",
        );
        assert_scrubs("bearer ABC.def-123_xyz==", "bearer «redacted»");
    }

    #[test]
    fn bearer_without_a_token_is_left_alone() {
        assert_scrubs("missing Bearer token", "missing Bearer token");
        assert_scrubs("Bearer", "Bearer");
    }

    #[test]
    fn jwts_are_redacted() {
        assert_scrubs(
            "got eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk here",
            "got «redacted» here",
        );
    }

    #[test]
    fn key_value_forms_keep_the_name_and_redact_the_value() {
        for (input, expected) in [
            ("api_key=hunter2hunter2", "api_key=«redacted»"),
            ("OFOX_API_KEY=abc", "OFOX_API_KEY=«redacted»"),
            ("api-key: abc123", "api-key: «redacted»"),
            ("apikey = abc123", "apikey = «redacted»"),
            ("token=abc&other=1", "token=«redacted»&other=1"),
            ("secret: hush", "secret: «redacted»"),
            ("password=p@ss!", "password=«redacted»"),
            (
                "Authorization: Basic dXNlcjpwYXNz",
                "Authorization: Basic «redacted»",
            ),
            (
                "Authorization: Bearer abc",
                "Authorization: Bearer «redacted»",
            ),
            ("Authorization: abc123", "Authorization: «redacted»"),
        ] {
            assert_scrubs(input, expected);
        }
    }

    #[test]
    fn quoted_key_value_forms_keep_quotes() {
        for (input, expected) in [
            (r#"{"api_key": "abc 123"}"#, r#"{"api_key": "«redacted»"}"#),
            (r#"{"token":"abc"}"#, r#"{"token":"«redacted»"}"#),
            (r#"password = 'p w'"#, r#"password = '«redacted»'"#),
            (
                r#"experimental_bearer_token = "short""#,
                r#"experimental_bearer_token = "«redacted»""#,
            ),
            (
                r#"{\"secret\": \"abc\"}"#,
                r#"{\"secret\": \"«redacted»\"}"#,
            ),
            (r#""api_key": "a\"b""#, r#""api_key": "«redacted»""#),
        ] {
            assert_scrubs(input, expected);
        }
    }

    #[test]
    fn empty_values_and_missing_separators_are_left_alone() {
        assert_scrubs(r#""token": """#, r#""token": """#);
        assert_scrubs("token expired", "token expired");
        assert_scrubs("the secret is out", "the secret is out");
        assert_scrubs("password\nnext: line", "password\nnext: line");
    }

    #[test]
    fn several_secrets_in_one_message_are_all_redacted() {
        assert_scrubs(
            "a=sk-of-abc123456789 b=Bearer zzzzzzzzzz c=token:xyz",
            "a=«redacted» b=Bearer «redacted» c=token:«redacted»",
        );
    }

    #[test]
    fn text_without_secrets_is_borrowed_unchanged() {
        for input in [
            "",
            "plain log line: started on port 3011",
            "路径 /Users/me/.codex/config.toml 解析失败",
            "model=gpt-5 base_url=https://api.ofox.ai/v1",
        ] {
            let out = scrub(input);
            assert!(matches!(out, Cow::Borrowed(_)), "{input:?} was copied");
            assert_eq!(out, input);
        }
    }

    #[test]
    fn already_redacted_text_is_stable() {
        let text = "api_key=«redacted» Bearer «redacted» \"token\": \"«redacted»\"";
        assert_eq!(scrub(text), text);
    }

    const KEY_LINE: &str = r#"experimental_bearer_token = "sk-of-abc123456789""#;

    #[test]
    fn toml_error_keeps_position_and_message_but_not_the_source_line() {
        let source = format!("model = \"gpt-5\"\n\n{KEY_LINE} oops\n");
        let error = source.parse::<toml_edit::DocumentMut>().unwrap_err();
        // 自带的 Display 会带出源码行，这正是要避开的
        assert!(error.to_string().contains("sk-of-abc123456789"));

        let described = describe_toml_error(&source, &error);
        assert!(!described.contains("sk-of"), "{described}");
        assert!(!described.contains("abc123456789"), "{described}");
        assert!(described.starts_with("line 3, column "), "{described}");
        assert!(described.len() > "line 3, column 1: ".len(), "{described}");
    }

    #[test]
    fn toml_error_on_the_key_line_itself_does_not_echo_the_key() {
        let source = format!("{KEY_LINE} oops\n");
        let error = source.parse::<toml_edit::DocumentMut>().unwrap_err();

        let described = describe_toml_error(&source, &error);
        assert!(described.starts_with("line 1, column "), "{described}");
        assert!(!described.contains("abc123456789"), "{described}");
    }

    #[test]
    fn toml_error_column_counts_characters_not_bytes() {
        let source = "name = \"中文\" oops\n";
        let error = source.parse::<toml_edit::DocumentMut>().unwrap_err();

        let described = describe_toml_error(source, &error);
        assert!(described.starts_with("line 1, column 13: "), "{described}");
    }

    #[test]
    fn toml_error_message_is_one_line() {
        let source = "a = = 1\n";
        let error = source.parse::<toml_edit::DocumentMut>().unwrap_err();

        assert!(!describe_toml_error(source, &error).contains('\n'));
    }

    #[test]
    fn json_syntax_error_keeps_position_and_message() {
        let source = r#"{"OPENAI_API_KEY": "sk-of-abc123456789", "x": }"#;
        let error = serde_json::from_str::<serde_json::Value>(source).unwrap_err();

        let described = describe_json_error(&error);
        assert_eq!(described, "line 1, column 47: expected value");
    }

    #[test]
    fn json_eof_error_keeps_position() {
        let error = serde_json::from_str::<serde_json::Value>("{\"a\": ").unwrap_err();

        assert_eq!(
            describe_json_error(&error),
            "line 1, column 6: EOF while parsing a value"
        );
    }

    #[test]
    fn json_data_error_never_echoes_the_offending_value() {
        #[derive(Debug, serde::Deserialize)]
        struct Auth {
            #[allow(dead_code)]
            key: bool,
        }
        let source = r#"{"key": "sk-of-abc123456789"}"#;
        let error = serde_json::from_str::<Auth>(source).unwrap_err();
        // 自带的 Display 会带出值
        assert!(error.to_string().contains("sk-of-abc123456789"));

        let described = describe_json_error(&error);
        assert!(!described.contains("abc123456789"), "{described}");
        assert!(described.starts_with("line 1, column "), "{described}");
        assert!(described.contains("invalid type"), "{described}");
    }

    #[test]
    fn json_missing_field_keeps_the_field_name() {
        #[derive(Debug, serde::Deserialize)]
        struct Auth {
            #[allow(dead_code)]
            key: String,
        }
        let error = serde_json::from_str::<Auth>("{}").unwrap_err();

        assert_eq!(
            describe_json_error(&error),
            "line 1, column 2: missing field `key`"
        );
    }

    fn format_with_log_format(
        level: log::Level,
        target: &str,
        message: std::fmt::Arguments,
    ) -> String {
        use std::sync::{Arc, Mutex};
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        let (_, logger) = tauri_plugin_log::fern::Dispatch::new()
            .format(log_format)
            .chain(tauri_plugin_log::fern::Output::call(move |record| {
                sink.lock().unwrap().push(record.args().to_string());
            }))
            .into_log();
        log::Log::log(
            &*logger,
            &log::Record::builder()
                .args(message)
                .level(level)
                .target(target)
                .build(),
        );
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 1);
        lines[0].clone()
    }

    #[test]
    fn log_lines_have_the_secrets_scrubbed() {
        let line = format_with_log_format(
            log::Level::Warn,
            "ofox::bind",
            format_args!(
                "codex bind failed: {} / Bearer abcdefgh12345678",
                "experimental_bearer_token = \"sk-of-abc123456789\""
            ),
        );
        assert!(!line.contains("abc123456789"), "{line}");
        assert!(!line.contains("abcdefgh12345678"), "{line}");
        assert!(
            line.contains("[WARN][ofox::bind] codex bind failed: "),
            "{line}"
        );
        assert!(
            line.contains("experimental_bearer_token = \"«redacted»\""),
            "{line}"
        );
    }

    #[test]
    fn log_lines_keep_a_timestamp_level_and_target() {
        let line = format_with_log_format(log::Level::Info, "app", format_args!("started"));
        let re = Regex::new(r"^\[\d{4}-\d{2}-\d{2}\]\[\d{2}:\d{2}:\d{2}\]\[INFO\]\[app\] started$")
            .unwrap();
        assert!(re.is_match(&line), "{line}");
    }

    #[test]
    fn toml_de_error_keeps_position_and_message_but_not_the_source_line() {
        let source = format!("model = \"gpt-5\"\n{KEY_LINE} oops\n");
        let error = toml::from_str::<toml::Table>(&source).unwrap_err();
        assert!(error.to_string().contains("abc123456789"));

        let described = describe_toml_de_error(&source, &error);
        assert!(described.starts_with("line 2, column "), "{described}");
        assert!(!described.contains("abc123456789"), "{described}");
    }

    #[test]
    fn toml_de_type_error_never_echoes_the_offending_value() {
        #[derive(Debug, serde::Deserialize)]
        struct Config {
            #[allow(dead_code)]
            flag: bool,
        }
        let source = "flag = \"sk-of-abc123456789\"\n";
        let error = toml::from_str::<Config>(source).unwrap_err();
        assert!(error.to_string().contains("abc123456789"));

        let described = describe_toml_de_error(source, &error);
        assert!(!described.contains("abc123456789"), "{described}");
        assert!(described.contains("invalid type"), "{described}");
    }
}
