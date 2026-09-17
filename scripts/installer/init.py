#!/usr/bin/env python3
"""
Ofox Desktop / cc-switch macOS 工具安装器 — 纯终端版
用法: bash init.sh [--tool TOOL_ID] [--skip-env] [--no-onboard]
  (由 init.sh 引导启动，不要直接执行本文件)

仅使用 Python 3.9 标准库，零外部依赖。

调用契约（供 Rust 端 install_tool command 用）：
  --tool TOOL_ID      只装单个工具：claude/codex/gemini/opencode/openclaw/hermes
  --skip-env          跳过 nvm/Node/镜像，假设公共环境已就绪
  --no-onboard        不启 onboard-server（cc-switch 不需要这个后端）

退出码：
  0    成功
  1    安装失败或用户中断
  2    参数错误（未知 tool）
  130  SIGINT（KeyboardInterrupt）

Xcode CLT 安装由 init.sh (bash) 处理，本文件处理剩余步骤。
"""
from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import urllib.request

# 确保能导入 app 包
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from app.display import (
    print_banner,
    print_region,
    print_step_start,
    print_checking,
    print_already_installed,
    print_skipped,
    print_waiting,
    print_poll_tick,
    print_poll_done,
    print_inline_log,
    print_step_done,
    print_failure_hint,
    ask_continue,
    print_summary,
    print_error,
    print_info,
)
from app.terminal import open_terminal_with_command, close_terminal_window, TerminalHandle
from app.monitor import wait_for_condition
from app.progress import emit_progress, make_poll_callback
from app.steps import Step, set_region, get_env_steps, get_tool_step, TOOL_STEPS, VerifyStep

# 与 openclaw-launcher 的 ~/.openclaw-init-state.json 隔离——同一台机器上
# 两个安装器可共存，各自独立断点续装。
STATE_FILE = os.path.expanduser("~/.cc-switch-install-state.json")

# 全装模式下默认装的工具（不含 hermes，hermes 走按需追加）。
DEFAULT_INSTALL_TOOLS = ["claude", "codex", "gemini", "opencode", "openclaw"]


# ── 环境预检 ─────────────────────────────────────────────────────────────────

def preflight_check() -> bool:
    """环境预检：架构、磁盘空间"""
    # 架构检查
    arch = platform.machine()
    if arch != "arm64":
        print_error(f"当前 CPU 架构为 {arch}，本工具仅支持 Apple Silicon (arm64) Mac。")
        return False

    # 磁盘空间检查（至少 5GB）
    try:
        usage = shutil.disk_usage("/")
        avail_gb = usage.free / (1024 ** 3)
        if avail_gb < 5:
            print_error(f"磁盘可用空间不足（{avail_gb:.1f}GB），至少需要 5GB。")
            return False
    except OSError:
        pass  # 无法检查则跳过

    return True


# ── 地理位置检测 ─────────────────────────────────────────────────────────────

def detect_region() -> str:
    """
    通过 ip-api.com 检测地理位置。
    CN → 国内镜像, INTL → 官方源, 超时 → 默认 CN
    """
    try:
        req = urllib.request.Request(
            "http://ip-api.com/json",
            headers={"User-Agent": "openclaw-init/1.0"},
        )
        resp = urllib.request.urlopen(req, timeout=5)
        data = json.loads(resp.read().decode("utf-8"))
        country_code = data.get("countryCode", "")
        if country_code == "CN":
            return "CN"
        elif country_code:
            return "INTL"
    except Exception:
        pass

    return "CN"  # 默认国内


# ── 进度持久化 ───────────────────────────────────────────────────────────────

def load_state() -> dict[str, str]:
    """从文件恢复进度"""
    if not os.path.isfile(STATE_FILE):
        return {}
    try:
        with open(STATE_FILE) as f:
            return json.load(f)
    except (OSError, json.JSONDecodeError):
        return {}


