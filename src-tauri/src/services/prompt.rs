use indexmap::IndexMap;

use crate::app_config::AppType;
#[cfg(test)]
use crate::config::write_text_file;
use crate::error::AppError;
use crate::prompt::Prompt;
use crate::prompt_files::prompt_file_path;
use crate::store::AppState;

/// 安全地获取当前 Unix 时间戳
fn get_unix_timestamp() -> Result<i64, AppError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .map_err(|e| AppError::Message(format!("Failed to get system time: {e}")))
}

pub struct PromptService;

impl PromptService {
    pub fn get_prompts(
        state: &AppState,
        app: AppType,
    ) -> Result<IndexMap<String, Prompt>, AppError> {
        state.db.get_prompts(app.as_str())
    }

    pub fn upsert_prompt(
        state: &AppState,
        app: AppType,
        _id: &str,
        prompt: Prompt,
    ) -> Result<(), AppError> {
        let _io = crate::config_io::lock();
        if _id != prompt.id {
            return Err(AppError::InvalidInput("library.resourceMissing".into()));
        }
        if prompt.enabled {
            Self::apply_owned_prompt(state, &app, &prompt)?;
        } else {
            let resource_id = Self::resource_id(&app, &prompt.id);
            let snapshot = state.db.library_snapshot()?;
            if snapshot
                .machine
                .assignments
                .get(&format!("instructions:{}", app.as_str()))
                == Some(&resource_id)
                || snapshot.machine.receipts.values().any(|receipt| {
                    receipt.resource_id == resource_id
                        && receipt.owners.contains(&app.as_str().to_string())
                })
            {
                let plan = crate::services::library::coordinator::preview_at(
                    &state.db,
                    &resource_id,
                    &[],
                )?;
                let result = crate::services::library::apply_operation(&state.db, &plan.id)?;
                if result.status != "applied" {
                    return Err(AppError::Config("library.partial".into()));
                }
            }
        }
        state.db.save_prompt(app.as_str(), &prompt)?;
        Ok(())
    }

    fn resource_id(app: &AppType, id: &str) -> String {
        format!("legacy-prompt:{}:{id}", app.as_str())
    }

    fn apply_owned_prompt(
        state: &AppState,
        app: &AppType,
        prompt: &Prompt,
    ) -> Result<(), AppError> {
        let resource_id = Self::resource_id(app, &prompt.id);
        let snapshot = state.db.library_snapshot()?;
        let expected = snapshot
            .definitions
            .instructions
            .get(&resource_id)
            .map(|set| set.revision);
        crate::services::library::save_instruction(
            &state.db,
            crate::services::library::InstructionSet {
                id: resource_id.clone(),
                name: prompt.name.clone(),
                content: prompt.content.clone(),
                revision: 0,
            },
            expected,
        )?;
        let plan = crate::services::library::coordinator::preview_at(
            &state.db,
            &resource_id,
            &[(app.as_str().into(), prompt_file_path(app)?)],
        )?;
        let result = crate::services::library::apply_operation(&state.db, &plan.id)?;
        if result.status != "applied" {
            return Err(AppError::Config("library.partial".into()));
        }
        Ok(())
    }

    pub fn delete_prompt(state: &AppState, app: AppType, id: &str) -> Result<(), AppError> {
        let prompts = state.db.get_prompts(app.as_str())?;

        if let Some(prompt) = prompts.get(id) {
            if prompt.enabled {
                return Err(AppError::InvalidInput("无法删除已启用的提示词".to_string()));
            }
        }

        state.db.delete_prompt(app.as_str(), id)?;
        Ok(())
    }

