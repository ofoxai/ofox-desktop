//! Generated conversation titles. After a conversation's first answer, while
//! its title is still automatic and the preference is on, the host asks the
//! runtime's short `title` operation (assistant-runtime README) to name it,
//! in a process of its own, once the run has published its end. A title that
//! fails, or that the user overtook by renaming or trashing the conversation,
//! changes nothing, and it is never asked for again.
use super::{
    catalog::{self, Model},
    host::{
        self, blocking, local_gateway_allowed, scrub_secrets_in, AssistantHost, Identity,
        RunControl, Runtime, CANCELLED, PROTOCOL, STDIO,
    },
    process,
    projection::recount_usage,
    store::SessionRecord,
    types::*,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

/// The runtime gives a title request up after 8 s; this also covers
/// starting its process.
const DEADLINE: Duration = Duration::from_secs(15);
/// How long a stopped title has to answer with what it may have cost.
const STOP_GRACE: Duration = Duration::from_secs(2);
/// Characters of the question, and of the reply, a title is made from.
const INPUT_CHARS: usize = 1_500;
/// The most characters a generated title keeps.
const TITLE_CHARS: usize = 40;
/// The reasoning a title may use, least first: the first one the model
/// offers is sent; with none of them, nothing is (the provider's default).
const LEVELS: [&str; 3] = ["off", "minimal", "low"];

/// What a conversation is named from: its first answered turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TitleInput {
    pub question: String,
    pub reply: String,
}

/// `run`'s question and reply (`reply_id`, the message the run wrote) when
/// they are the conversation's first answer and its title is still
/// automatic: a conversation is named once, from that answer. A retry,
/// edit or regenerate of a turn that failed or stopped can be it; one of a
/// turn that was answered is not, nor is any later turn.
pub(super) fn first_answer(
    record: &SessionRecord,
    run_id: &str,
    reply_id: &str,
) -> Option<TitleInput> {
    let session = &record.session;
    if session.deleted_at.is_some() || session.title_source != Some(TitleSource::Auto) {
        return None;
    }
    let answered = |m: &&Message| m.role == "assistant" && m.status == "completed";
    if record
        .messages
        .iter()
        .filter(answered)
        .any(|m| m.run_id.as_deref() != Some(run_id))
    {
        return None;
    }
    let reply = record
        .messages
        .iter()
        .filter(answered)
        .find(|m| m.id == reply_id)?;
    let user = record
        .messages
        .iter()
        .find(|m| m.role == "user" && m.run_id.as_deref() == Some(run_id))?;
    let question = if user.content.trim().is_empty() {
        let names: Vec<&str> = user.attachments.iter().map(|a| a.name.as_str()).collect();
        names.join(", ")
    } else {
        user.content.clone()
    };
    let clip = |text: &str| text.chars().take(INPUT_CHARS).collect::<String>();
    Some(TitleInput {
        question: clip(&question),
        reply: clip(&reply.content),
    })
}

/// A generated title as it may be saved: its first line, without the quotes
/// or trailing punctuation a model may add although told not to, at most
/// [`TITLE_CHARS`] characters and without any of `secrets`. `None` when
/// nothing is left.
pub(super) fn sanitize_title(raw: &str, secrets: &[String]) -> Option<String> {
    const QUOTES: &[char] = &[
        '"', '\'', '`', '*', '\u{201c}', '\u{201d}', '\u{2018}', '\u{2019}', '\u{300c}',
        '\u{300d}', '\u{300e}', '\u{300f}', '\u{300a}', '\u{300b}',
    ];
    const TRAILING: &[char] = &[
        '.', '!', '?', ',', ';', ':', '\u{3002}', '\u{ff01}', '\u{ff1f}', '\u{ff0c}', '\u{ff1b}',
        '\u{ff1a}', '\u{3001}', '\u{2026}',
    ];
    let text = host::scrub_secrets(raw, secrets);
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let words: Vec<&str> = line.split_whitespace().collect();
    let line = words.join(" ");
    let bare = line
        .trim_matches(QUOTES)
        .trim()
        .trim_end_matches(TRAILING)
        .trim_end()
        .trim_matches(QUOTES)
        .trim();
    let title: String = bare.chars().take(TITLE_CHARS).collect();
    let title = title.trim_end();
    (!title.is_empty()).then(|| title.to_owned())
}

