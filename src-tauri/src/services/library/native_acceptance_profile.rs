//! Test-only export of real library-writer profiles for native client acceptance.
use super::{batch, coordinator, mcp_resource, skill_resource, InstructionSet, ResourceKind};
use crate::{
    app_config::{InstalledSkill, McpServer, SkillApps},
    Database,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_json(path: &Path) -> Result<Value> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        return Err("invalid preparation input".into());
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn verify_tree(root: &Path, directory: &Path, remaining: &mut usize) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        *remaining = remaining
            .checked_sub(1)
            .ok_or("profile entry budget exceeded")?;
        let path = entry.path();
        if !fs::canonicalize(&path)?.starts_with(root) {
            return Err("profile path escapes sandbox".into());
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            verify_tree(root, &path, remaining)?;
        }
    }
    Ok(())
}
fn resource_scope(value: &Value) -> Result<bool> {
    let values = value.as_array().ok_or("resource kinds missing")?;
    let kinds: std::collections::BTreeSet<_> = values.iter().filter_map(Value::as_str).collect();
    if values.len() != kinds.len()
        || (kinds != ["mcp", "skills"].into_iter().collect()
            && kinds != ["instructions", "mcp", "skills"].into_iter().collect())
    {
        return Err("unsupported resource scope".into());
    }
    Ok(kinds.contains("instructions"))
}

#[test]
fn resource_scope_rejects_duplicates_nonstrings_and_incomplete_scopes() {
    assert!(!resource_scope(&json!(["mcp", "skills"])).unwrap());
    assert!(resource_scope(&json!(["instructions", "mcp", "skills"])).unwrap());
    for value in [
        json!(["mcp", "skills", "skills"]),
        json!(["mcp", "skills", null]),
        json!(["mcp"]),
        json!(["mcp", "skills", "other"]),
    ] {
        assert!(resource_scope(&value).is_err());
    }
}

