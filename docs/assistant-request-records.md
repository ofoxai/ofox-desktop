# Assistant request records

The conversation menu opens saved request records, including model replies,
generated titles, context summaries, image/video generation, explicit audio tools
and web searches. A reply's details show the requests counted under that reply;
its generated title remains in the conversation's records. Opening this view
never submits a model request or fetches a bill.

Each new record keeps the selected region/model, outcome, returned usage and
pricing evidence frozen for that operation. Gateway correlation IDs come only
from the first valid `x-ofox-request-id`, `x-request-id`, or `request-id` response
header, in that order. They are bounded opaque strings with their header name;
credentials and malformed values are rejected or scrubbed. A completion body's
`id`, local operation/tool-call ID and video job ID are separate identities.
The interface says **Not returned** when a gateway ID is absent. Older histories
cannot acquire identifiers retroactively.

Image/video submission attempts keep separate receipts when a refused key is
refreshed once. An ambiguous submission is never replayed automatically. Video
polling updates the known job's usage/status without replacing its submission
identifier; download failure keeps its job and available cost evidence. Images
freeze their catalog pricing, videos their accepted quote/parameters, audio its
billing dimensions, and chat/title/summary requests their token rates. Search
records keep returned usage and the run's frozen search price; the search tool
does not currently expose a platform correlation ID.

**Estimated cost** is calculated from usage and frozen prices (or explicitly
labeled speech character estimates). **Platform-reported cost** is a returned
video usage amount that has not been reconciled. Neither is an invoice. The
actual-charge field stays unknown until there is independent per-request billing
evidence. Binary speech responses with no usage never establish an actual cost;
a zero token count, account balance change or unrelated request total cannot fill
that gap. Unknown coverage stays unknown rather than becoming zero.

Facts live with private native histories/operation ledgers and survive compatible
ZIP backup/restore. They are not added to model conversation input, and receipt
metadata does not add another usage charge. Account/region changes fence the view
and native publication. If the app exits before a response receipt can be saved,
its known operation remains uncertain; this feature cannot recover server-only
identifiers. Native macOS and paid audio/invoice acceptance for this iteration
remain a separate final gate; Windows native acceptance is still unverified.
