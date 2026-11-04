# Conversation request context and retries

Every chat run includes the local calendar date, English weekday, IANA time
zone and current app language in its system prompt. The renderer passes its
resolved language even before an application preference is saved. Older IPC
callers use the saved language, with Chinese as the final fallback. Runtime
tests can override the clock with an ISO `now` value.

Web search permits six sent searches per turn; duplicate and rejected-before-send
queries retain their existing behavior. The tool-call ceiling remains 20 without
local configuration. Local configuration retains 60 steps, 15 minutes and USD 1.

Only conversation HTTP model requests use the bounded retry wrapper. Pi's agent
and provider retry settings remain disabled. A request may retry twice after
429/502/503 or a reliably unsent connection failure, waiting one and three
seconds. A valid 429 `Retry-After` overrides that wait, capped at ten seconds.
Cancellation and SDK timeout signals interrupt the wait.

After a successful HTTP response is returned to the SDK, stream failures,
partial text, reasoning or tool calls cannot enter the retry loop. Timeouts,
abort errors, ambiguous socket resets and other HTTP statuses are not retried.
The runtime never restarts the agent turn to retry a model request. Tools,
media generation, search, titles and compaction keep their existing policies.

Each HTTP attempt keeps a separate sanitized request receipt. A 502/503 remains
an unknown billing outcome even if a later attempt completes; request IDs are
correlation evidence, not proof of billing. The public retry event contains only
`type: "retrying"` and `data: { attempt: 1 | 2 }` within the existing run envelope.
The host validates the run and attempt and forwards no error-body diagnostics.
