"""
检测命令怎么构造——这决定了"工具装了没有"判断得准不准。

背景：应用从 Finder/Dock 启动时，PATH 只有 `/usr/bin:/bin:/usr/sbin:/sbin`，不含
nvm / fnm / volta / Homebrew 往用户 shell rc 里加的那些路径。而 install_tool.rs 用
`Command::new("/bin/bash")` 启动安装器时没有重设环境，所以安装器继承的就是这个
贫瘠的 PATH。用它裸查 `command -v codex` 必然找不到，于是已装的工具被判定未装、
反复重装（见 fizzy #865）。

原来的 NVM_PREFIX 想用 `. nvm.sh` 补 PATH，但只照顾了 nvm 用户；而且
`[ -s "$NVM_DIR/nvm.sh" ] && ...` 这个 && 链在无 nvm 时会短路，连后面真正的检测
命令都不执行。

结论是走 login shell：它加载用户自己的 rc，谁装的 node 都能找到。

只用标准库，运行：python3 -m unittest discover -s scripts/installer/tests
"""
from __future__ import annotations

import os
import subprocess
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app.shell import login_shell_argv, resolve_login_shell  # noqa: E402


class ResolveLoginShellTest(unittest.TestCase):
    def test_prefers_explicit_argument(self):
        self.assertEqual(resolve_login_shell("/bin/fish"), "/bin/fish")

    def test_falls_back_to_shell_env(self):
        old = os.environ.get("SHELL")
        os.environ["SHELL"] = "/bin/zsh"
        try:
            self.assertEqual(resolve_login_shell(), "/bin/zsh")
        finally:
            if old is None:
                os.environ.pop("SHELL", None)
            else:
                os.environ["SHELL"] = old

    def test_never_returns_empty_even_without_shell_env(self):
        # GUI 启动的进程不保证有 SHELL；拿不到也必须给出一个能用的 shell，
        # 否则检测命令根本拼不出来。
        old = os.environ.get("SHELL")
        os.environ.pop("SHELL", None)
        try:
            resolved = resolve_login_shell()
            self.assertTrue(resolved)
            self.assertTrue(os.path.isabs(resolved), f"应当是绝对路径：{resolved!r}")
        finally:
            if old is not None:
                os.environ["SHELL"] = old


class LoginShellArgvTest(unittest.TestCase):
    def test_builds_login_non_interactive_argv(self):
        argv = login_shell_argv("command -v codex", shell="/bin/zsh")

        self.assertEqual(argv, ["/bin/zsh", "-l", "-c", "command -v codex"])

    def test_is_not_interactive(self):
        # -i 会加载交互式配置、可能读 stdin 或输出提示符，在子进程里跑会挂住。
        # launch_tool.rs 用 -l -i 是因为它真的要给用户一个交互终端；检测不需要。
        argv = login_shell_argv("true", shell="/bin/zsh")

        self.assertNotIn("-i", argv)

    def test_command_is_passed_as_single_argument(self):
        # 命令必须整条作为一个 argv 元素传给 -c，不能被拆开，否则带空格/引号的
        # 复合命令会被 shell 当成多个参数。
        argv = login_shell_argv("command -v codex && echo ok", shell="/bin/zsh")

        self.assertEqual(argv[-1], "command -v codex && echo ok")
        self.assertEqual(len(argv), 4)


class LoginShellActuallyFindsToolsTest(unittest.TestCase):
    """
    真跑一次。这是本次修复的核心断言：在只有系统 PATH 的贫瘠环境里，login shell
    仍然能找到用户 shell rc 里配置的命令。
    """

    BARE_ENV = {
        "HOME": os.environ.get("HOME", ""),
        "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
        "SHELL": os.environ.get("SHELL", ""),
    }

    def _find(self, cmd: str, use_login_shell: bool) -> bool:
        probe = f"command -v {cmd}"
        argv = (
            login_shell_argv(probe)
            if use_login_shell
            else ["/bin/sh", "-c", probe]
        )
        return (
            subprocess.run(
                argv, env=self.BARE_ENV, capture_output=True
            ).returncode
            == 0
        )

    def test_finds_system_command_either_way(self):
        # sh 在系统 PATH 里，两种方式都该找到——用来确认测试本身没问题。
        self.assertTrue(self._find("sh", use_login_shell=False))
        self.assertTrue(self._find("sh", use_login_shell=True))

    @unittest.skipUnless(
        os.environ.get("SHELL"), "需要 SHELL 才能验证 login shell 行为"
    )
    def test_login_shell_sees_more_than_bare_path(self):
        """
        node 这类由版本管理器（nvm/fnm/volta）或 Homebrew 装的命令，在贫瘠 PATH 下
        查不到，但 login shell 能查到。如果本机 node 恰好装在系统路径里，这个用例
        就失去意义，跳过而不是假装通过。
        """
        if self._find("node", use_login_shell=False):
            self.skipTest("本机 node 在系统 PATH 里，此用例无法区分两种方式")

        self.assertTrue(
            self._find("node", use_login_shell=True),
            "login shell 应当能找到用户 shell rc 里配置的 node",
        )


