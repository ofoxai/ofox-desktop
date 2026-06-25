#!/usr/bin/env bash
#
# release.sh — 把 Ofox Desktop 打包并发布到 Cloudflare R2。
#
# 流程：
#   1. 校验三处版本号一致（package.json / tauri.conf.json / Cargo.toml）
#   2. pnpm tauri build（默认 macOS aarch64；可 --skip-build 复用已有产物）
#   3. 把 .dmg 上传到 R2: release/mac/arm/ofox_desktop_<version>.dmg
#   4. 生成并上传 latest.json（检查更新读这个）
#
# 前置条件：
#   - wrangler 已登录（CLOUDFLARE_API_TOKEN 或 wrangler login）
#   - bucket: ofox-desktop-releases；自定义域: https://desktop.ofox.ai
#   - 详见 .claude/skills/tool-release/SKILL.md
#
# 用法：
#   scripts/release.sh                      # 打包 + 发布当前版本
#   scripts/release.sh --notes "修了 X"      # 带 release notes
#   scripts/release.sh --skip-build          # 复用已有 dmg，只重传 + 更新清单
#   scripts/release.sh --dry-run             # 只打印将要做什么，不上传
#
set -euo pipefail

# ── 常量 ───────────────────────────────────────────────────────────────
BUCKET="ofox-desktop-releases"
BASE_URL="https://desktop.ofox.ai"
DOWNLOAD_PAGE="https://ofox.ai/download"
# latest.json 里 downloads 的平台 key，对齐后端 misc.rs::manifest_platform_key()
PLATFORM_KEY="darwin-aarch64"
# R2 内的目标路径（对齐用户约定的下载地址结构）
R2_PREFIX="release/mac/arm"

# ── 解析参数 ───────────────────────────────────────────────────────────
NOTES=""
SKIP_BUILD=0
DRY_RUN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --notes)      NOTES="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --dry-run)    DRY_RUN=1; shift ;;
    *) echo "未知参数: $1" >&2; exit 1 ;;
  esac
done

cd "$(dirname "$0")/.."
ROOT="$(pwd)"

# ── 1. 校验版本号三处一致 ─────────────────────────────────────────────
VER_PKG=$(node -p "require('./package.json').version")
VER_TAURI=$(node -p "require('./src-tauri/tauri.conf.json').version")
VER_CARGO=$(grep -m1 '^version = ' src-tauri/Cargo.toml | sed -E 's/version = "(.*)"/\1/')

if [[ "$VER_PKG" != "$VER_TAURI" || "$VER_PKG" != "$VER_CARGO" ]]; then
  echo "❌ 版本号不一致：" >&2
  echo "   package.json   = $VER_PKG" >&2
  echo "   tauri.conf.json= $VER_TAURI" >&2
  echo "   Cargo.toml     = $VER_CARGO" >&2
  echo "请先统一三处版本号再发布。" >&2
  exit 1
fi
VERSION="$VER_PKG"
echo "▶ 发布版本: v$VERSION"

# ── 2. 打包 ───────────────────────────────────────────────────────────
if [[ "$SKIP_BUILD" -eq 0 ]]; then
  echo "▶ 打包 (pnpm tauri build --target aarch64-apple-darwin) ..."
  if [[ "$DRY_RUN" -eq 0 ]]; then
    pnpm tauri build --target aarch64-apple-darwin
  else
    echo "  [dry-run] 跳过实际 build"
  fi
else
  echo "▶ --skip-build：复用已有产物"
fi

# ── 3. 定位 dmg ───────────────────────────────────────────────────────
# tauri 产物：src-tauri/target/<triple>/release/bundle/dmg/Ofox Desktop_<ver>_aarch64.dmg
DMG_SRC=$(find src-tauri/target -path "*/release/bundle/dmg/*.dmg" -name "*${VERSION}*" 2>/dev/null | head -1)
if [[ -z "$DMG_SRC" && "$DRY_RUN" -eq 0 ]]; then
  echo "❌ 找不到 v$VERSION 的 .dmg 产物。先 build 或检查 target 目录。" >&2
  exit 1
fi
echo "▶ dmg 源文件: ${DMG_SRC:-(dry-run, 未定位)}"

# 目标文件名（用户约定）：ofox_desktop_<version>.dmg
DMG_KEY="${R2_PREFIX}/ofox_desktop_${VERSION}.dmg"
DMG_URL="${BASE_URL}/${DMG_KEY}"

# ── 4. 生成 latest.json ───────────────────────────────────────────────
PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
LATEST_JSON=$(cat <<EOF
{
  "version": "${VERSION}",
  "pubDate": "${PUB_DATE}",
  "notes": $(node -p "JSON.stringify(process.argv[1])" "$NOTES"),
  "downloads": {
    "${PLATFORM_KEY}": "${DMG_URL}"
  },
  "downloadPage": "${DOWNLOAD_PAGE}"
}
EOF
)
echo "▶ latest.json:"
echo "$LATEST_JSON" | sed 's/^/    /'

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "✅ [dry-run] 完成，未上传。"
  echo "   将上传 dmg → r2://${BUCKET}/${DMG_KEY}"
  echo "   将上传清单 → r2://${BUCKET}/latest.json"
  exit 0
fi

# ── 5. 上传 R2 ────────────────────────────────────────────────────────
TMP_JSON="$(mktemp -t ofox-latest-XXXX.json)"
trap 'rm -f "$TMP_JSON"' EXIT
printf '%s' "$LATEST_JSON" > "$TMP_JSON"

echo "▶ 上传 dmg → r2://${BUCKET}/${DMG_KEY}"
wrangler r2 object put "${BUCKET}/${DMG_KEY}" \
  --file="$DMG_SRC" \
  --content-type "application/x-apple-diskimage" \
  --remote

echo "▶ 上传 latest.json → r2://${BUCKET}/latest.json"
wrangler r2 object put "${BUCKET}/latest.json" \
  --file="$TMP_JSON" \
  --content-type "application/json" \
  --remote

echo ""
echo "✅ 发布完成 v$VERSION"
echo "   下载: $DMG_URL"
echo "   清单: ${BASE_URL}/latest.json"
echo "   验证: curl -s ${BASE_URL}/latest.json"