/// Keep what a title brought back. Its cost joins the conversation's usage,
/// recounted from the native history it returned (`entries`); its title
/// replaces the automatic one unless the user renamed or trashed the
/// conversation meanwhile, or the authority changed (`current` is false).
/// The conversation keeps its place in the list. Returns the session when
/// its title changed.
pub(super) fn settle_title(
    record: &mut SessionRecord,
    entries: Option<&[Value]>,
    title: Option<&str>,
    current: bool,
) -> Option<Session> {
    if let Some(entries) = entries {
        recount_usage(record, entries);
    }
    let title = title?;
    let session = &mut record.session;
    if !current
        || session.title_source != Some(TitleSource::Auto)
        || session.deleted_at.is_some()
        || session.title == title
    {
        return None;
    }
    session.title = title.to_owned();
    Some(session.clone())
}

/// The model titles use while the catalog prices it: the search model, the
/// cheapest one the app already relies on. Otherwise the conversation's.
pub(super) fn title_model(models: &[Model]) -> Option<&Model> {
    models
        .iter()
        .find(|m| m.id == catalog::SEARCH_MODEL && m.pricing_known && m.for_conversations())
}

/// The least reasoning `model` offers among [`LEVELS`].
pub(super) fn title_level(model: &Model) -> Option<&'static str> {
    LEVELS
        .into_iter()
        .find(|level| model.offers_thinking_level(level))
}

/// A title to ask for after a run, under the authority the run ended in.
struct TitleJob {
    /// Names the job's `session` event, apart from every run's.
    id: String,
    identity: Identity,
    generation: u64,
    session_id: String,
    /// The answered turn: its usage entry is kept under this run.
    run_id: String,
    operation_id: String,
    input: TitleInput,
}

impl AssistantHost {
    /// Have the conversation of `control`'s run named from `input`, in the
    /// background: the run has already published its end.
    pub(super) fn spawn_title(self: &Arc<Self>, control: &RunControl, input: TitleInput) {
        let job = TitleJob {
            id: id(),
            identity: control.identity.clone(),
            generation: self.generation(),
            session_id: control.session_id.clone(),
            run_id: control.run_id.clone(),
            operation_id: control.operation_id.clone(),
            input,
        };
        let host = self.clone();
        self.track(format!("title-{}", job.id), async move {
            if let Err(error) = host.generate_title(&job).await {
                log::info!("[Assistant] a conversation keeps its title: {error}");
            }
        });
    }

