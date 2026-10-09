//! 解绑（及预览）的结果，发给前端展示。只含文件和字段路径，不含任何值。

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnbindWarning {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnbindReport {
    pub tool: String,
    pub dry_run: bool,
    /// 旧版本绑定、没有绑定前快照：只做了尽力清理。
    pub legacy: bool,
    /// 没找到 Ofox 的配置，什么都没改。
    pub already_unbound: bool,
    /// 恢复成绑定前原样（逐字节）的文件。
    pub exact_files: Vec<String>,
    pub restored_keys: Vec<String>,
    pub removed_keys: Vec<String>,
    pub files_removed: Vec<String>,
    pub provider_restored_to: Option<String>,
    /// 共用配置的其它绑定方还在用 Ofox，所以配置没动。
    pub shared_kept_by: Vec<String>,
    pub warnings: Vec<UnbindWarning>,
}

impl UnbindReport {
    pub(crate) fn new(tool: &str, dry_run: bool) -> Self {
        Self {
            tool: tool.to_string(),
            dry_run,
            ..Self::default()
        }
    }

    pub(crate) fn warn(&mut self, code: &str, file: Option<String>) {
        self.warnings.push(UnbindWarning {
            code: code.to_string(),
            file,
        });
    }
}

/// 把绝对路径显示成 `~/...`，给前端看。
pub(crate) fn display_path(path: &std::path::Path) -> String {
    let home = crate::config::get_home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