def save_state(results: dict[str, str]) -> None:
    """保存进度到文件"""
    try:
        with open(STATE_FILE, "w") as f:
            json.dump(results, f)
    except OSError:
        pass


def clear_state() -> None:
    """清除状态文件"""
    try:
        os.remove(STATE_FILE)
    except OSError:
        pass


# ── onboard-server 启动 ──────────────────────────────────────────────────────

NVM_PREFIX = 'export NVM_DIR="$HOME/.nvm" && [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh" && '


def start_onboard_server() -> bool:
    """调用 onboard-server.mjs --daemon 启动守护进程并打开浏览器"""
    script_dir = os.path.dirname(os.path.abspath(__file__))
    project_dir = os.path.dirname(script_dir)
    server_script = os.path.join(project_dir, "onboard-server.mjs")

    cmd = NVM_PREFIX + f'node "{server_script}" --daemon'
    result = subprocess.run(
        cmd,
        shell=True,
        executable="/bin/bash",
        timeout=30,
    )
    return result.returncode == 0


# ── 参数解析 ─────────────────────────────────────────────────────────────────


def parse_args() -> argparse.Namespace:
    """CLI 参数——主要供 cc-switch Tauri 后端按需调用。"""
    p = argparse.ArgumentParser(
        prog="cc-switch-installer",
        description="cc-switch macOS 工具安装器",
    )
    p.add_argument(
        "--tool",
        choices=list(TOOL_STEPS.keys()),
        help="只装单个工具；不传则进入全装模式",
    )
    p.add_argument(
        "--skip-env",
        action="store_true",
        help="跳过公共环境（nvm/Node/镜像），假设已就绪",
    )
    p.add_argument(
        "--no-onboard",
        action="store_true",
        help="不启 onboard-server（cc-switch 不需要这个后端）",
    )
    return p.parse_args()


# ── 主流程 ───────────────────────────────────────────────────────────────────

