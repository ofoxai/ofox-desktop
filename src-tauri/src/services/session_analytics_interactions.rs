//! Body-free tool/skill evidence and full-cohort summaries. The presentation and
//! categories follow Magpie bfaedf18 stats.go (MIT, Copyright 2026 yetone;
//! see ../session_manager/MAGPIE_LICENSE). Arguments and command text never persist.
use super::{
    advanced::{self, Distribution, MetricCoverage},
    AnalyticsInput, AnalyticsSession,
};
use crate::services::session_analytics_core::{Role, SessionIdentity};
use chrono::{Datelike, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const VERSION: u8 = 1;
const LIMIT: usize = 100_000;
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallScan {
    pub records: BTreeMap<String, Call>,
    pub incomplete: bool,
    #[serde(default)]
    pub conflicts: BTreeSet<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    pub at_ms: i64,
    pub name: String,
    pub skill: Option<String>,
}
fn token(value: Option<&str>) -> Option<String> {
    value
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 160
                && s.chars().all(|c| c.is_alphanumeric() || "_-.:".contains(c))
        })
        .map(str::to_owned)
}
impl CallScan {
    pub fn record(
        &mut self,
        id: Option<&str>,
        at_ms: Option<i64>,
        name: &str,
        input: Option<&Value>,
    ) {
        let (Some(id), Some(at_ms), Some(name)) = (
            id.filter(|id| !id.is_empty() && id.len() <= 512),
            at_ms,
            token(Some(name)),
        ) else {
            self.incomplete = true;
            return;
        };
        if self.conflicts.contains(id) {
            return;
        }
        if self.records.len() + self.conflicts.len() >= LIMIT && !self.records.contains_key(id) {
            self.incomplete = true;
            return;
        }
        let skill = if name.eq_ignore_ascii_case("skill") || name == "activate_skill" {
            token(
                input
                    .and_then(|v| v.get("skill").or_else(|| v.get("name")))
                    .and_then(Value::as_str),
            )
        } else if matches!(name.as_str(), "Read" | "read" | "read_file") {
            // Only an explicit SKILL.md read, never an arbitrary mention in text.
            input
                .and_then(|v| v.get("file_path").or_else(|| v.get("path")))
                .and_then(Value::as_str)
                .map(|s| s.replace('\\', "/"))
                .and_then(|path| {
                    path.strip_suffix("/SKILL.md")
                        .and_then(|folder| folder.rsplit('/').next())
                        .and_then(|name| token(Some(name)))
                })
        } else {
            None
        };
        let mut call = Call { at_ms, name, skill };
        if self.records.get(id).is_some_and(|prior| prior != &call) {
            // Streaming fragments often repeat a call with richer arguments.
            // Time/name disagreement cannot be resolved by file order.
            let prior = &self.records[id];
            if prior.at_ms != call.at_ms
                || prior.name != call.name
                || (prior.skill.is_some() && call.skill.is_some() && prior.skill != call.skill)
            {
                self.incomplete = true;
                self.records.remove(id);
                self.conflicts.insert(id.into());
                return;
            }
        }
        if call.skill.is_none() {
            call.skill = self.records.get(id).and_then(|prior| prior.skill.clone());
        }
        self.records.insert(id.into(), call);
    }
    pub fn blocks(&mut self, parts: Option<&Value>, at_ms: Option<i64>) {
        for part in parts.and_then(Value::as_array).into_iter().flatten() {
            if matches!(
                part["type"].as_str(),
                Some("tool_use" | "toolCall" | "functionCall")
            ) {
                self.record(
                    part["id"].as_str(),
                    at_ms,
                    part["name"].as_str().unwrap_or(""),
                    part.get("input").or_else(|| part.get("arguments")),
                );
            }
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Evidence {
    pub sessions: BTreeSet<String>,
    pub incomplete: BTreeSet<String>,
    pub calls: Vec<Event>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub session_key: String,
    pub event_key: String,
    #[serde(flatten)]
    pub call: Call,
}
impl Evidence {
    pub fn attach(&mut self, session: &str, scan: &CallScan) {
        self.sessions.insert(session.into());
        if scan.incomplete {
            self.incomplete.insert(session.into());
        }
        self.calls
            .extend(scan.records.iter().map(|(id, call)| Event {
                session_key: session.into(),
                event_key: crate::services::session_analytics_index::key("tool-call", id),
                call: call.clone(),
            }));
    }
    pub fn extend(&mut self, other: Self) {
        self.sessions.extend(other.sessions);
        self.incomplete.extend(other.incomplete);
        self.calls.extend(other.calls);
    }
    pub fn retained_bytes(&self) -> Option<usize> {
        let mut size = self
            .calls
            .capacity()
            .checked_mul(std::mem::size_of::<Event>())?;
        for key in self.sessions.iter().chain(&self.incomplete) {
            size = size.checked_add(key.capacity())?.checked_add(128)?;
        }
        for event in &self.calls {
            size = size
                .checked_add(event.session_key.capacity())?
                .checked_add(event.event_key.capacity())?
                .checked_add(event.call.name.capacity())?
                .checked_add(event.call.skill.as_ref().map_or(0, String::capacity))?;
        }
        Some(size)
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stat {
    pub name: String,
    pub category: String,
    pub calls: u64,
    pub sessions: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct Week {
    pub start: String,
    pub calls: BTreeMap<String, u64>,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct Tools {
    pub calls: u64,
    pub sessions: u64,
    pub top: Vec<Stat>,
    pub categories: Vec<Stat>,
    pub weeks: Vec<Week>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Skill {
    pub name: String,
    pub calls: u64,
    pub sessions: u64,
    pub last: String,
    pub agents: BTreeMap<String, u64>,
    pub projects: Vec<Project>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Project {
    pub name: String,
    pub calls: u64,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct Skills {
    pub calls: u64,
    pub count: usize,
    pub top: Vec<Skill>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Interactions {
    pub coverage: MetricCoverage,
    pub tools: Tools,
    pub skills: Skills,
    pub autonomy: Distribution,
}
fn category(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or(name) {
        "Bash" | "BashOutput" | "KillShell" | "KillBash" | "exec_command" | "shell"
        | "shell_command" | "local_shell_call" | "write_stdin" | "unified_exec" | "bash"
        | "powershell" => "Bash",
        "Edit" | "MultiEdit" | "NotebookEdit" | "apply_patch" | "edit" | "str_replace" => "Edit",
        "Read" | "NotebookRead" | "view_image" | "read_file" | "view" | "read" => "Read",
        "Write" | "write_file" | "create" | "write" => "Write",
        "Glob" | "LS" | "list_dir" | "glob" | "find" | "ls" | "fffind" => "Glob",
        "Grep" | "grep" | "grep_files" | "search" | "ffgrep" => "Grep",
        "Task" | "Agent" | "TodoWrite" | "update_plan" | "spawn_agent" | "send_input" | "wait"
        | "close_agent" | "Workflow" => "Task",
        _ if name.starts_with("mcp_") => "Tool",
        _ => "Other",
    }
}

pub(super) fn summarize<Tz: TimeZone>(
    input: &AnalyticsInput,
    rows: &[AnalyticsSession],
    roots: &BTreeMap<&str, &SessionIdentity>,
    query: &crate::services::usage_query::UsageQueryV1,
    attribution: Option<&str>,
    gaps: bool,
    tz: &Tz,
) -> Interactions {
    let start = query.start_ms;
    let end = query.end_ms;
    let cohort: BTreeMap<_, _> = rows.iter().map(|r| (r.session_key.as_str(), r)).collect();
    let evidence = &input.tool_evidence;
    let mut covered = BTreeSet::new();
    let mut incomplete = BTreeSet::new();
    for descriptor in &input.sessions {
        let Some(root) = roots
            .get(descriptor.identity.key.as_str())
            .filter(|root| cohort.contains_key(root.key.as_str()))
        else {
            continue;
        };
        if evidence.sessions.contains(&descriptor.identity.key) && descriptor.source_available {
            covered.insert(root.key.as_str());
        } else {
            incomplete.insert(root.key.as_str());
        }
        if evidence.incomplete.contains(&descriptor.identity.key) {
            incomplete.insert(root.key.as_str());
        }
    }
    let mut events = BTreeMap::new();
    let mut conflicts = BTreeSet::new();
    if attribution.is_none() {
        for event in &evidence.calls {
            let Some(root) = roots
                .get(event.session_key.as_str())
                .filter(|root| cohort.contains_key(root.key.as_str()))
            else {
                continue;
            };
            let id = (&event.session_key, &event.event_key);
            if events
                .insert(id, (root.key.as_str(), &event.call))
                .is_some_and(|prior| prior != (root.key.as_str(), &event.call))
            {
                conflicts.insert(id);
                incomplete.insert(root.key.as_str());
            }
        }
    } else {
        covered.clear();
    }
    for id in conflicts {
        events.remove(&id);
    }
    let eligible: BTreeSet<_> = covered.difference(&incomplete).copied().collect();
    let coverage = MetricCoverage {
        state: if attribution.is_some() || (covered.is_empty() && (!rows.is_empty() || gaps)) {
            "unavailable"
        } else if eligible.len() < rows.len() || gaps {
            "partial"
        } else {
            "complete"
        }
        .into(),
        eligible_sessions: eligible.len() as u64,
        excluded_sessions: (rows.len() - eligible.len()) as u64,
        unquantified_source_gaps: gaps,
        reasons: attribution.map(str::to_owned).into_iter().collect(),
    };
    let mut tools = Tools::default();
    let mut skills = Skills::default();
    let mut tool_counts = BTreeMap::<String, (u64, BTreeSet<&str>)>::new();
    let mut cat_counts = BTreeMap::<String, (u64, BTreeSet<&str>)>::new();
    let mut weeks = BTreeMap::<String, BTreeMap<String, u64>>::new();
    type SkillAccumulator<'a> = (Skill, BTreeSet<&'a str>, BTreeMap<String, u64>);
    let mut skill_counts = BTreeMap::<String, SkillAccumulator<'_>>::new();
    let mut by_session = BTreeMap::<&str, u64>::new();
    for (session, call) in events.values() {
        if call.at_ms >= end || start.is_some_and(|start| call.at_ms < start) {
            continue;
        }
        let Some(date) = tz
            .timestamp_millis_opt(call.at_ms)
            .single()
            .map(|d| d.date_naive())
        else {
            continue;
        };
        let cat = category(&call.name);
        tools.calls += 1;
        *by_session.entry(session).or_default() += 1;
        let stat = tool_counts.entry(call.name.clone()).or_default();
        stat.0 += 1;
        stat.1.insert(session);
        let stat = cat_counts.entry(cat.into()).or_default();
        stat.0 += 1;
        stat.1.insert(session);
        let monday =
            date - chrono::Duration::days(i64::from(date.weekday().num_days_from_monday()));
        *weeks
            .entry(monday.to_string())
            .or_default()
            .entry(cat.into())
            .or_default() += 1;
        if let Some(name) = &call.skill {
            let row = cohort[session];
            let (skill, sessions, projects) =
                skill_counts.entry(name.clone()).or_insert_with(|| {
                    (
                        Skill {
                            name: name.clone(),
                            calls: 0,
                            sessions: 0,
                            last: String::new(),
                            agents: BTreeMap::new(),
                            projects: vec![],
                        },
                        BTreeSet::new(),
                        BTreeMap::new(),
                    )
                });
            skill.calls += 1;
            sessions.insert(session);
            skill.last = skill.last.clone().max(date.to_string());
            *skill.agents.entry(row.tool_id.clone()).or_default() += 1;
            if let Some(project) = &row.project_label {
                *projects.entry(project.clone()).or_default() += 1;
            }
            skills.calls += 1;
        }
    }
    tools.sessions = by_session.len() as u64;
    tools.top = tool_counts
        .into_iter()
        .map(|(name, (calls, sessions))| Stat {
            category: category(&name).into(),
            name,
            calls,
            sessions: sessions.len() as u64,
        })
        .collect();
    tools.categories = cat_counts
        .into_iter()
        .map(|(name, (calls, sessions))| Stat {
            category: name.clone(),
            name,
            calls,
            sessions: sessions.len() as u64,
        })
        .collect();
    for stats in [&mut tools.top, &mut tools.categories] {
        stats.sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.name.cmp(&b.name)));
    }
    tools.top.truncate(50);
    // Include empty weeks between observations; return at most the latest 16.
    if let (Some(first), Some(last)) = (weeks.first_key_value(), weeks.last_key_value()) {
        if let (Ok(first), Ok(last)) = (
            chrono::NaiveDate::parse_from_str(first.0, "%Y-%m-%d"),
            chrono::NaiveDate::parse_from_str(last.0, "%Y-%m-%d"),
        ) {
            let mut day = first.max(last - chrono::Duration::weeks(15));
            while day <= last {
                let date = day.to_string();
                tools.weeks.push(Week {
                    calls: weeks.remove(&date).unwrap_or_default(),
                    start: date,
                });
                day += chrono::Duration::weeks(1);
            }
        }
    }
    skills.count = skill_counts.len();
    skills.top = skill_counts
        .into_values()
        .map(|(mut skill, sessions, projects)| {
            skill.sessions = sessions.len() as u64;
            skill.projects = projects
                .into_iter()
                .map(|(name, calls)| Project { name, calls })
                .collect();
            skill
                .projects
                .sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.name.cmp(&b.name)));
            skill.projects.truncate(3);
            skill
        })
        .collect();
    skills
        .top
        .sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.name.cmp(&b.name)));
    skills.top.truncate(30);
    let mut prompts = BTreeMap::<&str, BTreeSet<&str>>::new();
    for message in &input.messages {
        if message.main
            && message.role == Role::User
            && message.at_ms < end
            && start.is_none_or(|s| message.at_ms >= s)
        {
            if let Some(root) = roots.get(message.session_key.as_str()).filter(|root| {
                root.key == message.session_key && eligible.contains(root.key.as_str())
            }) {
                prompts
                    .entry(root.key.as_str())
                    .or_default()
                    .insert(&message.native_id);
            }
        }
    }
    let autonomy = advanced::distribution(
        "toolCallsPerPrompt",
        &[0, 1, 3, 6, 11, 21],
        rows.iter().map(|r| {
            let prompts = prompts.get(r.session_key.as_str()).map_or(0, BTreeSet::len) as u64;
            (attribution.is_none()
                && eligible.contains(r.session_key.as_str())
                && r.messages.is_some()
                && prompts > 0)
                .then(|| by_session.get(r.session_key.as_str()).copied().unwrap_or(0) / prompts)
        }),
        gaps,
    );
    Interactions {
        coverage,
        tools,
        skills,
        autonomy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn native_calls_are_deduplicated_and_do_not_retain_arguments_or_outputs() {
        let mut scan = CallScan::default();
        let blocks = json!([{"type":"tool_use","id":"a","name":"Skill","input":{"skill":"support-triage","secret":"never-save"}},{"type":"tool_result","tool_use_id":"a","content":"secret"}]);
        scan.blocks(Some(&blocks), Some(123));
        scan.blocks(Some(&blocks), Some(123));
        assert_eq!(scan.records.len(), 1);
        assert_eq!(scan.records["a"].skill.as_deref(), Some("support-triage"));
        assert!(!serde_json::to_string(&scan).unwrap().contains("never-save"));
        scan.record(None, Some(123), "Bash", None);
        assert!(scan.incomplete);
    }
}
