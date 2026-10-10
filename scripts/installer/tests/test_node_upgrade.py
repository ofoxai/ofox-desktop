"""
工具要求更新的 Node.js 时，先升级再安装（fizzy #1021）。

用户在 Ofox 里看过原因并同意后，Tauri 后端按 Node 的管理方式（fnm / nvm / volta /
Homebrew …）生成升级命令，经 `--node-upgrade-cmd` 交给这里；`--node-min` /
`--node-below` 是升级后至少要到、且要低于的版本，用来确认升级真的生效了。

只用标准库。
"""
from __future__ import annotations

import io
import os
import sys
import unittest
from contextlib import redirect_stdout
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import init as installer  # noqa: E402
from app import steps as steps_mod  # noqa: E402
from app.steps import GeminiStep, NodeUpgradeStep, Step  # noqa: E402


def _node(version):
    return mock.patch.object(steps_mod, "_node_version", return_value=version)


class NodeUpgradeStepTest(unittest.TestCase):
    def step(self, below=25):
        return NodeUpgradeStep(
            command="fnm install 24 && fnm default 24 && fnm use 24 && npm install -g opencode-ai@latest",
            minimum="24.16.0",
            below=below,
            manual="fnm install 24 && fnm default 24",
        )

    def test_done_only_when_node_is_inside_the_required_range(self):
        for version, expected in [
            ((24, 15, 0), False),
            ((24, 16, 0), True),
            ((24, 20, 1), True),
            ((25, 0, 0), False),
            (None, False),
        ]:
            with _node(version):
                self.assertEqual(self.step().check(), expected, version)
        with _node((26, 2, 0)):
            self.assertTrue(self.step(below=None).check())

    def test_runs_the_command_ofox_built(self):
        command = self.step().terminal_command()
        self.assertIn("fnm install 24 && fnm default 24 && fnm use 24", command)
        self.assertIn("npm install -g opencode-ai@latest", command)

    def test_failure_says_what_was_needed_and_how_to_do_it_by_hand(self):
        hint = self.step().failure_hint()
        self.assertIn("24.16.0", hint)
        self.assertIn("fnm install 24 && fnm default 24", hint)


class NpmFailureHintTest(unittest.TestCase):
    def test_does_not_suggest_rerunning_the_same_failing_command(self):
        hint = GeminiStep().failure_hint()
        self.assertNotIn("请手动运行", hint)
        self.assertIn("Node.js", hint)


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
    def __init__(self, step, code: int, window_id: int) -> None:
        self.step = step
        self.code = code
        self.window_id = window_id

    def is_script_done(self) -> bool:
        if self.code == 0:
            self.step.installed = True
        return True

    def exit_code(self) -> int:
        return self.code

    def cleanup(self) -> None:
        pass


class UpgradeFlowTest(unittest.TestCase):
    def run_flow(self, args, upgrade_code=0, tool_code=0):
        tool = FakeStep("OpenClaw")
        opened: list[str] = []
        closed: list[int] = []
        upgraded = []

        def open_terminal(name: str, command: str) -> FakeHandle:
            opened.append(name)
            if name == tool.name:
                return FakeHandle(tool, tool_code, 2)
            upgraded.append(command)
            target = upgrade_step_holder[0]
            return FakeHandle(target, upgrade_code, 1)

        upgrade_step_holder: list = []
        real_init = NodeUpgradeStep.__init__

        def remember(self, *a, **k):
            real_init(self, *a, **k)
            self.installed = False
            self.check = lambda: self.installed
            upgrade_step_holder.append(self)

        with (
            mock.patch.object(sys, "argv", ["init.py", "--tool", "openclaw", "--skip-env", "--no-onboard", *args]),
            mock.patch.object(installer, "preflight_check", return_value=True),
            mock.patch.object(installer, "detect_region", return_value="INTL"),
            mock.patch.object(installer, "get_tool_step", return_value=tool),
            mock.patch.object(installer, "load_state", return_value={}),
            mock.patch.object(installer, "save_state"),
            mock.patch.object(installer, "clear_state"),
            mock.patch.object(installer, "open_terminal_with_command", side_effect=open_terminal),
            mock.patch.object(installer, "close_terminal_window", side_effect=closed.append),
            mock.patch.object(installer, "ask_continue", return_value=False),
            mock.patch.object(NodeUpgradeStep, "__init__", remember),
            mock.patch.object(sys, "stdin", io.StringIO()),
            redirect_stdout(io.StringIO()),
        ):
            with self.assertRaises(SystemExit) as ended:
                installer.main()
        return ended.exception.code, opened, closed, upgraded

    UPGRADE = [
        "--node-upgrade-cmd", "fnm install 24 && fnm default 24 && fnm use 24",
        "--node-min", "24.16.0",
        "--node-below", "25",
        "--node-manual", "fnm install 24 && fnm default 24",
    ]

    def test_upgrades_node_before_installing_the_tool(self):
        code, opened, closed, upgraded = self.run_flow(self.UPGRADE)
        self.assertEqual(code, 0)
        self.assertEqual(opened, ["升级 Node.js", "OpenClaw"])
        self.assertIn("fnm install 24 && fnm default 24 && fnm use 24", upgraded[0])
        self.assertEqual(closed, [1, 2])

    def test_a_failed_upgrade_stops_before_the_tool_and_leaves_its_window_open(self):
        code, opened, closed, _ = self.run_flow(self.UPGRADE, upgrade_code=1)
        self.assertEqual(code, 1)
        self.assertEqual(opened, ["升级 Node.js"])
        self.assertEqual(closed, [])

    def test_a_failed_install_leaves_its_window_open_to_read_the_error(self):
        code, opened, closed, _ = self.run_flow([], tool_code=1)
        self.assertEqual(code, 1)
        self.assertEqual(opened, ["OpenClaw"])
        self.assertEqual(closed, [])


if __name__ == "__main__":
    unittest.main()