class ShellCheckUnderBareEnvTest(unittest.TestCase):
    """
    fizzy #865 的回归测试。

    起一个环境被剥光的子 Python（PATH 只留系统目录，等同 Finder 启动的 .app），
    在里面调真实的 `steps._shell_check`。修复前它找不到 fnm/nvm/volta/brew 装的
    node，于是已装的工具被判成未装、反复重装。
    """

    INSTALLER_DIR = os.path.join(os.path.dirname(__file__), "..")

    def _shell_check_in_bare_env(self, command: str) -> subprocess.CompletedProcess:
        code = (
            "import sys; sys.path.insert(0, {dir!r});"
            "from app.steps import _shell_check;"
            "sys.exit(0 if _shell_check({cmd!r}) else 1)"
        ).format(dir=self.INSTALLER_DIR, cmd=command)

        return subprocess.run(
            [sys.executable, "-c", code],
            env={
                "HOME": os.environ.get("HOME", ""),
                "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
                "SHELL": os.environ.get("SHELL", ""),
            },
            capture_output=True,
        )

    def test_finds_system_command(self):
        # 先确认这套子进程机制本身是通的
        result = self._shell_check_in_bare_env("command -v sh")

        self.assertEqual(
            result.returncode, 0, f"stderr: {result.stderr.decode(errors='replace')}"
        )

    @unittest.skipUnless(
        os.environ.get("SHELL"), "需要 SHELL 才能验证 login shell 行为"
    )
    def test_finds_version_managed_node_despite_bare_path(self):
        if subprocess.run(
            ["/bin/sh", "-c", "command -v node"],
            env={"PATH": "/usr/bin:/bin:/usr/sbin:/sbin"},
            capture_output=True,
        ).returncode == 0:
            self.skipTest("本机 node 在系统 PATH 里，此用例无法区分修复前后")

        result = self._shell_check_in_bare_env("command -v node")

        self.assertEqual(
            result.returncode,
            0,
            "贫瘠 PATH 下仍应找到用户 shell 里配置的 node —— 这正是 #865 的根因。"
            f" stderr: {result.stderr.decode(errors='replace')}",
        )


