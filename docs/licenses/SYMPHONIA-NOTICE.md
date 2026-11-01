# Symphonia audio decoder

Ofox Desktop uses the unmodified Symphonia 0.6.1 crates `symphonia`,
`symphonia-core`, `symphonia-metadata` and `symphonia-bundle-mp3`, by Philip
Deljanov and the Symphonia contributors, under the Mozilla Public License 2.0.
Only the MP3 feature is enabled; no codec binary or executable is downloaded
at runtime.

The full license is included as `SYMPHONIA-MPL-2.0.txt`. The corresponding
source is available without charge as versioned source packages for [symphonia](https://crates.io/crates/symphonia/0.6.1),
[symphonia-core](https://crates.io/crates/symphonia-core/0.6.1),
[symphonia-metadata](https://crates.io/crates/symphonia-metadata/0.6.1) and
[symphonia-bundle-mp3](https://crates.io/crates/symphonia-bundle-mp3/0.6.1).
Cargo.lock records the exact crate versions and checksums used in this build.
No Symphonia source files are modified or copied into Ofox's source tree.

## regex-lite 0.1.9

Symphonia metadata uses regex-lite, distributed here under its MIT option.
[Versioned source](https://crates.io/crates/regex-lite/0.1.9).

```text
Copyright (c) 2014 The Rust Project Developers

Permission is hereby granted, free of charge, to any
person obtaining a copy of this software and associated
documentation files (the "Software"), to deal in the
Software without restriction, including without
limitation the rights to use, copy, modify, merge,
publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software
is furnished to do so, subject to the following
conditions:

The above copyright notice and this permission notice
shall be included in all copies or substantial portions
of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
DEALINGS IN THE SOFTWARE.
```
