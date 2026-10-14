//! Session titles, adapted from Magpie internal/sessions/sessions.go (`assemble`,
//! `title`, `untagged`) and claude.go / codex.go (`ccPrompt`, `cxPrompt`).
//! Copyright (c) 2026 yetone, MIT; see MAGPIE_LICENSE.
//!
//! A session goes by the name it was given, else the agent's own name for it,
//! else the first prompt someone typed, else its first message with the tags
//! taken out, else its folder.
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// Characters a title keeps; a longer one is cut and ends in "…".
pub const TITLE_MAX_CHARS: usize = 160;

static SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"</?[A-Za-z][\w-]*[^<>]*>").unwrap());
static COMMAND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<command-name>\s*([^<]*?)\s*</command-name>").unwrap());
static ARGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<command-args>\s*([^<]*?)\s*</command-args>").unwrap());

/// Where a session's title came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TitleSource {
    /// The name the user gave it (Claude `/rename`, Pi `/name`).
    Custom,
    /// The agent's own name for it (Claude `ai-title`, Codex thread name).
    Agent,
    /// The first prompt someone typed.
    Prompt,
    /// The first message, with its tags taken out.
    Message,
    /// The folder it ran in.
    Directory,
}

/// What a title may be made from, best first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TitleCandidates {
    pub custom: Option<String>,
    pub agent: Option<String>,
    /// Claude's `summary` line: used only when there is no agent title.
    pub summary: Option<String>,
    pub prompt: Option<String>,
    pub message: Option<String>,
    pub directory: Option<String>,
}

impl TitleCandidates {
    /// Prompt and message candidates from the first user message's text.
    pub fn with_first_user_text(mut self, text: Option<&str>) -> Self {
        if let Some(text) = text {
            self.prompt = clean_prompt(text);
            self.message = non_empty(untagged(text));
        }
        self
    }

    /// Fill every candidate this one lacks from `other`.
    pub fn fill_from(&mut self, other: &TitleCandidates) {
        for (mine, theirs) in [
            (&mut self.custom, &other.custom),
            (&mut self.agent, &other.agent),
            (&mut self.summary, &other.summary),
            (&mut self.prompt, &other.prompt),
            (&mut self.message, &other.message),
            (&mut self.directory, &other.directory),
        ] {
            if mine.is_none() {
                mine.clone_from(theirs);
            }
        }
    }

    /// The folder candidate: the last component of the project directory.
    pub fn with_directory(mut self, project_dir: Option<&str>) -> Self {
        self.directory = project_dir.and_then(super::providers::path_basename);
        self
    }
}

/// The title, and where it came from: custom, then agent (its summary line only
/// when there is no agent title), then prompt, message and directory.
pub fn pick_title(candidates: &TitleCandidates) -> Option<(String, TitleSource)> {
    [
        (&candidates.custom, TitleSource::Custom),
        (&candidates.agent, TitleSource::Agent),
        (&candidates.summary, TitleSource::Agent),
        (&candidates.prompt, TitleSource::Prompt),
        (&candidates.message, TitleSource::Message),
        (&candidates.directory, TitleSource::Directory),
    ]
    .into_iter()
    .find_map(|(text, source)| Some((non_empty(one_line(text.as_deref()?))?, source)))
}

/// `pick_title` as `SessionMeta`'s title and title source.
pub fn titled(candidates: &TitleCandidates) -> (Option<String>, Option<TitleSource>) {
    pick_title(candidates).map_or((None, None), |(title, source)| (Some(title), Some(source)))
}

/// A text made one short line: whitespace collapsed, cut at 160 characters.
pub fn one_line(text: &str) -> String {
    let text = SPACE.replace_all(text, " ");
    let text = text.trim();
    if text.chars().count() <= TITLE_MAX_CHARS {
        return text.to_string();
    }
    let mut cut = text
        .chars()
        .take(TITLE_MAX_CHARS)
        .collect::<String>()
        .trim_end()
        .to_string();
    cut.push('…');
    cut
}

/// A message the agent or a harness wrapped in tags, as words.
pub fn untagged(text: &str) -> String {
    one_line(&TAG.replace_all(text, " "))
}

/// The words of a prompt someone typed, or `None` for what the agent put in
/// the conversation itself (tags, interruptions, caveats, AGENTS.md). A slash
/// command reads as its name and what was typed after it.
pub fn clean_prompt(text: &str) -> Option<String> {
    let text = text.trim();
    if let Some(command) = slash_command(text) {
        return non_empty(one_line(&command));
    }
    if text.is_empty()
        || ["<", "[Request interrupted", "Caveat:", "# AGENTS.md"]
            .iter()
            .any(|prefix| text.starts_with(prefix))
    {
        return None;
    }
    non_empty(one_line(text))
}