class InteractiveRcToolsTest(unittest.TestCase):
    """
    第一版修复（只用 `-l -c`）漏掉的场景。

    zsh 的加载规则：`-l`（login）只读 `.zprofile` / `.zlogin`，而 **`.zshrc` 只在
    interactive 时加载**。于是配在 `.zshrc` 里的 PATH —— pnpm 的 `PNPM_HOME`
    就是典型 —— 用 `-l -c` 根本看不到。

    实测：`~/.local/share/pnpm/opencode` 用 `-l -c` 找不到、`-l -i -c` 找得到。
    所以已装的 opencode 会被安装器判成未装，#865 没有修干净。

    但不能简单全改 `-i`：实测 interactive shell 每次约 800ms，是 `-l -c` 的 34 倍，
    6 个工具就要 4.8s。正确做法是只用一次 interactive shell 把完整 PATH 取出来
    缓存，之后所有检测复用。
    """

    INSTALLER_DIR = os.path.join(os.path.dirname(__file__), "..")

    def _cmd_exists_in_bare_env(self, command: str) -> subprocess.CompletedProcess:
        code = (
            "import sys; sys.path.insert(0, {dir!r});"
            "from app.steps import _cmd_exists;"
            "sys.exit(0 if _cmd_exists({cmd!r}) else 1)"
        ).format(dir=self.INSTALLER_DIR, cmd=command)

        return subprocess.run(
            [sys.executable, "-c", code],
            env={
                "HOME": os.environ.get("HOME", ""),
                "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
                "SHELL": os.environ.get("SHELL", ""),
            },
            capture_output=True,
        )

    @unittest.skipUnless(
        os.environ.get("SHELL"), "需要 SHELL 才能验证 login shell 行为"
    )
    def test_finds_tool_whose_path_lives_in_interactive_rc(self):
        import shutil

        tool = "opencode"
        if shutil.which(tool) is None:
            self.skipTest(f"本机没装 {tool}，无法验证 .zshrc-only 的 PATH 场景")
        if subprocess.run(
            ["/bin/sh", "-c", f"command -v {tool}"],
            env={"PATH": "/usr/bin:/bin:/usr/sbin:/sbin"},
            capture_output=True,
        ).returncode == 0:
            self.skipTest(f"{tool} 在系统 PATH 里，此用例无法区分")

        result = self._cmd_exists_in_bare_env(tool)

        self.assertEqual(
            result.returncode,
            0,
            f"{tool} 的 PATH 配在 interactive rc 里，检测必须也能覆盖到。"
            f" stderr: {result.stderr.decode(errors='replace')}",
        )

    @unittest.skipUnless(
        os.environ.get("SHELL"), "需要 SHELL 才能验证 login shell 行为"
    )
    def test_detection_of_many_tools_stays_fast(self):
        """
        性能护栏：把 interactive shell 的开销摊成一次，别让它乘以工具数。
        全用 `-l -i -c` 的话 6 个工具约 4.8s，用户打开"添加工具"要干等。
        """
        code = (
            "import sys, time; sys.path.insert(0, {dir!r});"
            "from app.steps import _cmd_exists;"
            "t0 = time.monotonic();"
            "[_cmd_exists(t) for t in "
            "('claude','codex','opencode','openclaw','hermes','gemini')];"
            "print(time.monotonic() - t0)"
        ).format(dir=self.INSTALLER_DIR)

        result = subprocess.run(
            [sys.executable, "-c", code],
            env={
                "HOME": os.environ.get("HOME", ""),
                "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
                "SHELL": os.environ.get("SHELL", ""),
            },
            capture_output=True,
        )
        self.assertEqual(
            result.returncode, 0, result.stderr.decode(errors="replace")
        )

        elapsed = float(result.stdout.decode().strip())
        self.assertLess(
            elapsed,
            2.5,
            f"检测 6 个工具用了 {elapsed:.2f}s —— interactive shell 的开销应当只付一次",
        )


class RunShellCommandUnderBareEnvTest(unittest.TestCase):
    """
    #865 的执行侧。

    MirrorsStep 用 `run_shell_command` 跑 `npm config set registry ...`，同一个
    PATH 问题会让 fnm / volta / Homebrew 用户的镜像配置静默失败：检测说"没配"，
    执行又找不到 npm，于是这一步永远成功不了。

    executor 原来自己注入 `~/.nvm/versions/node/<最新>/bin` 来补 PATH，同样只
    照顾 nvm 用户。
    """

    INSTALLER_DIR = os.path.join(os.path.dirname(__file__), "..")

    def _run_in_bare_env(self, command: str) -> subprocess.CompletedProcess:
        code = (
            "import sys; sys.path.insert(0, {dir!r});"
            "from app.executor import run_shell_command;"
            "sys.exit(0 if run_shell_command({cmd!r}) else 1)"
        ).format(dir=self.INSTALLER_DIR, cmd=command)

        return subprocess.run(
            [sys.executable, "-c", code],
            env={
                "HOME": os.environ.get("HOME", ""),
                "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
                "SHELL": os.environ.get("SHELL", ""),
            },
            capture_output=True,
        )

    def test_runs_system_command(self):
        result = self._run_in_bare_env("true")

        self.assertEqual(
            result.returncode, 0, f"stderr: {result.stderr.decode(errors='replace')}"
        )

    def test_reports_failure_for_failing_command(self):
        # 别把"命令失败"和"找不到命令"混为一谈——前者必须照实返回 False
        result = self._run_in_bare_env("false")

        self.assertEqual(result.returncode, 1)

    @unittest.skipUnless(
        os.environ.get("SHELL"), "需要 SHELL 才能验证 login shell 行为"
    )
    def test_runs_version_managed_command_despite_bare_path(self):
        if subprocess.run(
            ["/bin/sh", "-c", "command -v npm"],
            env={"PATH": "/usr/bin:/bin:/usr/sbin:/sbin"},
            capture_output=True,
        ).returncode == 0:
            self.skipTest("本机 npm 在系统 PATH 里，此用例无法区分修复前后")

        result = self._run_in_bare_env("npm --version")

        self.assertEqual(
            result.returncode,
            0,
            "贫瘠 PATH 下 run_shell_command 仍应能跑起 npm，否则镜像配置会静默失败。"
            f" stderr: {result.stderr.decode(errors='replace')}",
        )


if __name__ == "__main__":
    unittest.main()
