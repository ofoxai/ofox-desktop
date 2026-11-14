# Known limitations and prior review dispositions

This ledger preserves the accepted non-blocking findings from the implementation review. Deferred means known and not claimed fixed. It is not a claim that these paths were validated on Windows. Windows native execution and installed-agent version checks remain separate acceptance work.

## Assistant command verification scope (2026-11-14)

Host-confirmed changes (`changed: true`) always require verification, including
partial changes and pending repairs restored from the host's task snapshot.
Commands with uncertain effects (`requiresVerification: true`) require tool
verification only when the run also inspects or operates on a supported tool
(status, diagnose, verify, launcher repair, rebind, install, update, uninstall or
model selection). The tool operation may occur before or after the command.
An ordinary command-assisted answer creates no pending verification state.
The Host command read-only allowlist remains unchanged.

Repair intent is not inferred from user prose or model-supplied intent strings.
A repair performed entirely through shell commands without any supported-tool
operation or Host-confirmed change therefore does not trigger automatic tool
verification. Enabling local configuration alone is not evidence of a repair.
Models must still inspect and verify tool repairs as instructed by the local
configuration Skill; this runtime gate cannot prove the effects of arbitrary
shell scripts. Existing pending snapshots retain their conservative verification
requirement, since they do not record whether a previous command was a lookup.

Regression fixtures exercise both Ofox apex choices and the Bash/PowerShell
runtime adapters on macOS. They do not establish native Windows execution or
acceptance in the installed desktop application.

## Assistant catalog cache (2026-10-30)

Assistant catalog caching is in memory. A restart opens an asynchronous cold
load rather than reusing disk metadata; healthy sources can arrive independently.
Automatic failure retry has a thirty-second cooldown and successful rows have a
ten-minute TTL. Explicit Retry is available, including after a valid empty list.
See [catalog behavior](assistant-catalog.md). macOS cold startup and region
switching were exercised in the temporary QA application; sandbox/browser
results do not validate Windows.

## Assistant request records (2026-10-30)

New requests preserve bounded gateway header identifiers, outcomes, usage and
frozen pricing evidence for inspection. Missing/legacy identifiers remain
unavailable, and an interrupted response may never publish a receipt. Search
currently exposes usage rather than a platform request ID. Estimates and returned
video costs remain separate from unverified actual charges; binary speech without
usage never proves billing. See [request records](assistant-request-records.md).
The approved macOS audio samples have [per-request invoice evidence](assistant-audio-acceptance.md).
Header request IDs differ from billing record IDs, so reconciliation uses the
same account, model and submission time rather than claiming an exact ID join.
Observed speech debits were lower than the catalog estimates; their cause is
unverified and a binary response still does not establish actual cost.

## Assistant audio foundation (2026-10-30)

The audio catalog, bounded import, recording and editable ASR draft controls
are implemented. Speech synthesis now has independent preview/parameter controls
and private binary publication. Stored output now has play/pause/stop and native
save controls without another model request. Keep original is an explicit,
initially-off ASR option; retained originals use private history attachments and
whole-conversation backups. Independent audio operations remain in their source
conversation when forking native chat history. Import
accepts short PCM RIFF WAV and Layer III MP3 only, mono/stereo at 8–48 kHz,
up to 25 MiB and five minutes. Float/extensible WAV, FLAC and Ogg input are
outside this first release. Unsubmitted inputs are temporary memory buffers,
excluded from chat attachments and backups. A retention failure keeps the
transcript and billing facts with a warning; it never resubmits the request.
Approved macOS microphone and dual-apex live samples were exercised; see
[acceptance scope and deviations](assistant-audio-acceptance.md). Speech previews start with the
reply source text, including Markdown, and can be edited before generation.
Ogg Opus output checks container framing/CRCs and duration; full Opus decoding
is device-dependent. Unsupported codecs offer save without paid conversion.
macOS WKWebView played and saved local WAV/MP3/Opus fixtures and real WAV results;
this does not validate every provider/codec or Windows. Stored audio history is available from the conversation menu even when the
model catalog or Pi runtime is unavailable; playback and save make no new model
request. See [audio design](assistant-audio.md).

## Assistant backup and restore (2026-10-28)

Complete ZIP backups are owner-scoped and preserve stored run prices. Restore
adds missing sessions, skips exact matches and refuses same-ID differences;
it does not merge or overwrite conversations. Valid imports remain when a later
publication fails. Archives are unencrypted, bounded and validated against the
pinned Ofox/Pi history format; arbitrary Pi CLI histories and SDK-only empty
compaction/root-branch sentinels are not supported. Known managed keys are
excluded, but arbitrary secrets pasted into documents cannot be inferred.
See [backup contract](assistant-backup.md) and [native acceptance](assistant-acceptance.md).
Subsequent [macOS native acceptance](assistant-native-acceptance.md) covers the
real Tauri controls, file dialogs and bundled runtime with local mock services.
Windows is deferred at the user’s request. Search prices remain estimates from
the saved catalog: the account balance endpoint cannot reconcile individual requests.

## Assistant saved failures and attachment edits (2026-10-28)

On `feat/assistant-chat`, new failed replies retain a localized reason across
restarts. Older records without `errorCode` remain readable but cannot recover a
reason that was never saved. Removing a file from an edited turn preserves the
original version and its file; it is not permanent deletion or storage cleanup.
This increment has macOS automated coverage and a 900 × 600 mock-backend renderer
check, not native Windows or real-gateway acceptance. S4's remaining scope and
verification evidence are recorded in [assistant progress](assistant-s4-progress.md).

## Assistant Office text extraction (2026-10-28)

DOCX reads body/table text only; headers, footers, footnotes/endnotes and comments
are not included. XLSX values are unformatted cached values; formulas are not
recalculated. PPTX includes slide text and notes in presentation order. Embedded
images, charts, objects and layouts are excluded in all three formats. Localized
text-only/partial warnings accompany model input and attachment chips. Legacy
`.doc` is deliberately unsupported after malformed-file assessment of
word-extractor 1.0.4; users are directed to docx or PDF. Bounds, license evidence,
fixtures and platform verification limits are in the [Office audit](assistant-office.md).

## Assistant conversation forks (2026-10-30)

Forks copy local native context and historical usage without calling a provider.
An unresolved video job must finish before that part of the conversation can be
forked. Files explicitly excluded from a backup remain missing in its forks.
Ordinary failures clean unpublished staging folders; a hard application kill can
leave an ignored private `.fork-*` folder, which is never shown as a conversation
or resumed as provider work. Fork and quote behavior has local mock-gateway and
renderer coverage on macOS; native Windows acceptance remains deferred.

## Request dashboard alignment

- Cache-inclusive totals apply to Requests only. Overview and session-analysis
  contracts retain their existing token meanings and pricing behavior.
- Requests hides output speed and provider presentation. Existing compatible
  measurement and provider fields remain in the IPC projection. This page
  change does not restore native timing inference; missing HTTP status or
  first-token evidence remains unknown.
