# Branch installer packages

Pushes to `release/**` run [Branch Packages](../.github/workflows/branch-packages.yml)
and the independent [Windows Probe](../.github/workflows/windows-probe.yml).
The package jobs require full reusable CI and a successful push/manual Windows
Probe for the same commit in this repository. The bounded gate waits for that
separate run; pending, failed, foreign and PR merge-ref results cannot pass.
Branch Packages owns the full CI call for release-branch pushes; a second outer
CI run is not queued. Main and pull requests retain their existing CI triggers.

Download the `branch-packages-darwin-aarch64-<commit>` or
`branch-packages-windows-x86_64-<commit>` artifact from the successful run.
Artifacts are retained for 30 days and contain:

- A macOS Apple Silicon DMG or Windows x64 NSIS installer.
- `build-provenance.json` recording version, full source SHA, size and SHA-256.
- `SHA256SUMS.txt` and a short package description.

The Windows job also uploads
`assistant-parity-driver-windows-x86_64-<commit>`. This separate developer artifact
contains `assistant_host-parity.exe`, its SHA-256, `driver-provenance.json` and a
README. Cargo identifies exactly one release integration-test executable, and
its build-script output must report the expected `OFOX_BUILD_SOURCE_SHA` and
`OFOX_BUILD_SOURCE_CLEAN=true`. Compilation precedes the untracked installer
output files; the driver artifact is assembled outside the checkout. No model
test runs while producing it.

The driver requires the same clean Git checkout and locked JavaScript dependency
preparation on a native Windows 10/11 client. The bundled Node runs the strict
controller; the client does not need Rust/MSVC. This is a developer Host test,
separate from the offline probe and installed-app UI acceptance. See the
[client instructions](document-engine-client-acceptance.md).

The existing version metadata is retained. Preview installers use the normal
application identifier and user data. macOS packages use an ad-hoc signature
without Apple notarization; Windows packages are unsigned. Operating systems may
require explicit approval when opening these test packages.
The macOS build requests both `app,dmg`, retaining the app for signature
verification. Tauri removes that intermediate app after a DMG-only build.
Both platforms build all Cargo binary targets so bundled helper executables are
rebuilt from the same source instead of reused from a restored target cache.

This workflow has read-only repository permission, no release environment or
release secrets, and no publishing step. It creates no tags or GitHub Releases
and does not change the production download catalog. Official signed/notarized
distribution remains in the separate Release workflow. Native Windows agent/UI
acceptance is distinct from the automated Windows checks and installer build.
