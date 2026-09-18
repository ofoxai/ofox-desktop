"""
通过 osascript 控制 Terminal.app 在新窗口中执行命令
仅使用 Python 3.9 标准库
"""
from __future__ import annotations

import os
import subprocess
import tempfile
from typing import Optional


class TerminalHandle:
    """open_terminal_with_command 的返回结果，包含窗口 id 和完成标记文件路径。"""

    __slots__ = ("window_id", "done_file")

    def __init__(self, window_id: int, done_file: str) -> None:
        self.window_id = window_id
        self.done_file = done_file

    def is_script_done(self) -> bool:
        """脚本是否已执行完毕（标记文件存在即完毕）。"""
        return os.path.isfile(self.done_file)

    def cleanup(self) -> None:
        """清理标记文件。"""
        try:
            os.remove(self.done_file)
        except OSError:
            pass


def _script_prefix(title: str) -> str:
    """
    临时脚本的文件名前缀，带上步骤名便于在 /tmp 里追溯。

    以前这里硬编码 `openclaw-install-`（这个安装器源自 openclaw-launcher），用户点
    Gemini 却看到 `openclaw-install-xxx.sh`，排查时极其误导。

    步骤名要过滤：可能是中文（"国内镜像配置"），也不能让 `/`、`..` 之类进文件名。
    过滤后为空就只用通用前缀。
    """
    slug = "".join(
        c for c in title.lower() if c.isascii() and (c.isalnum() or c == "-")
    ).strip("-")
    return f"ofox-install-{slug}-" if slug else "ofox-install-"


#: 命令前垫的空白。Terminal 的 `do script` 是"键入文本"，如果新窗口的 shell 正在
#: rc 里等输入（oh-my-zsh 的更新提示用 `read -k 1`），键入的第一个字符会被它吃掉。
#: 垫上空格后，被吃的是空白而不是路径的 `/`；没被吃时 shell 也会忽略前导空格。
_COMMAND_PAD = "   "

#: 等 shell 跑完 rc 的轮询上限（× 0.2s）。提示可能一直等用户输入，`busy` 就一直是
#: true，所以必须有上限 —— 超时后仍然发命令，靠 _COMMAND_PAD 兜底。
_BUSY_WAIT_TICKS = 25


def _build_launch_applescript(script_path: str) -> str:
    """
    生成"在新 Terminal 窗口里执行脚本"的 AppleScript。

    抽成纯函数是为了可测：真实的弹窗行为没法单测，但"有没有加保险"可以。
    见 fizzy #873 与 tests/test_terminal.py 的说明。

    两层保险：
      1. 先 `do script ""` 开空窗口，轮询 `busy` 等 shell 把 rc 跑完，再发真正的
         命令。这样 oh-my-zsh 那类提示已经结束，不会来抢输入。
      2. 命令前垫 `_COMMAND_PAD`。万一提示还在（比如它一直等用户按键），被吃掉的
         是空格，路径依然完整。

    命令发给 `targetTab` 而不是 `front window`：用户在等待期间可能切到别的窗口。
    """
    return f'''
tell application "Terminal"
    activate
    set targetTab to do script ""
    set waited to 0
    repeat while busy of targetTab and waited < {_BUSY_WAIT_TICKS}
        delay 0.2
        set waited to waited + 1
    end repeat
    do script "{_COMMAND_PAD}{script_path}" in targetTab
    return id of front window
end tell
'''


def open_terminal_with_command(title: str, command: str) -> Optional[TerminalHandle]:
    """
    将命令写入临时脚本，通过 osascript 在 Terminal.app 新窗口中执行。

    脚本执行完成后会自动删除自身，并写入完成标记文件。
    返回 TerminalHandle（包含窗口 id 和标记文件），失败返回 None。
    """
    # 写入临时脚本，避免 AppleScript 转义问题
    fd, script_path = tempfile.mkstemp(
        prefix=_script_prefix(title),
        suffix=".sh",
        dir="/tmp",
    )

    # 脚本完成标记文件
    done_file = script_path + ".done"

    script_content = f"""#!/bin/bash
# Ofox Desktop 安装脚本: {title}
# 此文件由 Ofox Desktop 的安装器自动生成，执行完成后会自动删除

echo ""
echo "══════════════════════════════════════════════"
echo "  {title}"
echo "══════════════════════════════════════════════"
echo ""

{command}

EXIT_CODE=$?

echo ""
if [ $EXIT_CODE -eq 0 ]; then
    echo "✓ {title} — 完成"
else
    echo "✕ {title} — 失败 (退出码: $EXIT_CODE)"
fi

# 写入完成标记
touch "{done_file}"

# 清理临时脚本
rm -f "{script_path}"
"""

    with os.fdopen(fd, "w") as f:
        f.write(script_content)

    os.chmod(script_path, 0o755)

    result = subprocess.run(
        ["osascript", "-e", _build_launch_applescript(script_path)],
        capture_output=True,
        text=True,
        # 比脚本里的等待上限（约 5s）留足余量
        timeout=20,
    )

    if result.returncode == 0:
        try:
            wid = int(result.stdout.strip())
            return TerminalHandle(wid, done_file)
        except (ValueError, TypeError):
            return None
    return None


def close_terminal_window(window_id: int) -> bool:
    """
    通过 osascript 关闭指定 id 的 Terminal.app 窗口。
    窗口已被用户提前关闭也不会报错。
    """
    applescript = f'''
tell application "Terminal"
    try
        close (every window whose id is {window_id}) saving no
    end try
end tell
'''
    result = subprocess.run(
        ["osascript", "-e", applescript],
        capture_output=True,
        timeout=10,
    )
    return result.returncode == 0
