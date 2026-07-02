"""
简化的同步命令执行器
Popen + 逐行输出，用于 inline 步骤（不需要新终端窗口的步骤）
仅使用 Python 3.9 标准库
"""
from __future__ import annotations

import os
import subprocess
from typing import Callable, Optional


def run_shell_command(
    command: str,
    log_fn: Optional[Callable[[str], None]] = None,
    env_extra: Optional[dict[str, str]] = None,
) -> bool:
    """
    同步执行 shell 命令，逐行输出到 log_fn。
    自动注入 nvm 到 PATH。
    返回 True 表示命令成功（exit code 0）。
    """
    env = os.environ.copy()

    # 确保 nvm 安装的 node 在 PATH 中
    nvm_default = os.path.expanduser("~/.nvm/versions/node")
    if os.path.isdir(nvm_default):
        # 找到最新的 node 版本目录
        try:
            versions = sorted(os.listdir(nvm_default), reverse=True)
            if versions:
                node_bin = os.path.join(nvm_default, versions[0], "bin")
                if node_bin not in env.get("PATH", ""):
                    env["PATH"] = f"{node_bin}:{env.get('PATH', '')}"
        except OSError:
            pass

    if env_extra:
        env.update(env_extra)

    try:
        proc = subprocess.Popen(
            command,
            shell=True,
            executable="/bin/bash",
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
