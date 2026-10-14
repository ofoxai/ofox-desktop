//! Where Codex and Claude Code really keep their files.
//!
//! Both agents let the user move their directory with an environment variable
//! (`CODEX_HOME`, `CLAUDE_CONFIG_DIR`). A desktop app launched from the Dock or the
//! Start menu does not inherit what the user exported in a shell rc file, so the
//! variable is looked up in the places a terminal would get it from:
//!
//! 1. the directory the user typed into Ofox's settings (existing UI field);
//! 2. the variable in Ofox's own process environment;
//! 3. a snapshot of the variable taken once from a login shell (Windows: the
//!    `Environment` registry keys, which is what a freshly opened terminal sees);
//! 4. the default, `~/.codex` / `~/.claude`.
//!
//! An environment value only counts when it is a non-empty absolute path: a relative
//! one would mean whatever folder the agent is started in, so it is ignored.
//!
//! [`resolve`] is the pure decision. [`dir`] feeds it the live inputs; the shell
//! snapshot is taken by [`ready`], which every entry point that reads or writes an
//! agent's files awaits first. Before the snapshot exists [`dir`] simply skips that
//! source, so synchronous callers keep working.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Serialize, Serializer};
use tokio::sync::OnceCell;

/// Variable name to value; only non-empty values are kept.
pub type EnvMap = BTreeMap<String, String>;

/// Prefix of the lines the login-shell probe prints, so rc-file noise is ignored.
#[cfg(any(not(windows), test))]
const MARKER: &str = "__OFOX_ENV__";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentTool {
    Codex,
    Claude,
}

impl AgentTool {
    pub const ALL: [AgentTool; 2] = [AgentTool::Codex, AgentTool::Claude];

    pub fn env_var(self) -> &'static str {
        match self {
            AgentTool::Codex => "CODEX_HOME",
            AgentTool::Claude => "CLAUDE_CONFIG_DIR",
        }
    }

    fn default_dir_name(self) -> &'static str {
        match self {
            AgentTool::Codex => ".codex",
            AgentTool::Claude => ".claude",
        }
    }

    fn settings_override(self) -> Option<PathBuf> {
        match self {
            AgentTool::Codex => crate::settings::get_codex_override_dir(),
            AgentTool::Claude => crate::settings::get_claude_override_dir(),
        }
    }
}

/// Which of the sources decided the directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DirSource {
    Settings,
    Env,
    LoginShell,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDir {
    pub tool: AgentTool,
    #[serde(serialize_with = "serialize_path")]
    pub dir: PathBuf,
    pub source: DirSource,
    pub env_var: &'static str,
}

fn serialize_path<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.to_string_lossy())
}

/// The directory `tool` keeps its files in, from the first source that has one.
///
/// `settings_override` is what the user typed into Ofox's settings; the two maps hold
/// the variables of the Ofox process and of the login-shell snapshot.
pub fn resolve(
    tool: AgentTool,
    settings_override: Option<&Path>,
    process_env: &EnvMap,
    shell_env: &EnvMap,
    home: &Path,
) -> AgentDir {
    let var = tool.env_var();
    let (dir, source) = if let Some(dir) = settings_override.filter(|d| !d.as_os_str().is_empty()) {
        (dir.to_path_buf(), DirSource::Settings)
    } else if let Some(dir) = absolute_dir(process_env.get(var)) {
        (dir, DirSource::Env)
    } else if let Some(dir) = absolute_dir(shell_env.get(var)) {
        (dir, DirSource::LoginShell)
    } else {
        (home.join(tool.default_dir_name()), DirSource::Default)
    };
    AgentDir {
        tool,
        dir,
        source,
        env_var: var,
    }
}

/// An environment value as a directory: non-blank and absolute, else nothing.
fn absolute_dir(value: Option<&String>) -> Option<PathBuf> {
    let path = PathBuf::from(value?.trim());
    (!path.as_os_str().is_empty() && path.is_absolute()).then_some(path)
}