def main() -> None:
    args = parse_args()
    print_banner()

    # 1. 环境预检
    if not preflight_check():
        sys.exit(1)

    # 2. 地理位置检测
    print_info("正在检测网络区域...")
    region = detect_region()
    set_region(region)
    print_region(region)

    # 3. 按 args 动态拼步骤序列：
    #    - 不 --skip-env  →  公共环境层（nvm/Node/镜像）
    #    - --tool TOOL_ID →  只加这一个工具
    #    - 否则全装       →  DEFAULT_INSTALL_TOOLS + VerifyStep
    steps: list[Step] = []
    if not args.skip_env:
        steps.extend(get_env_steps())

    if args.tool:
        tool = get_tool_step(args.tool)
        if tool is None:
            # 理论上 argparse choices 已经拦了，这是兜底。
            print_error(f"未知工具: {args.tool}")
            sys.exit(2)
        steps.append(tool)
    else:
        for tid in DEFAULT_INSTALL_TOOLS:
            tool = get_tool_step(tid)
            if tool is not None:
                steps.append(tool)
        steps.append(VerifyStep())

    total = len(steps)

    # 4. 恢复已保存进度
    saved = load_state()
    state: dict[str, str] = {}  # 当前运行状态

    # 5. 逐步执行
    summary: list[tuple[str, bool, bool]] = []  # (name, success, skipped)

    for i, step in enumerate(steps):
        step_key = str(i)
        # 结构化进度：每步开始就发一条，前端据此把卡片文案从笼统的"安装中…"
        # 换成具体步骤名。人类可读输出仍走 display 的 print_*，两条通路互不干扰。
        emit_progress(step=i + 1, total=total, name=step.name, phase="start")

        # 检查是否应该跳过
        if step.should_skip():
            print_step_start(i + 1, total, step.name, step.description)
            reason = "海外用户无需配置" if step.name == "国内镜像配置" else ""
            print_skipped(reason)
            emit_progress(step=i + 1, total=total, name=step.name, phase="skipped")
            state[step_key] = "SKIPPED"
            save_state(state)
            summary.append((step.name, True, True))
            continue

        # 检查是否已完成（断点续装）
        if saved.get(step_key) == "SUCCESS":
            # 再次确认工具确实存在
            if step.check():
                print_step_start(i + 1, total, step.name, step.description)
                print_already_installed()
                emit_progress(
                    step=i + 1, total=total, name=step.name, phase="skipped"
                )
                state[step_key] = "SUCCESS"
                summary.append((step.name, True, True))
                continue
            # 工具不存在了，重新安装
            pass

        print_step_start(i + 1, total, step.name, step.description)

        # 检测是否已安装
        print_checking(step.name)
        if step.check():
            print_already_installed()
            emit_progress(step=i + 1, total=total, name=step.name, phase="skipped")
            state[step_key] = "SUCCESS"
            save_state(state)
            summary.append((step.name, True, True))
            continue
        else:
            # 清除检测行
            print(f"\r       {' ' * 40}\r", end="", flush=True)

        # 执行安装
        success = False
        if step.needs_terminal:
            # 在新终端窗口中执行
            cmd = step.terminal_command()
            if cmd:
                handle = open_terminal_with_command(step.name, cmd)
                if handle is None:
                    print_failure_hint("无法打开终端窗口，请检查 Terminal.app 权限。")
                    if not ask_continue():
                        save_state(state)
                        print_info("已保存进度，下次运行将从断点继续。")
                        sys.exit(1)
                    summary.append((step.name, False, False))
                    continue
                print_waiting(step.name)

                # 轮询等待安装完成
                ok = wait_for_condition(
                    condition_fn=step.check,
                    timeout=step.timeout,
                    interval=step.poll_interval,
                    progress_callback=make_poll_callback(
                        i + 1, total, step.name, tick_fn=print_poll_tick
                    ),
                )
                print_poll_done()

                if ok:
                    success = step.verify()
                else:
                    success = False

                # 等待终端脚本真正执行完毕（标记文件出现），再关闭窗口
                wait_for_condition(
                    condition_fn=handle.is_script_done,
                    timeout=300,  # 5 分钟，确保慢速安装也能完成
                    interval=1.0,
                )
                close_terminal_window(handle.window_id)
                handle.cleanup()
        else:
            # 在主进程中直接执行
            success = step.run_inline(log_fn=print_inline_log)
            if success:
                success = step.verify()

        # 显示结果
        print_step_done(step.name, success)
        emit_progress(
            step=i + 1,
            total=total,
            name=step.name,
            phase="done" if success else "failed",
        )

        if success:
            state[step_key] = "SUCCESS"
        else:
            state[step_key] = "FAILED"
            print_failure_hint(step.failure_hint())

            if not ask_continue():
                save_state(state)
                print_info("已保存进度，下次运行将从断点继续。")
                sys.exit(1)

        save_state(state)
        summary.append((step.name, success, False))

    # 6. 全部完成
    all_ok = all(s or k for _, s, k in summary)
    if all_ok:
        clear_state()
        # onboard-server 是 openclaw-launcher 特有的配置引导服务，cc-switch
        # 走自己的 GUI 流程，所以 Tauri 后端会传 --no-onboard。保留兼容路径
        # 让脚本仍能被 openclaw-launcher 那条链路直接调用。
        if not args.no_onboard:
            print_info("正在启动配置引导...")
            if not start_onboard_server():
                print_error("onboard 服务启动失败，请手动运行: node onboard-server.mjs --daemon")

    print_summary(summary)

    # 仅在交互 tty 下等用户确认——Tauri 起子进程没 tty，input() 会立刻 EOF
    # 抛 EOFError；用 isatty 守门免去 try/except。
    if sys.stdin.isatty():
        print("")
        input("  按 Enter 键退出...")

    sys.exit(0 if all_ok else 1)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("\n\n  中断。已保存进度，下次运行将从断点继续。")
        sys.exit(130)
