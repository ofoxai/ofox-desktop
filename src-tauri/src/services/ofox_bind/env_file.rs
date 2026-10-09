//! `.env` 文件的按行编辑与还原：只动受管的那几行，注释和其它变量保持原样。
//! 解析规则与 `gemini_config::parse_env_file` 一致。

use super::plan::RestorePlan;

fn key_of(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (key, _) = line.split_once('=')?;
    let key = key.trim();
    (!key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_')).then_some(key)
}

/// 该变量最后一次出现的那一行（原样）。
pub(crate) fn find_line<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().rfind(|line| key_of(line) == Some(key))
}

pub(crate) fn get_value(text: &str, key: &str) -> Option<String> {
    find_line(text, key)
        .and_then(|line| line.split_once('='))
        .map(|(_, value)| value.trim().to_string())
}

/// 把 `key` 的那一行换成 `line`（在第一次出现的位置），删掉其余同名行；
/// 原来没有就追加到末尾。`line` 为 `None` 时删除该变量的所有行。
pub(crate) fn set_line(text: &str, key: &str, line: Option<&str>) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut placed = false;
    for existing in text.lines() {
        if key_of(existing) != Some(key) {
            out.push(existing);
        } else if !placed {
            placed = true;
            if let Some(line) = line {
                out.push(line);
            }
        }
    }
    if !placed {
        if let Some(line) = line {
            out.push(line);
        }
    }
    if out.iter().all(|line| line.trim().is_empty()) {
        return String::new();
    }
    let mut joined = out.join("\n");
    joined.push('\n');
    joined
}

pub(crate) fn set_value(text: &str, key: &str, value: Option<&str>) -> String {
    let line = value.map(|value| format!("{key}={value}"));
    set_line(text, key, line.as_deref())
}

/// 变量相同、注释和其它非变量行也相同。
fn same_env(a: &str, b: &str) -> bool {
    let others = |text: &str| -> Vec<String> {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && key_of(line).is_none())
            .map(str::to_string)
            .collect()
    };
    crate::gemini_config::parse_env_file(a) == crate::gemini_config::parse_env_file(b)
        && others(a) == others(b)
}

/// 受管变量一律还原成绑定前的那一行（原来没有就删掉）。
pub(crate) fn plan_restore(
    original: Option<&str>,
    current: Option<&str>,
    keys: &[&str],
) -> RestorePlan {
    let original_text = original.unwrap_or_default();
    let mut restored = current.unwrap_or_default().to_string();
    let mut restored_keys = Vec::new();
    let mut removed_keys = Vec::new();
    for key in keys {
        match find_line(original_text, key) {
            Some(line) => {
                restored = set_line(&restored, key, Some(line));
                restored_keys.push((*key).to_string());
            }
            None => {
                if find_line(&restored, key).is_some() {
                    restored = set_line(&restored, key, None);
                    removed_keys.push((*key).to_string());
                }
            }
        }
    }
    let same = same_env(&restored, original_text);
    RestorePlan::finish(restored, original, same, restored_keys, removed_keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: &[&str] = &["GEMINI_API_KEY", "GOOGLE_GEMINI_BASE_URL", "GEMINI_MODEL"];

    #[test]
    fn set_value_replaces_in_place_and_keeps_comments() {
        let text = "# my key\nGEMINI_API_KEY=AIza-mine\nOTHER=1\n";
        let edited = set_value(text, "GEMINI_API_KEY", Some("sk-of-x"));
        assert_eq!(edited, "# my key\nGEMINI_API_KEY=sk-of-x\nOTHER=1\n");
        assert_eq!(set_value("", "A", Some("1")), "A=1\n");
        assert_eq!(set_value("A=1\n", "A", None), "");
    }

    #[test]
    fn restore_puts_back_the_original_lines_byte_for_byte() {
        let original = "# my key\nGEMINI_API_KEY=\"AIza-mine\"\nOTHER=1\n";
        let mut current = set_value(original, "GEMINI_API_KEY", Some("sk-of-x"));
        current = set_value(
            &current,
            "GOOGLE_GEMINI_BASE_URL",
            Some("https://api.ofox.ai/gemini"),
        );
        current = set_value(&current, "GEMINI_MODEL", Some("gemini-x"));
        let plan = plan_restore(Some(original), Some(&current), KEYS);
        assert!(plan.exact);
        assert_eq!(plan.content.as_deref(), Some(original));
        assert_eq!(plan.restored_keys, ["GEMINI_API_KEY"]);
    }

    #[test]
    fn restore_keeps_lines_added_while_bound() {
        let current = "GEMINI_API_KEY=sk-of-x\n# added later\nDEBUG=1\n";
        let plan = plan_restore(None, Some(current), KEYS);
        assert_eq!(plan.content.as_deref(), Some("# added later\nDEBUG=1\n"));
        assert!(!plan.exact);
    }

    #[test]
    fn restore_deletes_an_env_file_bind_created() {
        let plan = plan_restore(None, Some("GEMINI_API_KEY=sk-of-x\n"), KEYS);
        assert_eq!(plan.content, None);
        assert!(plan.exact);
    }

    #[test]
    fn get_value_reads_the_last_definition() {
        assert_eq!(get_value("A=1\nA=2\n", "A").as_deref(), Some("2"));
        assert_eq!(get_value("# A=1\n", "A"), None);
    }
}