fn write_profile(requested_root: &Path) -> Result<Value> {
    let root = fs::canonicalize(requested_root)?;
    let temporary = fs::canonicalize(std::env::temp_dir())?;
    if root == temporary || !root.starts_with(&temporary) {
        return Err("profile must be a prepared temporary directory".into());
    }
    verify_tree(&root, &root, &mut 10_000)?;
    let proof_path = root.join("fixture-private.json");
    let proof = read_json(&proof_path)?;
    let marker = read_json(&root.join(".ofox-acceptance-prepared.json"))?;
    let input = read_json(&root.join("writer-input.json"))?;
    let source = proof["sourceSha"].as_str().ok_or("source missing")?;
    let agent = proof["agentId"].as_str().ok_or("agent missing")?;
    let target = super::extension_targets::canonical(agent);
    let resource_kinds =
        input
            .get("resourceKinds")
            .cloned()
            .unwrap_or(json!(["instructions", "mcp", "skills"]));
    let with_instructions = resource_scope(&resource_kinds)?;
    if (with_instructions && !["claude", "codex", "gemini", "opencode", "hermes"].contains(&target))
        || (input.get("resourceKinds").is_some()
            && (proof["resourceKinds"] != resource_kinds
                || marker["resourceKinds"] != resource_kinds))
    {
        return Err("resource scope mismatch".into());
    }
    if source.len() != 40
        || !source.bytes().all(|b| b.is_ascii_hexdigit())
        || !super::extension_targets::CLIENTS.contains(&agent)
        || input["sourceSha"] != proof["sourceSha"]
        || input["agentId"] != proof["agentId"]
        || input["preparedRunId"] != proof["runId"]
        || marker["privateProofSha256"] != hash(&fs::read(&proof_path)?)
        || marker["resourcesMode"] != "writer"
        || marker["expiresAt"].as_i64().unwrap_or(0) <= chrono::Utc::now().timestamp_millis()
    {
        return Err("preparation evidence changed or expired".into());
    }
    // This function is test-only and called under the shared serial/sandbox guard.
    // Every environment variable changed below is restored by that same guard.
    let home = root.join("home");
    fs::create_dir_all(&home)?;
    for (name, path) in [
        ("HOME", home.clone()),
        ("USERPROFILE", home.clone()),
        ("CC_SWITCH_TEST_HOME", home.clone()),
        ("APPDATA", home.join("AppData/Roaming")),
        ("LOCALAPPDATA", home.join("AppData/Local")),
        ("XDG_CONFIG_HOME", home.join(".config")),
        ("XDG_DATA_HOME", home.join(".local/share")),
        ("XDG_CACHE_HOME", home.join(".cache")),
        ("XDG_STATE_HOME", home.join(".local/state")),
        ("CODEX_HOME", home.join(".codex")),
        ("CLAUDE_CONFIG_DIR", home.join(".claude")),
        ("GEMINI_CLI_HOME", home.clone()),
        ("HERMES_HOME", home.join(".hermes")),
        ("OPENCODE_CONFIG_DIR", home.join(".config/opencode")),
        ("OPENCLAW_STATE_DIR", home.join(".openclaw")),
        ("OPENCLAW_CONFIG_PATH", home.join(".openclaw/openclaw.json")),
        ("WORKBUDDY_CONFIG_DIR", home.join(".workbuddy")),
        ("PI_CODING_AGENT_DIR", home.join(".pi/agent")),
        ("DSH_HOME", home.join(".dsh")),
    ] {
        std::env::set_var(name, path);
    }
    crate::settings::reload_settings()?;
    let db = Database::memory()?;
    let mut operations = Vec::new();
    let mut files: Vec<(&str, PathBuf)> = vec![("proof", proof_path)];
    let instruction = if with_instructions {
        Some(super::save_instruction(
            &db,
            InstructionSet {
                id: "ofox-acceptance".into(),
                name: "OFox acceptance".into(),
                content: input["instructionText"]
                    .as_str()
                    .ok_or("instruction missing")?
                    .into(),
                revision: 0,
            },
            None,
        )?)
    } else {
        None
    };
    let staged = root.join("skill-source");
    fs::create_dir_all(&staged)?;
    crate::config::atomic_write(
        &staged.join("SKILL.md"),
        input["skillText"]
            .as_str()
            .ok_or("skill missing")?
            .as_bytes(),
    )?;
    skill_resource::save_draft(
        &db,
        InstalledSkill {
            id: "acceptance:ofox-acceptance".into(),
            name: "ofox-acceptance".into(),
            description: Some("Read-only native acceptance".into()),
            directory: "ofox-acceptance".into(),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            readme_url: None,
            apps: SkillApps::default(),
            installed_at: 0,
            content_hash: None,
            updated_at: 0,
        },
        &staged,
        None,
    )?;
    let mcp = input["mcp"].as_object().ok_or("MCP input missing")?;
    if mcp.len() != if with_instructions { 2 } else { 1 }
        || !mcp.contains_key("ofox_acceptance")
        || (with_instructions && !mcp.contains_key("microsoft_learn"))
    {
        return Err("unexpected MCP resources".into());
    }
    for (id, server) in mcp {
        super::save_mcp_draft(
            &db,
            McpServer {
                id: id.clone(),
                name: id.clone(),
                server: server.clone(),
                apps: Default::default(),
                description: None,
                homepage: None,
                docs: None,
                tags: vec![],
            },
            None,
        )?;
    }
    let mut resources = vec![
        (ResourceKind::Mcp, mcp.keys().cloned().collect::<Vec<_>>()),
        (
            ResourceKind::Skills,
            vec!["acceptance:ofox-acceptance".into()],
        ),
    ];
    if let Some(instruction) = instruction {
        resources.insert(0, (ResourceKind::Instructions, vec![instruction.id]));
    }
    for (kind, resources) in resources {
        let plan = batch::preview_batch(
            &db,
            kind.clone(),
            resources
                .into_iter()
                .map(|resource_id| batch::BatchResource {
                    resource_id,
                    expected_revision: 1,
                    add_tools: vec![],
                    withdraw_tools: vec![],
                })
                .collect(),
            vec![target.into()],
            batch::BatchAction::Apply,
            db.library_snapshot()?.machine.epoch,
        )?;
        for group in plan["groups"].as_array().ok_or("preview groups missing")? {
            let path = Path::new(group["path"].as_str().ok_or("target missing")?);
            if !coordinator::physical(path)?.starts_with(&root) {
                return Err("writer resolved outside prepared sandbox".into());
            }
        }
        let operation_id = plan["operationId"].as_str().ok_or("operation missing")?;
        let result = batch::run_batch(&db, operation_id)?;
        if result["status"] != "applied" {
            return Err("native resource writer did not apply".into());
        }
        operations.push(json!({"kind":kind,"operationId":operation_id,"result":"applied","receiptSha256":hash(&serde_json::to_vec(&result)?)}));
    }
    if with_instructions {
        files.push((
            "instructions",
            coordinator::native_instruction_paths()?[target].clone(),
        ));
    }
    let mcp_path = mcp_resource::native_paths()?[target].clone();
    files.push(("mcp", mcp_path.clone()));
    files.push((
        "skills",
        skill_resource::native_paths()?[target].join("ofox-acceptance/SKILL.md"),
    ));
    if agent == "claude" {
        // --bare reads only this explicit MCP input. Preserve the actual writer's
        // native definitions; do not substitute a separately rendered template.
        let native: Value = serde_json::from_slice(&fs::read(mcp_path)?)?;
        let explicit = root.join("mcp.json");
        crate::config::atomic_write(
            &explicit,
            &serde_json::to_vec(&json!({"mcpServers":native["mcpServers"]}))?,
        )?;
        files.push(("mcp", explicit));
    }
    let evidence: Vec<Value> = files.into_iter().map(|(kind, path)| -> Result<Value> {
        if !fs::canonicalize(&path)?.starts_with(&root) { return Err("exported resource escapes sandbox".into()); }
        Ok(json!({"kind":kind,"relativePath":path.strip_prefix(&root)?.to_string_lossy().replace('\\',"/"),"sha256":hash(&fs::read(path)?)}))
    }).collect::<Result<_>>()?;
    let receipt = json!({"schemaVersion":1,"writer":"ofoxSandboxWriterV1","sourceSha":source,
        "compiledSourceSha":option_env!("OFOX_BUILD_SOURCE_SHA").unwrap_or("unknown"),
        "compiledSourceClean":option_env!("OFOX_BUILD_SOURCE_CLEAN") == Some("true"),
        "preparedRunId":proof["runId"],"root":root,"agentId":agent,"resourceKinds":resource_kinds,"operations":operations,"files":evidence});
    crate::config::atomic_write(
        &root.join("writer-receipt.json"),
        &serde_json::to_vec_pretty(&receipt)?,
    )?;
    Ok(receipt)
}

