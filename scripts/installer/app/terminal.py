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


def open_terminal_with_command(title: str, command: str) -> Optional[TerminalHandle]:
    """
    将命令写入临时脚本，通过 osascript 在 Terminal.app 新窗口中执行。

    脚本执行完成后会自动删除自身，并写入完成标记文件。
    返回 TerminalHandle（包含窗口 id 和标记文件），失败返回 None。
    """
    # 写入临时脚本，避免 AppleScript 转义问题
    fd, script_path = tempfile.mkstemp(
        prefix="openclaw-install-",
        suffix=".sh",
        dir="/tmp",
    )

    # 脚本完成标记文件
    done_file = script_path + ".done"

    script_content = f"""#!/bin/bash
# OpenClaw 安装脚本: {title}
# 此文件由 OpenClaw 初始化工具自动生成，执行完成后会自动删除

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

    # 通过 osascript 在 Terminal.app 新窗口中执行，并获取窗口 id
    applescript = f'''
tell application "Terminal"
    activate
    do script "{script_path}"
    return id of front window
end tell
'''
    result = subprocess.run(
        ["osascript", "-e", applescript],
        capture_output=True,
        text=True,
        timeout=10,
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
