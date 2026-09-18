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
import subprocess
from typing import Dict, List, Optional

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


#: `login_shell_path()` 的进程内缓存。interactive shell 启动约 800ms，是非交互的
#: 34 倍（实测），所以这个开销只能付一次，绝不能乘以工具数。
_LOGIN_PATH_CACHE: Optional[str] = None


def login_shell_path(shell: Optional[str] = None) -> str:
    """
    取用户 login + **interactive** shell 的完整 PATH，进程内缓存。

    为什么非得 interactive：zsh 的 `-l` 只读 `.zprofile` / `.zlogin`，而 `.zshrc`
    只在 interactive 时加载 —— pnpm 的 `PNPM_HOME` 这类 PATH 通常就配在 `.zshrc`
    里。实测 `~/.local/share/pnpm/opencode` 用 `-l -c` 找不到、`-l -i -c` 找得到，
    于是已装的 opencode 被判成未装。

    为什么缓存：interactive shell 要加载完整 rc（插件、补全…），实测每次约 800ms；
    6 个工具各起一次就是 4.8s，用户打开"添加工具"得干等。取一次 PATH 之后，
    所有检测都用它，总代价固定。

    拿不到就回退到当前进程的 PATH —— 检测不准胜过整个流程起不来。
    """
    global _LOGIN_PATH_CACHE
    if _LOGIN_PATH_CACHE is not None:
        return _LOGIN_PATH_CACHE

    captured = ""
    try:
        result = subprocess.run(
            [resolve_login_shell(shell), "-l", "-i", "-c", 'printf %s "$PATH"'],
            capture_output=True,
            text=True,
            # interactive shell 可能读 stdin；给它 /dev/null 才不会挂住
            stdin=subprocess.DEVNULL,
            timeout=20,
        )
        captured = result.stdout.strip()
    except (OSError, subprocess.SubprocessError):
        # 超时 / shell 起不来 / rc 里有交互式阻塞 —— 都退回当前 PATH
        pass

    _LOGIN_PATH_CACHE = captured or os.environ.get("PATH", "")
    return _LOGIN_PATH_CACHE


def login_shell_env(shell: Optional[str] = None) -> Dict[str, str]:
    """当前环境，但 PATH 换成用户 login shell 的完整 PATH。

    配 [`login_shell_argv`]（非交互，约 23ms）一起用：shell 启动快，PATH 又是全的。
    """
    env = os.environ.copy()
    env["PATH"] = login_shell_path(shell)
    return env
