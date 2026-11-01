# Assistant model catalog

The assistant opens on its last usable in-memory model catalog and refreshes
stale sources in the background. On a cold start or restart there is no disk
catalog cache: the page opens with a loading state, and each healthy source can
populate its models before another source times out. Existing conversations
remain readable while models load.

Each apex/source has its own last good rows and failure state. Successful rows
are fresh for ten minutes. An automatic read does not ask a failed source again
for thirty seconds; explicit Retry bypasses that cooldown. A failed source keeps
its own last good rows, and the UI distinguishes updating from using a copy
after refresh failure. A valid empty catalog can also be retried immediately.
Missing-model lookups still perform a fresh check. Concurrent source reads share
one fetch, including sends that overlap a background refresh.

Refresh work is tracked, scoped to the signed-in owner, authority generation
and selected apex, and cancelled on authority changes and shutdown. The public
catalog fetch does not mint an assistant key or submit a model request. A valid
restored identity may read public metadata before paid-run admission reopens.
The runtime/auth/catalog/key endpoint builders remain region-aware.

Monotonic catalog revisions and a subscription-before-bootstrap handshake keep
an early source update from being lost or overwritten by an older response.
Updates replace only catalog fields. They do not replace account preferences,
conversation models, drafts or saved billing facts. ASR/TTS dialogs freeze their
model/format/price metadata while a request is active; subsequent requests can
use the refreshed directory. Saved operations keep their original snapshots.

First-start, failure/copy/cooldown, explicit retry, concurrent reads, both apex
choices, authority switching and shutdown are covered by native sandbox tests.
Browser checks use isolated mock events. Native macOS application acceptance and
actual per-request billing remain separate checks; Windows execution is deferred.
