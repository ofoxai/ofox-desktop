//! 各种配置格式共用的还原计划。

use std::path::{Path, PathBuf};

use crate::config::read_file_bytes;

/// 一个文件的还原计划。`content` 为 `None` 表示删除文件（绑定前不存在、还原后
/// 也没有别的内容）。
#[derive(Debug, PartialEq)]
pub(crate) struct RestorePlan {
    pub content: Option<String>,
    /// 还原结果与绑定前的文件语义相同，因此直接写回原文本（逐字节一致）。
    pub exact: bool,
    pub restored_keys: Vec<String>,
    pub removed_keys: Vec<String>,
}

impl RestorePlan {
    /// `same` 为真时写回原文本；否则写还原后的内容。
    pub(crate) fn finish(
        restored: String,
        original: Option<&str>,
        same: bool,
        restored_keys: Vec<String>,
        removed_keys: Vec<String>,
    ) -> Self {
        let (content, exact) = if same {
            (original.map(str::to_string), true)
        } else {
            (Some(restored), false)
        };
        Self {
            content,
            exact,
            restored_keys,
            removed_keys,
        }
    }
}

/// 没有绑定前快照时的尽力清理：对一个文件要做的改动。`content` 为 `None`
/// 表示删除文件。
#[derive(Debug, PartialEq)]
pub(crate) struct FileEdit {
    pub path: PathBuf,
    pub content: Option<String>,
    /// 改回默认值的字段。
    pub restored_keys: Vec<String>,
    pub removed_keys: Vec<String>,
}

pub(crate) fn read_text(path: &Path) -> Result<Option<String>, String> {
    read_file_bytes(path)
        .map_err(|e| e.to_string())?
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| format!("{} 不是 UTF-8 文本", path.display()))
        })
        .transpose()
}
