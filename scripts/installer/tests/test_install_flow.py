"""Fresh-Mac installation must resume the requested tool after Node succeeds."""
from __future__ import annotations

import io
import os
import sys
import unittest
from contextlib import redirect_stdout
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import init as installer  # noqa: E402
from app.steps import Step  # noqa: E402


class FakeStep(Step):
    needs_terminal = True

    def __init__(self, name: str) -> None:
        self.name = name
        self.installed = False

    def check(self) -> bool:
        return self.installed

    def terminal_command(self) -> str:
        return self.name


class FakeHandle:
    window_id = 1

    def __init__(self, step: FakeStep, code: int) -> None:
        self.step = step
        self.code = code

    def is_script_done(self) -> bool:
        if self.code == 0:
            self.step.installed = True
        return True

    def exit_code(self) -> int:
        return self.code

    def cleanup(self) -> None:
        pass


class FreshInstallFlowTest(unittest.TestCase):
    def run_flow(self, node_exit_code: int) -> list[str]:
        node = FakeStep("Node.js LTS")
        opencode = FakeStep("OpenCode")
        opened: list[str] = []

        def open_terminal(name: str, command: str) -> FakeHandle:
            opened.append(name)
            return FakeHandle(node if name == node.name else opencode,
                              node_exit_code if name == node.name else 0)

        with (
            mock.patch.object(sys, "argv", ["init.py", "--tool", "opencode", "--no-onboard"]),
            mock.patch.object(installer, "preflight_check", return_value=True),
            mock.patch.object(installer, "detect_region", return_value="INTL"),
            mock.patch.object(installer, "get_env_steps", return_value=[node]),
            mock.patch.object(installer, "get_tool_step", return_value=opencode),
            mock.patch.object(installer, "load_state", return_value={}),
            mock.patch.object(installer, "save_state"),
            mock.patch.object(installer, "clear_state"),
            mock.patch.object(installer, "open_terminal_with_command", side_effect=open_terminal),
            mock.patch.object(installer, "close_terminal_window"),
            mock.patch.object(installer, "ask_continue", return_value=False),
            mock.patch.object(sys, "stdin", io.StringIO()),
            redirect_stdout(io.StringIO()),
        ):
            with self.assertRaises(SystemExit) as ended:
                installer.main()
        self.assertEqual(ended.exception.code, 0 if node_exit_code == 0 else 1)
        return opened

    def test_continues_to_opencode_without_another_click(self):
        self.assertEqual(self.run_flow(0), ["Node.js LTS", "OpenCode"])

    def test_does_not_start_opencode_after_failed_node_install(self):
        self.assertEqual(self.run_flow(1), ["Node.js LTS"])

    def test_fails_when_the_installed_tool_cannot_start(self):
        """安装脚本退出 0 但工具起不来：整步失败，并把原因打成一行「提示:」。"""

        class BrokenStep(FakeStep):
            def verify(self) -> bool:
                return False

            def failure_hint(self) -> str:
                return "Hermes 已安装但无法启动: ModuleNotFoundError: No module named 'ruamel'。请在终端运行 hermes --version 查看详情。"

        hermes = BrokenStep("Hermes")
        output = io.StringIO()
        with (
            mock.patch.object(sys, "argv", ["init.py", "--tool", "hermes", "--skip-env", "--no-onboard"]),
            mock.patch.object(installer, "preflight_check", return_value=True),
            mock.patch.object(installer, "detect_region", return_value="CN"),
            mock.patch.object(installer, "get_tool_step", return_value=hermes),
            mock.patch.object(installer, "load_state", return_value={}),
            mock.patch.object(installer, "save_state"),
            mock.patch.object(installer, "clear_state"),
            mock.patch.object(installer, "open_terminal_with_command", side_effect=lambda name, cmd: FakeHandle(hermes, 0)),
            mock.patch.object(installer, "close_terminal_window"),
            mock.patch.object(installer, "ask_continue", return_value=False),
            mock.patch.object(sys, "stdin", io.StringIO()),
            redirect_stdout(output),
        ):
            with self.assertRaises(SystemExit) as ended:
                installer.main()
        self.assertEqual(ended.exception.code, 1)
        self.assertIn("提示: Hermes 已安装但无法启动", output.getvalue())


if __name__ == "__main__":
    unittest.main()