/// The variables the login-shell probe printed in `output`; everything else the
/// shell's rc files wrote is ignored. Empty values are dropped.
#[cfg(any(not(windows), test))]
pub fn parse_snapshot(output: &str) -> EnvMap {
    output
        .lines()
        .filter_map(|line| line.strip_prefix(MARKER))
        .filter_map(|pair| pair.split_once('='))
        .filter(|(name, value)| {
            AgentTool::ALL.iter().any(|tool| tool.env_var() == *name) && !value.trim().is_empty()
        })
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

// ---- the login-shell snapshot ----

/// A login shell that has not answered by now is left behind: the default directory
/// is used rather than keeping a bind waiting.
#[cfg(not(windows))]
const SNAPSHOT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Taken once per run; see [`ready`].
static LOGIN_SHELL_ENV: OnceCell<EnvMap> = OnceCell::const_new();
static NO_VARS: EnvMap = EnvMap::new();

/// Take the login-shell snapshot if it has not been taken yet. Awaited by every entry
/// point that reads or writes an agent's files, so they never see the directory
/// before the variable a terminal would have exported is known. Never fails: a shell
/// that cannot be run just leaves the snapshot empty.
pub async fn ready() {
    #[cfg(test)]
    READY_CALLS.with(|calls| calls.set(calls.get() + 1));
    LOGIN_SHELL_ENV.get_or_init(load_login_shell_env).await;
}

#[cfg(test)]
thread_local! {
    /// How often [`ready`] was awaited on this thread, so a test can check that an
    /// entry point waits for the snapshot (a `#[tokio::test]` runs on one thread).
    static READY_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The snapshot, or nothing before [`ready`] has finished.
fn login_shell_env() -> &'static EnvMap {
    LOGIN_SHELL_ENV.get().unwrap_or(&NO_VARS)
}

async fn load_login_shell_env() -> EnvMap {
    // A test (or a run pointed at a throwaway home) must not read the developer's own
    // shell rc files: they would leak their CODEX_HOME into it.
    if cfg!(test) || crate::config::is_home_overridden() {
        return EnvMap::new();
    }
    snapshot().await
}

/// The shell command that prints each agent variable on a marker line. The leading
/// newline keeps a marker off the end of an rc file's unterminated output.
#[cfg(not(windows))]
fn probe_script() -> String {
    let args: Vec<String> = AgentTool::ALL
        .iter()
        .map(|tool| format!("{var} \"${var}\"", var = tool.env_var()))
        .collect();
    format!("printf \"\\n{MARKER}%s=%s\" {}", args.join(" "))
}

/// Ask the user's login shell, the way `launch_tool` and `tool_update::probe` do, so a
/// variable exported in `.zshrc` / `.bash_profile` is seen.
#[cfg(not(windows))]
async fn snapshot() -> EnvMap {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    snapshot_from_shell(&shell, SNAPSHOT_TIMEOUT).await
}

#[cfg(not(windows))]
async fn snapshot_from_shell(shell: &str, timeout: std::time::Duration) -> EnvMap {
    let mut command = tokio::process::Command::new(shell);
    command.args(["-l", "-i", "-c", &probe_script()]);
    match crate::commands::tool_update::bounded_output_with_timeout(command, timeout).await {
        Ok(output) => {
            if !output.status.success() {
                log::warn!(
                    "[agent_env] login shell exited with {} while reading the agent variables",
                    output.status
                );
            }
            // Interactive rc files write to stderr (fnm's "Using Node ..."); only the
            // marker lines of stdout are read.
            parse_snapshot(&String::from_utf8_lossy(&output.stdout))
        }
        Err(error) => {
            log::warn!("[agent_env] could not read the agent variables from {shell}: {error}");
            EnvMap::new()
        }
    }
}

/// Windows has no login shell: a terminal opened now starts from the persistent
/// environment in the registry, the user's over the machine's.
#[cfg(windows)]
async fn snapshot() -> EnvMap {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE};
    use winreg::{RegKey, HKEY};

    let open = |root: HKEY, subkey: &str| {
        RegKey::predef(root)
            .open_subkey_with_flags(subkey, KEY_QUERY_VALUE)
            .ok()
    };
    let user = open(HKEY_CURRENT_USER, "Environment");
    let machine = open(
        HKEY_LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
    );
    let read = |key: &Option<RegKey>, name: &str| {
        key.as_ref()
            .and_then(|key| key.get_value::<String, _>(name).ok())
    };
    registry_snapshot(
        |name| read(&user, name),
        |name| read(&machine, name),
        |name| std::env::var(name).ok(),
    )
}

