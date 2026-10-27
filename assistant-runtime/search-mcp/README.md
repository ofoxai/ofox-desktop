# Ofox search MCP server

A small stdio MCP server, built on the official `@modelcontextprotocol/sdk`,
that gives the Ofox assistant two tools: `web_search` (one paid search
through the Ofox Responses API) and `web_fetch` (read one public web page).
`npm run build` bundles `src/server.mjs` into the single file
`dist/ofox-search-mcp.mjs`, which runs on the bundled Node 22 with no
`node_modules`. Only the stdio transport is bundled; the build fails if the
SDK's Express/Hono HTTP transports end up in it.

It is not wired into the assistant yet. The later client wrapper starts it
with pi-mcp's `McpClient` + `StdioTransport` and `inheritEnv: false`, and
exposes the tools to the model as Pi tools.

## Environment

Configuration comes from the environment only. Invalid configuration makes
the server exit with code 2 and a one-line message on stderr that never
contains the key.

| Variable                   | Required | Value                                                                |
| -------------------------- | -------- | -------------------------------------------------------------------- |
| `OFOX_ASSISTANT_KEY`       | yes      | The assistant's own Ofox API key: one printable token, 16-512 chars. |
| `OFOX_BASE_URL`            | yes      | Exactly `https://api.ofox.ai` or `https://api.ofox.io`.              |
| `OFOX_SEARCH_MODEL`        | no       | Search model id; default `openai/gpt-5-nano`.                        |
| `OFOX_ALLOW_LOCAL_GATEWAY` | no       | `1` also allows `http://localhost:<port>` as the base URL (tests).   |

The host derives `OFOX_BASE_URL` from the selected `ofoxApex`; the server
never picks a region itself.

## Tools

### `web_search({ query })`

`query` is 1-2000 characters with at least one non-space character. Each call
sends exactly one `POST {OFOX_BASE_URL}/v1/responses` with
`Authorization: Bearer <key>` and this body:

```json
{
  "model": "<OFOX_SEARCH_MODEL>",
  "input": "<instruction>\n\nQuery: <query>",
  "tools": [{ "type": "web_search", "search_context_size": "low" }],
  "include": ["web_search_call.action.sources"],
  "max_tool_calls": 1,
  "reasoning": { "effort": "low" },
  "max_output_tokens": 1500
}
```

The request times out after 60 s, and an MCP cancellation aborts it; both
end as `search_outcome_unknown`. pi-mcp's default request timeout is 30 s,
so the wrapper should call `web_search` with a longer one (about 70 s).

The result has a short summary, up to 25 results (citations first, then the
search's sources, deduplicated by URL) and usage:

```json
{
  "summary": "…",
  "results": [{ "title": "…", "url": "https://…", "snippet": "…" }],
  "usage": {
    "searchCalls": 1,
    "inputTokens": 8497,
    "outputTokens": 820,
    "model": "openai/gpt-5-nano"
  }
}
```

### `web_fetch({ url })`

Reads one page with the safety rules of the assistant runtime: HTTPS on port
443 only, no credentials in the URL, no local or Ofox hosts, every DNS answer
must be public, the connection is pinned to the checked address, every
redirect (at most 4) is checked again, 15 s timeout, 2 MiB body limit, static
HTML or plain text only, at most 20,000 characters of text, no cookies. The
result is `{ url, text, truncated }`, where `url` is the final URL after
redirects.

**The allowlist is not here.** `web_fetch` reads any URL that passes the
rules above. The client wrapper must only let the model fetch URLs that the
user wrote or that a `web_search` in the same conversation returned.

## Billing

The Ofox gateway bills every `web_search_call` output item, `open_page`
actions included, at the model's `web_search` price. Without
`max_tool_calls` the model also opened pages, each billed. With
`max_tool_calls: 1` a request makes one billed search (about $0.014 plus
tokens, verified 2026-10-27), and `include` still returns the 15-20 source
URLs that search saw. `usage.searchCalls` counts the `web_search_call` items,
so the host can estimate the cost. Missing token counts are `null`, not 0.

Nothing is ever retried automatically: a repeated request is a repeated
charge.

## Results and errors

Every result has one text block for the model plus the same data as
`structuredContent`. The text block is capped at 20 KB (UTF-8), because Pi
moves larger MCP text to a temp file the assistant cannot read; a long search
summary is shortened before the source list.

Errors are tool results with `isError: true`, text `"<code>: <message>"` and
`structuredContent: { "error": { "code", "message", "status"? } }`. Messages
are fixed strings: they never contain the key or a response body. Successful
results pass upstream data on as is, except that the key, written or
percent-encoded, is masked as `[REDACTED]***` (same length) before any field is
cut to its limit.

| Code                          | Meaning                                                                                                                                                                                                                                          |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `search_outcome_unknown`      | The search may have run and been billed: timeout, cancellation, any network error other than a connect()/DNS failure (TLS errors included), HTTP 5xx or 3xx, a body over 1 MiB, unreadable or malformed success. Do not repeat it automatically. |
| `search_unauthorized`         | HTTP 401/403: the key was rejected.                                                                                                                                                                                                              |
| `search_insufficient_balance` | HTTP 402, or a 4xx (never 429) with Ofox's own error shape `error.type: "insufficient_credits"` or `error.code: 402`. An upstream `insufficient_quota` is not the user's balance.                                                                |
| `search_failed`               | Another 4xx (with `status`; 429 means rate-limited, try later), or connect()/DNS failed (no `status`; nothing was sent). There is no separate rate-limit code: use `status` 429.                                                                 |
| `fetch_not_allowed`           | The URL, a DNS answer or a redirect target broke the `web_fetch` rules.                                                                                                                                                                          |
| `fetch_too_large`             | The page body is over 2 MiB.                                                                                                                                                                                                                     |
| `fetch_unsupported_content`   | The page is not HTML or plain text.                                                                                                                                                                                                              |
| `fetch_failed`                | HTTP error (with `status`), network or DNS failure, timeout, or too many redirects.                                                                                                                                                              |

Invalid arguments (an empty or too long query, extra fields) are rejected by
the SDK before the handler runs, as an `isError` result without
`structuredContent`; no request is made.

The server writes nothing but MCP messages to stdout. stderr gets only short
diagnostics with the key and any bearer token redacted, because the host
shows the tail of stderr to users.

## Build and test

Use the bundled runtime's Node version (22.23.4):

```sh
npm ci
npm run build   # dist/ofox-search-mcp.mjs, about 380 KiB
npm test        # builds first, then node --test; no live network calls
```

The tests use a local mock gateway on an ephemeral port with
`OFOX_ALLOW_LOCAL_GATEWAY=1`, real Ofox responses in `test/fixtures/`
(`encrypted_content` removed), and the SDK's `Client` +
`StdioClientTransport` against a copy of the built bundle in an empty temp
directory.
