# Assistant export, backup and restore

The assistant uses file-based storage. A Markdown export is a readable copy of
the current conversation branch. A complete backup preserves all conversations
for the signed-in owner, including trash, superseded messages, drafts, attachment
bytes, Pi history and the `operations` ledger with each run's original prices.
Restoring does not recalculate historical prices using today's catalog.

## User behavior

Before either transfer, the renderer saves pending drafts, including an unsent
draft that does not yet have a conversation. Conversation switching and editing
are disabled while this save and the file dialog run. Cancelling the dialog is
quiet; the draft remains saved. An unchanged draft is not rewritten.

A backup reports how many conversations it wrote and how many damaged ones it
left out: an unreadable or invalid record, a missing native history that
messages point into, a missing or resized attachment file, or a file over its
limit. A saved error that is not an `assistant_*` code is dropped on load, and
the backup archives the record that way. The empty folder of
a conversation whose creation never finished is ignored, as the conversation
list ignores it. A conversation added or removed while the backup runs aborts it.

Restore merges per conversation. Missing conversations are added; existing
conversations whose content matches the archive (compared in archived form, see
below) are skipped; a conversation with the same ID that differs locally, cannot
be read or is replying is counted as a conflict, left untouched and not imported.
Conflicts do not stop the others. Only archive-level problems fail the whole
restore: an invalid or oversized archive, another owner, an unsupported version,
a known key inside it, or a disk error before the first conversation is
published. If a disk or authority failure occurs after some conversations were
published, they remain valid and the result reports counts and a warning; a retry
imports the rest and skips the unchanged ones. If an earlier permanent deletion
of the same ID left an unfinished tombstone, restore removes it before publishing
(and counts a conflict if it cannot), so the restored conversation can be deleted
again.

Preferences are the remembered model and the assistant settings (custom
instructions, automatic titles, finish notifications). Restore merges them field
by field: a value this profile has saved stays, including empty instructions,
and the archive only fills in the fields the profile never saved, so an empty
profile receives all of them. An archive made before the settings existed holds
only the model: the current settings stay, or their defaults (no instructions,
both switches on) when none were saved. An unreadable local preferences file is
left as it is. The renderer reloads the model preference and the settings after
restoration.

The selected account and its authority generation are checked before opening
the native picker, after it closes and before writes. An archive belongs to its
original owner; this is not a cross-account migration tool. Account/region changes
invalidate in-flight transfers. While a reply is running for the account, backup
and restore are refused with `assistant_transfer_busy`, before the file dialog
and again before any file is read; a conversation found replying (an active run,
a running operation or a streaming message) during a backup refuses it with the
same code. The renderer refuses with the same code while it is sending or
importing files.

## Format and validation

ZIP format version 1 contains `manifest.json` with `nativeVersion: 3`, the owner
hash, UUID session IDs and a size/SHA-256 inventory. Allowed member names are:

- `prefs.json`
- `<session UUID>/session.json`
- `<session UUID>/session.jsonl`
- `<session UUID>/attachments/<attachment UUID>`

Directory members, duplicate names, absolute paths, path aliases, symlinks and
unexpected files are rejected. Before the `zip` crate parses anything, restore
checks the archive's layout itself. The crate searches end records backwards and
falls back to an earlier one when the last fails its checks, so only the last end
record is accepted, and only when the crate must use that same record: nothing
after it, one disk, the ZIP64 locator and end record exactly where they point
(when the classic record defers to them, with consistent counts, sizes and
versions), and a central directory of at most 10,240,000 bytes (20,000 × 512) that ends exactly at the end
record and holds exactly the declared number of headers. Every header must have
no extra field, be stored or deflated, and point to a local header that, with its
data start, lies before the directory. An archive written by this assistant
always has this layout. More than 20,000 members is refused before the directory
is read. The crate keeps one entry per name, so a parsed archive with fewer
entries than declared has duplicate names and is invalid. Each member must
inflate to exactly its declared size. Actual bounded reads, ZIP CRC and manifest
hashes are checked before imported sessions become visible.

