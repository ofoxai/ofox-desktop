# Known limitations and prior review dispositions

This ledger preserves the accepted non-blocking findings from the implementation review. Deferred means known and not claimed fixed. It is not a claim that these paths were validated on Windows. Windows native execution and installed-agent version checks remain separate acceptance work.

## Resolved follow-ups

- T7 menu selection, Tab focus, popup semantics, collision boundary, highlight stability, narrow menu, unknown TTL and aria-controls: resolved in T18 with regression tests.
- T8 account reply and stale balance toast: resolved in T9/T14, including StrictMode and consecutive balance samples.
- T8 initial settings health: T18 reads on mount after subscribing; reset needs confirmation, unreadable is danger, recovery is dismissible.
- T5 unbind preservation copy: narrowed in all three languages; production wiring is documented accurately.
- T13 tiny-slope run-out overflow: resolved by e26629f7. Bound-only unsupported tools, first-visit balance refresh and millisecond timestamps are implemented in T14.
- T15 stillBound/local binding sync and progress correlation: implemented in T16; remaining-copy partial outcomes are covered by regression tests.

## Accepted deferred findings

The following entries retain the specific review observations. Some contain several closely related subitems; each remains deferred except the explicit resolved follow-ups above.

1. **Task 1 — resolved by T17:** CODEX_HOME/CLAUDE_CONFIG_DIR readers and directory precedence now share the resolver. The current config comment explicitly documents unsupported override variables.
2. **Task 1 — deferred:** home_dir_guard.rs hand-rolled lexer 512 lines (br"" strings, EOF index panic risk, magic files.len()>50)
3. **Task 1 — deferred:** env_checker map/chain rewrite convoluted; drop(_guard) naming; windows-probe.yml header comment stale; CI concurrency head_ref collisions for forks
4. **Task 1 — deferred:** commits c4cb9cab/58924883 >400 lines — check bodies say what was run
5. **Task 1 — deferred:** test_sandbox module compiled into the shipped lib crate (dead code in app)
6. **Task 2 — deferred:** read-only symlink targets (nix/home-manager) now fail bind with EACCES instead of replacing the link
7. **Task 2 — deferred:** backup.rs:320 set_file_mode(backup_dir)? is a hard error inside migration/restore while DB chmod only warns
8. **Task 2 — deferred:** init_leaves_every_sqlite_file_it_made_owner_only nearly redundant; write_json_file/write_text_file redundant create_dir_all; no symlink-loop test; config.rs +700 lines (atomic/FileTxn could be own module); d16f19c7 bundles two goals
9. **Task 2 — deferred:** capture_baseline (ofox_bind/mod.rs:198) plain read could see a spurious None in the Windows rename window → original=None; consider the waiting read there
10. **Task 3 — deferred:** UTF-8 BOM not stripped in settings_json_valid (magpie lastgood.JSON strips it) — Windows Notepad/PS edits become unreadable
11. **Task 3 — deferred:** reset_unreadable acts on stale health without re-reading (transient Io pins unreadable; repaired file moved aside)
12. **Task 3 — deferred:** new Chinese literals in reset_unreadable errors (settings.rs ~777/784) violate global constraint
13. **Task 3 — deferred:** unreachable Err arm + double parse (settings.rs:646); reload_settings first-access double load hides Recovered; startup-event timing resolved by the T18 mount query; test gaps (Recovered without bad copy, Io path)
14. **Task 6 — deferred:** Windows thumb color ineffective with scrollbar-width:thin (command.tsx:54,57); uncontrolled indeterminate checkbox draws tick; ConfirmDialog `details &&` can render 0; danger button mixes RGB bg + HSL text; no exit animations
15. **Task 17 — deferred:** The original forever-cached failure was superseded by the retry/cached-startup implementation (see item 20). Remaining: no UI signal and possible startup stalls; /bin/zsh fallback when SHELL unset (Linux); misleading "Version probe timed out" text; config command readiness and gate-removal regression resolved in T17/integration (agent_env tests); remaining manage_tool readers are not all gated
16. **Task 15 — deferred:** key from another account counted revoked without usable_by check (retire_key_with ~896); -y-only Hermes help passes --yes; preview acquires the real guard (spurious busy); preview says revoke for already-stopped key; symlinked ChatGPT.app bundle_in_use miss; concurrent Codex+ChatGPT uninstalls can orphan shared key; manual_plan test never checks real-home native plans
17. **Task 4 — deferred:** over-scrubbing of *_tokens counts and prose ("invalid password: try again"); default panic hook still prints raw message to stderr (panic_hook.rs:189); AppError::Toml keeps Box<toml::de::Error> source whose Debug includes raw doc; unknown-variant serde errors echo values; workbuddy_config {e} errors
18. **Task 17 — deferred:** claude_mcp.rs error texts still say "~/.claude.json" (202/228/440); CLAUDE_CONFIG_DIR users with MCP entries only in ~/.claude.json see an empty MCP list (no migration)
19. **Task 4 — deferred:** commands/config.rs:31 invalid_toml_format_error raw {error}; provider/live.rs, mcp/*, deeplink/provider.rs raw parse errors; TOML multi-line secret not scrubbed; default panic hook stderr; widened sk- rule may scrub long --sk- flags; CRLF \r swallowed
20. **Task 17 — deferred:** stale cached snapshot used for one launch until refresh (only logged; get_agent_dirs not pushed); retry stall up to 5s per 60s window when shell hangs and nothing cached; start()/refresh() wiring untested (needs running app)
21. **Task 15 — deferred:** PYTHONIOENCODING ineffective under python3 -I; home prefix replace rewrites sibling paths; stillBound omission resolved in T16
22. **Task 10 — deferred:** backfill overwrite drops archive contributions not in transcripts (use max); Codex subagent calls from last 30 days pre-upgrade stay missing; OpenCode watermark moved before upsert; Gemini guard positional; old Codex rows count cache writes as input; migration runs even if pre-migration backup fails; Claude subagent output undercounted (note in UI/docs); OpenCode 2 session_v2 + $OPENCODE_DB not read
23. **Task 5 — deferred:** route leaves judged independently — kept user base URL can be paired with restored pre-bind credential of an older relay; new hard-coded Chinese error strings codex.rs:175, hermes.rs:134; stale written after downgrade→upgrade
24. **Task 15 — deferred:** scrub test 2nd assertion order-agnostic; custom HERMES_HOME name shows ~/.hermes in preview; extras not de-duplicated vs future dry-run
25. **Task 10 — deferred:** in-flight zero-token placeholder archived before next sync after >30d gap loses final tokens; DB watermark global (old time_updated imports never selected)
26. **Task 5 — deferred:** Unbind wording and test-helper documentation resolved in T18/T19. Remaining untested: Gemini .env missing route-restored, Hermes list→dict refill; Ofox-shaped top-level model can be kept under user relay if user picked it while bound
27. **Task 13 — deferred:** no exponent-form cost fixture; new Chinese error strings usage_stats.rs:975/1059/1142; priced heuristic (label "no price recorded"); bound-only unsupported display resolved in T14; tick ordering untested; overview reads under 5 lock acquisitions; duplicated test helpers; first-visit balance sampling resolved in T14

28. **Task 8 — deferred:** header aria-controls may reference an unmounted inactive panel. Traffic-light placement is a separate native-window check in the local delivery report.

## Acceptance boundaries

- The resource library manages verified global targets. Project/session overlays and OpenCode dynamic routing keys or unresolved Skills path references require an explicit supported adapter; they are blocked rather than treated as literal paths. OAuth still uses the native tool's authorization flow.
- New WebDAV skill snapshots carry immutable artifacts, hidden files, empty directories and portable execution intent. Old flat snapshots omitted some of that information; restoring their available content as drafts cannot reconstruct missing files or permissions. Native/legacy roots and existing artifacts remain intact.
- Resource compatibility rules were checked against OpenCode v1.19.0 and Hermes v0.22.0 source, including global precedence and Skills readers. This is source-contract verification, not a claim of native agent execution on every platform.

- Native Windows tests, UI/installer behavior and file-lock semantics remain pending until the exact delivery SHA is run with the Windows verification package. macOS type checking is not Windows execution.
- UI layout screenshots use synthetic accounts and session fixtures; they do not establish live backend compatibility. Native macOS window placement and installation are checked separately.
- No live uninstall or intentional damage to real settings is part of acceptance. Sandbox tests cover those destructive paths.
- Real-agent configuration compatibility is version-specific. The optional Codex strict-config probe is recorded by the Rust runner; other unavailable agent probes are not claimed passed.
- A prior manual probe left an empty app shell in system Trash. It is outside the source delivery and is not removed by these tests.

- Skill updates preserve damaged immutable objects that native links may still read. When a verified remote candidate has the same hash as an already damaged object, the current hash-addressed store cannot hold a second independent instance safely. The update UI reports `skillArtifactHashOccupied` and disables copy; it never replaces that object or claims automatic repair. A different-hash candidate can be saved as a separate draft. Back up local edits before any manual source repair.

- The compatibility `update_skill(id)` API resolves its tracked ref when that call starts; an earlier legacy `Vec<SkillUpdateInfo>` result does not freeze its commit. It checks its own revision/epoch and only updates the source draft. The resource-library update UI exclusively consumes a reviewed, fixed-SHA ticket.

- MCP checks inspect protocol and tools only. OAuth remains native; environment references use the OFox process environment. Package runners/shells and Windows command shims are blocked, while explicitly configured local programs retain user permissions and may perform their own I/O. Legacy HTTP sessions expire under server policy after transport cleanup. See [connection checks](mcp-connection-checks.md) for bounds and exact transport coverage; only local Node/HTTP fixtures have run, with Windows and real MCP vendor versions pending.

- The first optional-catalog automatic lifecycle slice is Copilot CLI on macOS arm64, using an existing compatible Node 22+ installation and its paired npm in an OFox-private prefix. It does not change PATH, adopt or remove an external copy, install Node, bind an account or configure Copilot resources. Other candidates and platforms retain official manual instructions. A paired npm with non-comment built-in configuration is rejected, so some customized runtime distributions require manual installation.

- Copilot lifecycle tests installed official 1.0.97, upgraded to 1.0.98 and removed only the temporary private copy with Node 22.12.0/npm 10.9.0; temporary settings, credentials and session sentinels remained unchanged. The native cancellation case cancels before npm spawn; running-process cancellation is covered by local process-tree fixtures. The opt-in native test constructs a frozen plan directly, so it does not certify Tauri/UI end-to-end behavior or Copilot instructions, MCP, Skills, authorization or AI execution.

- Private lifecycle operations recheck content and physical identity before mutation and before accepting success. They manage their own process group and retain an unresolved intent when cleanup or provenance cannot be verified. They are not an OS sandbox: another process with the user's permissions can change paths while npm runs, and a later mismatch cannot undo outside side effects. The older Terminal installer has a separate timeout/ownership boundary; this new chain does not fix it.

- Copilot is disclosed as a known potential reader of `~/.agents/skills`, not a library assignment target. Copilot CLI 1.0.98 `skill list --json` discovered private personal-Copilot and shared personal-Agents fixtures in a temporary HOME from an unrelated empty project; no skill script or AI request ran. Additional directories, disabled/ignored skills, `COPILOT_HOME` routing and project/plugin/session overrides are not fully resolved by the library, so the reader list is not an exhaustive actual-loading report. Copilot-specific instructions and MCP writes remain unsupported.
