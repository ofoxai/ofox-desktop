"""
简化的同步命令执行器
Popen + 逐行输出，用于 inline 步骤（不需要新终端窗口的步骤）
仅使用 Python 3.9 标准库
"""
from __future__ import annotations

import os
import subprocess
from typing import Callable, Optional

from app.shell import login_shell_argv


def run_shell_command(
    command: str,
    log_fn: Optional[Callable[[str], None]] = None,
    env_extra: Optional[dict[str, str]] = None,
) -> bool:
    """
    用 login shell 同步执行命令，逐行输出到 log_fn。
    返回 True 表示命令成功（exit code 0）。

    走 login shell 是必须的：应用从 Finder/Dock 启动时 PATH 只有系统目录，不含
    版本管理器装的 node/npm。原来这里手动往 PATH 里插
    `~/.nvm/versions/node/<最新>/bin`，只照顾了 nvm 用户 —— fnm / volta /
    Homebrew 用户的 `npm config set` 会静默失败，镜像永远配不上。login shell
    加载用户自己的 rc，几种装法全覆盖。详见 app/shell.py 与 fizzy #865。
    """
    env = os.environ.copy()
    if env_extra:
        env.update(env_extra)

    try:
        proc = subprocess.Popen(
            login_shell_argv(command),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            env=env,
            text=True,
            bufsize=1,
        )

        assert proc.stdout is not None
        for line in iter(proc.stdout.readline, ""):
            stripped = line.rstrip("\n")
            if log_fn is not None:
                log_fn(stripped)

        proc.stdout.close()
        rc = proc.wait()
        return rc == 0

    except Exception as e:
        if log_fn is not None:
            log_fn(f"[错误] {e}")
        return False
