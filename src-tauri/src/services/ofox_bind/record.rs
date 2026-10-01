//! `ofox_bind_snapshot.record` 的格式。
//!
//! 新格式是带 `v` / `kind` 的信封。从旧表搬来的行（Ofox 字段补丁、
//! OpenClaw/Hermes 的 `__ofoxDirectBackupVersion` 外层、WorkBuddy 状态）没有
//! 这两个字段，读出来归为 [`StoredRecord::Legacy`]。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const RECORD_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RecordKind {
    /// 绑定前拍下的快照，解绑时精确还原。
    Snapshot,
    /// 第一次见到时磁盘上已经是 Ofox 的配置（旧版本绑定的），原值无从得知；
    /// 解绑时只能尽力清理。记下它是为了防止之后把 Ofox 自己的值当成原值。
    LegacyAdopted,
}

/// 绑定前的当前服务商。settings 和 DB 各记一份，两边可能不一致。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreviousProvider {
    pub settings: Option<String>,
    pub db: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ManagedFile {
    CodexConfig,
    ClaudeSettings,
    GeminiEnv,
    GeminiSettings,
    OpenCodeConfig,
    OpenClawConfig,
    HermesConfig,
}

/// 一个受管文件在绑定前的样子。`original` 是原文件的完整文本：受管字段的原值
/// 在还原时从这里现场解析，所以之后才写进去的字段（比如模型）也在还原范围内。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileBaseline {
    pub file: ManagedFile,
    /// 拍快照时的绝对路径；还原写回这里（之后改了配置目录也不会写错地方）。
    pub path: String,
    pub existed: bool,
    pub dir_existed: bool,
    #[serde(default)]
    pub mode: Option<u32>,
    #[serde(default)]
    pub original: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BindEnvelope {
    pub v: u32,
    pub kind: RecordKind,
    /// 共用这份配置的绑定方：codex 这一行可能是 `{codex, chatgpt}`。
    #[serde(default)]
    pub holders: BTreeSet<String>,
    #[serde(default)]
    pub previous_provider: PreviousProvider,
    #[serde(default)]
    pub files: Vec<FileBaseline>,
}

impl BindEnvelope {
    pub(crate) fn snapshot(
        holder: &str,
        previous_provider: PreviousProvider,
        files: Vec<FileBaseline>,
    ) -> Self {
        Self {
            v: RECORD_VERSION,
            kind: RecordKind::Snapshot,
            holders: BTreeSet::from([holder.to_string()]),
            previous_provider,
            files,
        }
    }

    pub(crate) fn legacy_adopted(holder: &str) -> Self {
        Self {
            v: RECORD_VERSION,
            kind: RecordKind::LegacyAdopted,
            holders: BTreeSet::from([holder.to_string()]),
            previous_provider: PreviousProvider::default(),
            files: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StoredRecord {
    Envelope(BindEnvelope),
    /// 旧格式：没有绑定前快照，只能尽力清理。
    Legacy(Value),
}

pub(crate) fn parse_record(text: &str) -> Result<StoredRecord, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("绑定记录不是有效 JSON：{e}"))?;
    let is_envelope = value.get("v").is_some() && value.get("kind").is_some();
    if !is_envelope {
        return Ok(StoredRecord::Legacy(value));
    }
    let envelope: BindEnvelope =
        serde_json::from_value(value).map_err(|e| format!("绑定记录格式无效：{e}"))?;
    if envelope.v != RECORD_VERSION {
        return Err(format!("不支持的绑定记录版本：{}", envelope.v));
    }
    Ok(StoredRecord::Envelope(envelope))
}

pub(crate) fn serialize_record(envelope: &BindEnvelope) -> Result<String, String> {
    serde_json::to_string(envelope).map_err(|e| format!("序列化绑定记录失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trips() {
        let envelope = BindEnvelope::snapshot(
            "codex",
            PreviousProvider {
                settings: Some("default".into()),
                db: None,
            },
            vec![FileBaseline {
                file: ManagedFile::CodexConfig,
                path: "/home/u/.codex/config.toml".into(),
                existed: true,
                dir_existed: true,
                mode: Some(0o644),
                original: Some("model = \"gpt-5\"\n".into()),
            }],
        );
        let text = serialize_record(&envelope).unwrap();
        assert_eq!(
            parse_record(&text).unwrap(),
            StoredRecord::Envelope(envelope)
        );
    }

    #[test]
    fn rows_without_version_and_kind_are_legacy() {
        for text in [
            r#"{"auth":{"OPENAI_API_KEY":"sk-of-x"},"config":""}"#,
            r#"{"__ofoxDirectBackupVersion":1,"patch":{}}"#,
            r#"{"version":2,"entries":[]}"#,
        ] {
            assert!(matches!(
                parse_record(text).unwrap(),
                StoredRecord::Legacy(_)
            ));
        }
    }

    #[test]
    fn unknown_record_version_is_rejected() {
        assert!(parse_record(r#"{"v":9,"kind":"snapshot"}"#).is_err());
    }
}