/// A slash command (`<command-name>`) as its name and what was typed after
/// it (`<command-args>`), or `None` when the text holds none.
pub(crate) fn slash_command(text: &str) -> Option<String> {
    let command = COMMAND.captures(text)?;
    let mut line = command[1].to_string();
    if let Some(args) = ARGS.captures(text).filter(|a| !a[1].is_empty()) {
        line.push(' ');
        line.push_str(&args[1]);
    }
    Some(line)
}

fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some(text: &str) -> Option<String> {
        Some(text.to_string())
    }

    // Ported from Magpie sessions_test.go TestPrompt.
    #[test]
    fn a_prompt_is_what_was_typed_and_a_slash_command_reads_as_its_name_and_args() {
        for (input, want) in [
            (
                "<command-message>review</command-message>\n<command-name>/review</command-name>\n<command-args>42</command-args>",
                some("/review 42"),
            ),
            (
                "<command-name>/clear</command-name>\n            <command-message>clear</command-message>\n            <command-args></command-args>",
                some("/clear"),
            ),
            ("<local-command-stdout>hi</local-command-stdout>", None),
            ("[Request interrupted by user]", None),
            ("Caveat: The messages below were generated by the user", None),
            ("# AGENTS.md instructions for /tmp/project\n<INSTRUCTIONS>x", None),
            ("<external_codex_apps_open_page>{\"page_id\":null}</external_codex_apps_open_page>", None),
            ("   ", None),
            ("  fix the\n\n login   bug ", some("fix the login bug")),
        ] {
            assert_eq!(clean_prompt(input), want, "{input}");
        }
    }

    #[test]
    fn a_long_title_is_cut_at_160_characters_and_ends_in_an_ellipsis() {
        let cut = one_line(&"字".repeat(300));
        let chars = cut.chars().collect::<Vec<_>>();
        assert_eq!(chars.len(), 161);
        assert_eq!(chars[160], '…');
        assert_eq!(one_line(&"a".repeat(160)), "a".repeat(160));
    }

    #[test]
    fn a_tagged_message_reads_as_its_words() {
        assert_eq!(
            untagged(
                "<external_codex_apps_open_page>{\"page_id\":null}</external_codex_apps_open_page>"
            ),
            "{\"page_id\":null}"
        );
        assert_eq!(
            untagged("<task-notification>\n<summary>done</summary>\n</task-notification>"),
            "done"
        );
    }

    #[test]
    fn the_title_is_the_best_candidate_there_is() {
        let all = TitleCandidates {
            custom: some("Login, renamed"),
            agent: some("Fix login bug"),
            summary: some("Old summary"),
            prompt: some("fix the login"),
            message: some("hello"),
            directory: some("app"),
        };
        let mut candidates = all.clone();
        let mut picked = Vec::new();
        while let Some(pick) = pick_title(&candidates) {
            picked.push(pick.clone());
            match pick.1 {
                TitleSource::Custom => candidates.custom = None,
                TitleSource::Agent if candidates.agent.is_some() => candidates.agent = None,
                TitleSource::Agent => candidates.summary = None,
                TitleSource::Prompt => candidates.prompt = None,
                TitleSource::Message => candidates.message = None,
                TitleSource::Directory => candidates.directory = None,
            }
        }
        assert_eq!(
            picked,
            vec![
                ("Login, renamed".to_string(), TitleSource::Custom),
                ("Fix login bug".to_string(), TitleSource::Agent),
                ("Old summary".to_string(), TitleSource::Agent),
                ("fix the login".to_string(), TitleSource::Prompt),
                ("hello".to_string(), TitleSource::Message),
                ("app".to_string(), TitleSource::Directory),
            ]
        );
    }

    #[test]
    fn blank_candidates_are_skipped() {
        let candidates = TitleCandidates {
            custom: some("  \n "),
            prompt: some("  real   prompt "),
            ..Default::default()
        };
        assert_eq!(
            pick_title(&candidates),
            Some(("real prompt".to_string(), TitleSource::Prompt))
        );
        assert_eq!(pick_title(&TitleCandidates::default()), None);
    }

    #[test]
    fn the_first_user_text_gives_a_prompt_and_a_message() {
        let typed =
            TitleCandidates::default().with_first_user_text(Some("look at this [Image #2]"));
        assert_eq!(typed.prompt, some("look at this [Image #2]"));
        let tagged = TitleCandidates::default()
            .with_first_user_text(Some("<user_query>why</user_query>"))
            .with_directory(Some("/work/my-app/"));
        assert_eq!(tagged.prompt, None);
        assert_eq!(tagged.message, some("why"));
        assert_eq!(tagged.directory, some("my-app"));
        assert_eq!(
            pick_title(&tagged),
            Some(("why".to_string(), TitleSource::Message))
        );
    }
}
