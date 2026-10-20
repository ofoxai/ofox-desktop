use std::path::Path;
use std::sync::PoisonError;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("配置错误: {0}")]
    Config(String),
    #[error("无效输入: {0}")]
    InvalidInput(String),
    #[error("IO 错误: {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{context}: {source}")]
    IoContext {
        context: String,
        #[source]
        source: std::io::Error,
    },
    // 解析错误自带的信息会带出源文件里出错的那一行或那个值，里面可能是 key，
    // 所以只显示行列和原因（`redact::describe_*`）。
    #[error("JSON 解析错误: {path}: {}", crate::redact::describe_json_error(.source))]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("JSON 序列化失败: {source}")]
    JsonSerialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("TOML 解析错误: {path}: {detail}")]
    Toml {
        path: String,
        detail: String,
        // 装箱：不然这个变体会把整个 `AppError` 撑过 clippy 的 `result_large_err` 上限。
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("锁获取失败: {0}")]
    Lock(String),
    #[error("MCP 校验失败: {0}")]
    McpValidation(String),
    #[error("{0}")]
    Message(String),
    #[error("HTTP {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("{zh} ({en})")]
    Localized {
        key: &'static str,
        zh: String,
        en: String,
    },
    #[error("数据库错误: {0}")]
    Database(String),
    #[error("OMO 配置文件不存在")]
    OmoConfigNotFound,
    /// settings.json is there but neither it nor its backup parses, so nothing is
    /// saved over it. The message starts with the code the renderer matches on.
    #[error("settings_unreadable: {path} can't be read ({detail}) and has no good backup")]
    SettingsUnreadable { path: String, detail: String },
    #[error("所有供应商已熔断，无可用渠道")]
    AllProvidersCircuitOpen,
    #[error("未配置供应商")]
    NoProvidersConfigured,
}

impl AppError {
    pub fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().display().to_string(),
            source,
        }
    }

    pub fn json(path: impl AsRef<Path>, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.as_ref().display().to_string(),
            source,
        }
    }

    /// `text` 是解析失败的那份 TOML：算出错误的行列用，不会出现在错误信息里。
    pub fn toml(path: impl AsRef<Path>, text: &str, source: toml::de::Error) -> Self {
        Self::Toml {
            path: path.as_ref().display().to_string(),
            detail: crate::redact::describe_toml_de_error(text, &source),
            source: Box::new(source),
        }
    }

    pub fn localized(key: &'static str, zh: impl Into<String>, en: impl Into<String>) -> Self {
        Self::Localized {
            key,
            zh: zh.into(),
            en: en.into(),
        }
    }
}

impl<T> From<PoisonError<T>> for AppError {
    fn from(err: PoisonError<T>) -> Self {
        Self::Lock(err.to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Database(err.to_string())
    }
}

// 错误到 UI 的出口：任何变体里带着的 key（比如服务端把请求里的 key 回显在报错里）在这里抹掉。
impl From<AppError> for String {
    fn from(err: AppError) -> Self {
        crate::redact::scrub(&err.to_string()).into_owned()
    }
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&crate::redact::scrub(&self.to_string()))
    }
}

/// 格式化为 JSON 错误字符串，前端可解析为结构化错误
pub fn format_skill_error(
    code: &str,
    context: &[(&str, &str)],
    suggestion: Option<&str>,
) -> String {
    use serde_json::json;

    let mut ctx_map = serde_json::Map::new();
    for (key, value) in context {
        ctx_map.insert(key.to_string(), json!(value));
    }

    let error_obj = json!({
        "code": code,
        "context": ctx_map,
        "suggestion": suggestion,
    });

    serde_json::to_string(&error_obj).unwrap_or_else(|_| {
        // 如果 JSON 序列化失败，返回简单格式
        format!("ERROR:{code}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_error_display_never_echoes_the_source_line() {
        let text = "experimental_bearer_token = \"sk-of-abc123456789\" oops\n";
        let source = toml::from_str::<toml::Table>(text).unwrap_err();

        let error = AppError::toml("config.toml", text, source).to_string();

        assert!(error.contains("config.toml"), "{error}");
        assert!(error.contains("line 1, column"), "{error}");
        assert!(!error.contains("abc123456789"), "{error}");
    }

    #[test]
    fn json_error_display_never_echoes_the_value() {
        #[derive(Debug, serde::Deserialize)]
        struct Auth {
            #[allow(dead_code)]
            key: bool,
        }
        let source = serde_json::from_str::<Auth>(r#"{"key": "sk-of-abc123456789"}"#).unwrap_err();

        let error = AppError::json("auth.json", source).to_string();

        assert!(error.contains("auth.json"), "{error}");
        assert!(!error.contains("abc123456789"), "{error}");
    }

    #[test]
    fn errors_sent_to_the_ui_are_scrubbed() {
        let error = AppError::HttpStatus {
            status: 401,
            body: "bad key sk-of-abc123456789".into(),
        };

        let as_string = String::from(AppError::Message("Bearer abcdefgh12345678".into()));
        let serialized = serde_json::to_string(&error).unwrap();

        assert_eq!(as_string, "Bearer «redacted»");
        assert!(!serialized.contains("abc123456789"), "{serialized}");
    }
}
