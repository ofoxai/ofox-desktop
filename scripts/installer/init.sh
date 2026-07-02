#!/bin/bash
# Ofox Desktop / cc-switch macOS 工具安装器 — bash 引导入口
# 用法: bash init.sh [--tool TOOL_ID] [--skip-env] [--no-onboard]
#
# 白板 Mac 上 /usr/bin/python3 是 Apple CLT shim，
# 在未安装 Xcode CLT 时无法执行任何 Python 脚本。
# 本脚本先用纯 bash 处理 CLT 安装，再交接给 init.py。
# 公共环境层（nvm / Node / 镜像）和 6 个 AI 工具（claude / codex /
# gemini / opencode / openclaw / hermes）的具体安装由 init.py +
# app/steps.py 负责。

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Step 0: 架构检查
if [ "$(uname -m)" != "arm64" ]; then
    echo ""
    echo "  错误: 当前 CPU 架构为 $(uname -m)，仅支持 Apple Silicon (arm64) Mac。"
    echo ""
    exit 1
fi

# Step 1: 确保 Xcode Command Line Tools 已安装
if ! xcode-select -p &>/dev/null; then
    echo ""
    echo "══════════════════════════════════════"
    echo "  预备步骤：安装 Xcode Command Line Tools"
    echo "══════════════════════════════════════"
    echo ""
    echo "  正在触发安装，请在弹出的对话框中点击「安装」..."
    echo ""
    xcode-select --install 2>/dev/null

    # 轮询等待 CLT 安装完成
    elapsed=0
    while ! xcode-select -p &>/dev/null; do
        sleep 5
        elapsed=$((elapsed + 5))
        mins=$((elapsed / 60))
        secs=$((elapsed % 60))
        printf "\r  等待安装中... %02d:%02d " "$mins" "$secs"
    done
    echo ""
    echo "  ✓ Xcode Command Line Tools 安装完成"
    echo ""
fi

# 现在 python3 可用，交接给 Python 主程序
exec /usr/bin/python3 "$SCRIPT_DIR/init.py" "$@"
