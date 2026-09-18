"""
检测命令的构造：统一走 login shell

为什么需要这个模块：应用从 Finder/Dock 启动时 PATH 只有
`/usr/bin:/bin:/usr/sbin:/sbin`，不含 nvm / fnm / volta / Homebrew 往用户 shell rc
里加的那些路径。而 `install_tool.rs` 用 `Command::new("/bin/bash")` 启动安装器时
没有重设环境，安装器继承的就是这个贫瘠的 PATH —— 拿它裸查 `command -v codex`
必然找不到，已装的工具被判成未装、反复重装（fizzy #865）。

原来的做法是给命令拼一段 `NVM_PREFIX`（`. nvm.sh` 补 PATH），有两个问题：
  1. 只照顾 nvm 用户，fnm / volta / Homebrew 用户全部失效
  2. `[ -s "$NVM_DIR/nvm.sh" ] && ...` 这个 && 链在没有 nvm 时短路，连后面真正的
     检测命令都不会执行，返回码直接是失败

改用 login shell 一次解决：它加载用户自己的 rc，谁装的 node 都能找到。
实测（PATH 只留系统目录）：裸 sh 查不到 codex，login shell 查得到。

仅使用 Python 3.9 标准库。
"""
from __future__ import annotations

import os
from typing import List, Optional

try:
    import pwd  # POSIX 独有；Windows 上没有这个模块
except ImportError:  # pragma: no cover - 留给后续 Windows 适配
    pwd = None  # type: ignore[assignment]

#: 兜底 shell。macOS 从 Catalina 起默认 zsh，但这里选 sh —— 我们只需要一个能执行
#: `-l -c` 的 POSIX shell，而 /bin/sh 一定存在。
_FALLBACK_SHELL = "/bin/sh"


def resolve_login_shell(shell: Optional[str] = None) -> str:
    """
    决定用哪个 shell 跑检测命令。

    优先级：显式参数 > `$SHELL` > 用户账户记录里的 shell > `/bin/sh`。

    GUI 启动的进程不保证有 `$SHELL`（launchd 不一定注入），所以后两级兜底不能省，
    否则拼不出命令。
    """
    if shell:
        return shell

    env_shell = os.environ.get("SHELL")
    if env_shell:
        return env_shell

    if pwd is not None:
        try:
            account_shell = pwd.getpwuid(os.getuid()).pw_shell
            if account_shell:
                return account_shell
        except KeyError:
            # 容器里 uid 可能没有对应的 passwd 记录
            pass

    return _FALLBACK_SHELL


def login_shell_argv(command: str, shell: Optional[str] = None) -> List[str]:
    """
    把一条 shell 命令包成"用 login shell 执行"的 argv。

    用 `-l`（login）而不是 `-i`（interactive）：login 会加载用户的 profile/rc，
    这正是我们要的 —— PATH 里才会有版本管理器装的东西。而 `-i` 会额外加载交互式
    配置，可能读 stdin 或打印提示符，在子进程里容易挂住。
    （`launch_tool.rs` 用 `-l -i` 是另一回事：它真的要给用户一个可交互的终端。）

    返回 argv 列表而不是拼好的字符串，这样调用方可以 `shell=False` 执行，命令整条
    作为一个参数传给 `-c`，不会被二次解析。
    """
    return [resolve_login_shell(shell), "-l", "-c", command]
