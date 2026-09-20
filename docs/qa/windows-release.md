# Windows Native Install and Release Gate

## Installer ownership

Windows installation runs inside the Tauri backend without opening an external
terminal or requiring a preinstalled Python runtime.

- Codex opens Microsoft Store product `9PLM9XGG6VKS` and waits for the verified
  `OpenAI.Codex_2p2nqsd0c76g0` package.
- Claude runs the official `https://claude.ai/install.ps1` installer.
- Hermes runs the official Nous Research PowerShell installer with setup
  prompts disabled.
- Gemini, OpenCode, and OpenClaw use npm with a private Node.js LTS runtime in
  `%LOCALAPPDATA%\Ofox Desktop\toolchain`.

The Node archive is selected from `nodejs.org/dist/index.json`, checked against
the matching `SHASUMS256.txt`, extracted to a staging directory, and swapped
into place only after verification. npm packages are installed under the same
toolchain root, so administrator access is not required.

## VM acceptance

Use a clean Windows 11 x64 VM and exercise each tool through the UI:

1. Install, detect, bind, run configuration checks, and launch.
2. Repeat with an existing installation and without `winget`.
3. Interrupt the Node download, then retry and confirm no partial runtime is
   detected.
4. Test direct internet, proxy, offline, and low-disk conditions.
5. Confirm CLI-only Codex and an explicit WSL override still take precedence
   when configured.

Record the Ofox version, Windows build, tool versions, network condition, and
screenshots/logs for every run. Do not mark the Fizzy step complete until the
full matrix passes.

## Signed MSI gate

The release environment must define these GitHub secrets:

- `WINDOWS_CERTIFICATE`: base64-encoded code-signing PFX
- `WINDOWS_CERTIFICATE_PASSWORD`: PFX password
- `WINDOWS_TIMESTAMP_URL`: certificate-provider timestamp service

The release workflow imports the certificate temporarily, builds the MSI, and
requires `Get-AuthenticodeSignature` to report `Valid` for both the application
executable and MSI. Missing credentials or invalid signatures fail the build;
no Windows artifact is uploaded. The certificate is removed from the runner in
an `always()` cleanup step.
