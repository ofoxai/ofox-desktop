# MCP connection checks

The resource library opens a read-only check dialog. Only **Start check** prepares a frozen revision/epoch/spec and connects to a server or starts a local executable. Browsing, refreshing and importing definitions do not execute MCP programs. Results remain in memory; checks do not modify assignments, receipts or native tool configuration.

## Transports and protocol

The checker supports stdio, Streamable HTTP JSON/SSE responses and explicitly configured legacy HTTP+SSE. It probes `server/discover` for MCP 2026-07-28 and uses the legacy initialize/initialized lifecycle when the discovery response indicates an older protocol. It supports negotiated legacy versions 2024-11-05, 2025-03-26, 2025-06-18 and 2025-11-25. Recognized modern protocol errors, authentication/permission failures, redirects, cancellation and size violations do not trigger a legacy downgrade. Legacy `ping` receives an empty result; unsupported client requests receive an error. No tools are called, sampled or elicited.

Discovery and tool-list requests follow the [modern stdio](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio) and [HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http) contracts, with the [legacy lifecycle](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle) retained for compatibility. These implementation tests do not certify every vendor MCP server.

Limits: four prepared/running checks, one per resource; prepared IDs expire after 60 seconds; transport work has a 120-second deadline; the stdio discovery probe has a 3-second compatibility timeout. Messages/frames are limited to 256 KiB, received data to 2 MiB, pagination to ten pages and 200 tools. Empty cursors are opaque values; repeated cursors are rejected. Field counts, expanded values and redaction patterns also have explicit budgets. A failed or incomplete list is reported as a failure, not a complete empty list.

## Execution and credentials

Executable lookup uses the exact child PATH/cwd, verifies files and execution permission, and checks canonical aliases. Known package managers, automatic package runners, containers and shells are blocked. Node/Python/Ruby/Perl interpreters require an existing local script; inline eval/module loading is blocked. Windows `.cmd`/`.bat` shims require a direct executable/local script configuration. OFox does not launch a login shell, install a missing service or fetch packages for a check.

The child gets essential environment values plus explicitly configured env fields. `${NAME}` references resolve from the OFox process environment. No CLI OAuth state, keyring values, `.env` files or browser cookies are imported. Set the environment and restart OFox when necessary, or check/authenticate in the native tool. Explicitly configured programs still run with the user's permissions and can perform their own I/O; this feature is not an OS sandbox.

HTTP never follows redirects. Legacy SSE message endpoints must retain the same origin. Initial URLs cannot embed credentials, query parameters or fragments; headers can contain explicit credential references. Fixed error codes cross IPC instead of raw stderr, HTTP bodies or RPC errors. Known configured credential fragments are masked from tool names/descriptions, including overlapping matches and control-character normalization; over-budget matching hides the entire text.

## Cancellation and acceptance

The UI subscribes before execution and filters progress by operation ID. Cancel, close after cancellation and portal suspension clean up listeners and cancel late prepared jobs. A gate serializes cancellation with stdio spawn. Every normal/error/cancel exit closes pipes, requests termination of the dedicated Unix process group or Windows Job and performs a bounded wait; a Drop guard covers interrupted futures. Windows starts suspended, joins the Job before resuming and uses kill-on-close. HTTP response streams are dropped; legacy server session expiry remains controlled by the server.

macOS tests use temporary homes, local HTTP servers and Node 22.12.0 fixtures, including delayed partial discovery replies, ping, stderr draining and a descendant that must not survive cancellation. No existing user MCP server or agent login was executed. The Windows Job/executable policy passed a cross-target type probe; native Job behavior and real MCP vendor versions remain **unverified**. The complete `scripts/test-windows.ps1` runner includes these fixtures and requires Node as documented in [Windows testing](windows-testing.md).

The Windows-only process wrapper requires Rust 1.87 or newer; the repository toolchain is 1.95. Its locked Unix dependency remains on nix 0.31.2 so adding the Windows code did not upgrade the existing libc version.