    /// Ask for the title with the account's key, the way a run does, then
    /// keep its cost and, if the conversation is still named automatically,
    /// its title, and publish the session.
    async fn generate_title(self: &Arc<Self>, job: &TitleJob) -> Result<(), String> {
        let owner = &job.identity.owner;
        // Between the turn and the next one: no run writes the native history
        // while the title's record goes into it.
        let _history = self
            .native_lock(&host::session_key(owner, &job.session_id))?
            .lock_owned()
            .await;
        self.ensure_accepting_runs()?;
        self.ensure_current(&job.identity, job.generation).await?;
        let (record, native) = blocking({
            let store = self.store(owner)?;
            let sid = job.session_id.clone();
            move || Ok((store.load_record(&sid)?, store.native_history_path(&sid)?))
        })
        .await?;
        // Renamed or trashed while it waited: nothing to ask for.
        if record.session.title_source != Some(TitleSource::Auto)
            || record.session.deleted_at.is_some()
        {
            return Ok(());
        }
        let apex = &job.identity.apex;
        let runtime = self.runtime()?;
        let model = match title_model(&self.models(apex).await?.models) {
            Some(model) => model.clone(),
            None => self.model(apex, &record.session.model_id).await?,
        };
        let level = title_level(&model);
        let base_url = self.env.gateway_base(apex)?;
        let proxy = blocking({
            let env = self.env.clone();
            move || Ok(env.proxy())
        })
        .await?;
        let admit = || self.admit(job.generation);
        let key = self.env.api_key(&job.identity, None, &admit).await?;
        self.ensure_current(&job.identity, job.generation).await?;
        self.remember_key(&key);
        let mut params = json!({
            "sessionId": job.session_id,
            "runId": job.run_id,
            "operationId": job.operation_id,
            "sessionFile": native,
            "model": model.for_runtime(level.is_some()),
            "apiKey": key,
            "apex": apex,
            "baseUrl": base_url,
            "allowLocalGateway": local_gateway_allowed(&base_url),
            "proxy": proxy,
            "userText": job.input.question,
            "replyText": job.input.reply,
        });
        if let Some(level) = level {
            params["thinkingLevel"] = json!(level);
        }
        let answer = self.ask_title(&runtime, &params, job.generation).await?;
        let secrets = self.known_keys();
        let title = answer["title"]
            .as_str()
            .and_then(|raw| sanitize_title(raw, &secrets));
        let entries = answer["entries"].as_array().cloned();
        if title.is_none() && entries.is_none() {
            return Ok(());
        }
        let saved = blocking({
            let store = self.store(owner)?;
            let host = self.clone();
            let (sid, generation) = (job.session_id.clone(), job.generation);
            move || {
                store.mutate(&sid, |record| {
                    let current = host.generation() == generation;
                    Ok(settle_title(
                        record,
                        entries.as_deref(),
                        title.as_deref(),
                        current,
                    ))
                })
            }
        })
        .await?;
        if let Some(session) = saved {
            let mut data = json!({ "session": session });
            scrub_secrets_in(&mut data, &secrets);
            self.env.emit(Event {
                session_id: job.session_id.clone(),
                run_id: job.id.clone(),
                operation_id: job.id.clone(),
                r#type: "session".into(),
                data,
            });
        }
        Ok(())
    }

