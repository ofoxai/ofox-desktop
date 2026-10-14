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
}
