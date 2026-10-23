# Branch installer packages

Pushes to `release/**` run [Branch Packages](../.github/workflows/branch-packages.yml)
and the independent [Windows Probe](../.github/workflows/windows-probe.yml).
The package jobs require full reusable CI and a successful push/manual Windows
Probe for the same commit in this repository. The bounded gate waits for that
separate run; pending, failed, foreign and PR merge-ref results cannot pass.

Download the `branch-packages-darwin-aarch64-<commit>` or
`branch-packages-windows-x86_64-<commit>` artifact from the successful run.
Artifacts are retained for 30 days and contain:

- A macOS Apple Silicon DMG or Windows x64 NSIS installer.
- `build-provenance.json` recording version, full source SHA, size and SHA-256.
- `SHA256SUMS.txt` and a short package description.

The existing version metadata is retained. Preview installers use the normal
application identifier and user data. macOS packages use an ad-hoc signature
without Apple notarization; Windows packages are unsigned. Operating systems may
require explicit approval when opening these test packages.

This workflow has read-only repository permission, no release environment or
release secrets, and no publishing step. It creates no tags or GitHub Releases
and does not change the production download catalog. Official signed/notarized
distribution remains in the separate Release workflow. Native Windows agent/UI
acceptance is distinct from the automated Windows checks and installer build.
