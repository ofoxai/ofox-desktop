# Assistant audio acceptance

Date: October 31, 2026. Scope: Fizzy #1364, audio and stability changes after
`42616e3d`, with application code through `bcff8fd9`. This records macOS QA
evidence, not a Windows or every-provider certification. Final card acceptance
still awaits a successful live download using a non-expired video URL, or an
explicit decision to retain that limitation.

## Native audio

The temporary QA application completed one ElevenLabs Eleven v4 WAV synthesis
and one GPT Transcribe request through each Ofox apex. Each synthesis used the
18-character text `Hello, audio test.`, English and the same selected voice.
Each transcription used only its generated short WAV. All four requests were
submitted once; none was automatically retried. No microphone recording was
uploaded. The WAV results were 1.76 and 1.84 seconds, respectively.

Both results played to the end and were saved with bytes matching the private
stored output and permissions 0600. Both transcriptions returned the sample
text. Editing and explicitly inserting the transcript appended to an existing
draft; it did not send a chat request or replace the selected chat model.
Each test conversation had zero chat messages and two separate audio operations.
Explicit keep-original retained byte-identical source files; the default remained
off when the input dialog was reopened.

The approved microphone check started and stopped on native hardware. Tool
interaction latency extended it to 8.13 seconds, beyond the planned roughly two
seconds. It was discarded before submission; reopening showed empty input.
Permission-denial, device-loss and interruption paths have automated coverage,
not a claim that each hardware failure was reproduced manually.

Separate local eight-second WAV, MP3 and Ogg Opus fixtures exercised native
play/pause/stop/save. Those fixtures are not model output or billing evidence.
Three-language, light/dark, 900px and wide-window checks used controlled renderer
fixtures; actual gateway samples above used the native Chinese UI.

## History and backup

A native full backup contained all 13 QA conversations, including both real
audio test conversations, four audio operations and four retained/generated
audio files. All 39 manifest file hashes were checked. After quitting QA and
preserving one test conversation outside the live store, restarting and using
the native restore picker recreated it. Its JSON and both audio file hashes
matched the original exactly, including IDs, parameters, prices and unknown
cost fields. Restored files were 0600. Both restored history entries played
without a new model request; saving the retained original matched its source.

Separate native and automated checks cover older image/video history,
video-excluded archives, exact-match restore and forged native-media rejection.
The earlier backup regression fixture incorrectly used an assistant role;
the final fix and tests enforce the actual media role and matching lineage.

## Per-request billing

The authenticated organization's activity log showed four unique model/time
matches, with billing timestamps 0.19–1.01 seconds after native submission.
Full header identifiers and full billing record identifiers were preserved in
private evidence. They differ; this is time/model/account correlation, not an
exact identifier join. Account-wide balance differences were not used.

| Apex    | Operation                                | Frozen catalog estimate, USD | Observed debit, USD |
| ------- | ---------------------------------------- | ---------------------------- | ------------------- |
| ofox.io | Eleven v4, 18 characters                 | 0.00144                      | 0.00032000          |
| ofox.io | GPT Transcribe, returned usage 2 seconds | 0.00015                      | 0.00015000          |
| ofox.ai | Eleven v4, 18 characters                 | 0.00144                      | 0.00032000          |
| ofox.ai | GPT Transcribe, returned usage 2 seconds | 0.00015                      | 0.00015000          |

Observed total: USD 0.00094000, within the approved USD 0.05 budget. Both ASR
computations match their debits. The lower TTS debits have an unverified cause;
binary responses supplied no usage, so the application correctly retains unknown
actual TTS cost rather than replacing the estimate with an invented receipt.

## Existing video recheck

Querying the previously approved lake-video task returned completed with its
original cost. Its output URL had a 24-hour signature and the upstream returned
HTTP 403, `Request has expired`. The native download failed safely, retaining
the job, cost and existing cache, without generating another video. Cached video
played to four seconds; its 840,772-byte native save matched the original hash
and used permissions 0600. The temporary failure scenario was then restored
to the original record and file hashes. A fresh successful download through
the new pinned transport remains unverified; no new generation was authorized
by the existing-video approval.

## Gates and execution limits

Final code gates passed: 2,714 frontend tests in 220 files; 3,847 Rust tests
passed with 16 ignored, plus all integration suites; runtime 216 and search 85.
Typecheck, Prettier, renderer build, rustfmt and all-target Clippy with warnings
denied passed. Fixes had failing regression tests before implementation.
Review and re-review were direct; no independent-review claim is made.

QA executable SHA256:
`1803b8b0986c51cacd968f7813a7a79bb4e3dd77eb2cff36eb55b01e32b70308`.
Subsequent documentation-only commits do not change that executable.

An earlier UI attachment launched an unintended instance that opened the real
profile database and log. It was stopped and disclosed immediately; no model
request or tool binding was submitted in that instance. Startup writes were
not audited, and no real-profile inspection, cleanup or rollback was attempted.
Later launches use explicit LaunchServices sandbox variables and verify the
single process's actual database path before UI actions. After approved OAuth,
one WorkBuddy binding appeared in the temporary QA home; it was documented,
not silently revoked, and WorkBuddy was not launched for this acceptance.

Only local commits are delivered; no push, merge or production installation.
Long audio/meeting workflows, full-duplex realtime speech, raw PCM playback
and native Windows execution remain outside this iteration's accepted scope.