| Resource | Limit |
| --- | --- |
| ZIP | 512 MiB |
| Total expanded data | 2 GiB |
| Members | 20,000 |
| Manifest | 16 MiB |
| Session record | 64 MiB |
| Native history | 256 MiB |
| Individual attachment | 50 MiB |
| Preferences | 1 MiB |

The native validator checks the pinned Pi SDK 1.4.0 session-3 entry shapes,
parent links, message content, run/operation lineage and saved native entry IDs.
Image JSONL lines can exceed 8 MiB and are bounded by the native-file limit.
This format accepts Ofox assistant history, not arbitrary Pi CLI imports. SDK-only
sentinels such as self-referencing empty compaction and `branch_summary.fromId`
equal to `root` are not generated by this assistant and are currently rejected.

## File and credential handling

Runtime configuration, authentication files and managed keys are not archived.
The keys scrubbed are the account's stored assistant key (read locally, never
minted or fetched) and every key handed to a run since the app started; an older
key that was replaced before this app session is not known and not scrubbed.
If the stored key cannot be read, that transfer scrubs only the keys known to
this process. They are scrubbed from JSON and from Markdown exports; a matching key in
attachment bytes blocks the backup instead of silently changing the file, and an
archive containing one is refused on restore. Preferences serialize only known
fields; archived instructions over 4,000 characters, or settings of the wrong
type, make the archive invalid. This does not identify arbitrary secrets a user has pasted into a
document. ZIP archives are not encrypted and contain conversation data.

The native history's header names the conversation's absolute working folder
and, for a fork, its parent session file. The archive blanks `cwd` and drops
`parentSession`; the runtime sets its own working folder when it opens a history,
so a restored conversation continues. Restore treats a local file as unchanged
when either its bytes or its archived form (keys scrubbed, header blanked, record
repaired) match the archive, so restoring a backup into the profile it came
from, including one made before this blanking, skips its conversations instead
of reporting conflicts.

Export writes an atomic private file after validation. It rejects destinations
inside application configuration and non-regular/symlink targets, preserving an
existing output on failure. A backup is assembled in a temporary file and then
streamed into a private (0600) temporary file beside the destination, which
replaces it atomically; the archive is never held in memory whole (one member
file at a time is). Restore validates in a
private staging directory on the destination filesystem and publishes each
session by atomic directory rename, retried briefly on Windows when another
process holds a file inside.
The owner transfer lock and per-session attachment/record locks serialize related
operations. Startup removes only recognized abandoned restore staging folders;
it does not follow links or remove unknown directories.

## Verification boundary

Regression tests cover exact round trips, old prices, owner/generation changes,
native picker cancellation, malformed native entries, large image lines, unsafe
paths, checksum edits, compressed oversized members, members larger than declared,
duplicate names, declared member counts (including ZIP64), a fake trailing end
record, truncated and damaged archive ends, symlink metadata, damaged and
oversized conversations left out of a backup, active runs, per-conversation
conflicts, partial publication, retries, pending deletions, blanked native
working folders (and older archives without the blanking), invalid error codes,
staging cleanup, credential exclusion (including the stored key, on backup,
export and restore), the field-by-field settings merge (and older archives
without settings) and renderer draft/preferences races.

The macOS integration test uses prepared Node 22.23.4 and Pi SDK 1.4.0 with local
mock gateways for both `ofox.ai` and `ofox.io` identities. It creates a search and
attachment conversation, backs it up, deletes the original temporary config,
restores into an empty config and continues with a fresh host. The runtime reads
old history and the restored attachment while the saved search price remains
unchanged even when the catalog price changes. No real account data is modified.

Native macOS save/open/cancel, identical restore, empty-profile restore and
continuation were subsequently verified in the real Tauri application with local
mock services; see [native acceptance](assistant-native-acceptance.md). Windows
is deferred at the user’s request. Search fees remain estimates from the saved
catalog, as confirmed by the earlier real-gateway audit.
