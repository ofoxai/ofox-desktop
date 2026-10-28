# Assistant Office attachments — 2026-10-28

Scope: Fizzy #1234, S4. The first implementation (a DOM parse with
`fast-xml-parser` in a worker thread) failed an independent review: its
worker heap cap was overridden by the sidecar's `--max-old-space-size=512`,
it rejected files with bytes after the ZIP directory and ordinary 10,000-row
sheets, scrambled table text and passed dates as serial numbers. It was
replaced by the streaming reader below.

## What is read

The bundled Node 22 runtime reads DOCX, XLSX and PPTX text; the exact rules
are in the [runtime README](../assistant-runtime/README.md#attachments).
In short: the package's `officeDocument` relationship names the main part
(and so the family), and every other part is reached through relationships.

- Word: body, text boxes, tracked insertions, footnotes and endnotes. Not
  read: headers, footers, comments, tracked deletions, moved-from text, field
  codes and hidden text. Ruling: hidden (`w:vanish`) Word text is not read;
  the model is told so for every Word file. Only direct run formatting is
  seen, so text hidden through a character style is still read.
- Excel: sheets in workbook order, labelled with their names and
  `(hidden)`/`(veryHidden)`; hidden rows and columns (and, in PowerPoint,
  hidden shapes) are read without a label. Chart sheets are named, not read. Rows are lines
  of tab-separated cells with a coordinate on each row's first cell and after
  a gap. Dates are recognised from the cell's number format (built-in ids and
  custom codes, 1900 and 1904 systems) and written as ISO dates, date-times or
  times; other numbers stay as stored. Formulas show `value [=formula]` with
  the stored result and are never calculated; a cell that shares another's
  formula shows `[shared formula]`.
- PowerPoint: slides in presentation order with `(hidden)` labels, tables,
  and speaker notes.
- Word and PowerPoint tables: a line per row, tab-separated cells (empty
  cells keep their tab, a merged Word cell adds one per spanned column),
  paragraphs within a cell joined by a space.

Pictures, charts, diagrams and embedded objects are never opened; the warning
counts them. Office outcomes stay `partial` with `textOnly: true`, as before,
so the host and the renderer need no change.

## Design and limits

Extraction runs in a child process forked from the sidecar with
`process.execPath` (the bundled `node`/`node.exe`), `--max-old-space-size=128`,
`windowsHide`, ignored stdio and an empty environment; on Windows libuv still
adds the variables every process needs (`SYSTEMROOT`, `TEMP`, `USERPROFILE`
and the like), on macOS only the system's `__CF_USER_TEXT_ENCODING`. The
file's bytes are sent once over the IPC channel (V8 serialization).

Memory, as measured: the child's V8 heap limit is about 176 MiB (the young
generation on top of the 128 MiB old space) even when the sidecar runs with
512 MiB, which a test checks; its resident memory can reach about 350 MiB
with the inflate and decode buffers outside that heap. The child is killed
on Stop, after 10 seconds and after it answers; at 7.5 seconds it stops
reading and answers with what it has. A child that dies after starting (out
of memory, crash) or does not answer is `too_large`/`office_expansion`; one
that cannot start or never reports ready is a run error, never
`invalid_office`.

ZIP: yauzl 3.4.0 over the in-memory bytes. Bytes after the end-of-central-
directory record are trimmed first. At most 10,000 entries; duplicate,
absolute or `..` names, encrypted entries and size or CRC mismatches are
rejected. XML: saxes 5.0.1, namespace-aware (elements are matched by
namespace URI, transitional and strict), DOCTYPE refused (no entity can
expand or load anything), nesting at most 256, UTF-8 or UTF-16 by byte order
mark. Parts are inflated chunk by chunk straight into the parser; text is
collected only inside text elements, so neither markup nor whitespace is ever
held. Limits that end the read as `partial` with what was read: 128 MiB
inflated per file (counted while inflating), 16 Mi characters of output,
shared strings 8 Mi characters or 1 Mi entries, 65,536 cell formats, 32,767
characters per cell, the 7.5-second deadline. A number format code longer
than Excel's 255 characters counts as not a date (the deadline cannot
interrupt the synchronous classification, so its input is bounded instead),
and each code is classified once. Read-once memos are keyed by the ZIP entry,
so letter-case variants of one part name are one part. Only when nothing was read
before such a limit is the outcome `too_large`.

Dependencies: `yauzl` 3.4.0 (MIT, with `pend` 1.2.0, MIT) and `saxes` 5.0.1
(ISC, with `xmlchars` 2.2.0, MIT), pinned in the lockfile;
`fast-xml-parser` and the seven packages it brought are gone. The
[Office notices](../assistant-runtime/notices/office.txt) reproduce each
package's license, saxes' repository license (its npm package has none; it
includes the license of sax) and the buffer-crc32 and fd-slicer licenses of
code vendored into yauzl. A packaging test checks that the notice names
exactly the shipped packages.

