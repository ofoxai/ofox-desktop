"""
弹终端执行安装脚本的 AppleScript 构造。

fizzy #873：Terminal.app 的 `do script` 是**把文本键入 shell**，不是执行文件。新窗口
起的是 interactive login shell，会加载 `.zshrc` —— oh-my-zsh 的更新提示用
`read -k 1` 读单个字符，正好把键入的第一个 `/` 吃掉，于是：

    /tmp/openclaw-install-s0uwk38v.sh            ← 键入的完整路径
    [oh-my-zsh] Would you like to update? [Y/n]  ← 吃掉了 /
    ❯ tmp/openclaw-install-s0uwk38v.sh           ← 剩下的成了相对路径
    zsh: no such file or directory

安装脚本根本没跑，而安装器不知道，会一直轮询到超时。任何 `.zshrc` 里有交互提示的
用户都会中招，而报错信息完全指不到真因。

试过但不可行的路：`open -a Terminal <script>` 不执行脚本（实测标记文件不出现）。
Terminal 的 AppleScript 字典里也没有"执行文件"这个动作，只有 `do script`。

所以只能在 `do script` 上加保险，两层：
  1. 先开空窗口，等 shell 把 rc 跑完（`busy` 属性），再发真正的命令
  2. 命令前垫空白 —— 被吃掉的是空格，路径仍完整；没被吃则 shell 忽略前导空格

只用标准库。
"""
from __future__ import annotations

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app.terminal import _build_launch_applescript, _script_prefix  # noqa: E402


class BuildLaunchAppleScriptTest(unittest.TestCase):
    PATH = "/tmp/ofox-install-abc123.sh"

    def test_embeds_the_script_path(self):
        script = _build_launch_applescript(self.PATH)

        self.assertIn(self.PATH, script)

    def test_pads_command_with_leading_whitespace(self):
        """
        这是 #873 的核心防线：提示吃掉一个字符时，被吃的必须是无意义的空白，
        而不是路径的第一个 `/`。
        """
        script = _build_launch_applescript(self.PATH)

        self.assertIn(f' {self.PATH}', script)
        # 至少两个空格：吃掉一个还剩一个，shell 会忽略前导空格
        self.assertRegex(script, r'"  +' + self.PATH)

    def test_waits_for_shell_to_finish_loading_rc(self):
        """
        第二层防线：先开空窗口，等 `busy` 变 false（rc 跑完了）再发命令。
        光靠垫空白挡不住"读一整行"的提示。
        """
        script = _build_launch_applescript(self.PATH)

        self.assertIn("busy", script, "必须等 shell 就绪，不能开窗就立刻键入")

    def test_wait_loop_is_bounded(self):
        """
        提示可能一直等用户输入，`busy` 就一直是 true。必须有上限，否则 osascript
        会卡到 subprocess 超时，安装流程直接断在这儿。
        """
        script = _build_launch_applescript(self.PATH)

        self.assertIn("repeat while", script)
        self.assertRegex(script, r"waited\s*<\s*\d+", "等待循环要有次数上限")

    def test_sends_command_to_the_window_it_opened(self):
        # 不能用 "front window"——用户在等待期间可能点了别的窗口
        script = _build_launch_applescript(self.PATH)

        self.assertIn("in targetTab", script)

    def test_returns_a_window_id(self):
        # 调用方要拿 id 才能在装完后关窗口
        script = _build_launch_applescript(self.PATH)

        self.assertIn("return id of", script)


class ScriptPrefixTest(unittest.TestCase):
    """
    临时脚本名以前硬编码 `openclaw-install-`：用户点 Gemini，`/tmp` 里出现的却是
    `openclaw-install-xxx.sh`，排查时极其误导（这个安装器源自 openclaw-launcher）。
    """

    def test_does_not_mention_openclaw(self):
        self.assertNotIn("openclaw", _script_prefix("Gemini").lower())

    def test_includes_the_step_name_for_traceability(self):
        # 带上步骤名，/tmp 里一眼能看出是哪一步留下的
        self.assertIn("gemini", _script_prefix("Gemini"))

    def test_strips_non_ascii_step_names(self):
        # 步骤名可能是中文（"国内镜像配置"），不能让它进文件名
        prefix = _script_prefix("国内镜像配置")

        self.assertTrue(prefix.isascii(), f"文件名前缀必须是 ASCII：{prefix!r}")
        self.assertTrue(prefix)

    def test_strips_path_separators(self):
        # 防御：步骤名若含 / 或 .. 会写到意料之外的位置
        prefix = _script_prefix("../../etc/passwd")

        self.assertNotIn("/", prefix)
        self.assertNotIn("..", prefix)


if __name__ == "__main__":
    unittest.main()
