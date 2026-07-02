"""
终端 ANSI 彩色输出 + 动态进度条
仅使用 Python 3.9 标准库
"""
from __future__ import annotations

import sys
import shutil

# ── ANSI 颜色码 ─────────────────────────────────────────────────────────────

RESET = "\033[0m"
BOLD = "\033[1m"
DIM = "\033[2m"

RED = "\033[31m"
GREEN = "\033[32m"
YELLOW = "\033[33m"
BLUE = "\033[34m"
MAGENTA = "\033[35m"
CYAN = "\033[36m"
WHITE = "\033[37m"

BG_GREEN = "\033[42m"
BG_RED = "\033[41m"
BG_BLUE = "\033[44m"

# 状态图标
ICON_PENDING = f"{DIM}○{RESET}"
ICON_RUNNING = f"{CYAN}◉{RESET}"
ICON_SUCCESS = f"{GREEN}✓{RESET}"
ICON_FAILED = f"{RED}✕{RESET}"
ICON_SKIPPED = f"{DIM}–{RESET}"

# ── 终端宽度 ─────────────────────────────────────────────────────────────────

def _term_width() -> int:
    try:
        return shutil.get_terminal_size().columns
    except Exception:
        return 80


# ── 公共函数 ─────────────────────────────────────────────────────────────────

def print_banner() -> None:
    w = min(_term_width(), 60)
    line = "═" * w
    print()
    print(f"{CYAN}{BOLD}{line}{RESET}")
    print(f"{CYAN}{BOLD}  OpenClaw macOS 环境初始化{RESET}")
    print(f"{CYAN}{BOLD}{line}{RESET}")
    print()


def print_region(region: str) -> None:
    if region == "CN":
        label = "中国大陆（使用国内镜像）"
    else:
        label = "海外（使用官方源）"
    print(f"  {BLUE}区域{RESET}: {label}")
    print()


def print_step_start(num: int, total: int, name: str, description: str) -> None:
    print()
    print(f"  {BOLD}[{num}/{total}]{RESET} {CYAN}{name}{RESET}")
    print(f"       {DIM}{description}{RESET}")


def print_checking(name: str) -> None:
    print(f"       {DIM}检测 {name} ...{RESET}", end="", flush=True)


def print_already_installed() -> None:
    print(f"\r       {GREEN}已安装，跳过{RESET}          ")


def print_skipped(reason: str = "") -> None:
    msg = f"跳过" + (f"（{reason}）" if reason else "")
    print(f"       {DIM}{msg}{RESET}")


def print_waiting(name: str) -> None:
    print(f"       {YELLOW}已在新终端窗口中启动 {name} 安装{RESET}")
    print(f"       {DIM}请在新窗口中完成操作（如需输入密码等）{RESET}")


def print_poll_tick(elapsed: int, timeout: int) -> None:
    """动态进度条，用 \\r 覆写当前行"""
    bar_width = 30
    ratio = min(elapsed / timeout, 1.0) if timeout > 0 else 0
    filled = int(bar_width * ratio)
    bar = "=" * filled + ">" + " " * (bar_width - filled - 1)

    minutes = elapsed // 60
    seconds = elapsed % 60
    time_str = f"{minutes:02d}:{seconds:02d}"

    line = f"       等待中 [{bar}] {time_str}"
    # 清除到行尾
    print(f"\r{line}\033[K", end="", flush=True)


def print_poll_done() -> None:
    """结束进度条行"""
    print()


def print_inline_log(line: str) -> None:
    """打印 inline 步骤的日志行"""
    print(f"       {DIM}│{RESET} {line}")


def print_step_done(name: str, success: bool) -> None:
    if success:
        print(f"       {GREEN}{BOLD}✓ {name} 完成{RESET}")
    else:
        print(f"       {RED}{BOLD}✕ {name} 失败{RESET}")


def print_failure_hint(hint: str) -> None:
    print(f"       {YELLOW}提示: {hint}{RESET}")


def ask_continue() -> bool:
    """失败时询问是否继续，返回 True 表示继续"""
    try:
        print()
        answer = input(f"       {YELLOW}是否继续执行后续步骤？[Y/n]{RESET} ").strip().lower()
        return answer in ("", "y", "yes")
    except (EOFError, KeyboardInterrupt):
        return False


def print_summary(results: list[tuple[str, bool, bool]]) -> None:
    """
    打印最终汇总
    results: [(name, success, skipped), ...]
    """
    w = min(_term_width(), 60)
    line = "═" * w

    print()
    print(f"  {CYAN}{BOLD}{line}{RESET}")
    print(f"  {CYAN}{BOLD}  安装结果汇总{RESET}")
    print(f"  {CYAN}{BOLD}{line}{RESET}")

    for name, success, skipped in results:
        if skipped:
            icon = ICON_SKIPPED
            label = "跳过"
        elif success:
            icon = ICON_SUCCESS
            label = "成功"
        else:
            icon = ICON_FAILED
            label = "失败"
        print(f"    {icon} {name:<20s} {label}")

    all_ok = all(s or k for _, s, k in results)
    print()
    if all_ok:
        print(f"  {GREEN}{BOLD}环境初始化完成！请打开新终端窗口以使配置生效。{RESET}")
    else:
        print(f"  {YELLOW}{BOLD}部分步骤失败，请根据提示修复后重新运行。{RESET}")
    print()


def print_error(msg: str) -> None:
    print(f"  {RED}{BOLD}错误: {msg}{RESET}", file=sys.stderr)


def print_info(msg: str) -> None:
    print(f"  {BLUE}ℹ {msg}{RESET}")
