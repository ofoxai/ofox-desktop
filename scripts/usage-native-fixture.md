# Native usage UI fixture

`usage-native-fixture.html` renders the actual usage components with fictional in-memory IPC responses. It reads no native source files and has no account, credentials or paid calls. Unhandled IPC, external fetches and external window navigation are blocked. It does not verify native parsing, statistics parity, installed application behavior, Windows, signing or notarization.

Open this HTML through the integration owner's existing Vite renderer server, using its `/@fs/` route when the server root is `src/`. Do not replace the production entry point. The fixture is separate from the ordinary build.

Query parameters:

| Parameter | Values                                                 |
| --------- | ------------------------------------------------------ |
| `lang`    | `zh`, `en`, `ja`                                       |
| `theme`   | `light`, `dark`                                        |
| `state`   | `complete`, `partial`, `unavailable`, `zero`, `legacy` |
| `tab`     | `overview`, `requests`, `sessions`                     |
| `rebuild` | `running` (otherwise idle)                             |
| `apex`    | `ofox.ai`, `ofox.io`                                   |

The fixture toolbar switches the same states without accessing live data. Start a rebuild, use the product's cancel button, and confirm that the cancelled receipt retains the previous data. Finish a running rebuild to exercise query refresh. Expand the product data-status disclosure to enter original-history audit; its banner and return button use the real components. Select a request to examine the missing measured status/timing and the original saved-amount distinction.

At a 900px viewport, check all three languages in both themes. Verify no horizontal clipping, visible focus with Tab/Shift+Tab, metric-tab arrows, Enter, Escape and request-dialog focus return. Under `unavailable`, Token and cost values are unknown and no fake zero trend is drawn. Under `zero`, trusted zero values remain zero. `partial` retains known amounts with partial coverage; `legacy` shows the older-backend explanation. These controls are test aids, not additional product settings.

The hidden `fixture-render-metrics` output records one fresh-navigation first
model-marker frame with the product's React components and `requestAnimationFrame`.
A sample is emitted only after the synthetic model is present in the rendered surface.
It does not measure subsequent refreshes or tab changes, which can retain old DOM.
It measures renderer work in this small aggregate fixture, not native scanning,
10k-source parsing, real IPC latency or an end-to-end production percentile.
The separate release Rust benchmark measures the mixed 10k-session query path.
