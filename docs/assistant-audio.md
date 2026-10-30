# Assistant audio iteration

Scope: Fizzy #1364, following the validated chat/image/video assistant. This
iteration adds recorded/file speech input and explicit text-to-speech output.
Long meeting workflows and full-duplex realtime conversation are later work.

## User flow

Speech input is separate from the conversation's selected chat model. The user
records or imports an audio file, starts transcription and reviews the text in
the composer before sending it. An audio operation never fabricates a chat turn
or automatically sends recognized words. A late result cannot overwrite a newer
draft or insert into a different account/conversation. Offer the saved result
for explicit insertion when its original draft/selection is no longer current.

Speech output starts from a completed reply or explicitly supplied text. Model,
voice and output format are selected independently of chat. Generated audio can
be played, paused, stopped and saved; playing a stored result makes no model
request. Generating a different voice is a new, explicitly charged operation.
The first release plays Gemini WAV and ElevenLabs MP3/WAV/Opus. Raw PCM is a
provider capability, not directly playable until framing is implemented.

Product limits for the first short-audio release are 25 MiB / five minutes per
input, 4,000 Unicode characters per synthesis and 50 MiB per generated result.
These are application limits, not advertised provider limits. Do not silently
truncate or split a request. Recording produces a bounded mono WAV rather than
assuming every gateway accepts a browser's default recording container.

## Capability and pricing contract

The public catalogs classify speech and transcription but, in the audited
snapshot, provide no voice/format attribute blocks. Supported profiles are
therefore scoped to documented exact model IDs and matching endpoints. An
unknown future audio model remains unavailable until its contract is supported.
Suggested voices are examples, not a claim of an exhaustive provider voice list.

| Family | Request/output | Billing basis |
| --- | --- | --- |
| Gemini TTS | Named voice; WAV/PCM; tone instructions | Text/audio token rates |
| ElevenLabs Eleven v4 | Provider voice ID; MP3/Opus/WAV/PCM | Input characters |
| GPT Transcribe | Multipart audio; JSON text/usage | Input audio seconds |
| GPT mini/diarize transcription | Multipart audio; JSON text/usage | Recorded token categories |
| ElevenLabs Scribe v2 | Multipart audio; JSON/verbose JSON | Input audio seconds |

Rates come from the selected apex catalog and are stored with an operation.
Missing, malformed, negative or overflowing prices remain unknown. Token rates
must retain their text/audio distinction; zero text-token prices do not make a
duration- or character-billed request free. A binary speech response need not
include measurable usage: retain a known estimate separately from actual usage,
and keep actual cost unknown unless a trustworthy receipt establishes it.
No cost is inferred from account-wide balance changes.

## Ownership, persistence and cancellation

Rust owns credentials and all gateway requests, using the selected apex builders
and existing proxy transport. The renderer supplies bounded input/attachment IDs,
never arbitrary source paths or provider URLs. Microphone permission is requested
only after a recording gesture; tracks and buffers are released on cancel,
navigation, account change and unmount.

Use independent audio model preferences. Voice/format choices belong to a
synthesis model, and never replace chat/image/video preferences. Saved values
remain owner-scoped; restore fills only fields never saved on this machine.
A future settings writer must preserve unknown fields and damaged-file refusal.

Reuse the host's run admission, authority fences and per-conversation exclusion,
without an artificial user/reply bubble, as manual compaction does. Persist an
audio ledger operation before crossing the paid request boundary. Its model,
input, price and IDs are frozen. A cancelled/unknown submitted request is never
automatically replayed. Cached playback can continue independently; creating a
new audio result requires an idle conversation in the first release.

Persist transcription text, generated-audio references, parameters and billing
facts with the operation. Original recordings are temporary by default; an
explicit keep-original choice retains them as private history attachments.
History/backup/cleanup must include every retained audio-operation reference.
Audio bytes use the owner/generation-scoped private media protocol and bounded
range reads; native save is atomic and rechecks authority after the picker.
Audio model calls do not receive private standing chat instructions.

## Delivery and acceptance

Implement capability metadata/preferences, bounded import, transcription,
recording UI, synthesis, playback/storage, then stability work. Tests must cover
both apexes, denial/cancel/stale-owner paths, uncertain charging, old backups and
repeated playback without a request. UI covers zh/en/ja, both themes, keyboard
focus and 900px layouts. Native macOS microphone/play/save and approved small
live samples require separate evidence and per-request billing reconciliation.
Windows native acceptance remains explicitly deferred.

Sources checked October 30, 2026:

- [Speech endpoint](https://ofox.ai/docs/api/openai/audio-speech)
- [Transcription endpoint](https://ofox.ai/docs/api/openai/audio-transcriptions)
- [Gemini TTS](https://ofox.ai/docs/develop/models/gemini-tts)
- [ElevenLabs](https://ofox.ai/docs/develop/models/elevenlabs)
- [GPT transcription](https://ofox.ai/docs/develop/models/gpt-transcribe)
