#!/bin/bash

set -euo pipefail

readonly DOWNLOAD_URL="https://persistent.oaistatic.com/codex-app-prod/Codex.dmg"
readonly EXPECTED_BUNDLE_ID="com.openai.codex"
readonly EXPECTED_TEAM_ID="2DC432GLL2"
readonly RUNS=3
readonly MAX_DOWNLOAD_SECONDS=900

usage() {
  echo "Usage: $0 <network-label> <evidence.jsonl>"
  echo "Runs three direct downloads and verifies the signed/notarized app in each DMG."
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

if [[ $# -ne 2 ]]; then
  usage >&2
  exit 2
fi

readonly NETWORK_LABEL="$1"
readonly EVIDENCE_PATH="$2"

if [[ ! "$NETWORK_LABEL" =~ ^[A-Za-z0-9._-]+$ ]]; then
  echo "Network label may only contain letters, numbers, dots, underscores, and dashes." >&2
  exit 2
fi

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "This gate must run on macOS ARM64." >&2
  exit 2
fi

macos_major="$(/usr/bin/sw_vers -productVersion | /usr/bin/cut -d. -f1)"
if [[ ! "$macos_major" =~ ^[0-9]+$ ]] || (( macos_major < 13 )); then
  echo "This gate requires macOS 13 or newer." >&2
  exit 2
fi

for proxy_var in HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy; do
  if [[ -n "${!proxy_var:-}" ]]; then
    echo "Proxy variable $proxy_var is set; rerun in a direct-network shell." >&2
    exit 2
  fi
done

if /usr/sbin/scutil --proxy | /usr/bin/grep -Eq \
  '(HTTPEnable|HTTPSEnable|SOCKSEnable|ProxyAutoConfigEnable|ProxyAutoDiscoveryEnable) : 1'; then
  echo "A macOS system proxy is enabled; disable it before collecting gate evidence." >&2
  exit 2
fi

evidence_dir="$(dirname "$EVIDENCE_PATH")"
/bin/mkdir -p "$evidence_dir"
if [[ -e "$EVIDENCE_PATH" ]]; then
  echo "Evidence file already exists: $EVIDENCE_PATH" >&2
  exit 2
fi

work_dir="$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/ofox-codex-gate.XXXXXX")"
mounted_path=""

cleanup() {
  if [[ -n "$mounted_path" && -d "$mounted_path" ]]; then
    /usr/bin/hdiutil detach "$mounted_path" -quiet >/dev/null 2>&1 || true
  fi
  /bin/rm -rf "$work_dir"
}
trap cleanup EXIT INT TERM

printf '{"type":"session","network":"%s","runs":%d,"url":"%s","maxDownloadSeconds":%d,"macos":"%s","architecture":"arm64","proxy":"disabled"}\n' \
  "$NETWORK_LABEL" "$RUNS" "$DOWNLOAD_URL" "$MAX_DOWNLOAD_SECONDS" \
  "$(/usr/bin/sw_vers -productVersion)" >"$EVIDENCE_PATH"

for run in $(/usr/bin/seq 1 "$RUNS"); do
  dmg_path="$work_dir/codex-$run.dmg"
  mount_path="$work_dir/mount-$run"
  /bin/mkdir "$mount_path"

  echo "[$run/$RUNS] Downloading official DMG on network '$NETWORK_LABEL'..."
  started_at="$(date +%s)"
  /usr/bin/curl -q --fail --location --silent --show-error \
    --noproxy '*' --connect-timeout 30 --max-time "$MAX_DOWNLOAD_SECONDS" \
    --output "$dmg_path" "$DOWNLOAD_URL"
  finished_at="$(date +%s)"
  elapsed_seconds=$((finished_at - started_at))
  if (( elapsed_seconds > MAX_DOWNLOAD_SECONDS )); then
    echo "Download exceeded ${MAX_DOWNLOAD_SECONDS}s: ${elapsed_seconds}s" >&2
    exit 1
  fi

  bytes="$(/usr/bin/stat -f %z "$dmg_path")"
  sha256="$(/usr/bin/shasum -a 256 "$dmg_path" | /usr/bin/awk '{print $1}')"
  if [[ ! "$sha256" =~ ^[0-9a-f]{64}$ || "$bytes" -le 0 ]]; then
    echo "Downloaded DMG did not produce a valid size and SHA-256 digest." >&2
    exit 1
  fi

  /usr/bin/hdiutil attach -nobrowse -readonly -mountpoint "$mount_path" \
    "$dmg_path" >/dev/null
  mounted_path="$mount_path"

  app_path=""
  for candidate in "$mount_path/ChatGPT.app" "$mount_path/Codex.app"; do
    if [[ -d "$candidate" ]]; then
      app_path="$candidate"
      break
    fi
  done
  if [[ -z "$app_path" ]]; then
    echo "DMG does not contain ChatGPT.app or Codex.app." >&2
    exit 1
  fi

  bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' \
    "$app_path/Contents/Info.plist")"
  version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' \
    "$app_path/Contents/Info.plist")"
  if [[ "$bundle_id" != "$EXPECTED_BUNDLE_ID" ]]; then
    echo "Unexpected bundle ID: $bundle_id" >&2
    exit 1
  fi
  if [[ ! "$version" =~ ^[A-Za-z0-9._-]+$ ]]; then
    echo "Unexpected app version: $version" >&2
    exit 1
  fi

  signature_metadata="$(/usr/bin/codesign -dv --verbose=4 "$app_path" 2>&1)"
  team_id="$(printf '%s\n' "$signature_metadata" | /usr/bin/awk -F= \
    '/^TeamIdentifier=/{print $2; exit}')"
  if [[ "$team_id" != "$EXPECTED_TEAM_ID" ]]; then
    echo "Unexpected Team ID: $team_id" >&2
    exit 1
  fi
  /usr/bin/codesign --verify --deep --strict --verbose=2 "$app_path"
  /usr/sbin/spctl --assess --type execute --verbose=4 "$app_path"

  printf '{"type":"run","network":"%s","run":%d,"elapsedSeconds":%d,"bytes":%d,"sha256":"%s","version":"%s","bundleId":"%s","teamId":"%s","signatureValid":true,"notarizationValid":true}\n' \
    "$NETWORK_LABEL" "$run" "$elapsed_seconds" "$bytes" "$sha256" \
    "$version" "$bundle_id" "$team_id" >>"$EVIDENCE_PATH"

  /usr/bin/hdiutil detach "$mount_path" -quiet
  mounted_path=""
  /bin/rm -f "$dmg_path"
done

printf '{"type":"result","network":"%s","passed":true,"completedRuns":%d}\n' \
  "$NETWORK_LABEL" "$RUNS" >>"$EVIDENCE_PATH"
echo "Gate passed. Evidence: $EVIDENCE_PATH"
