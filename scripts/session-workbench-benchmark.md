# Session workbench renderer benchmark

This developer-only entry builds production React code without changing the
desktop application entry. It uses `UsageSessions` and `SessionsPage`, all three
application locales, 10,000 synthetic body-free session records, 100 projects,
16 models, four tools and 1,460 daily buckets. Two immutable fixtures are prepared
before timing. No account, saved user filters, real session files or credentials
are read. Browser storage is replaced with an in-memory map; native IPC and HTTP
fetches reject, and CSP prevents external connections. Local scripts/styles/logo
assets load from the static preview server.

```sh
pnpm exec tsc --project scripts/tsconfig.session-workbench-benchmark.json
pnpm exec vite build --config scripts/session-workbench-benchmark.vite.ts
pnpm exec vite preview --config scripts/session-workbench-benchmark.vite.ts
```

Open `http://127.0.0.1:4174/session-workbench-benchmark.html`, set the window to the
desired size (including 900×600), choose zh/en/ja and theme, then press **Run both
pages**. The tab must stay visible. The harness measures one cache/state data
replacement per sample, including the React commit and two animation frames.
Each page gets three discarded warmups followed by 30 recorded samples. Results
are printed in `#benchmark-results`, including every sample, nearest-rank p95,
viewport, language, timezone, production mode, blocked attempts and whether both
pages meet p95 < 500 ms. The combined value is the maximum of the two page p95s,
not a claimed whole-app or end-to-end result.

Timing starts in the refresh action immediately before replacing the already
prepared data. It includes React rendering/commit, metadata indexing, layout and
the second `requestAnimationFrame` boundary. Two frames provide a paint
opportunity; they do not prove compositor completion or measure human input
device latency. Fixture construction, source scanning/parsing, native IPC,
backend aggregation, HTTP and disk I/O are excluded. Do not report these numbers
as native scan, Rust query, cold-start, transcript or complete user-workflow
latency. Repeat each locale/theme/window-size combination needed for acceptance.

The build output lives only in ignored `dist/session-workbench-benchmark/`.
Remove that subdirectory to clean it; do not delete the application's other
build output. This helper neither installs nor launches the desktop application.
