## Behavior

<!-- Explain the concrete trigger, previous behavior, and resulting behavior. Link the issue. -->

## Verification

<!-- Record exact HEAD SHA, OS/toolchain, commands, results, skipped checks, and screenshots for UI changes. -->

- [ ] Regression test fails without the change and passes with it.
- [ ] Frontend: `pnpm typecheck`, `pnpm format:check`, `pnpm test:unit`.
- [ ] Rust: fmt, `cargo clippy --all-targets -- -D warnings`, full tests on each affected OS.
- [ ] Windows-specific paths ran natively; otherwise Windows is explicitly pending.
- [ ] Every sibling agent affected by the same invariant is covered.
- [ ] Real installed agent/version verified, or explicitly marked unverified.
- [ ] User files use `atomic_write` / `FileTxn`; tests are sandboxed and shared environment/settings use the common serial lock.
- [ ] No secrets in logs, errors, screenshots or committed fixtures.
- [ ] zh/en/ja keys and placeholders match; both OFox apex values verified.
- [ ] UI light/dark, 900px and keyboard focus checked where affected.
- [ ] `docs/agent-wiring.md`, lessons and limitations updated as needed.
- [ ] Independent review completed; accepted limitations are recorded, not described as fixed.
- [ ] Release changes preserve dry-run and require CI plus Windows Probe at the same SHA.