#[test]
#[serial_test::serial]
fn native_acceptance_profile_uses_three_real_writers() {
    let mut profile_roots = Vec::new();
    let legacy = ["claude", "codex", "gemini", "opencode", "hermes"]
        .into_iter()
        .map(|agent| (agent, true));
    let resources_only = super::extension_targets::CLIENTS
        .iter()
        .map(|agent| (*agent, false));
    for (agent, with_instructions) in legacy.chain(resources_only) {
        let kinds = if with_instructions {
            json!(["instructions", "mcp", "skills"])
        } else {
            json!(["mcp", "skills"])
        };
        let root = tempfile::Builder::new()
            .prefix("ofox-native-profile-")
            .tempdir()
            .unwrap();
        // Restore settings/environment before TempDir removes the profile. The
        // sandbox reset writes settings through the currently selected test home.
        let _sandbox = crate::test_sandbox::sandbox();
        profile_roots.push(root.path().to_path_buf());
        std::fs::write(root.path().join("fixture-private.json"), json!({
        "schemaVersion":1,"runId":"11111111-1111-4111-8111-111111111111", "sourceSha":"a".repeat(40),
        "agentId":agent, "resourceKinds":kinds, "nonce":"b".repeat(64), "instructionChallenge":"c".repeat(64),
        "skillChallenge":"d".repeat(64), "receipt":root.path().join("mcp-receipt.json")
    }).to_string()).unwrap();
        let proof = fs::read(root.path().join("fixture-private.json")).unwrap();
        fs::write(root.path().join(".ofox-acceptance-prepared.json"),json!({"privateProofSha256":hash(&proof),"resourcesMode":"writer","resourceKinds":kinds,"expiresAt":chrono::Utc::now().timestamp_millis()+60_000}).to_string()).unwrap();
        fs::write(root.path().join("writer-input.json"),json!({"schemaVersion":1,"sourceSha":"a".repeat(40),"agentId":agent,"resourceKinds":kinds,"preparedRunId":"11111111-1111-4111-8111-111111111111","instructionText":"Acceptance instruction", "skillText":"---\nname: ofox-acceptance\ndescription: Read-only acceptance\n---\nRead only.","mcp":{"ofox_acceptance":{"command":"fixture-node","args":["fixture-server"]},"microsoft_learn":{"type":"http","url":"https://learn.microsoft.com/api/mcp"}}}).to_string()).unwrap();
        if !with_instructions {
            let mut input = read_json(&root.path().join("writer-input.json")).unwrap();
            input["mcp"]
                .as_object_mut()
                .unwrap()
                .remove("microsoft_learn");
            fs::write(root.path().join("writer-input.json"), input.to_string()).unwrap();
        }
        let receipt = write_profile(root.path()).unwrap();
        assert_eq!(receipt["writer"], "ofoxSandboxWriterV1");
        assert_eq!(
            receipt["operations"].as_array().unwrap().len(),
            if with_instructions { 3 } else { 2 }
        );
        assert!(receipt["operations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|op| op["result"] == "applied"));
        assert_eq!(receipt["agentId"], agent);
        for kind in kinds.as_array().unwrap().iter().filter_map(Value::as_str) {
            assert!(
                receipt["files"].as_array().unwrap().iter().any(|entry| {
                    entry["kind"] == kind
                        && root
                            .path()
                            .join(entry["relativePath"].as_str().unwrap())
                            .is_file()
                }),
                "{agent}: missing {kind}"
            );
        }
        // Rejected preparation cannot create another batch or change a target file.
        let receipt_bytes = fs::read(root.path().join("writer-receipt.json")).unwrap();
        let mut marker = read_json(&root.path().join(".ofox-acceptance-prepared.json")).unwrap();
        marker["expiresAt"] = json!(0);
        fs::write(
            root.path().join(".ofox-acceptance-prepared.json"),
            marker.to_string(),
        )
        .unwrap();
        assert!(write_profile(root.path())
            .unwrap_err()
            .to_string()
            .contains("expired"));
        assert_eq!(
            fs::read(root.path().join("writer-receipt.json")).unwrap(),
            receipt_bytes
        );
        for entry in receipt["files"].as_array().unwrap() {
            assert_eq!(
                hash(&fs::read(root.path().join(entry["relativePath"].as_str().unwrap())).unwrap()),
                entry["sha256"]
            );
        }
    }
    assert!(
        profile_roots.iter().all(|root| !root.exists()),
        "profile sandboxes must be removed after restoring the process environment"
    );
}

#[test]
#[ignore = "Explicit offline profile export for native acceptance; requires a prepared temporary root"]
#[serial_test::serial]
fn export_native_acceptance_profile() {
    let _sandbox = crate::test_sandbox::sandbox();
    let root = std::env::var_os("OFOX_NATIVE_PROFILE_ROOT").expect("prepared root required");
    write_profile(Path::new(&root)).expect("export actual resource writer receipt");
}
