#!/usr/bin/env bash
#
# release.sh — 把 Ofox Desktop 打包并发布到 Cloudflare R2。
#
# ⚠️ 常规发版请走 CI，此脚本仅供应急。
#
#   常规路径：打 tag 触发 .github/workflows/release.yml
#     git tag <version>_$(date +%Y%m%d_%H%M) && git push origin --tags
#
#   CI 相比本脚本的额外保障：凭据存 GitHub Environment 而非本机环境变量、
#   tag 前缀与版本号交叉校验、latest.json 按平台合并（不会覆盖掉其他平台的
#   downloads key）。本脚本直接覆写 latest.json —— 将来 Windows/Linux 上线后
#   用它发版会抹掉其他平台的条目，届时务必只用 CI。
#
#   应急场景（CI 不可用 / GitHub 挂了 / 需要立刻出包）才用这个脚本。用完记得
#   确认线上 latest.json 与预期一致：curl -s https://desktop.ofox.ai/latest.json
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

# ── 代码签名 / 公证 ───────────────────────────────────────────────────
# 全部凭据从环境变量读，不在仓库里硬编码任何身份信息。
# 未设置时自动从钥匙串解析唯一的 Developer ID Application 身份。
if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  APPLE_SIGNING_IDENTITY=$(security find-identity -v -p codesigning \
    | grep "Developer ID Application" | grep -oE '"[^"]+"' | head -1 | tr -d '"')
  [[ -z "$APPLE_SIGNING_IDENTITY" ]] && {
    echo "❌ 钥匙串里找不到 Developer ID Application 身份，也未设置 APPLE_SIGNING_IDENTITY" >&2
    exit 1
  }
fi
# 公证用 app-specific password：兼容本机历史变量名 APPLE_APP_SPECIFIC_PASSWORD。
# Tauri 期望 APPLE_PASSWORD；我们把它对齐过去。
APPLE_PASSWORD="${APPLE_PASSWORD:-${APPLE_APP_SPECIFIC_PASSWORD:-}}"
export APPLE_SIGNING_IDENTITY
export APPLE_PASSWORD
# APPLE_ID / APPLE_TEAM_ID 需已在环境变量里（公证时校验，缺了会报错）。

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

# ── 2. 打包（本机 host 即 aarch64-apple-darwin，不带 --target）─────────
# Tauri 在 build 期间会用上面 export 的 APPLE_SIGNING_IDENTITY 签名，并用
# APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID 公证 + staple 内含的 .app。
# 注意：Tauri **不会**对 dmg 容器本身 staple，见第 3.5 步。
if [[ "$SKIP_BUILD" -eq 0 ]]; then
  echo "▶ 打包 (pnpm tauri build，含签名+公证 .app) ..."
  if [[ "$DRY_RUN" -eq 0 ]]; then
    pnpm tauri build
  else
    echo "  [dry-run] 跳过实际 build"
  fi
else
  echo "▶ --skip-build：复用已有产物"
fi

# ── 3. 定位 dmg ───────────────────────────────────────────────────────
# tauri 产物：src-tauri/target/release/bundle/dmg/Ofox Desktop_<ver>_aarch64.dmg
DMG_SRC=$(find src-tauri/target -path "*/release/bundle/dmg/*.dmg" -name "*${VERSION}*" 2>/dev/null | head -1)
if [[ -z "$DMG_SRC" && "$DRY_RUN" -eq 0 ]]; then
  echo "❌ 找不到 v$VERSION 的 .dmg 产物。先 build 或检查 target 目录。" >&2
  exit 1
fi
echo "▶ dmg 源文件: ${DMG_SRC:-(dry-run, 未定位)}"

# ── 3.5 公证 + staple dmg 容器 ────────────────────────────────────────
# 关键坑：Tauri 只公证并 staple 了 dmg **内含的 .app**，但 dmg 文件本身没有
# staple——用户下载 dmg 双击挂载时 Gatekeeper 仍判 Unnotarized。这里对 dmg
# 容器单独走 notarytool 公证 + stapler staple。dmg 内 app 已公证，这步很快。
# 用 stapler validate 判断是否已 staple，幂等：重跑 release 不会重复公证。
if [[ "$DRY_RUN" -eq 0 ]]; then
  if xcrun stapler validate "$DMG_SRC" >/dev/null 2>&1; then
    echo "▶ dmg 已 staple，跳过公证"
  else
    : "${APPLE_ID:?APPLE_ID 未设置（公证需要）}"
    : "${APPLE_TEAM_ID:?APPLE_TEAM_ID 未设置（公证需要）}"
    : "${APPLE_PASSWORD:?APPLE_PASSWORD/APPLE_APP_SPECIFIC_PASSWORD 未设置（公证需要）}"
    echo "▶ 公证 dmg 容器 (notarytool submit --wait) ..."
    xcrun notarytool submit "$DMG_SRC" \
      --apple-id "$APPLE_ID" \
      --password "$APPLE_PASSWORD" \
      --team-id "$APPLE_TEAM_ID" \
      --wait
    echo "▶ staple 票据到 dmg ..."
    xcrun stapler staple "$DMG_SRC"
  fi
  echo "▶ 验证 Gatekeeper ..."
  spctl -a -vvv -t open --context context:primary-signature "$DMG_SRC" 2>&1 | sed 's/^/    /'
fi

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