    /// The runtime's answer to one `title` request, from a process of its own
    /// that ends with it. Once a logout, an account or region change or
    /// shutdown makes the title unwanted, the runtime is asked to stop, and
    /// it still answers with what the request may have cost, within
    /// [`STOP_GRACE`]. Given up after [`DEADLINE`].
    async fn ask_title(
        &self,
        runtime: &Runtime,
        params: &Value,
        generation: u64,
    ) -> Result<Value, String> {
        use tokio::io::BufReader;
        let mut child = process::Child::spawn(host::runtime_command(runtime))?;
        let mut input = child.take_stdin().ok_or(STDIO)?;
        let mut output = BufReader::new(child.take_stdout().ok_or(STDIO)?);
        let stderr = child.take_stderr().ok_or(STDIO)?;
        // Drain, never publish: dependencies may print request headers.
        let drain = tokio::spawn(async move {
            let _ = process::bounded_output(stderr, 64 * 1024).await;
        });
        let request = host::frame(
            &json!({"jsonrpc": "2.0", "id": "title", "method": "title", "params": params}),
        )?;
        let stop = host::frame(&json!({
            "jsonrpc": "2.0", "id": "stop", "method": "stop",
            "params": {
                "sessionId": params["sessionId"],
                "runId": params["runId"],
                "operationId": params["operationId"],
            },
        }))?;
        let answer = tokio::time::timeout(DEADLINE, async {
            host::write_line(&mut input, &request).await?;
            let mut buffer = Vec::new();
            let mut ticker = tokio::time::interval(Duration::from_millis(250));
            let mut stopping: Option<tokio::time::Instant> = None;
            loop {
                let length = tokio::select! {
                    _ = ticker.tick() => {
                        let unwanted = self.generation() != generation
                            || self.ensure_accepting_runs().is_err();
                        if unwanted && stopping.is_none() {
                            host::write_line(&mut input, &stop).await?;
                            stopping = Some(tokio::time::Instant::now() + STOP_GRACE);
                        }
                        if stopping.is_some_and(|until| tokio::time::Instant::now() > until) {
                            return Err(CANCELLED.to_string());
                        }
                        continue;
                    }
                    read = host::read_frame(&mut output, &mut buffer) => read?,
                };
                if length == 0 {
                    return Err("assistant_runtime_exited".to_string());
                }
                let mut value: Value = serde_json::from_slice(&buffer).map_err(|_| PROTOCOL)?;
                buffer.clear();
                if value["id"] == "title" {
                    return match value.get("error") {
                        Some(_) => Err("assistant_runtime_failed".to_string()),
                        None => Ok(value["result"].take()),
                    };
                }
            }
        })
        .await
        .unwrap_or_else(|_| Err("assistant_run_timeout".to_string()));
        child.stop().await;
        drain.abort();
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(role: &str, run: &str, content: &str, status: &str) -> Message {
        serde_json::from_value(json!({
            "id": id(), "role": role, "content": content, "createdAt": 1,
            "status": status, "runId": run,
        }))
        .unwrap()
    }

    /// A conversation with an automatic title and these turns: (question,
    /// reply, reply status, superseded).
    fn conversation(turns: &[(&str, &str, &str, bool)]) -> (SessionRecord, Vec<(String, String)>) {
        let mut messages = vec![];
        let mut runs = vec![];
        for (question, reply, status, superseded) in turns {
            let run = id();
            let mut user = message("user", &run, question, "completed");
            let mut answer = message("assistant", &run, reply, status);
            user.superseded = *superseded;
            answer.superseded = *superseded;
            runs.push((run, answer.id.clone()));
            messages.extend([user, answer]);
        }
        let record = SessionRecord {
            session: serde_json::from_value(json!({
                "id": id(), "title": "Plan a trip", "titleSource": "auto", "modelId": "m",
                "createdAt": 1, "updatedAt": 7,
            }))
            .unwrap(),
            messages,
            attachments: vec![],
            usage: json!({}),
            operations: vec![],
        };
        (record, runs)
    }

    fn input(question: &str, reply: &str) -> Option<TitleInput> {
        Some(TitleInput {
            question: question.into(),
            reply: reply.into(),
        })
    }

    #[test]
    fn a_conversation_is_named_once_from_its_first_answer() {
        let (record, runs) = conversation(&[("Plan a trip", "Day one", "completed", false)]);
        let (run, reply) = &runs[0];
        assert_eq!(
            first_answer(&record, run, reply),
            input("Plan a trip", "Day one")
        );

        // A later turn, also one sent while the first was being named.
        let (record, runs) = conversation(&[
            ("Plan a trip", "Day one", "completed", false),
            ("And day two?", "Day two", "completed", false),
        ]);
        assert_eq!(first_answer(&record, &runs[1].0, &runs[1].1), None);

        // An edit or a regenerate of the answered first turn.
        let (record, runs) = conversation(&[
            ("Plan a trip", "Day one", "completed", true),
            ("Plan a long trip", "Week one", "completed", false),
        ]);
        assert_eq!(first_answer(&record, &runs[1].0, &runs[1].1), None);
    }

    #[test]
    fn a_turn_after_one_that_got_no_answer_can_be_the_first_answer() {
        for status in ["error", "stopped"] {
            // A retry or edit of the turn, or a new message after it.
            for superseded in [true, false] {
                let (record, runs) = conversation(&[
                    ("Plan a trip", "Day o", status, superseded),
                    ("Plan a trip again", "Day one", "completed", false),
                ]);
                assert_eq!(
                    first_answer(&record, &runs[1].0, &runs[1].1),
                    input("Plan a trip again", "Day one"),
                    "{status} {superseded}"
                );
            }
        }
        // The turn that got no answer is not named.
        let (record, runs) = conversation(&[("Plan a trip", "Day o", "error", false)]);
        assert_eq!(first_answer(&record, &runs[0].0, &runs[0].1), None);
    }

    #[test]
    fn a_renamed_or_trashed_conversation_is_not_named() {
        let (mut record, runs) = conversation(&[("Plan a trip", "Day one", "completed", false)]);
        let (run, reply) = runs[0].clone();
        record.session.title_source = Some(TitleSource::User);
        assert_eq!(first_answer(&record, &run, &reply), None);
        record.session.title_source = Some(TitleSource::Auto);
        record.session.deleted_at = Some(9);
        assert_eq!(first_answer(&record, &run, &reply), None);
    }

    #[test]
    fn a_title_reads_the_start_of_the_turn_and_a_file_sent_alone_by_its_name() {
        let long = "長".repeat(INPUT_CHARS + 10);
        let (mut record, runs) = conversation(&[("", &long, "completed", false)]);
        record.messages[0].attachments = vec![serde_json::from_value(json!({
            "id": id(), "name": "itinerary.pdf", "mimeType": "application/pdf",
            "sizeBytes": 3, "status": "ready",
        }))
        .unwrap()];
        let named = first_answer(&record, &runs[0].0, &runs[0].1).unwrap();
        assert_eq!(named.question, "itinerary.pdf");
        assert_eq!(named.reply, "長".repeat(INPUT_CHARS));
        record.messages[0].content = long.clone();
        let named = first_answer(&record, &runs[0].0, &runs[0].1).unwrap();
        assert_eq!(named.question.chars().count(), INPUT_CHARS);
    }

    #[test]
    fn a_generated_title_is_one_short_line_without_quotes_keys_or_trailing_punctuation() {
        let key = "sk-generated-title-key-123456".to_string();
        let clean = |raw: &str| sanitize_title(raw, std::slice::from_ref(&key));
        assert_eq!(clean("  Kyoto Trip Plan  "), Some("Kyoto Trip Plan".into()));
        assert_eq!(
            clean("\"Kyoto Trip Plan.\""),
            Some("Kyoto Trip Plan".into())
        );
        assert_eq!(clean("「京都三日游」。"), Some("京都三日游".into()));
        assert_eq!(
            clean("\n\nKyoto   Trip\nand more"),
            Some("Kyoto Trip".into())
        );
        assert_eq!(
            clean("Why is the sky blue?"),
            Some("Why is the sky blue".into())
        );
        assert_eq!(
            clean(&"x".repeat(60)).map(|t| t.chars().count()),
            Some(TITLE_CHARS)
        );
        assert_eq!(
            clean(&format!("{} y", "w".repeat(TITLE_CHARS - 1))),
            Some("w".repeat(TITLE_CHARS - 1)),
            "a cut leaves no trailing space"
        );
        assert_eq!(
            clean(&format!("Key {key}")),
            Some(format!("Key {}", crate::redact::REDACTED))
        );
        assert_eq!(clean("  \n \"\" 。"), None);
        assert_eq!(clean(""), None);
    }

    /// The turn's reply and the title's request as the runtime projects
    /// them: 10 + 5 tokens each, priced.
    fn native_with_title() -> Vec<Value> {
        let usage = json!({"input": 10, "output": 5, "cacheRead": 0, "cacheWrite": 0,
                           "totalTokens": 15, "cost": {"total": 0.00007}});
        vec![
            json!({"role": "assistant", "runId": "r", "pricingKnown": true,
                   "usageCoverage": "complete", "usage": usage}),
            json!({"role": "usage", "type": "title", "runId": "r", "pricingKnown": true,
                   "usageCoverage": "complete", "usage": usage}),
        ]
    }

    #[test]
    fn the_title_replaces_only_an_automatic_title_and_keeps_the_conversations_place() {
        let (mut record, _) = conversation(&[("Plan a trip", "Day one", "completed", false)]);
        let session = settle_title(&mut record, None, Some("Kyoto Trip Plan"), true).unwrap();
        assert_eq!(session.title, "Kyoto Trip Plan");
        assert_eq!(record.session.title, "Kyoto Trip Plan");
        assert_eq!(record.session.title_source, Some(TitleSource::Auto));
        assert_eq!(record.session.updated_at, 7, "the list order stays");
        // The same title again changes nothing.
        assert!(settle_title(&mut record, None, Some("Kyoto Trip Plan"), true).is_none());
    }

    #[test]
    fn the_titles_cost_lands_in_the_conversations_usage_whatever_happens_to_the_title() {
        for (source, deleted, current) in [
            (TitleSource::Auto, None, true),
            // Renamed while the title was generated: the rename wins.
            (TitleSource::User, None, true),
            (TitleSource::Auto, Some(9), true),
            (TitleSource::Auto, None, false),
        ] {
            let (mut record, _) = conversation(&[("Plan a trip", "Day one", "completed", false)]);
            record.session.title_source = Some(source);
            record.session.deleted_at = deleted;
            let saved = settle_title(
                &mut record,
                Some(&native_with_title()),
                Some("Kyoto Trip Plan"),
                current,
            );
            let renamed = source == TitleSource::Auto && deleted.is_none() && current;
            assert_eq!(saved.is_some(), renamed, "{source:?} {deleted:?} {current}");
            assert_eq!(
                record.session.title,
                if renamed {
                    "Kyoto Trip Plan"
                } else {
                    "Plan a trip"
                }
            );
            assert_eq!(record.usage["modelRequests"], 2);
            assert_eq!(record.usage["inputTokens"], 20);
            assert_eq!(record.usage["outputTokens"], 10);
            assert_eq!(record.usage["estimatedCostUsd"], "0.00014");
        }
    }

    fn model(id: &str, priced: bool, levels: &[&str]) -> Model {
        serde_json::from_value(json!({
            "id": id, "name": id, "contextWindow": 128000, "maxOutputTokens": 4096,
            "supportsImages": false, "inputModalities": ["text"],
            "supportedEndpoints": ["/v1/chat/completions"], "inputPricePerMillion": null,
            "outputPricePerMillion": null, "pricing": {}, "cost": {}, "pricingKnown": priced,
            "reasoning": !levels.is_empty(), "thinkingLevels": levels,
        }))
        .unwrap()
    }

    #[test]
    fn titles_use_the_search_model_while_the_catalog_prices_it() {
        let search = model(catalog::SEARCH_MODEL, true, &["minimal", "low"]);
        let chat = model("test/chat", true, &[]);
        assert_eq!(
            title_model(&[chat.clone(), search.clone()]).map(|m| m.id.as_str()),
            Some(catalog::SEARCH_MODEL)
        );
        assert!(title_model(&[chat.clone()]).is_none());
        let unpriced = model(catalog::SEARCH_MODEL, false, &["minimal"]);
        assert!(title_model(&[chat, unpriced]).is_none());
    }

    #[test]
    fn a_title_reasons_as_little_as_its_model_allows() {
        assert_eq!(
            title_level(&model("a", true, &["minimal", "low", "high"])),
            Some("minimal")
        );
        assert_eq!(
            title_level(&model("b", true, &["off", "low", "high"])),
            Some("off")
        );
        assert_eq!(
            title_level(&model("c", true, &["low", "high"])),
            Some("low")
        );
        assert_eq!(title_level(&model("d", true, &["high"])), None);
        assert_eq!(title_level(&model("e", true, &[])), None);
    }
}
