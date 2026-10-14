# Repository guidelines

Ofox Desktop is a Tauri 2 application. React/TypeScript lives in `src/`, Rust in `src-tauri/src/`, frontend tests in `tests/`, Rust integration tests in `src-tauri/tests/`, documentation in `docs/`, and maintenance helpers in `scripts/`.

## Work and verification

- Read [code standards](docs/code-standards.md), [agent wiring](docs/agent-wiring.md), and [lessons](LESSONS.md) before changing a related integration. Record limitations honestly in [known limitations](docs/known-limitations.md).
- Inspect Git/worktree state first and preserve existing work. Follow the user's authorization for commits, pushes, releases, and installation. Never infer permission to destroy real user data from a test request.
- For features and bug fixes, obtain a meaningful failing regression test first, implement the fix, run the relevant checks, and obtain independent review. Cover every sibling integration affected by the same assumption. Verify with an installed agent when available; state the exact version and what was tested, or explicitly mark it unverified.
- Tests that write user settings, config files, credentials, or sessions must use temporary sandboxes. Rust tests that mutate process environment or shared settings must use the same default `serial_test::serial` lock and `test_sandbox`; a differently named lock does not protect shared state. Never corrupt live settings or uninstall live tools to exercise failures.
- Serialize large Rust builds and reuse the integration target directory. Check free space before a large build; below 20 GiB, clean only confirmed completed build caches/incremental output, preserving source changes.

## Commands and style

`pnpm install --frozen-lockfile`; `pnpm dev` for Tauri, `pnpm dev:renderer` for Vite, `pnpm build:renderer` for renderer, `pnpm build` for desktop bundles. Final gates are `pnpm typecheck`, `pnpm format:check`, `pnpm test:unit`, `cargo fmt --check --manifest-path src-tauri/Cargo.toml`, `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`, and `cargo test --manifest-path src-tauri/Cargo.toml`. Run platform-specific code on its native OS; a macOS result does not validate Windows. See [Windows testing](docs/windows-testing.md).

Use existing Prettier/rustfmt style, PascalCase components, `use`-prefixed hooks, camelCase frontend values and IPC payloads, and snake_case Rust identifiers. Use `@/` imports. Preserve compatible optional fields and the current database schema unless a migration is necessary. Use Conventional Commit subjects and explain behavior and verification in PR descriptions.

## User files, secrets, and endpoints

- Write user files through `atomic_write` or `FileTxn`; preserve unrelated keys, permissions, symlink behavior, and rollback. Unknown/unreadable is never an empty/default config. Scrub secrets before errors, logs, progress events, and crash reports leave their source.
- New visible text must cover zh/en/ja with matching placeholders. Reuse UI primitives and semantic theme tokens; test keyboard focus and 900px light/dark layouts. Do not introduce native select/checkbox/radio controls in migrated surfaces.
- Derive runtime API, OAuth, catalog, API-key, recharge and statistics URLs from the resolved `ofoxApex` (`ofox.ai` / `ofox.io`): Rust `ofox_apex.rs`, renderer `ofoxUrls.ts` + `useOfoxApex`. Resolve before writing configuration. Reconcile only OFox-owned URLs on region changes and startup, preserving user edits and surfacing conflicts. Verify both apex values, first launch and restart. Fixed installer/CDN infrastructure is an explicit exception, never a runtime API shortcut.
- Never commit keys, tokens, personal paths, internal URLs, local configs or signing credentials. Version changes, when requested, must synchronize package, Cargo and Tauri metadata. Do not tag until release dry-run and the exact-SHA Windows gate pass.