- Renderer acceptance uses fictional IPC data at 620, 900 and 1280 pixels in
  zh/en/ja and both themes. It is not validation of Windows WebView rendering,
  new paid native traffic or a release. The separately authorized macOS build
  was signed, installed and checked against existing local history after the
  user completed system authorization; see the
  [installation record](usage-requests-alignment.md#subsequent-local-installation-2026-10-25).
  The previous app is retained in a verified backup; user configuration and
  history directories were not replaced.

## Resolved follow-ups

- T7 menu selection, Tab focus, popup semantics, collision boundary, highlight stability, narrow menu, unknown TTL and aria-controls: resolved in T18 with regression tests.
- T8 account reply and stale balance toast: resolved in T9/T14, including StrictMode and consecutive balance samples.
- T8 initial settings health: T18 reads on mount after subscribing; reset needs confirmation, unreadable is danger, recovery is dismissible.
- T5 unbind preservation copy: narrowed in all three languages; production wiring is documented accurately.
- T13 tiny-slope run-out overflow: resolved by e26629f7. Bound-only unsupported tools, first-visit balance refresh and millisecond timestamps are implemented in T14.
- T15 stillBound/local binding sync and progress correlation: implemented in T16; remaining-copy partial outcomes are covered by regression tests.

## Accepted deferred findings

- The usage workbench custom date fields now use the app's local-calendar popover and editable `YYYY-MM-DD` text. Arrow keys, Home/End, Page Up/Down and Shift + Page Up/Down cover day/week/month/year navigation; Escape restores trigger focus. Local dates, leap years, reversed ranges, cancel/reopen and query boundaries have regression coverage. The UI was checked at 900px in light and dark browser previews on macOS; that preview does not certify a native Windows date picker run.

- Manufacturer API price fallback uses first-party provider entries from models.dev with a bundled 2026-10-21 snapshot. Refresh failure retains verified cached prices; this environment returned HTTP 403 from that public feed, so a live catalog download was not validated. Unknown/custom model aliases and unsupported billing dimensions remain unpriced or partial. These are current list-price estimates, not historical invoices. See [price fallback](usage-official-price-fallback.md).

The following entries retain the specific review observations. Some contain several closely related subitems; each remains deferred except the explicit resolved follow-ups above.

1. **Task 1 — resolved by T17:** CODEX_HOME/CLAUDE_CONFIG_DIR readers and directory precedence now share the resolver. The current config comment explicitly documents unsupported override variables.
2. **Task 1 — deferred:** home_dir_guard.rs hand-rolled lexer 512 lines (br"" strings, EOF index panic risk, magic files.len()>50)
3. **Task 1 — deferred:** env_checker map/chain rewrite convoluted; drop(\_guard) naming; windows-probe.yml header comment stale; CI concurrency head_ref collisions for forks
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
17. **Task 4 — deferred:** over-scrubbing of \*\_tokens counts and prose ("invalid password: try again"); default panic hook still prints raw message to stderr (panic_hook.rs:189); AppError::Toml keeps Box<toml::de::Error> source whose Debug includes raw doc; unknown-variant serde errors echo values; workbuddy_config {e} errors
18. **Task 17 — deferred:** claude_mcp.rs error texts still say "~/.claude.json" (202/228/440); CLAUDE_CONFIG_DIR users with MCP entries only in ~/.claude.json see an empty MCP list (no migration)
19. **Task 4 — deferred:** commands/config.rs:31 invalid_toml_format_error raw {error}; provider/live.rs, mcp/\*, deeplink/provider.rs raw parse errors; TOML multi-line secret not scrubbed; default panic hook stderr; widened sk- rule may scrub long --sk- flags; CRLF \r swallowed
20. **Task 17 — deferred:** stale cached snapshot used for one launch until refresh (only logged; get_agent_dirs not pushed); retry stall up to 5s per 60s window when shell hangs and nothing cached; start()/refresh() wiring untested (needs running app)
21. **Task 15 — deferred:** PYTHONIOENCODING ineffective under python3 -I; home prefix replace rewrites sibling paths; stillBound omission resolved in T16
22. **Task 10 — deferred:** historical archive contributions cannot be verified event by event. Unsafe automatic archive reconstruction is stopped by the cache foundation described below; this does not repair historical rows. Codex subagent calls from last 30 days pre-upgrade stay missing; OpenCode watermark-before-upsert resolved by the first U-A atomic batch slice; Gemini guard positional; old Codex rows count cache writes as input; migration runs even if pre-migration backup fails; Claude subagent output undercounted (note in UI/docs); OpenCode 2 session_v2 + $OPENCODE_DB not read
23. **Task 5 — deferred:** route leaves judged independently — kept user base URL can be paired with restored pre-bind credential of an older relay; new hard-coded Chinese error strings codex.rs:175, hermes.rs:134; stale written after downgrade→upgrade
24. **Task 15 — deferred:** scrub test 2nd assertion order-agnostic; custom HERMES_HOME name shows ~/.hermes in preview; extras not de-duplicated vs future dry-run
25. **Task 10 — partly resolved:** changed OpenCode databases now enumerate all native messages rather than relying on a global update watermark, including zero-token messages completed with an older `time_updated`. Previously archived contributions without event-level evidence remain unverified and are not reconstructed automatically.
26. **Task 5 — deferred:** Unbind wording and test-helper documentation resolved in T18/T19. Remaining untested: Gemini .env missing route-restored, Hermes list→dict refill; Ofox-shaped top-level model can be kept under user relay if user picked it while bound
27. **Task 13 — deferred:** no exponent-form cost fixture; new Chinese error strings usage_stats.rs:975/1059/1142; priced heuristic (label "no price recorded"); bound-only unsupported display resolved in T14; tick ordering untested; overview reads under 5 lock acquisitions; duplicated test helpers; first-visit balance sampling resolved in T14

28. **Task 8 — deferred:** header aria-controls may reference an unmounted inactive panel. Traffic-light placement is a separate native-window check in the local delivery report.

## Acceptance boundaries

- Legacy v11 archive backfill no longer infers missing contributions by subtracting live totals from newly parsed transcripts. Its aggregates have no event identities and may contain proxy contributions. The maintenance step preserves existing archives, reads no transcript body and atomically records `partial` with an explicit reason; this is not `done` and does not run again on every refresh. Some old session archives that could previously be reconstructed now require a separately reviewed indexing/reconciliation path and explicit requeue. Their individual historical contributions are unverified, not established as missing or duplicated. Normal source synchronization and transactional pruning remain responsible for newly verified facts. The separate legacy Claude cost recomputation policy remains unchanged.

- This cache/parser foundation is a dependency savepoint, not completed usage calibration. Changed OpenCode databases are fully enumerated; JSON receipts also depend on a canonical session-parent graph, including missing legacy dependency receipts. Owned live attribution can update, while a changed ancestry graph for an already archived native event records `historicalSessionAncestryUnverified` and leaves its archive intact. Existing live rows with unknown timestamp/identity provenance and archived attribution are not automatically repaired.

- Missing or invalid native timestamps stay unknown in normalized facts. Claude, Codex and Gemini now skip those ledger writes and retain partial source coverage instead of manufacturing today's date; a later valid native timestamp can follow the ordinary deduplication path. OpenCode already skips undated facts. Previously saved rows with a fabricated date are not corrected without source provenance. Extreme negative integer/float timestamp conversions now return unknown instead of overflowing during seconds-to-milliseconds conversion.

- Pruning now publishes a per-app UTC frontier in the same transaction as rollups, archives and detail deletion; later timezone changes cannot move it backwards. A legacy archive's absolute `last_at + 1` and each valid rollup date's next UTC midnight plus 24 hours provide a conservative temporal upper bound, using the supported offset range of [Chrono](https://docs.rs/chrono/latest/chrono/offset/struct.FixedOffset.html) and SQLite's [local-date rollups](https://www.sqlite.org/lang_datefunc.html). Invalid dates or overflowing bounds remain explicitly unknown, and unknown frontier formats are not overwritten. These bounds never establish native event identity or upgrade provider completeness. Recent unknown events remain eligible as partial data for compatibility: an old native ID whose time later changes can be counted again when no event receipt survives. That legacy limitation requires separately reviewed identity reconciliation with frozen source evidence and CAS; aggregate archives cannot supply the missing IDs and are left intact.

- The former Codex ordinal-ledger reconciliation and whole-file multi-header refusal are superseded by the [native rebuild](usage-native-rebuild.md). The shared parser uses the first canonical thread header and verified history boundaries, accepts proven owned events, and isolates ambiguous inherited history and conflicting events. Legacy ledger IDs and saved amounts remain unchanged in the raw audit view. Effective statistics replace only proven Codex imports/rollups with the accepted generation; unverified archives are not reconstructed by subtraction.
- Gemini 0.64.0 compatibility was checked against the installed recorder implementation, read-only native structure and sandbox fixtures. Top-level native JSONL and legacy JSON preserve paid message snapshots independently of context rewind. The ledger's existing flat `chats/` discovery is not expanded to nested child directories in this fix. Analytics retains its bounded recursive discovery; discovered subagent records retain paid facts but cannot supply a verified parent SID or main-session activity. Missing or future JSONL roles remain unknown. Common health combines ledger and analytics outcomes, so a nested child with unknown parent keeps the source partial. This is not a claim of exhaustive subagent history, real AI execution or Windows acceptance.

- Ingestion evidence is machine-local and bounded: each usage part accepts at most 100,000 exact event digests (an 8 MiB digest budget within a 16 MiB usage part and 32 MiB envelope), each provider roster at most 20,000 receipt hashes / 4 MiB, and its in-memory positive index at most 1,000,000 digests / 80 MiB serialized digest budget. Native directory manifests stop above 100,000 entries or eight directory levels. Hitting a proof budget preserves existing acknowledgements and degrades coverage to partial; it never evicts proof and then claims completeness. Missing receipts, unknown formats, untracked contributions (including proxy facts) and legacy history also invalidate negative evidence persistently. A full-roster ledger-count fence detects concurrent untracked inserts across pruning; cardinality is never substituted for native event identity. Windows physical-file identity code has not been exercised on Windows in this slice.

- The local session cache has independently revisioned metadata and usage-evidence parts. Unknown peer usage versions remain opaque to metadata. A full SQL import or database restore must revoke local usage trust before replacing the ledger; an unknown usage version, unsupported envelope version or malformed cache JSON can therefore reject that restore while preserving the current database. Such rows are not silently cleared or interpreted as empty. WebDAV preserves usage evidence only together with the local ledger, checkpoint and restore epoch.

- Source health is machine-local coordination state. SQL/WebDAV exports omit only the exact `usage_source_health_v1` settings key, and imports or database restores preserve the current local value under the final database replacement lock. Restoring the ledger invalidates old health through its epoch. Legacy health arrays, unsupported formats and damaged health are displayed as unknown; unsupported or damaged formats are not overwritten by a new attempt.

- Common source health combines independently fenced ledger and analytics attempts. A successful ledger read cannot hide an analytics failure or a required channel that has not run. `lastSuccessAtMs` means the last complete, trustworthy success; partial and failed attempts retain that older value. Cache-specific read timestamps describe more recent partial reads. The health store is bounded to 128 source identities and 256 KiB; this coordination foundation alone does not establish that every reader is wired to publish health.

- Local-only usage refresh coordinates the supported ledger readers and the [nine current analytics bridges](native-analysis-bridges.md) behind a worker-owned gate. It checks native file/dependency stamps and database revision signals before parsing; failed providers remain eligible for an unchanged-file retry. Cancelling an IPC waiter does not cancel the running worker, and a manual burst can add one followup. A failed maintenance step can retry on stable input without rescanning successful providers. Database writes from pricing or accounts can cause an extra warm validation; their revision is an invalidation signal, never evidence that an event was counted. The local-only command makes no account or pricing request; manual refresh and startup pricing keep their existing behavior. Sandbox regressions do not certify native IPC, release polling performance or Windows execution.
- Request first-token timing and generation speed currently have no verified samples. The legacy `first_token_ms` writer measures the first SSE event, which can be lifecycle or role metadata; a generic duration does not prove generated-token start/end times. Versioned detail and request measurements therefore return unknown values with zero samples, while observed status and latency retain their own valid sample counts. Protocol-aware instrumentation and historical provenance would be a separate change; this implementation does not relabel old first-event data as token timing.

- Each local refresh freezes readable settings and its native source roots under a short configuration barrier, then enumerates and parses outside that barrier. Configuration changes during the scan apply to the next round. Unreadable settings stop discovery and native reads, retain existing ledger/cache facts and record a generic failure for the required source-health channels. WorkBuddy and dsh observe their complete physical file rosters; deleting a dsh generation wakes the scan without authorizing a rollback to an older native generation.

- Model slots: Ofox writes each agent's default where the agent itself saves one, so an in-agent "save as default" wins and Desktop follows it. Session-only switches (Pi Enter, Hermes/OpenClaw `/model`, Gemini `/model` without Tab) stay session-only and Desktop keeps showing the default. A shell-exported `ANTHROPIC_MODEL` / `GEMINI_MODEL`, or project-level `.claude` / `.gemini` settings, still outrank both. dsh 0.1 (`settings.yaml`) is refused at bind; dsh 0.2's `session.v4` files are read by the same rules as v0.
- Gemini skips `~/.gemini/.env` in untrusted folders, so `GOOGLE_GEMINI_BASE_URL` and the key are not loaded there; this predates the model-slot change.

- Hermes has no per-request usage. Each `session_model_usage` row (session × model × task × billing route) is one ledger row dated at its `last_seen`, as in magpie, so a long session's whole row sits on one day and moves forward as the session grows. A session resumed after its earlier row was pruned into daily rollups (30 days) adds no further growth to the overview; session analytics still shows the full total. Session totals the rows leave unexplained (sessions from before model rows existed) become one unpriced `unknown` fact. Schema-31 model rows seeded by Hermes's v20 migration used the then-current routing and cannot be told apart from native attribution. Only the active `HERMES_HOME` is read (magpie also reads `profiles/*`). Each changed refresh copies `state.db` and its WAL twice (ledger and analytics), up to 512 MiB; a gateway that writes constantly keeps the copy repeating.

- The [OpenClaw event-time reader](usage-openclaw-reader.md) targets native 2026.10.3 schema-24 TEXT/zstd events with agent/session/event identity and four independent token buckets. Unknown model, time, identity or usage buckets remain partial. Cold/reset archives are not reconstructed, and unknown fork seeds do not enter the ledger. SQLite primary authority prevents stale legacy projections from being replayed, including a remembered primary disappearing. SQLite main/WAL leaf symlinks fail closed in the shared OpenClaw/Hermes snapshot path and OpenCode's shared SQLite evidence/warm-stamp path; symlinked parent directories remain supported. The separately added [analysis bridge](native-analysis-bridges.md) retains those limits and does not invent message/activity metrics. Synthetic installed-native API evidence alone does not certify the bridge, Windows or real agent execution.

- The resource library manages verified global targets. Project/session overlays and OpenCode dynamic routing keys or unresolved Skills path references require an explicit supported adapter; they are blocked rather than treated as literal paths. OAuth still uses the native tool's authorization flow.
- New WebDAV skill snapshots carry immutable artifacts, hidden files, empty directories and portable execution intent. Old flat snapshots omitted some of that information; restoring their available content as drafts cannot reconstruct missing files or permissions. Native/legacy roots and existing artifacts remain intact.
- Resource compatibility rules were checked against OpenCode v1.19.0 and Hermes v0.22.0 source, including global precedence and Skills readers. This is source-contract verification, not a claim of native agent execution on every platform.

- Native Windows tests, UI/installer behavior and file-lock semantics remain pending until the exact delivery SHA is run with the Windows verification package. macOS type checking is not Windows execution.
- One macOS release launch remained blank after a long wait for the system Keychain authorization. The native window existed, and WebKit logged a content-process launch failure; quitting through the application menu and relaunching restored the interface. This recovery does not establish a root-cause fix. System authorization was handled by the user. Subsequent window acceptance is recorded separately from this observed startup limitation.
- Development-preview screenshots use synthetic accounts and session fixtures; they do not establish live backend compatibility. Final native macOS screenshots, window placement and installation are recorded separately.
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

- A separate native macOS debug-sandbox UI check prepared and installed Copilot 1.0.98 with Node 22.12.0, cancelled an update preview, and removed the private copy while preserving four data fixtures. The UI update cancellation is not an additional real version-upgrade test. Release performance, real Copilot resource execution, and Windows remain unverified.

- Catalog inspection and preview rehash Node/npm and, when present, the managed prefix. Debug builds can take substantial time; repeated mounts or rapid reopening can duplicate read-only checks. These checks currently have per-tree limits but no combined discovery budget or cancellation inside the synchronous hash loops. Release performance and safe in-flight deduplication remain to be measured; mutation authorization still requires fresh content validation.

## Session workbench acceptance boundaries (2026-10-16; statistics updated 2026-10-18)

- The [native statistics rebuild](usage-native-rebuild.md) separates tool runtime events from conversational messages, uses the Magpie strictly-under-five-minute/later-event allocation rule, and nearest-rank quantiles. Message/activity coverage is a verified subset, not a universal completion percentage. Unknown model/source attribution stays unavailable, and Codex historical replay is still partial. The initial four-source analysis stage is now extended to eight session/project bridges: Claude, Codex, Gemini, OpenCode, WorkBuddy, Pi, dsh and OpenClaw. The [new native bridges](native-analysis-bridges.md) retain source-specific evidence requirements; dsh/OpenClaw message counts and activity remain unavailable. Hermes lifetime totals remain separate. This capability change does not certify actual agent consumption, complete historical attribution or Windows execution.
- Explicit local search and the new verified reader share bounded native projections. The legacy message API and explicit Markdown export have not all been migrated to this budget; do not generalize it to every session operation. Each session is limited to 32 MiB decoded / 8 MiB per line; a search batch has 256 MiB decoded, 15 seconds, 200 hits and three snippets per session. Compressed input and decoder window sizes are also bounded. Immutable SQLite copies have a separate 512 MiB source / 1 GiB batch I/O budget and are reused only within the current run. Limits preserve partial results with a reason; they are not empty-success results. Search jobs expire after 60 seconds of inactivity, and no persistent body index is built.
- Codex native archive and cross-root copies are readable/exportable where their identity can be established. Ambiguous copies retain unknown message counts and do not gain continue/delete capabilities. Native archives are distinct from the OFox recycle bin.
- The warm analytics input projection retains one body-free entry for ten minutes, with an estimated 512 MiB holding limit (a 1,700-session history decodes to about 290 MB; one over the limit is decoded again on every query). Every query rereads the raw cache rows, effective source health and price catalog in its SQLite transaction; the input key excludes pricing and query filters because they are recalculated. This budget is separate from ranking retention and does not cap old Arcs held by in-flight requests.
- Full ranking snapshots contain metadata only, last ten minutes and are limited to four snapshots / 128 MiB / 100k rows each. Oversized sets explicitly disable paging instead of presenting a truncated list as complete. Switching sort or filtering the list does not change the KPI cohort.
- Release performance fixtures and native acceptance results are recorded in [the implementation record](implementation-progress-2026-10-16.md). Synthetic 10k data cannot establish real-history scan or IPC latency. Windows native file identity, SQLite replacement semantics, UI and installation remain separate final-SHA acceptance work.

- The final synthetic renderer benchmark met p95 < 500 ms at 900px and 1280px, but individual session-page samples reached 836 ms and 983.7 ms. This is prepared-data replacement through React commit and a second animation-frame opportunity; it excludes source scanning, native IPC and compositor completion. Raw samples accompany the delivery; this is not a guarantee that every UI update stays below 500 ms.

- Native macOS background AX automation can return an empty retained session-list subtree after Reader → usage detail → session management, despite correct visual location, an exposed DOM and target focus. An isolated WK fixture reproduced the observation; two DOM integration routes passed. A VoiceOver product defect or specific platform cause has not been established. Four diagnostic candidates were not validated and were not shipped; full screen-reader return-path acceptance remains unverified. See the implementation record and delivery diagnostic evidence.

### Overview and Codex thread-display increment

- The overview project ranking is compact and full width, initially six rows with batches of 50. Long names are bounded and full paths remain in title hints; hiding/restoring rows affects presentation only. Routine sync times, catalog versions and the repeated cost explanation are folded into data status. Source and price warnings remain visible. Static session capability notes are grouped by tool and folded; size, native ID and I/O details remain available in session details.
- A Codex native thread ID can differ from the old billing `session_id`. The old exact join could therefore show no child-thread usage. The new optional `usageDisplay` receipt describes a proven native thread's own four token buckets and current-apex OFox estimate; it never fills a child with a root's combined totals. Available counters, unknown prices, partial prices and verified zero are separate states.
- This display correction does not rewrite the ledger, historical request IDs, billing session IDs or saved amounts. Multiple headers, forks, uncertain inherited baselines, incomplete counters, conflicting replicas, stale sources and missing indexes remain unknown. It does not certify complete historical session/project analytics coverage. See [display provenance](usage-session-analytics.md#概览与会话列表的显示口径补充).
- The shared bounded Arc cache retains body-free input and source-proof projections. Warm reads still verify current source stat and cache observations; unchanged projections skip transcript reads and repeated JSON decoding. Prices are reloaded in the same transaction and frontend queries are isolated by apex. Same-SID replicas require a full-source-byte SHA-256 digest; equal token facts with different source bytes are not sufficient. This does not broaden the shared envelope writer/CAS compatibility rules or eliminate cold parsing costs. Legacy Entry arrays remain query-readable but are still refused by the owned CAS; they are not rebuilt automatically.
- Code `a8ae76c1` passed the current frontend run of 133 files / 1004 tests, typecheck and format; Rust fmt, all-targets Clippy with warnings denied, and 20 suites / 3010 passing tests with 11 defined ignores. All 18 added Rust targeted tests passed, including a 601-source warm fixture with zero transcript reads and zero cache decodes; both final independent reviews passed. The macOS arm64 app was built, signature-verified and installed; native Codex 0.162.0 records confirmed four previously empty child-thread amounts and an explicit ambiguous-history refusal, plus the full-width six-row overview and folded data status. Exact screenshots and package hashes are in the delivery manifest. Live source failure/partial warnings remain visible; this does not certify complete ingestion. See the [implementation record](implementation-progress-2026-10-16.md). Earlier final results above belong to earlier stages. Windows and the native AX return path remain unverified.

### Request measurements and tool presentation

- The request detail display distinguishes absent original measurements, read failures and verified zero. It does not infer HTTP success from output or Tokens. The supported local-history readers do not provide HTTP response status or first-generated-token timing; these cannot be reconstructed from historical totals. OpenCode's positive native message interval is an elapsed-message measurement, not HTTP round-trip time. Existing measured request history remains separate. See the [source matrix](usage-request-evidence.md).
- Tool presentation changes retain the existing configuration and lifecycle capabilities. Copilot's optional “Manage installation” only manages the OFox-owned copy where the backend supports it; it does not add model binding or usage ingestion. Other catalog entries retain their manual commands. The per-tool “Usage details” action is inside More by default and still opens the existing OFox account website for the bound API key. The row’s “Configure” button retains full management capabilities; inline model selection is a shortcut into the same versioned configuration, not a separate model setting. No external website layout is changed in this repository.

## Resource library workbench

See the [workbench boundaries](library-workbench.md#已知边界): Codex override discovery does not expand the writer; Gemini implicit HTTP/SSE fallback cannot be represented losslessly by a fixed transport; damaged optional OpenCode/Hermes scopes remain explicitly incomplete. Older clients that do not know the batch settings key cannot be certified safe for SQL/WebDAV export of new batch evidence. Native Windows and actual client consumption remain separate acceptance gates.

- The second resource-library iteration retains five global write targets. Local presence never proves OFox ownership, enabled state or connectivity. Source detail contains field shapes only; OpenCode layered fields describe the effective merged structure, while file reveal identifies the selected physical source.
- Curated skill matching verifies a collected fixed origin, revision and artifact, not the newest remote commit. New collection and the library's single-resource update-check path fetch selected-directory blobs; legacy repository discovery and the old compatibility bulk-check/update APIs retain their existing repository-download boundary. Standard Git LFS pointers are rejected with a local-import next step; this version does not resolve the LFS protocol. No skill script, package installation, vendor MCP connection, native OAuth or Windows acceptance is implied by these sandbox tests.
- Source evidence is checked on actions, rescan, resource/configuration changes and ticket expiry; open source/compare views revalidate every five seconds and on window focus. This iteration does not add a continuous operating-system watcher for arbitrary external file edits. A detail cannot be used to reveal, copy or locate from stale source evidence; an external change invalidates the displayed evidence on the next validation and requires a new scan before further source operations.

## Follow-up configuration and analysis delivery

- Scene kits stage existing curated resources, with explicit confirmation for each type. Successfully applied types stay applied when a later type is cancelled or fails. This is not a cross-type transaction or a universal rollback. A saved MCP whose refresh fails is locked to its acknowledged revision until revalidated; unsaved edits are never silently discarded on retry.
- Configuration issue actions use current resource revision and restore epoch. An unsupported OFox checker means that the native tool must perform the check; it is not evidence of a service failure. Credential problems without an exact field reference open the relevant editor area rather than guessing an environment-variable name.
- WorkBuddy, Pi, dsh and OpenClaw analytics consume the same validated native source roster as their ledger projections. WorkBuddy conflicting project evidence can conservatively mark an estimate partial. dsh and OpenClaw do not expose invented message or active-time metrics when the original source lacks trustworthy timestamps/identities. Missing source evidence remains explicit, and cannot justify a guessed session link or historical reconstruction. See [native analysis bridges](native-analysis-bridges.md).
- Native acceptance reports distinguish a synthetic runner test, a real library-writer sandbox profile, actual client consumption, and final server billing. A clean compiled SHA and hashed resource receipts are necessary but are not proof that an agent loaded those resources. Claude's isolated explicit instruction mode does not certify default global instruction discovery. Hermes runtime availability and native Windows acceptance are independent prerequisites. See [native acceptance](native-acceptance.md).
- Release artifact validation now belongs to both dry-run and publish paths. The workflow has not been executed remotely without push authorization. Local ad-hoc bundles do not establish Developer ID signing, Apple notarization, Gatekeeper acceptance, NSIS installation, or cross-version automatic updating. Those results must be recorded separately for the final SHA.

- Library inline editors keep private drafts in memory only. Hiding the retained macOS window preserves them; quitting the process does not provide persistent draft recovery. The inline discovery/check fixtures use synthetic data and do not certify Agent resource consumption. This UI iteration does not change the existing Windows, notarization or paid native acceptance status.

## Library beginner presentation boundary

The My resources / Discovered in tools presentation changes neither the native reader roster nor saved definitions. Only supported native adapters contribute discovered configurations; a tool outside that roster is not established absent. Saved items can outlive their current native configuration. Exact matched sources move into each resource's expanded details, while conflicting same-name variants remain independently addable. Adding still does not apply, take ownership or prove connection success.

This increment corrects Ofox display spelling without renaming compatibility identifiers or historical user data. Read-only installed-window verification and synthetic responsive/keyboard fixtures do not certify real client consumption, Windows installation or Apple notarization; those external acceptance limits remain unchanged.

- Interaction insights: OpenClaw and DeepSeek Harness do not yet provide verified tool/Skill projections and are shown as unavailable/incomplete in mixed cohorts. Pi uses the active owned conversation branch; calls on abandoned branches are excluded. Skills invoked through shell commands, opaque wrappers, or implicit prompt expansion are not inferred. The macOS native readers passed compilation, all-targets Clippy and the Rust suite; the installed 1.3.7 app rendered real local history in the new analysis cards. Windows and paid agent-request acceptance remain unverified. The separate responsive design previews use synthetic data.

### MCP 新增应用适配（2026-10-21）

资源库已补齐 OpenClaw、WorkBuddy、DeepSeek Harness、Pi；Codex / ChatGPT 共用配置目标。新增原生格式与验证范围见 [资源库说明](library-workbench.md#mcp-可绑定应用补齐2026-10-21)。Pi 低于 0.99.0 或版本无法确认时不允许新增原生 MCP 分配。DeepSeek 可执行 YAML 标签、外部 include、未验证的凭据表达式会明确拒绝处理；不会执行表达式或覆盖原文件。WorkBuddy 自身授权仍需在客户端完成。macOS 的配置读取/schema 示例验证未覆盖 Windows，也未执行完整 MCP 工具会话；WorkBuddy 运行加载仍待验收。

2026-10-23 的 DeepSeek 默认注释误报修复只改变标签识别：`!!js` 等标记出现在注释或普通字符串中时不再被当成可执行标签。实际 YAML 标签和 `%TAG` 指令仍不受支持；不执行表达式，也不推断运行连接成功。Windows 原生读取及写入仍需单独验证。

该修复的首个本机重编译版本仍为 1.3.7，可执行文件摘要前缀 `20927a36`。前端 1515 项、Rust 3421 项通过（14 项忽略），类型、格式和 all-targets Clippy 通过。用户完成 macOS 钥匙串授权并正常重启后，原生 MCP 页面显示“状态已更新”，DeepSeek 列恢复正常复选框，原来的读取失败提示消失；DeepSeek home/web/headless 配置摘要未变。证据为 `/tmp/ofox-dsh-mcp-after.png` 和 `/tmp/ofox-installed-dsh-accepted.json`。此项只验证配置扫描，不代表已执行 MCP 工具调用。Computer Use 未操作 SecurityAgent。

随后合并仓库迁移和单日小时趋势修复的本机 1.3.7 构建摘要前缀为 `bf1d1398`，已签名并替换为唯一安装版。前端 1517 项、Rust 3431 项通过（14 项忽略），类型、格式和 all-targets Clippy 通过；最终安装版原生 MCP 扫描正常，Skills 四仓库合计 917 项，其中原 `cexll/myclaude` 来源成功读取 11 项，无失败提示。DeepSeek 配置摘要未变。用户将 Ofox 置于前台后，真实用量页恢复查询，“今天”按 00:00–20:00 显示小时分布；证据为 `/tmp/ofox-hourly-real-installed.png`。此前未加载是后台窗口可见性门禁暂停，本次未改动该暂停策略。小时图同时通过后端回归、前端集成和 900px 三语明暗预览。安装与证据记录为 `/tmp/ofox-installed-latest.json`、`/tmp/ofox-repo-hourly-verification.json`。

独立 Skills 发现视图版仍为本机 1.3.7，最新摘要前缀 `793ea225`，已签名并替换为唯一安装版。前端 1521 项、Rust 3431 项通过（14 项忽略），类型、格式和 all-targets Clippy 通过；独立复核和 900px 三语明暗、长列表返回、选择后可见安装区等测试数据检查已完成。钥匙串授权后，实际新版窗口已验证默认只显示已有内容、顶部“发现”切换独立视图、返回后恢复列表及入口焦点；四个仓库合计 917 项，其中一次临时读取失败经显式重试恢复。实机证据为 `/tmp/ofox-discovery-view-installed-open.png` 和 `/tmp/ofox-discovery-view-installed-back.png`；验收未执行原生安装或配置写入。DeepSeek 配置摘要未变；详细记录为 `/tmp/ofox-discovery-view-verification.json`。

### MCP / Skills 分发验收边界（2026-10-22）

十个可绑定客户端已纳入共同名单和真实 writer。Pi 1.3.0 的原生离线加载能识别受控 Skill 和内置 MCP 扩展，公开锁定版 filesystem MCP 包也完成独立下载及协议调用，但这些不能代替每个 Agent 的模型消费。专用 Key 的旧 Gemini 费用条目仍待对账，按既有预算保护暂停后续付费调用并保持禁用。Hermes 缺少依赖环境；ChatGPT / WorkBuddy 桌面隔离及消费未验收。逐项证据和未执行项见 [资源分发验收](resource-distribution-acceptance.md)。

## Skills 不同副本的写入范围（2026-10-23）

Magpie 样式的差异提示与导入已覆盖同目录身份的多个版本。嵌套或独立 reader 目录中的副本只读显示差异，不会因为它与资源库同名而被替换；“用资源库的”仅面向已解析的直接应用目录。SKILL.md 显示名相同但目录身份不同的技能保留独立入口。Claude Desktop 自带技能不可接管。Windows 原生文件替换及链接恢复尚未运行，macOS 沙箱和合成 UI 验证不能替代该验收。

### Loading and Skills discovery verification (October 2026)

The loading/discovery changes are verified on macOS; Windows discovery/install paths require a Windows run. First native synchronization and a changed/expired projection still perform full validation. Public repository discovery and skills.sh search require network access and can report GitHub rate limits/timeouts; cached results do not prove remote freshness. skills.sh identifiers are resolved before collection, so uncertain collection matches are not hidden by a leaf-name heuristic. The benchmark in usage-session-analytics.md covers a synthetic warm projection only.

The live skills.sh search probe returned a page-sized count and ignored offset. The discovery view currently requests up to 40 public results per keyword; narrow the keyword for other matches. Full configured repository catalogs use local pagination. No claim of exhaustive or unlimited skills.sh pagination is made.

The locally rebuilt macOS app (1.3.7) was signed, installed and verified as the sole installed Ofox bundle. After the user handled the first-launch Keychain prompt, the original window remained blank; normal quit/relaunch restored the renderer. This is the previously observed startup limitation above, not an established root-cause fix. Installed-window checks then verified real repository search, partial-repository failure/retry feedback, 40 skills.sh matches for a public keyword, actual nested-path resolution before collection, and Escape focus restoration. The 7-day analysis (375 sessions / 48.1M tokens) switched back to today (15 sessions / about 5M tokens) with a matching displayed date. Native sync/query progress was visible; GUI automation round trips are not a page-latency benchmark. Some old Codex source paths no longer exist, so partial/stale-source warnings remain appropriate; the existing generation consistency check can also reject a scan during live updates. No real skill installation, Agent configuration test writes or credential changes were performed. The separate 900px light/dark checks used synthetic UI data. Windows remains unverified.

### Assistant media acceptance and downloads

Assistant video downloads use unsigned HTTPS URLs returned by the selected Ofox gateway, without gateway credentials, cookies or redirects. Download hosts and every DNS answer must be public; direct sockets, HTTP/HTTPS CONNECT and SOCKS5/5h tunnels use checked literal addresses while TLS/SNI and Host retain the original name. An explicit proxy failure never falls back to direct. All-fake-IP answers in 198.18.0.0/15 use the same fixed Alibaba public DNS-over-HTTPS infrastructure as web reading; other private/mixed answers are refused. A failed resolver/proxy/certificate check keeps the known job available for download retry without generation replay. See [download transport](assistant-media-downloads.md). Image generation returns bounded base64 data and does not download arbitrary output URLs.

New download partials use private owner-scoped staging, are excluded from backups and are removed on cancellation/normal exit or the next owner recovery/download. Windows may retain a busy path until its open handle closes. Anonymous temporary files left by older app versions cannot be identified safely and are not retroactively deleted. Transport and cleanup have local fixture coverage; native Windows execution and this iteration's final macOS/live acceptance remain pending.

Image editing accepts PNG, JPEG and WebP up to 15 MiB. [Ofox editing documentation](https://ofox.ai/docs/develop/models/gpt-image/image-edit) lists PNG/JPEG in its prerequisites but also WebP in its format guidance; the [upstream GPT Image API](https://developers.openai.com/api/reference/resources/images/methods/edit) explicitly supports WebP. WebP request construction is covered by local tests; its provider acceptance has not been separately billed and tested.

## GLib dependency on Linux (2026-11-02)

Linux GTK3/Tauri still resolves `glib` 0.18.5. [GHSA-wrw7-89jp-8q8g](https://github.com/advisories/GHSA-wrw7-89jp-8q8g) affects its `VariantStrIter` implementation and is fixed in the incompatible 0.20 line. The official [0.18 maintenance source](https://github.com/gtk-rs/gtk-rs-core/blob/42b9caf98e03ded086362d9653ca58fe94dc8658/glib/src/variant_iter.rs) still contains the affected immutable out-pointer. A compatible upstream backport or GTK dependency migration is needed. This alert remains unresolved; application-level reachability has not been established. The inspected macOS and Windows dependency graphs do not enable this GTK/GLib path.

## ZIP import metadata bounds

ZIP imports require single-disk archives with zero archive offset and an unambiguous end record. Archives with another end-record signature in the final 128 KiB, over 32 MiB of central metadata (lower limits apply to smaller imports), or over 1 MiB of ZIP64 extensible metadata are rejected before metadata allocation. This intentionally excludes ambiguous/polyglot or unusually metadata-heavy archives; ordinary stored, deflated, Unicode-name and legacy ZIP64 archives have compatibility fixtures.

## Assistant conversation request retries

[Conversation retries](assistant-request-retries.md) preserve unknown billing
receipts for 502/503 attempts even if a later attempt succeeds. Gateway errors
are not proof that a model did no work. A partially received stream, timeout
or ambiguous socket failure still requires a manual retry. This change has
macOS fixture coverage through the prepared Pi runtime and real Rust event
bridge; native Windows and live billed-provider acceptance remain unverified.

## Assistant skill target

The library target and modes now feed a fresh host snapshot into runtime skill loading. Automatic skills are listed within a bounded prompt and loaded through `use_skill`; broken and manual skills cannot be loaded by that tool. Runs without local command tools can still read skill instructions. A leading / in the composer selects Auto or Manual for one message. Its current body is injected as XML-escaped ordinary user input; history displays only the original words and skill chip. Retry/regenerate revalidate the current installed body, so a changed skill can change replay input; unavailable skills fail clearly. Edit can replace or clear the chip. Unsent chips are session-scoped memory only, and queued chips are checked again at dispatch. A message with a skill can queue during a reply but cannot steer that existing run. Chat installation and creation now require one-shot approval in every permission mode. The optional creation-origin sidecar has a strict reader and transactional writer. Missing origin evidence does not reconstruct ownership from names.

Official popular metadata does not contain a unique repository directory. Origin classification verifies its full publisher repository and the actual fixed artifact's frontmatter name; it does not assert a unique skills.sh URL-to-directory mapping or a safety audit. Ambiguous/mismatching metadata remains unreviewed. Native Windows execution and live-agent acceptance remain unverified. Configuration locks coordinate application writes, not arbitrary external processes; skill scripts are not placed in an OS sandbox.

Generic assistant file tools protect the private AppConfig tree, with read/execute exceptions only for current verified automatic skills or an explicitly user-selected run grant. Traversal from outside prunes private descendants. Generic writes and recognized direct shell mutations are denied in every permission mode; dedicated approval-backed library operations retain their own authority. Linked skills grant only their verified artifact tree, never the whole artifact store. An unrelated epoch refresh preserves an explicit manual grant only when every other proof field is unchanged.

These are host tool and direct-command checks, not filesystem isolation. Approved scripts still have the user's OS rights and can compute paths, read manual files or write their own directory; static command checks cannot prevent every such effect. Manual controls discovery and `use_skill`, not confidentiality of bytes also available through an automatic skill's shared canonical artifact or an ordinary outside copy. Python is not bundled. Native Windows execution remains unverified; public skill directories use verified ordinary lexical paths instead of Windows device/verbatim paths.

Assistant chat skill transactions reject stale approval epochs rather than rebasing automatically. An unresolved private intent blocks the assistant skill snapshot until its affected state can be verified; recovery never guesses or replays a pending install. A commit followed by snapshot-refresh failure remains installed/saved but unusable for that response; retry verifies the receipt and refreshes without overwriting later mode changes. If storing the installed-card update fails after commit, the tool reports the committed result with cardPersisted:false and does not reinstall. Private orphan staging/artifacts from a failure before intent publication confer no runtime authority.

This chat-skill increment has macOS sandbox transaction/restart/fault tests, prepared Node/Pi with a local synthetic gateway and real Rust callbacks on both apexes, and 900px Chrome light/dark checks in three languages. It does not establish live public-source downloads, billed model behavior, native Ofox rendering, or native Windows replacement/recovery. Only invented skill bodies are used in acceptance fixtures.

## Assistant document engines (2026-11-07)

The assistant's `create_document` tool makes PDF, Word, PowerPoint and Excel
files (and md, txt, csv, html and json) with bundled engines: officecli and
Typst, and on macOS a Python engine for PowerPoint decks. See
[document engines](assistant-engines.md) for how it works and for the
acceptance checklist. Everything below is known and not claimed fixed or
verified. The engines ran on macOS arm64 only; none of this validates Windows
or Linux.

Isolation and platforms:

- The engines' read sandbox is macOS only (`sandbox-exec` with a profile that
  hides `/Users` and `/Volumes` from officecli and the network from both).
  Windows and Linux rely on the host's batch validator (which refuses any
  property that names a file or an address) and on the dead proxy every engine
  gets. A batch the validator misses would not be stopped by the operating
  system there.
- Windows: .NET may resolve the profile folder through the Known Folder API
  rather than `USERPROFILE`, so officecli's private `HOME` may not be what it
  uses; this is unverified. The Windows branches of the runner (process jobs,
  no sandbox) are not compiled in the macOS development environment, so they
  have had no compiler or test run. Windows acceptance is a separate gate.
- A `/tmp/officecli-<uid>` folder that is not the current user's own folder
  (another owner, a link, a file) makes every Word, Excel and PowerPoint
  document fail closed on macOS: officecli keeps temporary files there whatever
  `TMPDIR` says, and the sandbox profile needs to allow exactly that folder.
  Documents work again once the folder is removed. The user's own folder with
  looser permissions is made private (0700) and used.
- A crash, a forced quit, or an app shutdown that aborts a run's task during a
  run leaves that run's `outputs/<docId>` folders in the conversation's folder
  until the conversation is removed; a normal end of a run, including a stop or
  a failure, removes them.

Signing and packaging:

- officecli is signed with the hardened runtime and `allow-jit` only. That is
  what upstream's own signature carries and what the signed binary needed in
  the spike, but notarization has not been run: it needs the release owner's
  `APPLE_ID`, `APPLE_PASSWORD` and `APPLE_TEAM_ID`. Until a notarized build has
  run both engines from the installed bundle, a hardened-runtime failure that
  only appears under Gatekeeper is not ruled out. See
  [notarization](assistant-engines.md#notarization-to-do-by-the-release-owner).
- The branch-package and CI smoke tests run the engines unsigned, so they do not
  exercise the hardened runtime or the entitlements.

Live assistant progress now uses real tool outcomes and model-reported plans;
it never estimates a completion percentage. The latest four supported tool calls
are an ephemeral summary, not a persisted task history. Unknown tools use generic
working status, and a quiet-wait hint means no recent semantic stage update, not
that the task has stopped. UI replay is not a new live generation acceptance run.
See the [conversation progress record](superpowers/research/2026-11-10-assistant-conversation-progress.md).

What the files and previews are:

- PPT style resources: 24 pinned public source candidates are bundled, with
  five experimentally selectable native adaptations plus compatible `navy`.
  The assistant now offers these six styles through the PPT starter and composer
  plus menu, with authored eight-page native-style examples and an explicit selection chip.
  Original public decks have a separate preview of all actual 10/10/7/8/6 pages,
  with known rendering and dependency limitations. These 41 source pages do not
  establish complete native conversion. See the [source coverage audit](superpowers/research/2026-11-10-ppt-source-coverage-audit.md).
  The new workflow performs license/capability/requirements filtering before
  model design judgment, freezes the chosen style and page plans, and preserves
  manual selection precedence through queues and retries. Actual model selection
  quality has not been accepted; offline tests exercise its contracts. Draft
  choices do not persist across app restarts.
  Enabled is not quality acceptance. HTML source layouts are only partially
  mapped; no HTML-to-editable-PPTX converter is implemented. System font
  substitutions preserve type roles, but target-OS CJK metrics, visual parity
  and PowerPoint save/reopen editing remain unverified. Third-party font/image
  binaries are not imported or automatically cleared by the repository license.
  Python `relation()` and `fork_join()` use native shapes/connectors. Parallel
  branches are not a sequential process. Diagram bounds use the owning
  presentation's actual dimensions; lesson-region diagnostics do not silently
  move explicit geometry. These helpers do not guarantee a model chooses the
  right relationship or composition. `equation()` accepts
  bounded structured expressions or a restricted LaTeX subset, rejecting unknown
  macros; LaTeX uses native OMML plus a rendered PNG compatibility fallback.
  Structured expressions retain a literal text fallback. Fallbacks are not
  editable math. Legacy `formula()` remains plain text and now emits a warning.
  SVG/matplotlib figures remain raster. Slide 8 of the actual 14-slide repair
  displayed correctly in WPS, and an independent copy retained that display
  after save, close and reopen. In a later real 10-slide Transformer task,
  macOS WPS 12.1.25895 retained one native symbol edit (page 5 main equation,
  A→B) after save, close and reopen. More complex internal edits, editing all
  14 equations, and PowerPoint/Windows behavior remain unverified. The separate
  corrected deck was visually checked in WPS on pages 3/4/5/10 only; a missing-font
  warning remained, so font parity is not accepted. Structural/schema checks do
  not prove visual quality or native editing. See the [repair record](superpowers/research/2026-11-10-ppt-math-ui-repair.md),
  [live Transformer evidence](superpowers/research/2026-11-10-ppt-transformer-live-validation.md),
  and [validation evidence](superpowers/research/2026-11-10-ppt-validation.md).
- The six-page A/B/C/D evaluator prepares frozen prompts and records receipts;
  it does not call a model. Current shared runtime guidance would contaminate
  A/B unless condition-specific baseline/tool/skill snapshots are established.
  Model comparisons remain blocked, distinct from offline fixed-code smoke
  tests; no quality uplift or billed API cost is claimed.
- A PDF's page count is a heuristic: the number of `/Type /Page` objects in the
  file. It can be off for unusual PDFs; Typst's own output is what it was
  checked against.
- The file workspace previews owned input/generated PPTX by its actual OPC page
  order/count. It renders pages on demand through sandboxed OfficeCLI and a
  bounded raster worker. At most page 1000 is renderable; a larger real total is
  shown with the unrendered range. Persisted batches remain at most 60 JPEGs,
  while selected source page numbers can reach 1000. HTML uses an opaque scripted
  iframe on a token-scoped loopback endpoint, with external dependencies blocked
  and reported. Native WebView attack and Windows execution acceptance remain
  separate from CSP/static checks. Native preview currently fails closed when
  its process sandbox is unavailable. Existing generated PowerPoint attachments
  can still persist bounded page JPEGs, rasterized from officecli SVG. The media protocol
  serves only validated JPEG pages within the current attachment/session/account
  authority. Legacy attachments and failed rasterization keep save/open working
  without a preview. Word and Excel have no page preview.
- OfficeCLI does not faithfully render native OMML. Math preview uses a temporary
  copy selecting the equation's existing compatibility fallback; the delivered
  PPTX retains native OMML. A preview of a fallback cannot establish editable
  mathematics or identical rendering in WPS/PowerPoint. The raster worker only
  adapts strict, aspect-matched rectangular PNG patterns from OfficeCLI; this
  is not general SVG/pattern compatibility. Missing/failed math
  preview checks must remain visible as incomplete, rather than zero findings.
- Generated HTML guidance includes task-specific interactions, visible selected/
  empty/reset states and keyboard access. Saving HTML does not run a browser
  test. The bundled behavior example has DOM regressions; it is not evidence
  that every model-generated page works. Historical English model text is kept;
  new tool status labels are localized, and new generation guidance asks the
  model to use the conversation language.
- There are no `submitting` or `failed` document cards: a record is written
  only once the file exists, so a failed or interrupted document leaves no card
  (the model gets the error and may retry, at most twice).
- When officecli gives no usable issues report (for example one cut at the
  runner's output limit), the document gets one warning, "issues unavailable",
  instead of a count of zero. At most 200 issues are asked for.
- CSV files are written as the model wrote them and are not formula-checked: a
  cell that starts with `=`, `+`, `-` or `@` is a formula when the file is
  opened in Excel (CSV formula injection). Only xlsx batches are filtered.
- **Save and open** (保存并打开) of an html document opens the saved copy in the
  default browser from `file://`, where the model-written page and its scripts
  run. The in-app HTML workspace is the separate opaque, token-scoped loopback
  preview described above; the application media protocol still refuses HTML
  and SVG documents.
- In an Office batch, a literal `\n` (backslash, n) in the `text` and `notes`
  props is read as a newline, as officecli's command line reads it, because
  models escape it twice (`use_skill` now hands the skill body over as plain
  text, which was the trigger: inside JSON its examples showed `\\n`). Those
  two props cannot hold a literal backslash-n.
- A signed local build (`pnpm tauri build` with `APPLE_SIGNING_IDENTITY`) signs
  the prepared runtime in place: `assistant-runtime/resources/node/node` and
  the runtime's native addons keep the Developer ID signature with the hardened
  runtime, and `pnpm assistant:test`, which runs with that Node, then fails to
  load the development `node_modules` addons ("different Team IDs"). The
  preparation's currency check does not notice the identity change; run
  `rm -rf assistant-runtime/resources/node && pnpm assistant:prepare` without
  the identity before the tests.

Features deliberately rejected by the generic OfficeCLI batch surface, because
making a document must read nothing outside it and run nothing. Managed native
chart helpers can create their own embedded XLSX from supplied data; this does
not admit arbitrary OLE or embedded-package inputs. An Excel workbook's
formulas run later, when it is opened: they are filtered, not sandboxed.

- diagrams (officecli renders them in a browser it starts and loads a script
  from the network), OLE objects and embedded packages;
- pictures and any other file from a path or an address: images are `data:`
  URIs only (the document's own bytes, up to the content limit);
- Word fields other than PAGE, NUMPAGES, DATE, TIME and TOC, and any field
  instruction that runs, fetches or links (DDE, INCLUDEPICTURE, LINK, and so on);
- number formats that escape a character with a backslash, such as `\$`: a
  backslash reads as a path separator to the check. `$#,##0` is accepted;
- Typst packages (`#import "@…"`);
- Excel formulas outside an allowlist. A formula is any key with `formula` in
  its name (such as `arrayFormula`), `ref`, `refersTo` or `calculatedField`;
  every key of a conditional format, validation, named range, chart,
  sparkline, table or pivot table except its text keys (titles, names, labels,
  legends, prompts, messages, captions, formats and fonts) and its inline lists;
  and any other value that starts with `=`, `+`, `-` or `@` and is not a
  number. A formula may call only maths and rounding, statistics, logic, text,
  lookup and reference, date and time, finance, information and dynamic-array
  functions (SUM, IF, IFERROR, VLOOKUP, XLOOKUP, INDEX, MATCH, TEXT, DATE, PMT,
  ISNUMBER, UNIQUE, FILTER and the like), and HYPERLINK only with one quoted
  https:, http:, mailto: or # address as its first argument. Refused even inside
  quoted text: functions that fetch, run code or build a reference from text
  (WEBSERVICE, IMAGE, RTD, FILTERXML, STOCKHISTORY, CUBE…, IMPORT…, CALL,
  REGISTER, EXEC, PY, DDE, INDIRECT and others), a DDE `|`, another workbook
  (`[book]Sheet!A1`, a quoted path before `!`), `\\`, `//` and `file:` where a
  word starts (so "Risk profile: by region" passes and "see file:x" in a
  formula does not). So a formula with such a word or character in its text
  (`"Call us"`, `"a|b"`, an address outside HYPERLINK) is refused too. An
  inline list (chart `data` and `categories`, series `values`, table
  `columns`, pivot fields) that is one reference, or that holds a `!` or `|`
  anywhere, is read whole as a formula: officecli writes a chart list with a
  `!` as it is into the chart's formula (`Sheet1!A1,WEBSERVICE(B2)` would be
  fetched when the workbook opens), so such a list may hold only references
  separated by `,`. A list with `|` is refused, and so is a list of labels
  where one holds a `!`: `categories: "Hello!,World"` is refused with "holds !, so
  officecli writes it as a formula, and it is not a reference such as
  Sheet1!A2:A5 or several separated by ,"; put such labels in cells and give
  their range. Any other inline list is checked cell by cell, and officecli
  writes its cells as literals: a cell may not name another workbook, a share,
  an address or a file, and a cell that starts like a formula is checked as
  one; a table's `data` rows hold values only, so no cell there may start with
  `=`, `+`, `-` or `@` unless it is a number. A sparkline's `data` is a
  reference, read as a formula like its `dataRange`. Text keys are checked the
  same way as a whole, so `North (EU)` or "Call volume" passes and a title
  with an address in it does not. A cell value that starts with `=`, `+`, `-`
  or `@` (`+1 555 0100`, `- Item`) is refused as a formula; a leading
  apostrophe keeps it text. A text key that starts that way is checked as a
  formula too, and its refusal offers no apostrophe: officecli shows the
  apostrophe of a chart title or a validation prompt, so the text has to be
  reworded. A text key is matched by the last part of its name, so a key
  officecli has that the list does not (a table's `totalLabel`, a chart's
  `referenceLine`) is read as a formula.

Tool behavior changed in this round:

- `wrapTool` now reports `success:false` for a tool result with `isError`. In
  local-config mode pi's bash tool returns a non-zero exit as such a result, so
  those calls now show as failed.
- The bundled skill folders (`ofox-office-*`) are neither denied nor added as
  readable roots for the generic file tools in local-config mode: they have no
  rule of their own there. The model reads a bundled skill through `use_skill`.

Tests:

- The runner's 3 s timeout test (`run_engine_kills_what_the_engine_started_and_keeps_its_output`)
  is fragile under heavy parallel load: a new stand-in executable can take longer
  than the deadline to start for the first time on macOS. The document tests take
  turns for that reason; a run of the whole workspace beside other heavy jobs can
  still fail it.
- The process-reaping case of `assistant-runtime/test/mcp.test.mjs` failed once
  under load and passed on rerun; its cause is not established.

## PPT native acceptance follow-up (2026-11-11)

The task-private macOS WKWebView harness used real Host preview commands to
reach actual page 70 of a 70-page file. All 70 backend pages plus one scale-2
render passed; no model/key/provider calls were made by browsing. HTML input
and script interactions worked in the opaque frame. Direct WK message-handler
probes with missing/invalid invoke keys did not reach the dummy native command;
main-frame positive control remained 1. This is bounded private macOS evidence,
not installed-package or Windows certification.

WPS 12.1.25895 retained one native equation symbol edit and one native chart
style edit after save/close/reopen. Chart data-cell editing remains unverified:
both the generated business probe and a control chart created and saved inside
WPS failed to expose a usable data table, including after a normal restart.
Valid embedded XLSX and matching caches were found, so this does not establish
a generator defect; no source change was made for that behavior. PowerPoint
16.109.1 required an editing subscription. Font warnings remain.
See the [implementation acceptance record](superpowers/research/2026-11-10-ppt-file-workspace-acceptance.md).

A signed task-private full Desktop bundle subsequently opened the real 70-page
fixture and interactive HTML through the ordinary account/session UI, with
explicit missing-resource errors and draft-only page/file references. All 41
upstream source pages were reachable in that bundle's gallery. The installed
application was not replaced. This verifies bounded macOS browsing behavior,
not complete native conversion or all platform security properties.
The private v2 full application also passed a 900px CSS-width light/dark check:
draft and selected page 6 survived chat/file switching, then the original theme
and window size were restored. Browsing made no model requests. This is distinct
from the earlier synthetic responsive fixture and does not certify every locale
or Windows.

The first real v1 automatic academic case produced no deck: the model inferred
pre-certified round-trip acceptance from native-editability requirements and
repeatedly selected an ineligible style. The hard filter correctly rejected it.
The v2 real retest reached an eligible Course Module plan, but delivered its six
pages only after a manual protocol correction. Before that correction the model
repeatedly omitted plan references, did not load the skill and used raw
`Presentation()` despite the style guard. The delivered file still has an
objective-rail overlap, literal/malformed auxiliary math labels and misleading
serial connections for parallel heads and causal masking. Schema success and
six native OMML objects do not turn this into unassisted Auto or quality
acceptance; target-application editing of this specific file was not tested.

Current source primes only the verified, unshadowed builtin PPTX instruction
body for eligible Host PPT context, preserves explicit skill selection and
does not autoload an external skill. It fills missing plan references only from
this run's Host-confirmed frozen tuple; explicit mismatches remain rejected.
The native parallel helper and lesson-region diagnostics have focused
regressions and independent review. A one-page self-authored fork/join example
also passed a bounded WPS round trip: four native branch paths and arrowheads
were visible, one branch's text edit survived save/close/reopen, and all eight
native connectors remained. Connector movement and editing every object were
not tested. These focused repairs do not establish unassisted native model
acceptance; the first v4 terminal cases below still failed quality acceptance. A helper
example does not repair or certify the v2 artifact. Legacy PPT guidance can redundantly call `use_skill` after priming;
ordinary chat does not prime merely because it carries workflow version 1.
See the [primer boundary](superpowers/research/2026-11-11-ppt-builtin-primer.md).

A separate saved-stream reference gap was demonstrated and repaired without
weakening identity or canonical missing-file guards. An initial completed-file
reference no-op did not recur after reopening and remains unexplained.

At the end of the v2 step, the shared US$3 task ledger's conservative upper
amount was US$0.765071, leaving US$2.234929 at that checkpoint;
no images have been generated and the one-image limit remains. These are
conservative quoted amounts and retained unknown reservations, not reconciled
invoices. In the v2 retest, 25 complete provider receipts correspond to the
displayed `modelRequests=26` because that aggregate adds one local cost-zero
document generation. The metric has not been changed; it is not evidence of an
extra provider POST, and it does not release the older unknown network outcome.

Historical v3 checks remain distinct: that four-worker frontend run passed 3282 tests
with one skip, and Rust passed 4494 tests with 27 ignores across 22 result
groups. An earlier concurrent frontend run expired a module-level 60-second
Library fixture during its 67.632-second suite; its isolated 33 tests passed
and the unrelated fixture was left unchanged. Runtime tests use the existing
task-private Node 22 with explicit Python/stdio overrides; this does not waive
packaged hardened-runtime checks. The first final runtime run
passed 673 of 674 tests, with one process-exit observation failure. Its isolated
security case then passed, and the same source passed a complete serial rerun
of 674/674 with no failures or skips. The initial failure remains recorded; its
cause is not established. Those checks do not establish v3 native model
acceptance.

The final private v4 bundle, still version 1.5.0 and not installed or published,
has main SHA `6914a1e747c2a7808a966c0f316d18b0f09e573865b4a5ab1cd9872a17fa5550`.
Its deep/strict signature, nine nested Mach-O signatures, Team, entitlements,
source/staged/bundled runtime and skills, and embedded current renderer payloads
passed independent static inspection. The installed `810f…57a93` bundle remains
unchanged. Notarization was explicitly skipped; these checks do not establish
Gatekeeper, Windows or generated-deck quality acceptance. The final v4 gates
passed 4499 Rust tests with 27 ignores across 22 result groups, 685/685 runtime
tests without skips, and 3314 frontend tests with one skip; Rust fmt/all-targets
Clippy and frontend type/format checks passed. Prior failed runs remain history.

Five original upstream decks expose all 41 source pages (10/10/7/8/6), but all
five native adaptations remain partial, each with
`originalPagesIndividuallyMapped=0`. Navy is a zero-content-page builtin master
with three layouts. The eight-page native examples are self-authored previews,
not complete upstream layout conversions or generated user deliverables. Source
font substitution, architectural clipping and fixed preview-only Chart.js
remain disclosed; viewing every source page does not certify conversion parity.

The v4 bounded math subset now supports transpose/ellipses, braced nested square
roots and short literal upright names, while unsupported macros and malformed
input still fail before shapes are added. A separate self-authored proof passed
one actual WPS radical-internal `+1` edit and save/close/reopen: all five native
equations and five radical structures remain, and the source is immutable.
An earlier plain-paragraph prefix edit was not counted. Font warnings and WPS
left-alignment of the edited equation remain; other objects, generated cases,
chart data-cell editing and PowerPoint/Windows are not certified. The preview
worker also repairs the narrowly bounded two-decimal OfficeCLI image-fill size
rounding observed in a real formula fixture; it preserves the exact-ratio path
and existing SVG/image security guards without changing the source SVG/PPTX.

Preparation of streamed tool arguments now has a distinct next-step status,
not an execution or completion record; arguments/source are not exposed. Plan
confirmation has its own label. The optional text cost setting is a per-message
pause threshold, including an explicit USD0.10 value, not a server-enforced
hard cap: the final request and retries can add charges. Omission preserves
existing monitoring behavior; explicit cost-only monitoring grants no new
local/MCP permissions. Queue and branch snapshots retain their original value,
and unknown usage still stops later requests. Public quotes and reserved upper
amounts are not reconciled invoices; the first v4 terminal cases and their
checkpoint ledger are recorded below, separately from any later requests.

The v4 compact workspace keeps secondary draft actions and long notes in menus
or preview details. Native 1000px split and 900px light/dark captures show the
current page; original System theme, window size and test draft were restored.
An initial persistent-blank interpretation was withdrawn after two independent
reviews found the full page readable in all four dark/System captures. A
possible transient observation was not reproduced; no speculative repaint
patch or v5 was made for that observation. This browsing check caused no additional paid calls and
does not certify every language, viewport or platform. The separate v4 UI
receipt preserves this scope without rewriting the earlier copy-time receipt.

### First native generation terminal results and repairs

The real v3 GLM academic Auto case and all three v4 GPT-5 Mini Auto cases are
not quality acceptance passes. The v4 cases used ordinary Desktop sends,
search disabled and no human tool-protocol correction. Their actual user
deliverables are distinct from the self-authored eight-page style previews.
The private evidence paths below are relative to this document and are not
published assets; see the
[main native acceptance record](superpowers/research/2026-11-10-ppt-file-workspace-acceptance.md).

| Native case               | Verified result and remaining failure                                                                                                                                                                                                                                                                                              | Quoted conservative upper amount                                                                                                                      |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| v3 GLM academic Auto      | Six pages and six OMML objects; serial head topology, literal math labels and one omitted formula in the old JPEG. A connection error followed delivery. Quality **FAIL**; 22 complete provider receipts and one unknown outcome.                                                                                                  | [US$0.841670, including US$0.668469 unknown reservation](../.real-eval/ppt-completion-2026-11-10/native-glm-v3-academic-auto-final/budget-audit.json) |
| v4 academic Auto          | Six pages; parallel head topology improved, but only page 4 has the two OMML objects. Core formulas on pages 3/5 remain plain text, page 6 exposes dictionary text and page 5 has literal backslash-newline labels. The file was retained before `assistant_cost_limit` at the explicit US$0.10 pause threshold. Quality **FAIL**. | [US$0.164670](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v4-academic-auto/budget-audit.json)                                             |
| v4 business Auto          | Eight pages; two native charts and two embedded workbooks match the supplied simulated data. This is partial structural/content evidence: the page 4 chart frame overlaps external explanatory text by 0.25 inches, also visible in WPS. Quality **FAIL**, despite completed delivery and zero automated findings.                 | [US$0.102397](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v4-business-auto/budget-audit.json)                                             |
| v4 fictional product Auto | Eight pages, nine remaining warnings and dictionary text on page 4. The optional cover image fell back to native visuals without an image-generation call; a missing trusted budget summary caused repeated requests for authorization already given. Quality/workflow acceptance **FAIL**.                                        | [US$0.118891](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v4-product-auto/budget-audit.json)                                              |

At this terminal-record checkpoint, the shared US$3 task ledger's conservative
occupied amount is **US$1.992699**, leaving **US$1.007301** unreserved, with
**zero generated images out of the one-image allowance**. Both historical
unknown-outcome reservations remain. These are quoted upper amounts and
retained reservations, not invoices; actual invoice fields remain null. A later
send must reserve against the then-current ledger rather than reset this
checkpoint. Provider receipts are counted separately from the aggregate metric
that also includes local document operations.

The academic case's
[independent quality inspection](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v4-academic-auto/quality-independent-audit.json)
and [preview/vision evidence](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v4-academic-auto/preview-vision-audit.json)
remain separate from cost accounting. JPEG blocks reached later vision SDK
context; no raw HTTP request body was captured, and this does not prove that
the model understood or corrected each page. The Host's existing native-math
check requires an actual OMML object on each page whose `visual.kind` is
`native-math`. Inherited `editing.formulas=true` is a conditional editing policy,
not an object inventory or a demand to add formulas to every page. Page 4
therefore met that structural contract without certifying all core formulas.

WPS 12.1.25895 also retained a native line-chart style edit in a private copy of
the actual v4 business file after save/close/reopen; all eight pages, both
charts and original data remained, and the source SHA was unchanged. This is
the bounded positive in the
[business WPS receipt](../.real-eval/ppt-completion-2026-11-10/office-edit/business-v4-wps-receipt.json).
Data-cell editing still did not expose a usable table and remains unverified.
The style probe introduced an axis-title placeholder and is not a corrected
layout deliverable. The earlier WPS-created control has the same data-table
limitation; valid embedded workbooks do not establish either application data
editing success or a generator defect. PowerPoint/Windows remain unverified.

Meaningful failing regressions and independent reviews now cover the targeted
source repairs: typed text/relation/body inputs reject structured-object
stringification before adding objects; legal rich text, numeric KPI values and
intentional literal strings remain supported. Final-save chart/text frame
diagnostics include late-added external text and direct native charts, inspect
only authorized source pages, retain bounded warnings and never move objects.
These are top-level, axis-aligned frame checks, excluding groups, rotations,
chart internals and actual glyph bounds. See the
[typed-text/chart focused receipt](../.real-eval/ppt-completion-2026-11-10/python-typed-chart-focused-receipt.json).
The repair does not infer mathematics from every ordinary string or certify
visual quality.

The Host freeze response now exposes a trusted visual-budget snapshot derived
from the frozen policy, carried spend, held reservations and available planned
assets. Unknown remainder stays unknown; count-only authorization stays
distinct. The runtime preserves this summary and avoids asking for the same
authorization again when `canAttempt=true`. It grants no capability and makes
no reservation: each real image attempt still checks identity, plan, price and
atomic limits. See the
[Rust snapshot receipt](../.real-eval/ppt-completion-2026-11-10/visual-budget-rust-implementation-receipt.json)
and [Node snapshot receipt](../.real-eval/ppt-completion-2026-11-10/presentation-visual-budget-node-final-receipt.json).
The shared three-language cost-stop copy now describes the configured cost
pause threshold without hard-coding US$1 or claiming a hard cap; already-sent
requests and retries may add charges. See its
[focused receipt](../.real-eval/ppt-completion-2026-11-10/text-cost-pause-copy-focused-receipt.json).

The post-live renderer gates passed typecheck, formatting and **3323 tests with
one skip and no failures**; 1083 frontend source/test/configuration hashes were
unchanged during the gate. The
[frontend receipt](../.real-eval/ppt-completion-2026-11-10/frontend-post-live-fix-receipt.json)
does not cover Rust/runtime or native quality. The final runtime gate separately
passed **700/700 with no failures, skips or cancellations**, with all 323 raw
source/skill/test inventory records unchanged; see the
[final runtime receipt](../.real-eval/ppt-completion-2026-11-10/runtime-title-layer-final-receipt.json).
After one Clippy-only equivalent predicate rewrite, the new pipeline reran fmt,
all-targets Clippy and all Rust tests: **4503 passed, 27 ignored, zero failed**.
The original pipeline stopped at Clippy and remains a failed historical
pipeline, despite its earlier passing tests. The original freeze was retained;
the new 1953-file after-Clippy freeze was used for the successful
[v5 pipeline](../.real-eval/ppt-completion-2026-11-10/v5-after-clippy-v5-private-check-build-pipeline.json).

The normal signed private v5 bundle has main SHA
`64aff399ba6d0859ce71a36de5b5cfe51f6dcb3612405e4b2eab09c24435a78c`.
Its independent [static audit](../.real-eval/ppt-completion-2026-11-10/private-full-desktop-static-audit-v5-1794352951870.json)
passed nine Mach-O signatures, deep/strict verification, Team/DR/entitlements,
source-to-staged-to-bundle equality and exact embedded current renderer
evidence. The audit matched the 1953-file freeze, 323 runtime records and 1083
frontend records at that time. The installed `810f…57a93` application remains
unchanged; this is not publication, notarization/Gatekeeper, Windows or new
native-model quality acceptance. The immutable v4 deliverables and failures
remain; a rebuilt helper does not repair them retrospectively.

The original v4 product page 2 also hid its native main title in WPS while the
application JPEG showed it: an opaque header painted above the cloned title
placeholder. Other content pages share the XML risk, but only page 2 was
directly observed in WPS. The helper repair now keeps the same native title
element/ID and moves it above existing decorations, preserving other object
order. Real saved-XML regressions and independent review passed, with 11 related
focused checks. A separate two-page self-authored proof showed the page 2 title
and subtitle in WPS 12.1.25895, retaining native placeholder ID 2 and unchanged
file bytes; see the [bounded display receipt](../.real-eval/ppt-completion-2026-11-10/title-layer-proof/wps-native-receipt.json).
That proof was not edited or saved in WPS and is not the regenerated product
case, an all-page review or an editing/save/reopen certification.

v5 does not include the prepared Pro image compatibility repair. Its planned
Host path injects `n=1` even when the model omits the optional argument; the
exact `volcengine/doubao-seedream-5.0-pro` contract rejects that wire field.
Prompt omission is therefore insufficient. The earlier admission advice is
[explicitly superseded](../.real-eval/ppt-completion-2026-11-10/seedream-image-response-admission-receipt.json)
and is not executable authorization for paid planned images in v5. After a
meaningful four-case RED with two failures and two passes, the narrow source
fix was applied and independently reviewed. It omits the wire field only for
that exact model while retaining one-image Host counting and budget guards.
The [focused GREEN passed 4/4](../.real-eval/ppt-completion-2026-11-10/pro-wire-green.log).
The v6 Rust rerun passed **4507 tests with 27 ignores and zero failures** across
22 result groups; fmt and all-targets Clippy exited 0. Its normal signed private
release build, copy and deep/strict checks completed in the
[v6 pipeline](../.real-eval/ppt-completion-2026-11-10/v6-private-check-build-pipeline.json).
The v6 main SHA `c5e6b633385253c7896c429cb387d430e7836e8d7c5d1f2afa729a72d158f51f`
also passed independent [static inspection](../.real-eval/ppt-completion-2026-11-10/private-full-desktop-static-audit-v6-1794354879039.json):
nine Mach-O signatures, the 1953-file v6 freeze, runtime resources, exact
embedded renderer, 323 runtime records and 1083 frontend records matched. The
Pro-specific compiled marker was present and the installed application remained
unchanged. The later actual image/import result and newly discovered auth P1 are
recorded below; static provenance did not certify security or deck quality. The source repair is not part of the immutable v5 bundle.

### Native v5 academic Auto and manual terminal checkpoint

The two actual v5 academic runs used the same prompt hash, GPT-5 Mini and
US$0.15 text pause threshold, with search and image generation off. Auto chose
Course Module; the other run explicitly chose and froze Cartesian. This is a
bounded same-prompt comparison, not a general routing or model-quality result.

The [Auto terminal receipt](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v5-academic-auto/run-audit.json)
records a six-page, 117329-byte file, SHA `bb4e06d1…ecfd4`, with 16 complete
provider receipts and no unknown request. After repeated Python failures and
one successful preview, the next create call was stopped by
`assistant_cost_limit`; the preview was retained automatically. All required
core formulas are now native across pages 3/4/5, with seven OMML objects, but
the [independent quality audit](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v5-academic-auto/quality-independent-audit.json)
still reports **QUALITY_FAIL**: incorrect diagram connections, raw noncore
notation, an undersized formula frame and page 6 overlap remain. Native object
coverage is a partial positive, not final quality acceptance.

A private copy of that actual Auto file passed one bounded WPS 12.1.25895
formula edit/save/close/reopen: the internal `X` in the first formula on page 3
was changed to `Z`, remained after reopening, and all six pages/seven equations
remained. The source SHA was unchanged. An initial ordinary-text `+0` attempt
was undone and was not counted. See the
[actual-file WPS receipt](../.real-eval/ppt-completion-2026-11-10/office-edit/academic-v5-auto-wps-receipt.json).
The deliberately altered copy is not a corrected scientific deliverable, not
all seven equations were edited, and the original quality failure remains.
Chart data-cell editing and PowerPoint/Windows are not certified by this edit.

The [manual Cartesian terminal receipt](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v5-academic-manual-cartesian/run-audit.json)
records six pages, 107281 bytes, SHA `1a11d3ac…f62c8`, 13 complete receipts, no
unknown request and normal completed delivery. Two previews reduced automated
findings from seven to zero. The independent
[scientific/math/visual review](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v5-academic-manual-cartesian/quality-independent-audit.json)
and actual WPS inspection nevertheless conclude **QUALITY_FAIL**: page 3 role
text crosses column separators, and page 6 reverses the ordinary query-row,
key-column causal-mask visibility convention without defining alternative axes.
Four OMML objects correctly cover the six core expressions because Q/K/V share
one composite object. Four head-to-Concat geometric arrows do exist on page 5;
their coincident baseline and frame overlap weaken readability, rather than
proving missing head connections or serial dependency. Raw math captions also
remain. The [manual WPS receipt](../.real-eval/ppt-completion-2026-11-10/office-edit/academic-v5-manual-wps-receipt.json)
records read-only visits to all six pages, not a formula-edit round trip for
this file. Neither completed delivery nor zero automated findings is a quality
pass. All six right-workspace pages were visited for both real files;
the `ui/v5-native-academic-page1..6.png` and
`ui/v5-native-manual-cartesian-page1..6.png` captures are page-accessibility
evidence, not scientific or editing certification. Browsing made no extra
generation request.

At this later terminal-record checkpoint, the Auto
[quoted upper amount is US$0.248488](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v5-academic-auto/budget-audit.json)
and manual [US$0.156354](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v5-academic-manual-cartesian/budget-audit.json).
Their SDK estimates are approximately US$0.15459431 and US$0.09676194,
respectively; invoices remain unknown. Added to the earlier US$1.992699
checkpoint, the shared US$3 task's conservative occupied amount is now
**US$2.397541**, with **US$0.602459** unreserved and **zero generated images out
of the one-image allowance**. Both historical unknown-outcome reservations
remain. This is a timestamped quote/reservation checkpoint, not an invoice or a
budget reset, and the documentation update makes no additional paid call or
overall completion claim.

### Native v6 product revision and auth pause

The actual private v6 run revised all eight referenced pages of the existing
fictional product deck, with text pause threshold US$0.10 and at most one Pro
image under US$0.05. The source SHA `53542a63…2051219` stayed unchanged. The
[run audit](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v6-product-targeted-one-image/run-audit.json)
records a cost-paused terminal state, not a completed final model review.

One logical image submission produced two HTTP attempts: a known-unbilled 401
refusal followed by a successful 200. One valid 185044-byte JPEG was imported,
SHA `4b47e829…bd16d`; five model tool attempts are not five provider image
submissions. The separate JPEG was **never embedded** in either eight-page
revision: v2 SHA `6666a885…efa80` has 19 warnings, and v3 SHA
`3861ddcc456f4e37ff7dfa2d73e608a6bb773390e14944f8d4e85b66ca94b10d`
has 17 warnings, 39732 bytes, zero media parts and zero image relationships.
Slide IDs 256–263 and all notes were retained, but the
[independent audit](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v6-product-targeted-one-image/quality-independent-audit.json)
concludes **QUALITY_FAIL**: inherited titles remain behind opaque headers with
white-run formatting lost, multiple pages duplicate overlapping body text, and
page 4 retains old giant cards/connectors beneath new objects. Removing its raw
dictionary text is only a partial improvement. No new empirical product claims
were found, but readability and image placement still fail.

The successful code passed a chat attachment ID to `image()`, which requires a
frozen planned asset ID, then swallowed the resulting exception. The Host had
returned a controlled `presentationAssetId` and completed the binding; generic
model-visible generation guidance still pointed toward attachment IDs. This
confirmed namespace/guidance gap does **not** demonstrate a broken Host asset
resolver. See the [image import audit](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v6-product-targeted-one-image/image-import-audit.json).
All eight v3 pages were reached in the right workspace, and switching the
original `53542…` to revision `3861…` made no new model/image call. The
`ui/v6-native-product-v3-page1..8.png` captures establish accessibility, not
WPS/PowerPoint/Windows editing or final visual acceptance.

The [cost audit](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v6-product-targeted-one-image/budget-audit.json)
contains 13 complete new text receipts, no new text unknown outcome and a
US$0.246775 conservative text upper amount, plus US$0.050000 for the actual
single-image quote. This run's upper amount is **US$0.296775**. SDK text/image
estimates are approximately US$0.10194537/US$0.05; invoices remain unknown. At
this later checkpoint, the shared US$3 task is conservatively occupied by
**US$2.694316**, leaving **US$0.305684** unreserved, with **one actual generated
image out of the one-image allowance**. Earlier checkpoints and both retained
historical unknown-outcome reservations remain. The dollar balance does not
renew image-count authorization or represent an invoice.

A separate code-level **P1** was then found: a RunControl scrub-value collection
holds OFox keys plus MCP env/header private values, while five media auth
getters select `.secrets().last()` as the Bearer. Nine OFox-key update sites
must also be separated from scrub-only additions. The scope includes chat
image/video/speech and standalone image/video paths. The required boundary is
a dedicated in-memory OFox key slot, scrub-only MCP additions, and post-refresh
identity/generation/cancellation checks before accepting a rotated key. Picking
the first scrub entry would instead break rotation. Synthetic RED and the fix
are still pending at this checkpoint; prior build/static/general test passes
do not establish this security fix.

No actual MCP credential values were read or compared, and no actual leak is
established. The observed 401→200 alone neither proves the P1 caused that
request nor explains its refusal. Root confirmed private v6 was idle and
normally quit PID 90014, with further paid calls paused. This code issue,
failed deliverables and unverified fixes keep the PPT topic incomplete; the
local todo and unrelated old tasks were not changed.

### V7 bounded auth regressions and private-build gates

The source now separates the in-memory OFox authentication slot from scrub-only
MCP values. Asynchronous rotation rechecks identity, Host generation and
cancellation before accepting the replacement. A matching private pending RPC
reply updates the same run-local redaction array before resolving content, using
a top-level private sideband rather than model-visible result/details. Stream
flushes recalculate their retained tail for longer rotated credentials. The
[1956-file source review](../.real-eval/ppt-completion-2026-11-10/v7-source-security-independent-review.json)
is a bounded source/freeze observation, not a successful signed-v7 or final wire
gate.

The initial [Host wire run](../.real-eval/ppt-completion-2026-11-10/v7-auth-final-wire-green.log)
failed one of three tests: OFox A→B→B authentication and public event
redaction worked, but raw native JSONL retained the synthetic MCP canary. The
initial trusted run parameters omitted `knownSecrets`; only whole-run key retry
seeded that list, while the MCP protocol cleaner did not cover ordinary model
prose. The [initial-list fix](../.real-eval/ppt-completion-2026-11-10/ofox-key-slot-security-draft/08-seed-trusted-run-scrub-list-after-red.patch)
and [non-rotation regression](../.real-eval/ppt-completion-2026-11-10/ofox-key-slot-security-draft/09-initial-mcp-native-echo-regression.patch)
received independent static review and were then applied. The later actual
[Host wire rerun](../.real-eval/ppt-completion-2026-11-10/v7b-auth-wire-green.log)
passed **3/3**, zero failures, in 16.30 seconds: image and video/speech auth on
both apex choices, initial native long-canary redaction without a 401/MCP tool
call, and rotated A→B→B auth plus public/event/native redaction. The earlier
failure remains historical evidence, not a first-attempt pass.

The new [v7b source freeze](../.real-eval/ppt-completion-2026-11-10/v7b-final-source-freeze.json)
contains 1956 files, SHA
`dbb6447d5dfccb7f260a4c6e19a42f64f93c91c1f9c5c007e326a0c99e8c3a32`.
The [final Rust summary](../.real-eval/ppt-completion-2026-11-10/v7b-final-check-summary.json)
records **4514 passed, zero failures, 27 ignored, 22 result groups**; library
4276/25 ignored and Host 122/2 ignored are included. Fmt and all-targets Clippy
exited zero. The complete runtime [710/710 receipt](../.real-eval/ppt-completion-2026-11-10/runtime-v7-guidance-final-receipt.json)
is reused with all 326 source/skill/test hashes unchanged after the subsequent
Rust-only initialization/test edits. The frontend [3323 passed, one skipped,
zero failed receipt](../.real-eval/ppt-completion-2026-11-10/frontend-post-live-fix-receipt.json),
including typecheck/format, is likewise reused with all 1083 frontend/test/config
hashes unchanged. Neither reuse is described as another complete test execution.

All [six build/copy/check stages](../.real-eval/ppt-completion-2026-11-10/v7b-v7-private-check-build-pipeline.json)
exited zero. The normal signed private v7 app has main SHA
`ae8f8b1434e58d1aff9e6ca3b72c25cdd94ab9735126d9b48a8ad3239caa20e8`.
Its [static audit](../.real-eval/ppt-completion-2026-11-10/private-full-desktop-static-audit-v7-1794359945682.json),
receipt SHA `356395a9f6f2e745af2f8ba9ceb3e4053a468d8c89ec4c1ef75b62a00529f81f`,
has no findings: nine Mach-O signatures/Team/DR/runtime/entitlements, exact
source→staged→bundle resources, 11854 staged/11855 bundled runtime files with
only declared `PI_LICENSE` added, and 234 renderer asset keys/233 raw payloads
plus processed entry HTML accounting for all 234 dist files. The installed app
remains `810f6031…1d57a93`, without replacement, publication or new notarization.
The pipeline's historical `appLaunched:false` describes the copy checkpoint,
not a subsequent native GUI result. These checks do not establish a new v7
model deliverable or native quality pass and add no new case cost.

Short MCP credentials remain an explicit limitation. Generic
[conversation scrubbing](../assistant-runtime/src/security.mjs) replaces known
strings only when they have at least eight characters, while
[MCP credential identification](../assistant-runtime/src/mcp-security.mjs) can
recognize shorter passwords. Long-canary and rotated-key regressions therefore
do not establish complete native-history or model-context redaction for every
configured credential. This PPT-scoped repair does not broaden generic protocol
control-scalar rewriting. No universal credential-scrubbing or security-clean
claim is made; all security fixtures are synthetic, with no actual MCP
credential read or actual leak established.

### Offline authored product control, not model acceptance

A separate **OFFLINE AUTHORED ADAPTATION** control produced eight pages,
193447 bytes, SHA
`9f8692abaea9b7a252d177e05f774a6fe76e825c8ad5b395888870351966cb20`.
Its [structural/visual record](../.real-eval/ppt-completion-2026-11-10/offline-focus-v7-control/structural-visual-receipt.json)
retains all eight source slide IDs and notes; pages 2/3/5–8 retain all non-title
slide XML. Existing native title ID 2 and rich-text runs remain intact, with
seven content titles raised above the opaque headers. The cover embeds only the
existing JPEG `4b47e829…bd16d`, and page 4 removes the old body before adding
three compact native nodes and two directional native connectors. Independent
review checked ZIP structure, all notes/title runs, the contact sheet and full
pages 1/4.

Root actually viewed all eight pages read-only in WPS 12.1.25895 and confirmed
visible titles, the cover image and two rightward page-4 arrows. The file SHA
was unchanged; the [WPS receipt](../.real-eval/ppt-completion-2026-11-10/offline-focus-v7-control/wps-readonly-receipt.json)
has SHA `435e4a88132ea3f99d99eb360869178fcf8316d48597d64971268c5d1186f25a`.
The cover still breaks the Chinese word 工作 across lines, and WPS reported
missing fonts. This is not an editing/save/reopen test, a PowerPoint/Windows
pass, a new Desktop/model quality pass or complete source-template conversion.
It makes zero new API calls and does not rewrite the actual failed artifacts,
change the shared budget or complete the PPT topic.

### V7 native Nano runs failed before document execution

Engineering/build gates passed; the two Nano workflows below failed. A later
human-assisted Mini file-revision check has a bounded positive result, recorded
separately below; autonomous/overall quality acceptance has **not passed**.
Private v7 was actually launched through native
UI with its expected executable SHA, as recorded in the
[launch receipt](../.real-eval/ppt-completion-2026-11-10/native-v7-launch-receipt.json).
The original eight source pages were viewed and cited in both subsequent Nano
attempts. This establishes source-preview/reference access, not a new revision.
The original source `53542a63…2051219` and existing JPEG `4b47e829…bd16d`
remained unchanged.

The first `openai/gpt-5-nano` source-repair run, `1a1c668f…`, used a US$0.03
text pause threshold and made seven completed HTTP 200 text requests. Its seven
tool attempts comprised six pre-execution schema refusals and one current-plan
refusal: misspelled `diaphrams`, missing style/name/format/scenario, wrongly nested
source, and a historical plan reference. It ended with `assistant_cost_limit`
and a failed operation, without an accepted plan, generation or artifact. The
[run audit](../.real-eval/ppt-completion-2026-11-10/native-nano-v7-product-source-reuse/run-audit.json)
does not show execution of the Python/PPTX engine; this is not evidence of an
engine execution failure.

The second run, `9b62f8ce…`, supplied a complete human-authored plan and
offline-tested code, with an actual US$0.04 text pause threshold. All five
candidate argument objects exactly matched the supplied `01` JSON. The first
four executions returned `eligible`, `product-launch` and an explicit next
action to freeze; the fifth identical attempted call hit the normal repeat
guard. The last same-run assistant message is `assistant_repeat_limit` and the
operation failed. No freeze/create, supplied-code submission/execution,
preview ID or artifact followed. See the corrected
[run audit](../.real-eval/ppt-completion-2026-11-10/native-nano-v7-assisted-source-reuse/run-audit.json).
An initial private audit selected the earlier completed assistant segment;
selecting the final error segment corrects that audit and does not establish a
product status bug. The actual sent-body SHA is
`17796a567ec3d5de40907a2a144d355aa25ae6b80d465dd2f830cdd6f10a239c`;
the unsent US$0.02 draft was superseded before Send. Its US$0.239862 task
reservation was not actual spend.

These are observed tool-orchestration reliability failures, including repeated
successful first-step calls despite available next-action feedback. The second
run was explicitly human-assisted, not autonomous design acceptance. Native
tool text proves feedback existed in the local SDK conversation; no remote
request-body capture or hidden-model motivation is asserted. Neither run
produced a new deck to assess visually, and their failure does not invalidate
the separate bounded offline/WPS control proof.

The [first cost audit](../.real-eval/ppt-completion-2026-11-10/native-nano-v7-product-source-reuse/budget-audit.json)
records a US$0.043250 conservative text upper amount and approximately
US$0.03253561 SDK estimate; the [second audit](../.real-eval/ppt-completion-2026-11-10/native-nano-v7-assisted-source-reuse/budget-audit.json)
records US$0.023796 and approximately US$0.01726515. Their seven/six text
receipts are complete, with no new unknown outcomes. Actual image/video/speech,
search and MCP tool calls were zero in both runs. At this Nano-only settlement
checkpoint, conservative occupancy was **US$2.761362**, the authorized total
was **US$3**, and **US$0.238638** was unreserved with no active reservation.
The one-image allowance is already consumed; historical unknown reservations
of **US$1.057077** remain, and actual invoices are unknown. Earlier budget
checkpoints remain historical, not resets. The final
[identity/idle/budget check](../.real-eval/ppt-completion-2026-11-10/v7-idle-final-identity-budget-check.json)
also confirms all 1956 source files unchanged, the same private/installed app
identities, no active run/reservation and no admitted budget increase at that
historical checkpoint.

A later offline-file import attempt was blocked by the locked Mac before an
import action occurred; it adds no new native import/editing pass. The
[next reviewable proposal](../.real-eval/ppt-completion-2026-11-10/next-native-gpt5mini-reviewable-proposal.json)
asks to raise the total authorization from US$3 to US$4 for one GPT-5 Mini
existing-source/image assisted-flow check, with no new image or installation.
Its US$0.08 SDK pause, tenfold quote factor, three terminal POSTs and bookkeeping
cushion give a proposed **US$1.189608** reservation. The proposal was not admitted
or reserved when written; the user subsequently approved US$4 and the actual
execution is recorded below. The historical proposal file is not rewritten to
erase its earlier state. The PPT topic and local todo remain incomplete, with
PowerPoint/Windows and broader quality limitations unchanged.

### Actual Mini assisted file revision and bounded WPS round trip

The user explicitly increased total text/image authorization to **US$4**, while
retaining the maximum of one generated image. Before/after cold-start public V1
catalog bytes matched, SHA `c123c8f3…6933400`, as recorded
[before](../.real-eval/ppt-completion-2026-11-10/v7-gptmini-cold-catalog-before.receipt.json)
and [after](../.real-eval/ppt-completion-2026-11-10/v7-gptmini-cold-catalog-after.receipt.json).
Actual request price snapshots also confirmed input/output/cache-read rates of
US$0.25/US$2/US$0.03 per million tokens. Private v7 PID 14344 retained main SHA
`ae8f8b14…caa20e8`; the installed app was not replaced and the 1956-file source
freeze was unchanged. The [ordinary Send snapshot](../.real-eval/ppt-completion-2026-11-10/v7-gpt5mini-assisted-source-reuse-sent.json)
used GPT-5 Mini, text pause US$0.08, image off/0, no search, all eight original
page references and the existing JPEG, rather than Retry of a Nano operation.

Run `265d5ddc…` and its operation completed. One freeze and two create calls
produced current plan `4b769dd8-9ed0-4796-b314-c3972bc2a79c`, revision 1,
Product Launch, then generated a preview and delivered that same fresh preview
without a second generation. The [run audit](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-assisted-source-reuse/run-audit.json)
records **no explicit candidates tool call**; the freeze entry's `checkedPlan`
still applied candidate/manual-style hard constraints. This is not evidence
that the model followed the explicit candidates step. It also did not forward
the provided plan/code verbatim: a closing composition string changed between
simplified/traditional characters, and actual code SHA `66a48fd0…f81190b`
differs from supplied `6dcdbbae…812f423`, with added `getattr` and broad exception
handlers. The artifact must stand on its own evidence; success here does not
validate suppressed failure paths as a general repair.

The actual Native output `Focus_V7_人工辅助功能复验.pptx` has **eight pages,
193447 bytes**, SHA
`757cf3afc05eb84dabf82c25ea0d4b58f2f32173063ef44bca59d3a5c72da728`.
All original slide IDs 256–263, eight notes parts, native title IDs 2 and title
`a:r` formatting were retained, with seven content titles above their headers.
Non-title bodies on 2/3/5–8 were unchanged. The sole media part is the exact
prior 185044-byte JPEG `4b47e829…bd16d`, only on the cover; page 4 removes the
old body and contains three compact native nodes and two native arrows. All
uncompressed parts match the previously verified UUID control, independently
compared against this actual Native artifact rather than substituted for it.
The [independent structural/eight-page render audit](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-assisted-source-reuse/quality-independent-audit.json),
SHA `c91666fe74df55d7a72d0b791da4d4de79978732633b60942c15d7ea623c3d2f`,
found no new blocker in this bounded repair. Separate sibling review checked
ZIP structure and all eight sidecar images. Cover 工作 wrapping, sparse retained
pages, small body text and font substitution remain limitations; this is not
publication-quality redesign or an autonomous Auto pass.

There were two actual JPEG tool-result blocks and matching pinned-SDK image
conversion evidence, recorded in the [vision-input audit](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-assisted-source-reuse/vision-input-audit.json).
No outbound HTTP body, provider image decoding or substantive per-page model
visual review was independently established. Image input availability is not
model visual acceptance. Root also actually viewed all eight new pages in the
Desktop right workspace, with `ui/v7-gptmini-native-page1..8.png` and the
[Desktop receipt](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-assisted-source-reuse/root-desktop-check-receipt.json).

The initial [partial WPS receipt](../.real-eval/ppt-completion-2026-11-10/office-edit/product-v7-gpt5mini-wps-partial-receipt.json)
recorded page 1 and independent picture selection before the Mac locked. That
historical partial evidence and the earlier Desktop `wpsNotYetOpened:true`
timestamp remain intact. Root subsequently actually viewed **all eight pages**
of this new Native file in WPS 12.1.25895 on macOS 26.5.1 (25F80), observing all
titles, the selectable cover image and page 4's three native nodes/two rightward
arrowheads. The [actual WPS receipt](../.real-eval/ppt-completion-2026-11-10/office-edit/product-v7-gpt5mini-wps-receipt.json),
SHA `aaf8c012ada3df012cd76e4d8f818daa10ead5d36e7e91c72a588b6640912314`,
records twelve screenshots.

In the separate copy `office-edit/product-v7-gpt5mini-wps.pptx`, Root changed
the middle page-4 native flow-node body from 选择目标 to 选择本次目标, saved with
Command-S, closed with Command-W to the WPS home page, and reopened the exact
same absolute path through Finder. The replacement and three nodes/two arrows
remained visible; `ui/wps-v7-gptmini-node-reopened.png` and the saved copy's XML
support this bounded round trip. The original generated file remains 193447
bytes, SHA `757cf3af…c72da728`. The WPS copy changed from that same initial SHA
to **230671 bytes**, SHA
`8176ac5ec5b6bf5c5ded158f8bddd8f9b4b7bf13b0bf4d4c1e7fad4a4064490b`;
the edited copy is not presented as the unchanged original deliverable.

The [independent post-save structural audit](../.real-eval/ppt-completion-2026-11-10/office-edit/product-v7-gpt5mini-wps-independent-audit.json),
SHA `8d33c3343470dc36eb60ee404ee50d1e86eba74cc187a38dc903e634b27dad5d`,
corroborates retained slide IDs, object IDs/z order/direct geometry, three node
properties and both canonical native-arrow XML trees. The only ordinary text
content change is page-4 textbox ID 14. The JPEG part was renamed `.jpg` to
`.jpeg` with all 185044 bytes unchanged. This was **a whole-WPS save, not a
one-part XML patch**: 52 common file parts were rewritten and 16 directory
entries added. All eight notes text sequences were preserved, but their raw
XML was rewritten. East Asian font declarations changed from `Microsoft YaHei`
to 微软雅黑, with added lang/panose/default declarations. Logical title size,
weight and colors are retained; raw title-run/notes XML is not byte-identical.
The original Native-file byte-preservation checks do not apply to the edited
WPS copy, and all eight pages were not individually re-viewed after reopening.

This proves this actual Native file's eight-page WPS display and **one native
node-text edit/save/close/reopen**, not editing of every object. Nodes or
connectors were not moved; chart/data-cell/formula acceptance is not inferred,
and this product file contains no native chart or formula. Missing-font
warnings and the cover 工作 line break remain, with PowerPoint/Windows
unverified. It is still a human-plan/code-assisted positive, not autonomous or
overall quality acceptance. The earlier offline control's separate WPS
eight-page proof is not used to certify the new file. These WPS operations made
zero new model/image calls and changed no budget ledger or todo status. The
next autonomous academic case initially had only a manually named empty chat
and Auto/off0/US$0.06 preparation. The Mac locked before prompt paste; its
[not-sent checkpoint](../.real-eval/ppt-completion-2026-11-10/v7-academic-autonomous-not-sent-lock-checkpoint.json)
preserves that historical zero-message/operation, null-activeRun and no-new-call
checkpoint. The later actual Send and failed outcome are recorded below; this
old empty-chat snapshot does not describe its current status.

Four text requests had complete usage, no new unknown outcome and actual
`.25/2/.03` price snapshots. The [budget audit](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-assisted-source-reuse/budget-audit.json)
charges all input/cache at the highest input rate and rounds each request up,
giving a conservative quoted upper amount of **US$0.081768**, separately from
the **US$0.05062229** SDK estimate and null invoice. Root independently
recomputed and settled it. At this assisted-case checkpoint the shared US$4
budget was **US$2.843130** occupied, **US$1.156870** unreserved, with no active
reservation. Generated images were then **one out of one**, with no new
image/video/speech/search/MCP call
in this run; historical unknown reservations of US$1.057077 remain. The earlier
US$4 proposal had been approved and executed for this case; later authorizations
and runs are recorded separately below. This assisted functional positive does not erase
earlier Auto/manual failures or change 41-page source coverage/partial-native,
PowerPoint/Windows or incomplete-topic boundaries.

### Fresh autonomous academic failure and bounded v8 math repair

After unlock, the fresh academic chat actually sent the six-page natural-language
task without user-supplied page JSON or tested Python. This is a new corrective
case, not a same-prompt causal A/B. Run `f1ff9140…` corrected three schema refusals
and froze Course Module plan `5c22ba79-bd00-418b-b807-cde3617fe6c7`, revision 1.
Its first create **reached real Python** and failed on unsupported `\sin` in
`equation()`; the second revised-code create was stopped by the cost gate before
execution. Unlike the earlier Nano orchestration failures, this exposes an
actual bounded helper limitation. It produced no file, preview or quality pass.
See the [run audit](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-academic-autonomous-corrective/run-audit.json).

Eight complete HTTP 200 receipts, no new unknown outcome, gave a conservative
upper **US$0.079948**, separately from SDK estimate **US$0.07107410** and unknown
invoice. US$0.06 was a post-usage pause trigger, not a hard spend cap. Root's
[settlement](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-academic-autonomous-corrective/root-settlement-receipt.json)
then recorded occupancy **US$2.923078**, remaining **US$1.076922**, without
releasing historical unknowns. The [budget audit](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-academic-autonomous-corrective/budget-audit.json)
does not treat no artifact as zero model calls.

The v8 parser adds only exact `sin/cos` to the existing upright roman structure
and `quad` as U+2003. Unknown macros and size/depth/atomicity limits remain, with
no TeX interpreter, dependency, file or network permission added. Fixed EM SPACE
is a character guarantee, not identical physical spacing across fonts/Office.
Meaningful RED was two failures/one pass, followed by **36 focused passes** and
independent review; Skill body is 25795 characters, and only LaTeX gets a PNG
fallback. Planning guidance also went RED→**41 passes**, clarifying density,
visualNeeds and `sequence` without loosening enums or guards. See the
[repair receipt](../.real-eval/ppt-completion-2026-11-10/math-sincos-v8-proof/implementation-receipt.json).

An [exact zero-API offline replay](../.real-eval/ppt-completion-2026-11-10/math-sincos-v8-proof/offline-replay-receipt.json)
of the original 4017-character failed code now yields six pages/five OMML and
zero schema errors, but retains four math-width warnings and the ordinary
`\cdot` string's invalid escape warning. It is **not accepted quality or a
repaired Native run**, and no substitute attachment was added to the failed
chat. A separate one-page self-authored sin/cos probe passed one WPS internal
variable edit x→z, save/close/same-path reopen; four OMML/four fallbacks remain
and the third PNG was visibly updated to z. The source `1d527bcb…0219fa` stayed
unchanged; edited copy SHA
`b726551d085e1c92700ca726afe1c25df5849f83acf44c160dc74fb44547b27a`,
67281 bytes. The [WPS receipt](../.real-eval/ppt-completion-2026-11-10/office-edit/sin-cos-v8-wps-receipt.json)
and [independent audit](../.real-eval/ppt-completion-2026-11-10/office-edit/sin-cos-v8-wps-independent-audit.json)
record 25 rewritten common parts, all four regenerated PNGs and changed third
formula geometry. Neither raw XML/typography identity nor arbitrary cross-engine
compatibility is established. The one-variable change breaks the generic
identity; that edited probe is not a correct scientific deliverable or an
autonomous lecture pass.

### Historical v8 freeze and resource update; subsequent V8b fixes

The [v8 final summary](../.real-eval/ppt-completion-2026-11-10/v8-final-check-summary.json)
belongs to the [1957-file freeze](../.real-eval/ppt-completion-2026-11-10/v8-final-source-freeze.json),
SHA `e7b55c907c8e47247809208a6c1d880fe2ec44888848b1cffa32917d77df48de`.
Runtime passed **715**, zero failures/skips; typecheck/format exited zero.
Two initial concurrent frontend failures in unchanged LibrarySourcesWorkbench
remain preserved. A 33-test focused pass and two-worker full rerun then passed
**3323 tests, one skipped, zero failures** without production edits; suspected
load/timing is not a proven exclusive cause. See the [frontend recheck](../.real-eval/ppt-completion-2026-11-10/v8-frontend-recheck-receipt.json).
Rust fmt/all-targets Clippy passed. Initial full-test admission stopped at the
disk preflight, not a test assertion; after cleanup of completed release
intermediates, resumed full Rust passed **4514, 27 ignored, 22 result groups**.

Private v8 did **not** recompile Rust release. It inherited the verified v7
bundle, updated only document-tools/planning/deck.py/PPT Skill resources, and
received normal Developer ID signing. Its [resource update receipt](../.real-eval/ppt-completion-2026-11-10/v8-private-runtime-update-receipt.json)
and [static audit](../.real-eval/ppt-completion-2026-11-10/private-full-desktop-static-audit-v8.json)
(SHA `143a150d40b7404abee2529329a622a24a6d0f32b07c93236c3edde78a0422dc`)
identify main SHA
`c420b4578cbc2e4e79e7a3cfe123ce051e79a8a1c19b3e5553bef40b2c6434a2`.
Nine Mach-O/deep-strict signatures and resource/renderer checks passed; the
entire main-code prefix before its signature matched v7. No installation,
replacement, publication or new notarization occurred. Native v8 subsequently
launched; historical copy-time `launched:false` and earlier gate snapshots
remain historical.

Those counts and signatures certify **that v8 freeze**. The later missing
Skia JPEG arrowheads and Architectural Plan helper metadata/index collision
were still pending at that checkpoint. Both now have bounded V8b fixes and
fresh final gates below; the earlier 715/3323/4514 and signed v8 bundle are not
substituted for those checks. The footer collision was a deterministic helper
defect, not solely model-authored drawing.

### Adaptive multi-image authorization and actual v8 product quality failure

The user removed the earlier global one-image limit and allowed page-adaptive
multiple illustrations, while keeping total authorization **US$4**. Ledger
`maximumGeneratedImages=null` removes that count restriction, not per-run frozen
image count/USD, Host asset checks, unknown reservations or the remaining dollar
cap. Zero images can still be appropriate; an image maximum is not a quota, and
charts/math/flows should retain native editable structure. The
[free admission review](../.real-eval/ppt-completion-2026-11-10/adaptive-multiimage-deepseek-admission/adaptive-admission-review.json)
did not itself make a paid call.

Current DeepSeek V4.1 Flash is 1M context/384k output with US$0.006/M cache read.
The actual ordinary-request path has no lower fixed output cap; 16k/32k cannot
be assumed. Highest input/output rates US$0.30/US$1.20 per million, a 50-fold
cache factor, minimum US$0.01 UI threshold and three terminal POSTs require
**US$2.783400**, exceeding the then-US$1.076922 balance. It was not admitted or
sent. Prompt brevity is not an admission bound. The Mini product candidate,
text US$0.05 plus up to three images/US$0.15, required US$1.039608 and was
separately admitted after fresh checks, not together with another worst-case
reservation.

The actual ordinary natural-language v8 run `675aafb1…` had no source refs or
human-provided plan/code. Auto froze Architectural Plan
`f2bd2589-2d2f-4294-83bc-cf1c2b5b2013`, revision 1, with the
[sent snapshot](../.real-eval/ppt-completion-2026-11-10/v8-product-adaptive-sent.json)
auto/max3/US$0.15, text US$0.05 and exact default Pro image model. Five image tool
attempts consisted of three local parameter refusals before paid submission,
then two successful gateway-200 generations. Both new images were embedded
byte-for-byte on their planned pages 1/2. The eight-page artifact is **650768
bytes**, SHA
`1d9a0da6b52d0fd33e3583567adb5d8ab462996415c5b297cda1d4dd13210357`.
The first successful preview was auto-delivered when the cost gate blocked a
second layout revision; the operation failed with `assistant_cost_limit`.
A retained file is not normal completed delivery or final model review. See
the [run audit](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v8-product-adaptive/run-audit.json).

The [all-eight-page quality audit](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v8-product-adaptive/quality-audit.json),
SHA `ada673312d5af772ab5d9f320777d1dd392554d5a8b9642b1e96fad9188ec2fd`,
concludes **QUALITY FAIL**, retaining the functional multi-image positive.
Page 1's later picture occludes a title glyph in WPS, hidden by the JPEG
renderer drawing text on top; index 01 also collides with the team footer.
Page 3 adds sync/quiet-mode scope without a proposed qualifier, but no fabricated
customer/price/release/quantitative-effect claim was found. Five warnings mean
oversized underfilled cards, not text overflow. Page 6 lacks its planned
workboard/timeline; pages 6–8 repeat sparse text layouts. Native page-4 arrows
exist with correct direction, but lack node bindings; JPEG arrowhead behavior
is a separate renderer issue. Architectural Plan remains a partial native
adaptation, not full upstream-template conversion.

Root's [Desktop receipt](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v8-product-adaptive/desktop-ui-receipt.json)
records all eight original pages and the disabled final next button. WPS
12.1.25895/macOS 26.5.1 displayed all eight before editing. In a private copy,
Root moved the page-1 independent image downward and changed page-4 整理任务 to
整理今日任务; save/close/Finder same-path reopen retained both edits. The
[WPS receipt](../.real-eval/ppt-completion-2026-11-10/office-edit/product-v8-adaptive-wps-receipt.json)
identifies the edited **698558-byte** copy, SHA
`1020a5f9883436fe6176002322cd833e8eb0c2fddaf2e84097efedc2f0498336`.
The [independent audit](../.real-eval/ppt-completion-2026-11-10/office-edit/product-v8-adaptive-wps-independent-audit.json),
SHA `702bcdd6c944e31251d92d4455aafd24e0e7cd43209c9949fb61280ed66d2d43`,
confirms only textbox ID 12's text and image ID 11's y geometry changed, the
picture moved 561340 EMU (~0.614in), and both original JPEG blobs stayed intact.
Slide/object IDs/order, notes text and connector properties remained; WPS
rewrote notes/title XML, fonts, relationships and image filenames. No raw-run
byte identity, all-page post-reopen visual check or node-binding acceptance is
claimed. Manual correction does not repair the autonomous original, whose
`1d9a…10357` bytes and quality failure remain. Missing fonts, chart/formula
coverage and PowerPoint/Windows limitations remain.

Twelve complete text receipts, no new unknown outcome, give a text quoted upper
**US$0.113663**, separately from SDK estimate **US$0.05550785**. Two Pro images
add **US$0.100000**, total **US$0.213663**, with invoice unknown. The
[budget audit](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v8-product-adaptive/budget-audit.json)
and [Root settlement](../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v8-product-adaptive/root-settlement-receipt.json)
release **US$0.825945** unused from the US$1.039608 reservation. The latest US$4
checkpoint is **US$3.136741** conservatively occupied including historical
unknowns, **US$0.863259** unreserved, no active reservation, **three verified
generated images/global max=null**. Historical US$1.057077 unknown reservations
were not released. The original reservation was not actual spend. All earlier
failures, 41-page/partial source boundaries and incomplete PPT todo are retained;
future paid runs require fresh admission, with no installed-app replacement.

### Scoped V8b blocking-fix candidate; broader visual work remains open

The user narrowed this delivery to two concrete blockers and a clear list of
remaining defects. General visual and narrative iteration continues in
[Fizzy #1615, 智能创作工作流与文件预览优化](https://app.fizzy.do/6179973/cards/1615),
and no longer blocks this candidate. The card is located, open, with all ten
steps unchecked; the local roadmap is unchanged. The historical autonomous
originals' **QUALITY FAIL** verdicts remain, and neither all-platform nor full
visual acceptance nor the whole PPT todo is declared complete.

| This delivery's blocking fixes                                                                                                                                                                                                                                 | General visual/narrative backlog in #1615                                                                                                  |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Architectural Plan cover metadata overlapped its own Panel page index. Actual native-box RED became 12/12 related passes, covering both alternating panel sides and other styles; independent review passed. The fix chooses a free horizontal metadata band.  | Model-authored picture/title partition and z-order; the helper fix does not move model pictures or repair old files.                       |
| Correct native arrows/SVG markers lost their heads in JPEG. Bounded triangle-marker normalization now covers persisted previews, on-demand workspace pages, and model thumbnail/overview; actual pixel RED→GREEN, 64/64 related passes and independent review. | Card whitespace/layout richness, plan/narrative consistency and proposed qualifiers for unapproved functionality; original defects remain. |

See the [footer regression receipt](../.real-eval/ppt-completion-2026-11-10/v8b-cover-metadata/focused-receipt.json),
[arrowhead change receipt](../.real-eval/ppt-completion-2026-11-10/v8b-preview-arrowheads/change-receipt.json)
and [actual three-path proof](../.real-eval/ppt-completion-2026-11-10/v8b-preview-arrowheads/actual-three-path-proof.json).
Only recognized plain-line/triangle markers are normalized; arbitrary paths,
curves, clipping and inherited paint are not certified. Original PPTX/SVG and
old generation caches stay intact. An additional paid visual-repair proposal
was [cancelled before sending](../.real-eval/ppt-completion-2026-11-10/native-product-v8b-autonomous-repair-preparation/preparation.json),
with zero API calls or reservations. The US$3.136741 occupied/US$0.863259
unreserved checkpoint, three verified images/global max=null, and unknown
invoice remain unchanged.

The [final V8b summary](../.real-eval/ppt-completion-2026-11-10/v8b-final-delivery-check-summary.json),
SHA `05c44958e1a99889c31a4e2ba29004c0a3080e50fd6bd60a579da76067143c1e`,
binds the unchanged [1960-file freeze](../.real-eval/ppt-completion-2026-11-10/v8b-final-source-freeze.json),
SHA `690e3953ac9fa8f76a414e9520cee9df73797d1b16174e7e29ef40ed824d630d`:
runtime **724 passed, zero failed/skipped**; frontend typecheck/format exit zero,
**3323 passed/one skipped**, 264 files passed/one skipped; Rust fmt/all-targets
Clippy exit zero. The initial full Rust run's **4275 passed/one failed/25
ignored** library timeout in existing pnpm cache-manager discovery remains
recorded. That test passed alone, 1/1 in 3.30s. A same-source complete cached
rerun then passed **4514, zero failed, 27 ignored, 22 summaries**, exit zero,
441.458s (cached compilation 1.65s). No Rust, test or timeout change was made;
load is not established as the exclusive cause of the first failure.

Private v8b is version 1.5.0, Team R65NVCLBMY, main SHA
`d61a59cab4876fe33374d41eaaa7e1485d1fbb01355a023905727f2f32bbad34`.
The [static audit](../.real-eval/ppt-completion-2026-11-10/private-full-desktop-static-audit-v8b.json),
SHA `6cf908ce86daed6375fea4efb338b88ef71a7d1a82ab04628c6df8f6277946aa`,
passed nine Mach-O/deep-strict signatures, Team/DR/entitlements, resource
source→staged→bundle and embedded renderer checks with no findings. Four
production runtime files changed since v8, plus two tests and one fixture;
Rust and renderer were not recompiled for this resource update. The main-code
prefix before its signature is identical to v8, followed by normal Developer
ID signing, not a fresh Rust release compilation.

Root launched the private candidate (PID 43355) and observed all three right
arrowheads on page 4 of unchanged actual original `1d9a…10357`, then restored
left-chat/right-file split with no send. The [Native UI receipt](../.real-eval/ppt-completion-2026-11-10/v8b-native-preview-ui-receipt.json),
SHA `1df905a80da47d84406c3cea7bba1c60ea736da28d18edde9d47c20787e3ae71`,
is a bounded page-4 preview regression, not a new autonomous generation or
whole-deck quality pass. The cover-helper fix has native-box/source-identity
proof, without an additional paid generation.

The [macOS arm64 private candidate ZIP](../.real-eval/ppt-completion-2026-11-10/delivery/Ofox-Desktop-PPT-candidate-2026-11-11-macOS-arm64.zip)
is **180814658 bytes**, SHA
`4f7c30455dfa65af16bd56b6342dfa6f1b4750ffb1acb9d561ef27d4af88d3db`.
CRC and a [fresh temporary extraction's signature/main identity](../.real-eval/ppt-completion-2026-11-10/delivery/archive-unpack-signature-receipt.json)
passed; only the owned extraction was cleaned. No `/Applications` replacement,
notarization or publication occurred; installed `810f…57a93` is unchanged.
Root appended and read back one [scope/remaining-defects comment](https://app.fizzy.do/6179973/cards/1615/comments/03h7s7onv5syxpcgiq67ms7oe),
recorded in the [Fizzy receipt](../.real-eval/ppt-completion-2026-11-10/fizzy-1615-scoped-delivery-receipt.json).
The original description is unchanged, the card stays open and all ten steps
remain unchecked. This is the scoped blocking-fix candidate delivery; broader
visual, font, source adaptation, PowerPoint/Windows and short-credential
limitations above remain.

### Compact preview UI source increment (2026-11-11)

The right pane now defaults to one file heading plus simple paging; optional
tools and version information are behind More. The
[increment record](superpowers/research/2026-11-11-ppt-preview-ui-simplification.md)
distinguishes renderer-fixture evidence from native backend/generation acceptance.
This UI source is not included in the earlier V8b ZIP or the installed app;
installed Tauri/Windows and a fresh independent review are not certified.
General PPT visual work and the overall open status of Fizzy #1615 remain.

## PPT style library completion (2026-11-11)

Record: [library completion](superpowers/research/2026-11-11-ppt-library-completion.md),
[inventory](superpowers/research/2026-11-11-ppt-library-inventory.md),
[public workflow reuse](superpowers/research/2026-11-11-ppt-public-workflow-reuse.md).
The library step is complete in the sense of mapped, selectable, routable and
fully previewable; it is not a quality certification. Every style stays
`verified: false` with visual quality and target-application round trip `pending`.

- **Page mapping is mostly approximate.** Of 236 actual source pages, 90 are
  `adapted`, 145 `approximate` with a named structural difference and 1
  `unsupported` (broadside page 6). The five first-enabled styles are 2 adapted
  and 39 approximate; their hand-written native compositions have known defects
  that this round recorded and did not fix (architectural-plan panel side and
  empty panel, product-launch dark content pages, blue-professional quote and
  closing, course-module objective rail).
- **Declarative compositions are Python-only.** The Node deck renderer draws
  the 19 later styles on their base canvas with the open editorial composition;
  canvases per page kind, decoration shapes and block styles are not drawn
  there. The Python engine is macOS-only, so those styles are simplified on
  other platforms. Not run on Windows.
- **Engine gaps behind `approximate` pages**: one decoration set per page kind,
  no second variant of a page kind, single-colour bold-or-regular titles, no
  rotated text blocks, one text colour on a filled block, one bar colour per
  style, no doughnut or per-bar chart colours, no vertical timeline, 2×2 cycle
  or stepped hierarchy. Content canvases alternate by page number, not by page
  kind.
- **Sample pages are sparse.** With the shared sample text most content pages
  fill only the upper half; default body sizes are small for the canvas. General
  layout density is unresolved.
- **Preview**: chart text now uses a CJK-capable system font; pie legend labels
  are still missing in the preview raster. Source previews use local fallback
  fonts. The five first decks keep their earlier forced-display captures.
- **Image rules**: `image()` refuses a picture that covers existing title or
  text (footer and page number excepted); a picture drawn before the text, and
  text inside tables or charts, are not checked. Full-bleed pictures need
  `behind=True`, which older model code did not pass. Plan fields
  `placement`/`textZone`/`aspect`/`imageStyle`/`pageKind`/`facts` are optional;
  generation size is not derived from `aspect`, and labelling of illustrative
  or proposed facts is a model rule, not enforced.
- **Row fitting**: a row of cards that are all more than half empty is drawn at
  its content height; an earlier lint line may remain after a fuller sibling
  restores the row. Objects the model places relative to the requested height
  are not moved.
- **Not verified this round**: real model generation with the new styles, the
  picker and previews inside the Tauri app (browser harness and jsdom only;
  Japanese not viewed), WPS round trip for the new styles, Microsoft PowerPoint,
  Windows. No candidate package was built; the V8b package contains none of
  this work. `pageMapping[].differences` is English only.
- **Bundle size**: about 21 MB of source previews and 9 MB of native sample
  pages are bundled with the renderer; whether they are also staged with the
  runtime skill resources was not measured.

- **Real runs (2026-11-11 evening)**: one academic case (manual `signal`, no
  images) and one product case (Auto, GPT Image 2) delivered through the real
  Host with DeepSeek V4.1 Flash, via a test-only guarded driver and the
  hal-vault test key, not through the installed app or the Desktop account.
  Four paid attempts cost at most US$0.558269 by upper bound; billing is not
  reconciled. The runs found and this round fixed: planned images on a
  token-priced model were always refused; a shape covered a closing title;
  25 full candidates exhausted the model's output on reasoning. The token
  upper bound uses the published GPT Image output-token table with a 1.5 margin;
  a provider that bills more than that table is not detected before sending.
- **Short list**: style candidates are given in full for the six best rule-based
  fits only; the others are listed briefly and remain eligible.
- **Not verified**: WPS and PowerPoint display, editing and save-reopen of the
  two real decks; the picker and previews inside the Tauri app; cancellation,
  retry, insufficient credit and account switching on the real path.
- **Slide images and code figures (2026-11-12)**: the composer's image setting
  is now stated in the PPT turn's opening Host text. How many pictures a deck
  gets (at least one, up to the setting) is still the model's choice; the Host
  does not force one. Code-drawn figures are a plan visual kind (`code-figure`)
  on the Python route only. `plot()` recolours only matplotlib defaults (white
  background, black frame text) to the page and relies on matplotlib 3.10
  private attributes for side titles and figure-level labels. A figure whose
  root `<svg>` names a non-CJK font still draws Chinese as boxes; only a root
  without `font-family` gets the CJK list. Daisy Days draws its cover pill
  without a kicker, and a centred cover with `art` misaligns. Verified by two
  guarded real runs (record section 11), not in the installed app.
- **Dropped model requests and unknown costs (2026-11-12).** A model request whose connection drops with no answer is asked again (three attempts in all); a drop before the request was written costs nothing. A request whose cost cannot be read no longer ends a task: it counts toward the task's cost limit at its ceiling (request bytes as input tokens plus a full-length answer) and its receipt stays "unknown", so the total shown remains incomplete. A retried request that had in fact reached the provider may be billed twice. Still ending a run: a model with no price at all (before any request), a web search with an unknown outcome or price, and an image with an unknown outcome outside a planned deck. Image requests (Rust host) do not yet tell a handshake drop from a sent request. Covered by runtime tests only; not yet exercised in the installed app.
- **Deck images: count, reuse and backgrounds (2026-11-12).** The image setting is none / auto (a ceiling the model fills as it sees fit) / a set number, up to eight generated images a deck (and a reply). Pictures the conversation already holds (generated earlier, or from the user) are listed to the model each presentation turn and placed free as `user-image` visuals, so changing a deck keeps its images; a deck places at most twelve pictures. With images off, a picture the user's message asks for is still generated as an ordinary image and then placed. These are instructions to the model, not Host guarantees: it may still drop a picture, plan fewer than a set number, or generate an asked-for picture it then fails to place. One picture per page, so a page cannot carry both a background and a side image. A background image gets the page colour laid over it at 60%; very busy pictures can still leave small text hard to read. The running-steps list shows a failed step that was done again as "Retried".
- **A reply that runs out of output while reasoning (2026-11-13).** When the model spends its whole output limit on reasoning (no text, no tool call), the runtime asks it again without showing a user message, twice at most; a run that still has no answer ends as an error instead of "completed". A long answer cut off at the limit is kept as it is. Office round trips of sampled decks were checked only with headless LibreOffice (which drops embedded chart workbooks and native equations on save) and one read-only open in an unactivated PowerPoint; editing, saving and reopening in WPS or an activated PowerPoint, and Windows, remain unverified.
- **Presentation styles in real runs (2026-11-13).** All 24 catalog styles and navy now have a real generated deck (kept outside the repository; covers in `assets/readme/presentation-styles.webp`). On `architectural-plan` pages (title in a side panel), content across the panel edge or over its title is now refused with the regions to use; a real rerun laid every page out inside them, so the refusal itself is covered only by a test. Text in a code-drawn figure that names a Latin-only font now gets a CJK font of the same kind first; only covered by a test, not seen again in a real run. Two real runs failed on the provider's temporary refusals (request burst, server overload) and passed when run again.
