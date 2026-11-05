//! Host-owned discovery identities and explicit, once-only skill approvals.
use super::{
    host::{blocking, AssistantHost, RunControl},
    tools::{approval::Choice, registry},
};
use crate::{
    app_config::InstalledSkill,
    services::{
        library::assistant_skill_transaction as transaction,
        skill::{SkillRepo, SkillService},
    },
    Database,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillResult {
    pub result_id: String,
    pub name: String,
    pub source: String,
    pub description: String,
    pub installs: Option<u64>,
    pub reviewed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<bool>,
}
#[derive(Clone)]
pub struct Candidate {
    pub result: SkillResult,
    pub skill_id: String,
    pub directory: Option<String>,
}
#[derive(Default)]
pub struct State {
    pub epoch: u64,
    pub reply_id: String,
    pub closed: bool,
    pub saved: BTreeMap<String, transaction::Success>,
    pub results: BTreeMap<String, Candidate>,
    pub installed: BTreeMap<String, transaction::Success>,
}
pub fn is_tool(name: &str) -> bool {
    matches!(name, "find_skills" | "install_skill" | "save_skill")
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Find {
    query: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Install {
    result_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    name: String,
    description: String,
    body: String,
}
pub fn validate(name: &str, args: &Value) -> Result<(), String> {
    match name {
        "find_skills" => {
            let a: Find =
                serde_json::from_value(args.clone()).map_err(|_| "invalid_skill_query")?;
            if a.query.trim().chars().count() < 2 || a.query.len() > 512 {
                return Err("invalid_skill_query".into());
            }
        }
        "install_skill" => {
            let a: Install = serde_json::from_value(args.clone()).map_err(|_| "not_in_results")?;
            if a.result_id.is_empty() || a.result_id.len() > 200 {
                return Err("not_in_results".into());
            }
        }
        "save_skill" => {
            let a: Save = serde_json::from_value(args.clone()).map_err(|_| "invalid_skill")?;
            transaction::validate_save(&a.name, &a.description, &a.body)
                .map_err(|e| e.to_string())?;
        }
        _ => return Err("assistant_unknown_tool".into()),
    }
    Ok(())
}

pub async fn search(query: &str) -> Result<Vec<Candidate>, String> {
    let remote = SkillService::search_skills_sh(query, 8, 0).await;
    merge_search(query, remote.ok().map(|r| r.skills))
}
fn merge_search(
    query: &str,
    remote: Option<Vec<crate::services::skill::SkillsShDiscoverableSkill>>,
) -> Result<Vec<Candidate>, String> {
    let popular: Value = serde_json::from_str(include_str!(
        "../../../../src/components/library/popularSkills.json"
    ))
    .map_err(|_| "skill_catalog_unavailable")?;
    let recommended: Value = serde_json::from_str(include_str!(
        "../../../../src/components/library/recommendedSkills.json"
    ))
    .map_err(|_| "skill_catalog_unavailable")?;
    let mut entries = BTreeMap::<String, Candidate>::new();
    let query = query.trim().to_lowercase();
    for entry in popular["skills"].as_array().into_iter().flatten() {
        let name = entry["skillId"].as_str().unwrap_or_default();
        let source = entry["source"].as_str().unwrap_or_default();
        let description = entry["description"].as_str().unwrap_or_default();
        if !format!("{name} {source} {description}")
            .to_lowercase()
            .contains(&query)
        {
            continue;
        }
        entries.insert(
            format!("{source}:{name}"),
            Candidate {
                result: SkillResult {
                    result_id: super::types::id(),
                    name: name.into(),
                    source: source.into(),
                    description: description.chars().take(500).collect(),
                    installs: entry["installs"].as_u64(),
                    reviewed: entry["official"] == true,
                    installed: None,
                },
                skill_id: name.into(),
                directory: None,
            },
        );
    }
    for entry in recommended.as_array().into_iter().flatten() {
        let name = entry["id"].as_str().unwrap_or_default();
        let source = format!(
            "{}/{}",
            entry["owner"].as_str().unwrap_or_default(),
            entry["repo"].as_str().unwrap_or_default()
        );
        if !format!("{name} {source}").to_lowercase().contains(&query) {
            continue;
        }
        let key = format!("{source}:{name}");
        let old = entries.remove(&key);
        entries.insert(
            key,
            Candidate {
                result: SkillResult {
                    result_id: super::types::id(),
                    name: name.into(),
                    source,
                    description: old
                        .as_ref()
                        .map(|o| o.result.description.clone())
                        .unwrap_or_else(|| name.replace('-', " ")),
                    installs: old.and_then(|o| o.result.installs),
                    reviewed: true,
                    installed: None,
                },
                skill_id: name.into(),
                directory: entry["directory"].as_str().map(str::to_owned),
            },
        );
    }
    if let Some(remote) = remote {
        for entry in remote {
            let source = format!("{}/{}", entry.repo_owner, entry.repo_name);
            let key = format!("{source}:{}", entry.directory);
            entries.entry(key).or_insert(Candidate {
                result: SkillResult {
                    result_id: super::types::id(),
                    name: entry.name.chars().take(128).collect(),
                    source,
                    description: String::new(),
                    installs: Some(entry.installs),
                    reviewed: false,
                    installed: None,
                },
                skill_id: entry.directory,
                directory: None,
            });
        }
    }
    let mut entries: Vec<_> = entries.into_values().collect();
    entries.sort_by(|a, b| {
        b.result
            .reviewed
            .cmp(&a.result.reviewed)
            .then(b.result.installs.cmp(&a.result.installs))
            .then(a.result.source.cmp(&b.result.source))
            .then(a.skill_id.cmp(&b.skill_id))
    });
    entries.truncate(8);
    Ok(entries)
}

pub async fn prepare_install(
    db: Arc<Database>,
    candidate: Candidate,
) -> Result<transaction::Prepared, String> {
    let parts = candidate
        .result
        .source
        .split_once('/')
        .ok_or("source_ambiguous")?;
    let service = SkillService::new();
    let repo = SkillRepo {
        owner: parts.0.into(),
        name: parts.1.into(),
        branch: "HEAD".into(),
        enabled: true,
    };
    // Library-owned fixed versions take precedence over a new HEAD. Exact source only.
    if let Some(prepared) = transaction::prepare_existing(
        &db,
        &candidate.result.source,
        &candidate.skill_id,
        candidate.directory.as_deref(),
        serde_json::to_value(&candidate.result).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?
    {
        return Ok(prepared);
    }
    let choices = service
        .discover_assistant_repository(&repo)
        .await
        .map_err(|e| e.to_string())?;
    let selected = select_source(choices, &candidate)?;
    let directory = selected.key.split_once(':').ok_or("source_ambiguous")?.1;
    let url = url::Url::parse(selected.readme_url.as_deref().ok_or("source_ambiguous")?)
        .map_err(|_| "source_ambiguous")?;
    let commit = url
        .path_segments()
        .and_then(|mut p| p.nth(3))
        .filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("source_ambiguous")?
        .to_owned();
    let fixed = SkillRepo {
        branch: commit.clone(),
        ..repo
    };
    let (download, sha) = service
        .download_selected_repo(&fixed, directory)
        .await
        .map_err(|e| e.to_string())?;
    if sha != commit {
        return Err("source_changed".into());
    }
    let row = InstalledSkill {
        id: selected.key.clone(),
        name: selected.name.clone(),
        description: Some(selected.description.clone()),
        directory: selected.name.clone(),
        repo_owner: Some(fixed.owner.clone()),
        repo_name: Some(fixed.name.clone()),
        repo_branch: Some("HEAD".into()),
        readme_url: selected.readme_url.clone(),
        apps: Default::default(),
        installed_at: chrono::Utc::now().timestamp(),
        content_hash: None,
        updated_at: 0,
    };
    let mut details = serde_json::to_value(candidate.result).map_err(|e| e.to_string())?;
    details["directory"] = json!(directory);
    details["commitSha"] = json!(commit);
    transaction::prepare_download(&db, row, download, directory, details).map_err(|e| e.to_string())
}

fn select_source(
    choices: Vec<crate::services::skill::DiscoverableSkill>,
    candidate: &Candidate,
) -> Result<crate::services::skill::DiscoverableSkill, String> {
    let mut matching = choices.into_iter().filter(|s| {
        s.name == candidate.skill_id
            && candidate
                .directory
                .as_ref()
                .is_none_or(|path| s.key.split_once(':').is_some_and(|(_, p)| p == path))
    });
    let selected = matching.next().ok_or("source_ambiguous")?;
    if matching.next().is_some() {
        return Err("source_ambiguous".into());
    }
    Ok(selected)
}

impl AssistantHost {
    pub(super) fn check_skill_result(&self, run: &RunControl, args: &Value) -> Result<(), String> {
        let state = run
            .skill_tools
            .lock()
            .map_err(|_| "assistant_state_unavailable")?;
        if args["resultId"]
            .as_str()
            .is_none_or(|id| !state.results.contains_key(id))
        {
            return Err("not_in_results".into());
        }
        Ok(())
    }
    pub(super) async fn dispatch_skill_tool(
        &self,
        run: &Arc<RunControl>,
        call: &registry::Call,
    ) -> Result<Value, String> {
        match call.name.as_str() {
            "find_skills" => {
                let epoch = run
                    .skill_tools
                    .lock()
                    .map_err(|_| "assistant_state_unavailable")?
                    .epoch;
                let query = call.args["query"].as_str().ok_or("invalid_skill_query")?;
                let entries = self.env.skill_search(query).await?;
                self.validate_run(run)?;
                if self.identity().await? != run.identity {
                    return Err("assistant_identity_changed".into());
                }
                let results = {
                    let mut state = run
                        .skill_tools
                        .lock()
                        .map_err(|_| "assistant_state_unavailable")?;
                    if state.epoch != epoch || state.reply_id != run.reply_id() {
                        return Err("not_in_results".into());
                    }
                    state.results = entries
                        .into_iter()
                        .take(8)
                        .map(|c| (c.result.result_id.clone(), c))
                        .collect();
                    state
                        .results
                        .values()
                        .map(|c| c.result.clone())
                        .collect::<Vec<_>>()
                };
                let results_value = serde_json::to_value(&results).map_err(|e| e.to_string())?;
                self.save_skill_results(run, &call.tool_call_id, results_value.clone())
                    .await?;
                Ok(json!({"results":results_value}))
            }
            "install_skill" | "save_skill" => {
                let install = call.name == "install_skill";
                let save_key = serde_json::to_string(&call.args).map_err(|_| "invalid_skill")?;
                let reply = run.reply_id();
                let (candidate, epoch, already) = {
                    let state = run
                        .skill_tools
                        .lock()
                        .map_err(|_| "assistant_state_unavailable")?;
                    let id = call.args["resultId"].as_str().unwrap_or_default();
                    if install && !state.results.contains_key(id) {
                        return Err("not_in_results".into());
                    }
                    (
                        state.results.get(id).cloned(),
                        state.epoch,
                        if install {
                            state.installed.get(id).cloned()
                        } else {
                            state.saved.get(&save_key).cloned()
                        },
                    )
                };
                if let Some(success) = already {
                    if install {
                        let _ = self
                            .mark_skill_installed(
                                run,
                                call.args["resultId"].as_str().unwrap_or_default(),
                                &reply,
                            )
                            .await;
                        return Ok(self.skill_committed_result(run, &success, true, true).await);
                    }
                    let db = self.env.skill_database()?;
                    if transaction::verify_success(&db, &success).is_ok() {
                        let result = self
                            .skill_committed_result(run, &success, false, true)
                            .await;
                        if result["code"] != "skill_refresh_failed" {
                            run.skill_tools
                                .lock()
                                .map_err(|_| "assistant_state_unavailable")?
                                .saved
                                .remove(&save_key);
                        }
                        return Ok(result);
                    }
                    // A superseded receipt is no longer an outstanding recovery.
                    // The new request still needs current ownership checks and a new click.
                    run.skill_tools
                        .lock()
                        .map_err(|_| "assistant_state_unavailable")?
                        .saved
                        .remove(&save_key);
                }
                let db = self.env.skill_database()?;
                let prepared = if let Some(candidate) = candidate {
                    self.env.skill_prepare(db, candidate).await?
                } else {
                    let a: Save =
                        serde_json::from_value(call.args.clone()).map_err(|_| "invalid_skill")?;
                    blocking(move || {
                        transaction::prepare_save(&db, &a.name, &a.description, &a.body)
                            .map_err(|e| e.to_string())
                    })
                    .await?
                };
                self.validate_run(run)?;
                if run.reply_id() != reply {
                    return Err("not_in_results".into());
                }
                if install {
                    self.check_skill_result(run, &call.args)?;
                    if run
                        .skill_tools
                        .lock()
                        .map_err(|_| "assistant_state_unavailable")?
                        .epoch
                        != epoch
                    {
                        return Err("not_in_results".into());
                    }
                }
                if !self
                    .approve_skill(
                        run,
                        &call.tool_call_id,
                        &call.name,
                        prepared.details.clone(),
                    )
                    .await?
                {
                    return Ok(if install {
                        json!({"installed":false})
                    } else {
                        json!({"saved":false})
                    });
                }
                self.validate_run(run)?;
                if self.identity().await? != run.identity {
                    return Err("assistant_identity_changed".into());
                }
                if install {
                    self.check_skill_result(run, &call.args)?;
                    if run
                        .skill_tools
                        .lock()
                        .map_err(|_| "assistant_state_unavailable")?
                        .epoch
                        != epoch
                    {
                        return Err("not_in_results".into());
                    }
                }
                let db = self.env.skill_database()?;
                let control = run.clone();
                let commit_reply = reply.clone();
                let success = blocking(move || {
                    transaction::commit(&db, &prepared, || {
                        let state = control.skill_tools.lock().map_err(|_| {
                            crate::error::AppError::Message("assistant_state_unavailable".into())
                        })?;
                        if state.closed
                            || control.reply_id() != commit_reply
                            || state.reply_id != commit_reply
                            || (install && state.epoch != epoch)
                            || control.cancelled()
                            || tokio::time::Instant::now() >= control.deadline
                        {
                            Err(crate::error::AppError::Message(
                                "assistant_cancelled".into(),
                            ))
                        } else {
                            Ok(())
                        }
                    })
                    .map_err(|e| e.to_string())
                })
                .await?;
                let mut result = self
                    .skill_committed_result(run, &success, install, false)
                    .await;
                {
                    let mut state = run
                        .skill_tools
                        .lock()
                        .map_err(|_| "assistant_state_unavailable")?;
                    if !state.closed && state.reply_id == reply {
                        if install && state.epoch == epoch {
                            state.installed.insert(
                                call.args["resultId"].as_str().unwrap_or_default().into(),
                                success.clone(),
                            );
                        }
                        if !install && result["code"] == "skill_refresh_failed" {
                            state.saved.insert(save_key, success);
                        }
                    }
                }
                if install
                    && self
                        .mark_skill_installed(
                            run,
                            call.args["resultId"].as_str().unwrap_or_default(),
                            &reply,
                        )
                        .await
                        .is_err()
                {
                    result["cardPersisted"] = json!(false);
                }
                Ok(result)
            }
            _ => Err("assistant_unknown_tool".into()),
        }
    }
    async fn skill_committed_result(
        &self,
        run: &Arc<RunControl>,
        success: &transaction::Success,
        install: bool,
        already: bool,
    ) -> Value {
        let name = success.name.as_str();
        let mut result = if install {
            json!({"installed":true,"name":name,"already":already,"usable":false})
        } else {
            json!({"saved":true,"name":name,"usable":false})
        };
        let verified = self
            .env
            .skill_database()
            .and_then(|db| transaction::verify_success(&db, success).map_err(|e| e.to_string()));
        if verified.is_err() {
            result["code"] = json!("skill_refresh_failed");
            return result;
        }
        match self.refresh_run_skills(run).await {
            Ok(refresh) => {
                result["usable"] = json!(refresh["skills"]
                    .as_array()
                    .is_some_and(|s| s.iter().any(|s| s["name"] == name && s["mode"] == "auto")));
                result["refresh"] = refresh;
            }
            Err(_) => {
                result["code"] = json!("skill_refresh_failed");
            }
        }
        result
    }
    async fn approve_skill(
        &self,
        run: &Arc<RunControl>,
        call_id: &str,
        name: &str,
        details: Value,
    ) -> Result<bool, String> {
        self.validate_run(run)?;
        let binding = self.local_binding(run);
        let (id, pending) = self.local_approvals.request_once(binding.clone())?;
        let display = json!({"approvalId":id,"toolCallId":call_id,"name":name,"category":if name=="install_skill"{"skillInstall"}else{"skillSave"},"details":details,"allowSession":false,"expectedOwner":binding.owner,"expectedGeneration":binding.generation});
        self.local_approvals.describe(&id, display.clone());
        self.emit(run, "approval", display);
        let timeout = run
            .deadline
            .saturating_duration_since(tokio::time::Instant::now())
            .min(Duration::from_secs(120));
        let result = self
            .local_approvals
            .wait(&id, pending, run.cancel.subscribe(), timeout)
            .await;
        self.emit(run, "approval_end", json!({"approvalId":id}));
        if !matches!(result, Ok(Choice::Once)) {
            return Ok(false);
        }
        self.validate_run(run)?;
        self.ensure_current(&run.identity, binding.generation)
            .await?;
        Ok(true)
    }
    async fn mark_skill_installed(
        &self,
        run: &RunControl,
        result_id: &str,
        message_id: &str,
    ) -> Result<(), String> {
        let store = self.store(&run.identity.owner)?;
        let sid = run.session_id.clone();
        let mid = message_id.to_owned();
        let id = result_id.to_owned();
        let message_id = mid.clone();
        let group = blocking(move || {
            store.mutate(&sid, |record| {
                let Some(message) = record.messages.iter_mut().find(|m| m.id == message_id) else {
                    return Ok(None);
                };
                for group in &mut message.skill_results {
                    if let Some(result) = group["results"]
                        .as_array_mut()
                        .and_then(|rs| rs.iter_mut().find(|r| r["resultId"] == id))
                    {
                        if result["installed"] == true {
                            return Ok(None);
                        }
                        result["installed"] = json!(true);
                        return Ok(Some(group.clone()));
                    }
                }
                Ok(None)
            })
        })
        .await?;
        if let Some(group) = group {
            self.emit(run, "skill_results", json!({"messageId":mid,"group":group}));
        }
        Ok(())
    }
    async fn save_skill_results(
        &self,
        run: &RunControl,
        call_id: &str,
        results: Value,
    ) -> Result<(), String> {
        let store = self.store(&run.identity.owner)?;
        let sid = run.session_id.clone();
        let mid = run.reply_id();
        let saved = json!({"toolCallId":call_id,"results":results});
        let copy = saved.clone();
        let id = mid.clone();
        blocking(move || {
            store.mutate(&sid, |record| {
                let message = record
                    .messages
                    .iter_mut()
                    .find(|m| m.id == id && m.role == "assistant")
                    .ok_or("assistant_state_unavailable")?;
                if message.skill_results.len() >= 80 {
                    return Err("assistant_tool_limit".into());
                }
                message.skill_results.push(copy);
                Ok(())
            })
        })
        .await?;
        self.emit(run, "skill_results", json!({"messageId":mid,"group":saved}));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skill_tools_catalog_metadata_merges_deduplicates_and_caps() {
        let remote = (0..10)
            .map(|i| crate::services::skill::SkillsShDiscoverableSkill {
                key: format!("remote-{i}"),
                name: format!("fixture-{i}"),
                directory: format!("fixture-{i}"),
                repo_owner: "fixture".into(),
                repo_name: "skills".into(),
                repo_branch: "HEAD".into(),
                installs: i,
                readme_url: None,
            })
            .collect();
        let results = merge_search("skills", Some(remote)).unwrap();
        assert_eq!(results.len(), 8);
        assert!(
            results
                .iter()
                .all(|r| !r.result.result_id.is_empty()
                    && r.result.description.chars().count() <= 500)
        );
        let recommended = merge_search("frontend-design", None).unwrap();
        let exact = recommended
            .iter()
            .filter(|r| r.result.source == "anthropics/skills" && r.skill_id == "frontend-design")
            .collect::<Vec<_>>();
        assert_eq!(exact.len(), 1);
        assert_eq!(
            exact[0].directory.as_deref(),
            Some("skills/frontend-design")
        );
        assert!(exact[0].result.reviewed);
        let missing = merge_search("no-such-fixture-4456", None).unwrap();
        assert!(missing.is_empty());
    }
    #[test]
    fn skill_tools_ambiguous_source_cannot_be_chosen_by_the_model() {
        let mut candidate = Candidate {
            result: SkillResult {
                result_id: "id".into(),
                name: "fixture".into(),
                source: "fixture/repo".into(),
                description: String::new(),
                installs: None,
                reviewed: false,
                installed: None,
            },
            skill_id: "fixture".into(),
            directory: None,
        };
        let make = |path: &str| {
            serde_json::from_value::<crate::services::skill::DiscoverableSkill>(json!({"key":format!("fixture/repo:{path}"),"name":"fixture","description":"Fixture","directory":path,"repoOwner":"fixture","repoName":"repo","repoBranch":"HEAD","readmeUrl":format!("https://github.com/fixture/repo/blob/{}/{path}/SKILL.md","a".repeat(40))})).unwrap()
        };
        assert!(select_source(vec![make("one"), make("nested/two")], &candidate).is_err());
        candidate.directory = Some("nested/two".into());
        assert!(
            select_source(vec![make("one"), make("nested/two")], &candidate)
                .unwrap()
                .key
                .ends_with(":nested/two")
        );
        for (name, args) in [
            ("find_skills", json!({"query":"fixture","path":"evil"})),
            ("install_skill", json!({"resultId":"id","directory":"one"})),
            (
                "save_skill",
                json!({"name":"fixture","description":"D","body":"B","overwrite":true}),
            ),
        ] {
            assert!(validate(name, &args).is_err());
            assert!(validate(name, &Value::Null).is_err());
        }
    }
}