    pub fn enable_prompt(state: &AppState, app: AppType, id: &str) -> Result<(), AppError> {
        let _io = crate::config_io::lock();
        let mut prompts = state.db.get_prompts(app.as_str())?;
        let selected = prompts
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::InvalidInput("library.resourceMissing".into()))?;
        Self::apply_owned_prompt(state, &app, &selected)?;
        for prompt in prompts.values_mut() {
            prompt.enabled = prompt.id == id;
            state.db.save_prompt(app.as_str(), prompt)?;
        }
        Ok(())
    }

    pub fn import_from_file(state: &AppState, app: AppType) -> Result<String, AppError> {
        let _io = crate::config_io::lock();
        let file_path = prompt_file_path(&app)?;

        if !file_path.exists() {
            return Err(AppError::Message("提示词文件不存在".to_string()));
        }

        let content =
            std::fs::read_to_string(&file_path).map_err(|e| AppError::io(&file_path, e))?;
        if content.contains("<!-- OFOX:") {
            return Err(AppError::Config("library.nativeContainsManaged".into()));
        }
        if let Some(existing) = state
            .db
            .get_prompts(app.as_str())?
            .values()
            .find(|prompt| prompt.content == content)
        {
            return Ok(existing.id.clone());
        }
        let timestamp = get_unix_timestamp()?;

        let id = format!("imported-{timestamp}");
        let prompt = Prompt {
            id: id.clone(),
            name: format!(
                "导入的提示词 {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M")
            ),
            content,
            description: Some("从现有配置文件导入".to_string()),
            enabled: false,
            created_at: Some(timestamp),
            updated_at: Some(timestamp),
        };

        Self::upsert_prompt(state, app, &id, prompt)?;
        Ok(id)
    }

    pub fn get_current_file_content(app: AppType) -> Result<Option<String>, AppError> {
        let file_path = prompt_file_path(&app)?;
        if !file_path.exists() {
            return Ok(None);
        }
        let content =
            std::fs::read_to_string(&file_path).map_err(|e| AppError::io(&file_path, e))?;
        Ok(Some(content))
    }

    /// 首次启动时从现有提示词文件自动导入（如果存在）
    /// 返回导入的数量
    pub fn import_from_file_on_first_launch(
        state: &AppState,
        app: AppType,
    ) -> Result<usize, AppError> {
        // 幂等性保护：该应用已有提示词则跳过
        let existing = state.db.get_prompts(app.as_str())?;
        if !existing.is_empty() {
            return Ok(0);
        }

        let file_path = prompt_file_path(&app)?;

        // 检查文件是否存在
        if !file_path.exists() {
            return Ok(0);
        }

        // 读取文件内容
        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("读取提示词文件失败: {file_path:?}, 错误: {e}");
                return Ok(0);
            }
        };

        // Startup import never creates an unusable draft or takes ownership.
        if content.contains("<!-- OFOX:") {
            log::info!("Skipped native instructions containing managed markers");
            return Ok(0);
        }

        // 检查内容是否为空
        if content.trim().is_empty() {
            return Ok(0);
        }

        log::info!("发现提示词文件，自动导入: {file_path:?}");

        // 创建提示词对象
        let timestamp = get_unix_timestamp()?;
        let id = format!("auto-imported-{timestamp}");
        let prompt = Prompt {
            id: id.clone(),
            name: format!(
                "Auto-imported Prompt {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M")
            ),
            content,
            description: Some("Automatically imported on first launch".to_string()),
            enabled: false, // Existing content is a draft, never ownership evidence.
            created_at: Some(timestamp),
            updated_at: Some(timestamp),
        };

        // 保存到数据库
        state.db.save_prompt(app.as_str(), &prompt)?;

        log::info!("自动导入完成: {}", app.as_str());
        Ok(1)
    }
}

#[cfg(test)]
mod library_regression_tests {
    use super::*;
    use serial_test::serial;
    use std::sync::Arc;

    #[test]
    #[serial]
    fn deep_link_enable_flags_import_a_draft_without_applying_native_config() {
        use base64::Engine;
        let _sandbox = crate::test_sandbox::sandbox();
        let state = AppState::new(Arc::new(crate::Database::memory().unwrap()));
        let path = prompt_file_path(&AppType::Codex).unwrap();
        write_text_file(&path, "personal\n").unwrap();
        let request = crate::deeplink::DeepLinkImportRequest {
            resource: "prompt".into(),
            app: Some("codex".into()),
            name: Some("Imported".into()),
            enabled: Some(true),
            content: Some(base64::engine::general_purpose::STANDARD.encode("imported shared")),
            ..Default::default()
        };
        let id = crate::deeplink::import_prompt_from_deeplink(&state, request).unwrap();
        assert!(!state.db.get_prompts("codex").unwrap()[&id].enabled);
        assert!(state
            .db
            .library_snapshot()
            .unwrap()
            .machine
            .assignments
            .is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "personal\n");
    }