### Measured (Apple Silicon, Node 22.23.4, 128 MiB old space)

The reader run in a process started like the child (Node 22.23.4,
`--max-old-space-size=128`, Apple Silicon); peak RSS includes building the
input in that process. The 127 MiB case ran through the real child.

| Input (compressed size) | Outcome | Time | Peak RSS |
| --- | --- | --- | --- |
| 10,000 × 6 sheet with shared strings and dates (346 KB) | read in full | 0.13 s | 81 MB |
| 30,000 × 6 sheet (1.0 MB) | read in full | 0.30 s | 115 MB |
| Honest bomb: one paragraph, 200 MiB of spaces (200 KB) | partial at the budget | 0.51 s | 68 MB |
| Lying bomb: same, size declared as 1 KiB | `invalid_office` | 6 ms | 60 MB |
| 120 MiB of empty paragraphs with attributes (358 KB) | read in full | 2.6 s | 71 MB |
| One 100 Mi-character text run (100 KB) | partial at 16 Mi characters | 0.46 s | 289 MB |
| One 127 MiB ASCII text run, through the child | child out of memory: `office_expansion` | 0.71 s | about 200 MB (sampled) |
| 120 MiB of shared strings (300 KB) | partial at 1 Mi entries | 0.70 s | 132 MB |
| 150,000 sheets naming one part (19 KB) | read once, 150,000 labels | 0.33 s | 121 MB |
| 60,000 entries | `office_expansion` from the directory | 1 ms | 61 MB |
| 30 MiB number format code, or a 200,000-character one used by 65,536 formats | not a date, read | 0.46 s for both | — |
| 20,000 letter-case variants of one sheet part | read once | 0.34 s | — |
| XML nested 300 deep / DOCTYPE with an external entity | `office_expansion` / `invalid_office` | < 10 ms | 57 MB |

The review had measured 640–770 MB and 2.4–4.3 s for hostile parts in the old
worker under the sidecar's flags.

## Legacy `.doc` assessment

`word-extractor` 1.0.4 is MIT; npm reports that version's latest metadata change
as 2022-06-29. Its current published parser correctly extracted a synthetic
Chinese `.doc` produced by macOS `textutil` (Chinese/Japanese/English, tabs and
numbers). It did not pass malformed-file robustness testing: changing the
19,456-byte fixture's directory FAT chain to a two-sector cycle exhausted a
child Node process's 32 MiB heap and terminated it with SIGABRT. The package's
`getSecIdChain` detects self-loops but not longer cycles. This test ran in a
bounded child process with a five-second timeout, never in the application or
against a user's document.

Decision: do not ship `word-extractor`; legacy `.doc` remains unsupported with
an explicit “save as docx or PDF” hint. This is a reliability decision, not a
claim that ordinary Chinese `.doc` text cannot be decoded. Source:
[word-extractor 1.0.4](https://www.npmjs.com/package/word-extractor/v/1.0.4),
[project](https://github.com/morungos/node-word-extractor).

## Verification

`assistant-runtime/test/office.test.mjs` (56 tests) covers the rules above
with exact expected text: Word and PowerPoint table layout (empty, merged and
multi-paragraph cells), sheets with names, order, hidden and chart sheets,
sparse coordinates, shared strings, inline strings, booleans, errors and
formulas, shared formulas, dates in both systems with built-in and custom
formats, hidden slides and notes, `w:vanish`, tracked changes and field codes,
compatibility alternatives, footnotes, endnotes and text boxes, UTF-16 parts,
bytes after the ZIP directory, missing and external relationships, a
realistic 10,000 × 6 workbook, repeated part references, honest and lying zip
bombs, a budget shared across parts, the deadline, a hung, a dying and a
failing-to-start and a never-ready child, Stop killing the child,
pathological number format codes, letter-case variants of one part, the child's heap limit under
a 512 MiB parent, 60,000 entries, deep XML, DTDs, encryption and damaged
packages. Bombs are built in the test with `node:zlib`; no bomb file is
committed. The checked-in fixtures
`assistant-runtime/test/fixtures/office/chinese.{docx,xlsx,pptx}` are
synthetic files generated by python-docx, openpyxl and python-pptx (see the
adjacent `generate.py`); production and tests only read their bytes. The
research fixtures written by LibreOffice and macOS `textutil` (Chinese,
Japanese, English, a table, a date, sparse cells, notes) also read correctly,
and their encrypted copies report `encrypted_office`. `pnpm assistant:smoke`
reads the three Chinese fixtures through the prepared production tree, so
the child is forked from the bundled Node there too.
The Rust host integration test (`src-tauri/tests/assistant_host.rs`) imports
the same three files and captures the real Pi SDK request through local mock
gateways, for both apex choices, a region switch and a restart; the outcomes
stay `partial` with `textOnly`.

Native Windows execution and final application click acceptance remain part of
S5. See [S4 progress](assistant-s4-progress.md) for the other unfinished work.
