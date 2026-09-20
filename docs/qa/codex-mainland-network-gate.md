# Codex Mainland Network Gate

Run this gate before deciding whether Ofox Desktop needs an R2 mirror for the
official ChatGPT/Codex macOS installer. Do not enable or populate a mirror based
on a single failed download.

## Preconditions

- Use a clean macOS 13+ Apple Silicon VM in mainland China.
- Disable VPNs, shell proxy variables, PAC files, and macOS HTTP/HTTPS/SOCKS
  proxies. The script rejects detectable proxy settings.
- Test two materially different networks, such as China Telecom broadband and
  China Mobile cellular. Record the provider and city outside the evidence file.
- Keep the VM awake and ensure at least 5 GB of free disk space.

## Run the gate

Use a unique, non-sensitive label for each network. Each invocation performs
three fresh downloads directly from the official OpenAI URL, with a 15-minute
limit per download.

```bash
scripts/qa/verify-codex-download-macos.sh \
  shanghai-telecom evidence/shanghai-telecom.jsonl

# Switch to the second direct network before running again.
scripts/qa/verify-codex-download-macos.sh \
  shanghai-mobile evidence/shanghai-mobile.jsonl
```

The script records duration, byte count, SHA-256, app version, Bundle ID, and
Team ID. It also requires valid nested code signatures and a successful
Gatekeeper/notarization assessment for every DMG. A non-zero exit means that
network has not passed; retain the terminal error together with any partial
JSONL evidence.

## Decision rule

Continue using the official source only when both evidence files end with
`"passed":true`, for six successful runs total. Any TLS/connection error,
download over 900 seconds, invalid identity, signature failure, or notarization
failure blocks the gate.

If the gate fails, first confirm the failure on both networks. Before building
an R2 fallback, obtain permission to redistribute the installer. A mirror must
accept only a package that passed this same identity/signature verification,
store it under an immutable versioned key, and update its manifest atomically.
Never mirror an unverified or mutable download.
