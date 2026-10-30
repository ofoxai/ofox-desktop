"""
npm 工具「装了没有」不看 Ofox 自己的启动脚本（fizzy #1351）。

工具装在 fnm / nvm 的某个 Node 版本里时，Ofox 在 `~/.ofox-desktop/bin` 写启动脚本，
固定用那个 Node 运行，这个目录在 PATH 最末尾。默认 Node 里没有这个工具时（换了默认
版本、旧版本被删），只剩启动脚本还能被找到；把它当成已安装，点「安装」就会直接跳过，
而不是装进现在的默认 Node。

只用标准库，运行：python3 -m unittest discover -s scripts/installer/tests
"""
from __future__ import annotations

import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app import steps as steps_mod  # noqa: E402
from app.steps import CodexStep  # noqa: E402


def _executable(directory: str, name: str) -> None:
    os.makedirs(directory, exist_ok=True)
    path = os.path.join(directory, name)
    with open(path, "w") as f:
        f.write("#!/bin/sh\n")
    os.chmod(path, 0o755)


class NpmStepCheckTest(unittest.TestCase):
    def setUp(self) -> None:
        self.home = tempfile.mkdtemp()
        self.launchers = os.path.join(self.home, ".ofox-desktop", "bin")
        _executable(self.launchers, "codex")

    def _check(self, login_path: str) -> bool:
        with mock.patch.dict(os.environ, {"HOME": self.home}), mock.patch.object(
            steps_mod, "login_shell_path", return_value=login_path
        ):
            return CodexStep().check()

    def test_ofox_launcher_alone_is_not_an_installation(self):
        self.assertFalse(self._check(f"/usr/bin:/bin:{self.launchers}"))

    def test_real_install_still_counts(self):
        node_bin = os.path.join(self.home, "node", "bin")
        _executable(node_bin, "codex")
        self.assertTrue(self._check(f"{node_bin}:/usr/bin:{self.launchers}"))


if __name__ == "__main__":
    unittest.main()