/// The agent variables of the registry's environment: the user's value beats the
/// machine's, and `%NAME%` references are expanded with `lookup`.
#[cfg(any(windows, test))]
fn registry_snapshot(
    user: impl Fn(&str) -> Option<String>,
    machine: impl Fn(&str) -> Option<String>,
    lookup: impl Fn(&str) -> Option<String>,
) -> EnvMap {
    AgentTool::ALL
        .iter()
        .filter_map(|tool| {
            let name = tool.env_var();
            [user(name), machine(name)]
                .into_iter()
                .flatten()
                .map(|raw| crate::commands::windows_tools::expand_env(&raw, &lookup))
                .find(|value| !value.trim().is_empty())
                .map(|value| (name.to_string(), value))
        })
        .collect()
}

// ---- the live answer ----

/// The directory `tool` keeps its files in right now: Ofox's settings, the process
/// environment, the login-shell snapshot (once [`ready`]), else the default.
pub fn dir(tool: AgentTool) -> AgentDir {
    resolve_with(tool, login_shell_env())
}

/// Both agents' directories, for the renderer.
pub fn dirs() -> Vec<AgentDir> {
    AgentTool::ALL.into_iter().map(dir).collect()
}

fn resolve_with(tool: AgentTool, shell_env: &EnvMap) -> AgentDir {
    let process_env: EnvMap = std::env::var(tool.env_var())
        .map(|value| (tool.env_var().to_string(), value))
        .into_iter()
        .collect();
    resolve(
        tool,
        tool.settings_override().as_deref(),
        &process_env,
        shell_env,
        &crate::config::get_home_dir(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> EnvMap {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    fn abs(name: &str) -> String {
        std::env::temp_dir()
            .join(name)
            .to_string_lossy()
            .into_owned()
    }

    fn resolve_codex(settings: Option<&Path>, process: &EnvMap, shell: &EnvMap) -> AgentDir {
        resolve(AgentTool::Codex, settings, process, shell, Path::new("/h"))
    }

    #[test]
    fn nothing_set_falls_back_to_the_default_directory_in_home() {
        let got = resolve_codex(None, &env(&[]), &env(&[]));
        assert_eq!(got.dir, Path::new("/h").join(".codex"));
        assert_eq!(got.source, DirSource::Default);
        assert_eq!(got.env_var, "CODEX_HOME");

        let claude = resolve(
            AgentTool::Claude,
            None,
            &env(&[]),
            &env(&[]),
            Path::new("/h"),
        );
        assert_eq!(claude.dir, Path::new("/h").join(".claude"));
        assert_eq!(claude.env_var, "CLAUDE_CONFIG_DIR");
    }

    #[test]
    fn settings_override_wins_over_every_environment_source() {
        let (settings, process, shell) = (abs("s"), abs("p"), abs("l"));
        let got = resolve_codex(
            Some(Path::new(&settings)),
            &env(&[("CODEX_HOME", &process)]),
            &env(&[("CODEX_HOME", &shell)]),
        );
        assert_eq!(got.dir, PathBuf::from(&settings));
        assert_eq!(got.source, DirSource::Settings);
    }

    #[test]
    fn process_env_wins_over_the_login_shell() {
        let (process, shell) = (abs("p"), abs("l"));
        let got = resolve_codex(
            None,
            &env(&[("CODEX_HOME", &process)]),
            &env(&[("CODEX_HOME", &shell)]),
        );
        assert_eq!(got.dir, PathBuf::from(&process));
        assert_eq!(got.source, DirSource::Env);
    }

    #[test]
    fn login_shell_is_used_when_the_process_has_nothing() {
        let shell = abs("l");
        let got = resolve_codex(None, &env(&[]), &env(&[("CODEX_HOME", &shell)]));
        assert_eq!(got.dir, PathBuf::from(&shell));
        assert_eq!(got.source, DirSource::LoginShell);
    }

    #[test]
    fn a_relative_process_value_is_ignored_and_the_next_source_is_used() {
        let shell = abs("l");
        let got = resolve_codex(
            None,
            &env(&[("CODEX_HOME", "relative/codex")]),
            &env(&[("CODEX_HOME", &shell)]),
        );
        assert_eq!(got.dir, PathBuf::from(&shell));
        assert_eq!(got.source, DirSource::LoginShell);
    }

    #[test]
    fn a_relative_shell_value_falls_back_to_the_default() {
        let got = resolve_codex(None, &env(&[]), &env(&[("CODEX_HOME", "~/codex")]));
        assert_eq!(got.source, DirSource::Default);
    }

    #[test]
    fn empty_and_blank_values_are_ignored() {
        let shell = abs("l");
        let got = resolve_codex(
            None,
            &env(&[("CODEX_HOME", "")]),
            &env(&[("CODEX_HOME", &shell)]),
        );
        assert_eq!(got.source, DirSource::LoginShell);

        let blank = resolve_codex(None, &env(&[("CODEX_HOME", "   ")]), &env(&[]));
        assert_eq!(blank.source, DirSource::Default);
    }

    #[test]
    fn each_tool_reads_only_its_own_variable() {
        let value = abs("claude");
        let got = resolve_codex(None, &env(&[("CLAUDE_CONFIG_DIR", &value)]), &env(&[]));
        assert_eq!(got.source, DirSource::Default);

        let claude = resolve(
            AgentTool::Claude,
            None,
            &env(&[("CLAUDE_CONFIG_DIR", &value)]),
            &env(&[]),
            Path::new("/h"),
        );
        assert_eq!(claude.dir, PathBuf::from(&value));
        assert_eq!(claude.source, DirSource::Env);
    }

    #[test]
    fn an_agent_dir_serializes_for_the_renderer() {
        let dir = resolve_codex(None, &env(&[]), &env(&[("CODEX_HOME", &abs("l"))]));
        let json = serde_json::to_value(&dir).unwrap();
        assert_eq!(json["tool"], "codex");
        assert_eq!(json["source"], "loginShell");
        assert_eq!(json["envVar"], "CODEX_HOME");
        assert_eq!(json["dir"], abs("l"));
    }

    #[test]
    fn the_snapshot_parser_keeps_only_marker_lines() {
        let output = "Using Node v22.1.0\n\
            \n\
            __OFOX_ENV__CODEX_HOME=/data/codex\n\
            __OFOX_ENV__CLAUDE_CONFIG_DIR=\n\
            some rc banner CODEX_HOME=/not/this\n\
            __OFOX_ENV__UNRELATED=/x\n";
        let parsed = parse_snapshot(output);
        assert_eq!(parsed, env(&[("CODEX_HOME", "/data/codex")]));
    }

    #[test]
    fn a_marker_after_unterminated_noise_is_still_found() {
        let parsed = parse_snapshot("no newline here\n__OFOX_ENV__CLAUDE_CONFIG_DIR=/c\r\n");
        assert_eq!(parsed, env(&[("CLAUDE_CONFIG_DIR", "/c")]));
    }

    #[test]
    fn a_value_may_contain_an_equals_sign() {
        let parsed = parse_snapshot("__OFOX_ENV__CODEX_HOME=/a=b\n");
        assert_eq!(parsed, env(&[("CODEX_HOME", "/a=b")]));
    }

    #[cfg(not(windows))]
    #[test]
    fn the_probe_script_asks_for_every_agent_variable() {
        let script = probe_script();
        assert!(script.contains("CODEX_HOME \"$CODEX_HOME\""), "{script}");
        assert!(
            script.contains("CLAUDE_CONFIG_DIR \"$CLAUDE_CONFIG_DIR\""),
            "{script}"
        );
    }

    #[test]
    fn the_registry_snapshot_prefers_the_user_value_and_expands_references() {
        let user = |name: &str| (name == "CODEX_HOME").then(|| r"%DATA%\codex".to_string());
        let machine = |name: &str| match name {
            "CODEX_HOME" => Some(r"C:\machine\codex".to_string()),
            "CLAUDE_CONFIG_DIR" => Some(r"C:\machine\claude".to_string()),
            _ => None,
        };
        let lookup = |name: &str| (name == "DATA").then(|| r"D:\data".to_string());

        let got = registry_snapshot(user, machine, lookup);

        assert_eq!(
            got,
            env(&[
                ("CODEX_HOME", r"D:\data\codex"),
                ("CLAUDE_CONFIG_DIR", r"C:\machine\claude"),
            ])
        );
    }

    #[test]
    fn a_blank_registry_user_value_defers_to_the_machine_value() {
        let user = |name: &str| (name == "CODEX_HOME").then(|| "  ".to_string());
        let machine = |name: &str| (name == "CODEX_HOME").then(|| r"C:\m".to_string());

        let got = registry_snapshot(user, machine, |_| None);

        assert_eq!(got, env(&[("CODEX_HOME", r"C:\m")]));
    }

    #[cfg(unix)]
    mod login_shell {
        use super::*;
        use crate::test_sandbox::sandbox;
        use serial_test::serial;
        use std::time::Duration;

        /// `/bin/sh -l` reads `~/.profile`, which stands in for the user's rc file: it
        /// prints noise on both streams and exports one of the two variables.
        fn write_profile(home: &Path, body: &str) {
            std::fs::write(home.join(".profile"), body).expect("write .profile");
        }

        #[tokio::test]
        #[serial]
        async fn the_login_shell_snapshot_reads_the_exported_variable_and_ignores_rc_noise() {
            let sandbox = sandbox();
            write_profile(
                sandbox.home(),
                "echo 'Using Node v22.1.0' >&2\n\
                 echo 'welcome back, CODEX_HOME=/not/this'\n\
                 printf 'unterminated'\n\
                 export CODEX_HOME=/data/codex\n",
            );

            let got = snapshot_from_shell("/bin/sh", Duration::from_secs(10)).await;

            assert_eq!(got, env(&[("CODEX_HOME", "/data/codex")]));
        }

        #[tokio::test]
        #[serial]
        async fn a_login_shell_that_exports_nothing_gives_an_empty_snapshot() {
            let sandbox = sandbox();
            write_profile(sandbox.home(), "echo hello\n");

            let got = snapshot_from_shell("/bin/sh", Duration::from_secs(10)).await;

            assert!(got.is_empty(), "{got:?}");
        }

        #[tokio::test]
        #[serial]
        async fn a_login_shell_that_hangs_is_given_up_on() {
            let sandbox = sandbox();
            write_profile(sandbox.home(), "sleep 3\nexport CODEX_HOME=/late\n");

            let started = std::time::Instant::now();
            let got = snapshot_from_shell("/bin/sh", Duration::from_millis(200)).await;

            assert!(got.is_empty(), "{got:?}");
            assert!(started.elapsed() < Duration::from_secs(2));
        }

        #[tokio::test]
        async fn a_shell_that_cannot_be_started_gives_an_empty_snapshot() {
            let got = snapshot_from_shell("/nonexistent/shell", Duration::from_secs(1)).await;

            assert!(got.is_empty());
        }
    }

    mod live {
        use super::*;
        use crate::test_sandbox::sandbox;
        use serial_test::serial;

        fn set_codex_override(dir: &Path) {
            let mut settings = crate::settings::get_settings();
            settings.codex_config_dir = Some(dir.to_string_lossy().into_owned());
            crate::settings::update_settings(settings).expect("save the override");
        }

        #[test]
        #[serial]
        fn the_default_directory_is_below_the_home() {
            let sandbox = sandbox();

            let got = resolve_with(AgentTool::Codex, &env(&[]));

            assert_eq!(got.dir, sandbox.home().join(".codex"));
            assert_eq!(got.source, DirSource::Default);
        }

        #[test]
        #[serial]
        fn the_process_environment_moves_the_directory() {
            let _sandbox = sandbox();
            let moved = std::env::temp_dir().join("moved-codex");
            std::env::set_var("CODEX_HOME", &moved);

            let got = resolve_with(AgentTool::Codex, &env(&[]));

            assert_eq!(got.dir, moved);
            assert_eq!(got.source, DirSource::Env);
        }

        #[test]
        #[serial]
        fn the_shell_snapshot_is_used_when_the_process_has_no_variable() {
            let _sandbox = sandbox();
            let shell = abs("from-shell");

            let got = resolve_with(AgentTool::Claude, &env(&[("CLAUDE_CONFIG_DIR", &shell)]));

            assert_eq!(got.dir, PathBuf::from(&shell));
            assert_eq!(got.source, DirSource::LoginShell);
        }

        #[test]
        #[serial]
        fn the_settings_override_beats_the_environment() {
            let _sandbox = sandbox();
            let typed = std::env::temp_dir().join("typed-codex");
            set_codex_override(&typed);
            std::env::set_var("CODEX_HOME", std::env::temp_dir().join("moved-codex"));

            let got = resolve_with(AgentTool::Codex, &env(&[]));

            assert_eq!(got.dir, typed);
            assert_eq!(got.source, DirSource::Settings);
        }

        #[test]
        #[serial]
        fn a_relative_process_variable_is_not_followed() {
            let sandbox = sandbox();
            std::env::set_var("CODEX_HOME", "relative/codex");

            let got = resolve_with(AgentTool::Codex, &env(&[]));

            assert_eq!(got.dir, sandbox.home().join(".codex"));
        }

        #[test]
        #[serial]
        fn dirs_lists_codex_then_claude() {
            let _sandbox = sandbox();

            let tools: Vec<_> = dirs().into_iter().map(|d| d.tool).collect();

            assert_eq!(tools, [AgentTool::Codex, AgentTool::Claude]);
        }
    }

    /// Ofox writes where the agent reads: with the variable set, a bind and an unbind
    /// go through the moved directory and the default one is never created.
    mod moved_directory {
        use crate::app_config::AppType;
        use crate::database::Database;
        use crate::provider::Provider;
        use crate::services::ofox_bind::{bind, unbind, Tool};
        use crate::test_sandbox::sandbox;
        use serde_json::json;
        use serial_test::serial;

        const KEY: &str = "sk-of-TEST";
        const USER_CONFIG: &str = "# my codex setup\nmodel = \"gpt-5-codex\"\n\n[mcp_servers.docs]\ncommand = \"docs-mcp\"\n";
        const CODEX_TEMPLATE: &str = "model_provider = \"ofox\"\nmodel = \"openai/gpt-6-luna\"\n\n[model_providers.ofox]\nname = \"ofox\"\nbase_url = \"https://api.ofox.ai/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n";

        fn db_with_template(app: AppType, id: &str, settings: serde_json::Value) -> Database {
            let db = Database::memory().expect("db");
            db.save_provider(
                app.as_str(),
                &Provider::with_id(id.into(), "OfoxAI".into(), settings, None),
            )
            .expect("save the template");
            db
        }

        #[tokio::test]
        #[serial]
        async fn codex_is_bound_and_unbound_in_codex_home() {
            let sandbox = sandbox();
            let moved = sandbox.home().join("data").join("codex-state");
            std::fs::create_dir_all(&moved).unwrap();
            std::fs::write(moved.join("config.toml"), USER_CONFIG).unwrap();
            std::env::set_var("CODEX_HOME", &moved);
            let db = db_with_template(
                AppType::Codex,
                "ofox-codex",
                json!({ "auth": { "OPENAI_API_KEY": "" }, "config": CODEX_TEMPLATE }),
            );

            bind(&db, Tool::Codex, "codex", KEY).await.expect("bind");
            let bound = std::fs::read_to_string(moved.join("config.toml")).unwrap();
            assert!(bound.contains(KEY), "the key is written under CODEX_HOME");
            assert!(
                !sandbox.home().join(".codex").exists(),
                "~/.codex is not created"
            );

            unbind(&db, Tool::Codex, "codex", &[], false)
                .await
                .expect("unbind");

            assert_eq!(
                std::fs::read_to_string(moved.join("config.toml")).unwrap(),
                USER_CONFIG,
                "the file under CODEX_HOME is restored"
            );
            assert!(!sandbox.home().join(".codex").exists());
        }

        #[tokio::test]
        #[serial]
        async fn claude_is_bound_and_unbound_in_claude_config_dir() {
            let sandbox = sandbox();
            let moved = sandbox.home().join("data").join("claude-state");
            std::fs::create_dir_all(&moved).unwrap();
            let original = "{\n  \"model\": \"opus\"\n}\n";
            std::fs::write(moved.join("settings.json"), original).unwrap();
            std::env::set_var("CLAUDE_CONFIG_DIR", &moved);
            let db = db_with_template(
                AppType::Claude,
                "ofox-claude",
                json!({ "env": {
                    "ANTHROPIC_BASE_URL": "https://api.ofox.ai/anthropic",
                    "ANTHROPIC_AUTH_TOKEN": ""
                } }),
            );

            bind(&db, Tool::Claude, "claude", KEY).await.expect("bind");
            let bound = std::fs::read_to_string(moved.join("settings.json")).unwrap();
            assert!(
                bound.contains(KEY),
                "the key is written under CLAUDE_CONFIG_DIR"
            );
            assert!(
                !sandbox.home().join(".claude").exists(),
                "~/.claude is not created"
            );

            unbind(&db, Tool::Claude, "claude", &[], false)
                .await
                .expect("unbind");

            assert_eq!(
                std::fs::read_to_string(moved.join("settings.json")).unwrap(),
                original
            );
            assert!(!sandbox.home().join(".claude").exists());
        }
    }

    /// Every entry point that reads or writes an agent's files waits for the login-shell
    /// snapshot before it looks at the directory.
    mod gates {
        use super::READY_CALLS;
        use crate::database::Database;
        use crate::services::ofox_bind::{self, Tool};
        use crate::test_sandbox::sandbox;
        use serde_json::json;
        use serial_test::serial;
        use std::future::Future;

        /// Runs `call` and reports whether `ready()` was awaited meanwhile.
        async fn awaits_ready<T>(call: impl Future<Output = T>) -> bool {
            let before = READY_CALLS.with(|calls| calls.get());
            call.await;
            READY_CALLS.with(|calls| calls.get()) > before
        }

        #[tokio::test]
        #[serial]
        async fn the_bind_operations_wait_for_the_snapshot() {
            let _sandbox = sandbox();
            let db = Database::memory().expect("db");
            let key = "sk-of-TEST";

            assert!(awaits_ready(ofox_bind::bind(&db, Tool::Codex, "codex", key)).await);
            assert!(awaits_ready(ofox_bind::bind_existing(&db, Tool::Codex, "codex", key)).await);
            assert!(
                awaits_ready(ofox_bind::unbind(&db, Tool::Codex, "codex", &[], true)).await,
                "unbind"
            );
            assert!(
                awaits_ready(ofox_bind::restore_missing_binding(
                    &db,
                    Tool::Codex,
                    "codex",
                    &[],
                    key
                ))
                .await,
                "restore_missing_binding"
            );
            assert!(
                awaits_ready(ofox_bind::replace_key(&db, Tool::Codex, "old", "new")).await,
                "replace_key"
            );
            assert!(
                awaits_ready(ofox_bind::persist_bound_settings(
                    &db,
                    Tool::Codex,
                    key,
                    &json!({}),
                    &json!({})
                ))
                .await,
                "persist_bound_settings"
            );
            assert!(
                awaits_ready(ofox_bind::sync_live_model(&db, Tool::Codex, Some(key))).await,
                "sync_live_model"
            );
            assert!(
                awaits_ready(ofox_bind::migrate_codex_on_startup()).await,
                "migrate_codex_on_startup"
            );
        }

        #[tokio::test]
        #[serial]
        async fn the_status_readers_wait_for_the_snapshot() {
            let _sandbox = sandbox();
            let db = Database::memory().expect("db");

            assert!(
                awaits_ready(ofox_bind::status::binding_status(&db, Tool::Claude, None)).await,
                "binding_status"
            );
            assert!(
                awaits_ready(ofox_bind::model_state::snapshot(
                    &db,
                    Tool::Claude,
                    "claude",
                    None
                ))
                .await,
                "model_state::snapshot"
            );
        }

        #[tokio::test]
        #[serial]
        async fn endpoint_reconcile_and_the_config_commands_wait_for_the_snapshot() {
            let _sandbox = sandbox();
            let db = Database::memory().expect("db");

            assert!(
                awaits_ready(crate::commands::ofox_apex::reconcile_tool_endpoints(&db)).await,
                "reconcile_tool_endpoints"
            );
            assert!(awaits_ready(crate::commands::get_agent_dirs()).await);
            assert!(awaits_ready(crate::commands::get_config_dir("codex".into())).await);
            assert!(awaits_ready(crate::commands::list_sessions()).await);
            assert!(awaits_ready(crate::commands::get_claude_config_status()).await);
        }

        #[tokio::test]
        #[serial]
        async fn the_skill_service_waits_for_the_snapshot_before_it_writes_app_dirs() {
            let _sandbox = sandbox();
            let db = std::sync::Arc::new(Database::memory().expect("db"));
            let service = crate::services::skill::SkillService::new();

            assert!(awaits_ready(service.update_skill(&db, "missing")).await);
        }
    }
}