    #[test]
    #[serial]
    fn importing_the_same_native_instructions_is_idempotent() {
        let _sandbox = crate::test_sandbox::sandbox();
        let state = AppState::new(Arc::new(crate::Database::memory().unwrap()));
        let path = prompt_file_path(&AppType::Codex).unwrap();
        write_text_file(&path, "personal\n").unwrap();
        let first = PromptService::import_from_file(&state, AppType::Codex).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1050));
        let second = PromptService::import_from_file(&state, AppType::Codex).unwrap();
        assert_eq!(first, second);
        assert_eq!(state.db.get_prompts("codex").unwrap().len(), 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "personal\n");
    }

    #[test]
    #[serial]
    fn withdrawing_applied_a_after_b_fails_preserves_desired_b() {
        let _sandbox = crate::test_sandbox::sandbox();
        let state = AppState::new(Arc::new(crate::Database::memory().unwrap()));
        let path = prompt_file_path(&AppType::Codex).unwrap();
        write_text_file(&path, &"personal\n".repeat(150_000)).unwrap();
        let a = draft();
        state.db.save_prompt("codex", &a).unwrap();
        PromptService::enable_prompt(&state, AppType::Codex, &a.id).unwrap();
        let b = Prompt {
            id: "b".into(),
            content: "b".repeat(1024 * 1024),
            ..draft()
        };
        state.db.save_prompt("codex", &b).unwrap();
        assert!(PromptService::enable_prompt(&state, AppType::Codex, &b.id).is_err());
        let desired_b = PromptService::resource_id(&AppType::Codex, "b");
        assert_eq!(
            state.db.library_snapshot().unwrap().machine.assignments["instructions:codex"],
            desired_b
        );
        PromptService::upsert_prompt(&state, AppType::Codex, &a.id, a.clone()).unwrap();
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("OFOX:BEGIN"));
        assert_eq!(
            state.db.library_snapshot().unwrap().machine.assignments["instructions:codex"],
            desired_b
        );
    }

    #[test]
    #[serial]
    fn native_import_with_managed_markers_refuses_an_unusable_draft() {
        let _sandbox = crate::test_sandbox::sandbox();
        let state = AppState::new(Arc::new(crate::Database::memory().unwrap()));
        let path = prompt_file_path(&AppType::Codex).unwrap();
        write_text_file(&path, "personal\n").unwrap();
        state.db.save_prompt("codex", &draft()).unwrap();
        PromptService::enable_prompt(&state, AppType::Codex, "fixture").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(PromptService::import_from_file(&state, AppType::Codex).is_err());
        assert_eq!(state.db.get_prompts("codex").unwrap().len(), 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    #[serial]
    fn startup_import_skips_managed_native_text() {
        let _sandbox = crate::test_sandbox::sandbox();
        let state = AppState::new(Arc::new(crate::Database::memory().unwrap()));
        let path = prompt_file_path(&AppType::Codex).unwrap();
        let text =
            "personal\n<!-- OFOX:BEGIN instructions -->\nshared\n<!-- OFOX:END instructions -->\n";
        write_text_file(&path, text).unwrap();
        assert_eq!(
            PromptService::import_from_file_on_first_launch(&state, AppType::Codex).unwrap(),
            0
        );
        assert!(state.db.get_prompts("codex").unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    fn draft() -> Prompt {
        Prompt {
            id: "fixture".into(),
            name: "Fixture".into(),
            content: "shared instructions".into(),
            description: None,
            enabled: false,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    #[serial]
    fn saving_an_unassigned_draft_never_clears_native_instructions() {
        let _sandbox = crate::test_sandbox::sandbox();
        let state = AppState::new(Arc::new(crate::Database::memory().unwrap()));
        let path = prompt_file_path(&AppType::Codex).unwrap();
        write_text_file(&path, "user-owned instructions\n").unwrap();
        PromptService::upsert_prompt(&state, AppType::Codex, "fixture", draft()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "user-owned instructions\n"
        );
    }

    #[test]
    #[serial]
    fn enabling_a_prompt_preserves_bom_crlf_and_user_content() {
        let _sandbox = crate::test_sandbox::sandbox();
        let state = AppState::new(Arc::new(crate::Database::memory().unwrap()));
        let path = prompt_file_path(&AppType::Codex).unwrap();
        write_text_file(&path, "\u{feff}user-owned\r\n").unwrap();
        state.db.save_prompt("codex", &draft()).unwrap();
        PromptService::enable_prompt(&state, AppType::Codex, "fixture").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("\u{feff}user-owned\r\n"));
        assert!(text.contains("<!-- OFOX:BEGIN instructions -->"));
        assert!(text.contains("shared instructions"));
    }
}
