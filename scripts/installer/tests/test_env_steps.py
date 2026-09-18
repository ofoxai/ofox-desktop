"""
公共环境层（nvm / Node / 镜像）该不该跑。

fizzy #874：已经有可用 node 的用户（fnm / volta / Homebrew 装的），点任意工具的
「安装」，第一步都是装 nvm —— 一个他们不需要的版本管理器。

根因是 `NvmStep.check()` 判的是 `~/.nvm/nvm.sh` 这个文件存不存在，而不是"node 能
不能用"。nvm 只是**手段**，目的是有一个可用的 node。

后果不只是浪费时间：`NvmStep.terminal_command()` 会往用户的 `.zshrc` 追加 nvm 初始化
代码，而 nvm 和 fnm 都改 PATH，顺序打架就会让本来好用的 node 出问题。

只用标准库（unittest.mock 也是标准库）。
"""
from __future__ import annotations

import os
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app import steps as steps_mod  # noqa: E402
from app.steps import NvmStep, VerifyStep  # noqa: E402


def _cmd_exists_returning(available: set) -> "mock._patch":
    """把 `_cmd_exists` 换成只认 `available` 里那些命令的桩。"""
    return mock.patch.object(
        steps_mod, "_cmd_exists", side_effect=lambda cmd: cmd in available
    )


class NvmStepSkipTest(unittest.TestCase):
    def test_skipped_when_node_and_npm_already_usable(self):
        """fnm / volta / brew 用户：node 能用，就别再装 nvm。"""
        with _cmd_exists_returning({"node", "npm"}):
            self.assertTrue(NvmStep().should_skip())

    def test_not_skipped_when_node_missing(self):
        """真·干净的机器：该装就得装。"""
        with _cmd_exists_returning(set()):
            self.assertFalse(NvmStep().should_skip())

    def test_not_skipped_when_npm_missing(self):
        """
        有 node 没 npm 是不完整的环境（比如只装了 node 二进制），
        后续 `npm install -g` 一定失败，所以环境层还得跑。
        """
        with _cmd_exists_returning({"node"}):
            self.assertFalse(NvmStep().should_skip())

    def test_skip_decision_does_not_depend_on_nvm_file(self):
        """
        判据必须是"node 可用"，不能回头看 `~/.nvm/nvm.sh`——那正是 #874 的错误判据。
        这里 node/npm 都可用但（桩里）没有 nvm，依然要跳过。
        """
        with _cmd_exists_returning({"node", "npm"}):
            with mock.patch.object(os.path, "isfile", return_value=False):
                self.assertTrue(NvmStep().should_skip())


class VerifyStepTest(unittest.TestCase):
    def test_passes_without_nvm_when_node_is_usable(self):
        """
        VerifyStep 原来把"`~/.nvm/nvm.sh` 存在"算作环境就绪的必要条件，于是 fnm
        用户的验证永远失败、被告知"环境未就绪"，而他的环境明明是好的。
        """
        with mock.patch.object(steps_mod, "_shell_check", return_value=True):
            with _cmd_exists_returning({"node", "npm"}):
                with mock.patch.object(os.path, "isfile", return_value=False):
                    self.assertTrue(VerifyStep().verify())

    def test_fails_when_node_is_not_usable(self):
        with mock.patch.object(steps_mod, "_shell_check", return_value=False):
            with _cmd_exists_returning(set()):
                self.assertFalse(VerifyStep().verify())


if __name__ == "__main__":
    unittest.main()
