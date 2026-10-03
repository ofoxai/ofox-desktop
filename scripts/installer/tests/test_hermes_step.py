"""Hermes 安装：装完必须真正能启动；国内走镜像并锁定配套 commit，海外走官方脚本。"""
from __future__ import annotations

import os
import subprocess
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app import steps as steps_mod  # noqa: E402
from app.steps import (  # noqa: E402
    HERMES_MIRROR_URL,
    HERMES_OFFICIAL_URL,
    HermesStep,
    parse_upstream_commit,
    set_region,
)

MIRROR_SCRIPT = (
    '#!/usr/bin/env bash\n'
    'MIRROR_VERSION="2026.07.11-59c36de"\n'
    'UPSTREAM_COMMIT="3b2ef789dfcf92f5b7b18c08c59d25948e50857f"\n'
    'PYTHON_VERSION="3.11"\n'
)
HERMES_PATH = os.path.expanduser("~/.local/bin/hermes")


def _completed(code: int, stderr: str = "", stdout="") -> subprocess.CompletedProcess:
    return subprocess.CompletedProcess(args=[], returncode=code, stdout=stdout, stderr=stderr)


def _only_local_bin_exists(path: str) -> bool:
    return path == HERMES_PATH


class ParseUpstreamCommitTest(unittest.TestCase):
    def test_reads_the_declared_commit(self):
        self.assertEqual(
            parse_upstream_commit(MIRROR_SCRIPT),
            "3b2ef789dfcf92f5b7b18c08c59d25948e50857f",
        )

    def test_missing_or_invalid_commit_is_none(self):
        self.assertIsNone(parse_upstream_commit('MIRROR_VERSION="2026.07.11-59c36de"\n'))
        self.assertIsNone(parse_upstream_commit('UPSTREAM_COMMIT="not-a-sha"\n'))
        self.assertIsNone(parse_upstream_commit('# UPSTREAM_COMMIT="3b2ef789dfcf"\n'))


class HermesInstallCommandTest(unittest.TestCase):
    def tearDown(self):
        set_region("CN")

    def test_intl_uses_the_official_installer_non_interactively(self):
        set_region("INTL")
        step = HermesStep()
        with mock.patch.object(step, "_fetch_mirror_script") as fetch:
            command = step.terminal_command()
        fetch.assert_not_called()
        self.assertIn(f"curl -fsSL {HERMES_OFFICIAL_URL} | bash -s -- --non-interactive --skip-setup --skip-browser", command)
        self.assertNotIn(HERMES_MIRROR_URL, command)
        self.assertIn(HERMES_OFFICIAL_URL, step.failure_hint())

    def test_cn_pins_the_mirror_script_to_its_upstream_commit(self):
        set_region("CN")
        step = HermesStep()
        with mock.patch.object(step, "_fetch_mirror_script", return_value=MIRROR_SCRIPT):
            command = step.terminal_command()
        self.assertIn(
            f"curl -fsSL {HERMES_MIRROR_URL} | bash -s -- --commit 3b2ef789dfcf92f5b7b18c08c59d25948e50857f --skip-setup",
            command,
        )
        self.assertIn("锁定上游提交 3b2ef789dfcf92f5b7b18c08c59d25948e50857f", command)
        self.assertNotIn(HERMES_OFFICIAL_URL, command)

    def test_cn_without_a_declared_commit_runs_the_mirror_unpinned(self):
        set_region("CN")
        step = HermesStep()
        with mock.patch.object(step, "_fetch_mirror_script", return_value="#!/bin/bash\n"):
            command = step.terminal_command()
        self.assertIn(f"curl -fsSL {HERMES_MIRROR_URL} | bash -s -- --skip-setup", command)
        self.assertNotIn("--commit", command)

    def test_cn_falls_back_to_the_official_installer_when_the_mirror_is_down(self):
        set_region("CN")
        step = HermesStep()
        with mock.patch.object(step, "_fetch_mirror_script", return_value=None):
            command = step.terminal_command()
        self.assertIn("镜像不可用", command)
        self.assertIn(HERMES_OFFICIAL_URL, command)

    def test_mirror_script_is_fetched_with_curl(self):
        """和终端里执行安装的是同一条路径；失效的系统代理不能把镜像误判成不可用。"""
        step = HermesStep()
        with mock.patch.object(
            steps_mod.subprocess, "run", return_value=_completed(0, stdout=MIRROR_SCRIPT.encode())
        ) as run:
            self.assertEqual(step._fetch_mirror_script(), MIRROR_SCRIPT)
        self.assertEqual(run.call_args.args[0][:3], ["curl", "-fsSL", "--max-time"])
        with mock.patch.object(steps_mod.subprocess, "run", return_value=_completed(22, stdout=b"")):
            self.assertIsNone(step._fetch_mirror_script())


class HermesVerifyTest(unittest.TestCase):
    def setUp(self):
        # 不真的起登录 shell 去取 PATH。
        patcher = mock.patch.object(steps_mod, "login_shell_env", return_value={})
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_search_paths_include_the_mirror_wrapper_location(self):
        self.assertIn("~/.local/bin/hermes", HermesStep.bin_search_paths)

    def test_verify_runs_the_installed_binary(self):
        step = HermesStep()
        with (
            mock.patch.object(os.path, "isfile", side_effect=_only_local_bin_exists),
            mock.patch.object(steps_mod.subprocess, "run", return_value=_completed(0)) as run,
        ):
            self.assertTrue(step.verify())
        self.assertEqual(run.call_count, 2)
        self.assertEqual(run.call_args_list[0].args[0], [HERMES_PATH, "--version"])
        self.assertEqual(run.call_args_list[1].args[0], [HERMES_PATH, "--help"])
        self.assertEqual(run.call_args_list[0].kwargs["timeout"], 60)
        self.assertEqual(step.failure_hint(), "Hermes 安装失败。请手动运行: curl -fsSL https://res1.hermesagent.org.cn/install.sh | bash")

    def test_failure_hint_carries_the_last_stderr_line_on_one_line(self):
        step = HermesStep()
        stderr = (
            "Traceback (most recent call last):\n"
            '  File "hermes_yaml.py", line 10, in <module>\n'
            "ModuleNotFoundError: No module named 'ruamel'\n"
        )
        with (
            mock.patch.object(os.path, "isfile", side_effect=_only_local_bin_exists),
            mock.patch.object(steps_mod.subprocess, "run", return_value=_completed(1, stderr=stderr)),
        ):
            self.assertFalse(step.verify())
        hint = step.failure_hint()
        self.assertIn("Hermes 已安装但无法启动: ModuleNotFoundError: No module named 'ruamel'", hint)
        self.assertNotIn("\n", hint)

    def test_timeout_and_missing_binary_are_reported(self):
        step = HermesStep()
        with (
            mock.patch.object(os.path, "isfile", side_effect=_only_local_bin_exists),
            mock.patch.object(
                steps_mod.subprocess,
                "run",
                side_effect=subprocess.TimeoutExpired(cmd="hermes", timeout=60),
            ),
        ):
            self.assertFalse(step.verify())
        self.assertIn("hermes --version 超时（60s）", step.failure_hint())

        with (
            mock.patch.object(os.path, "isfile", return_value=False),
            mock.patch.object(steps_mod.shutil, "which", return_value=None),
        ):
            self.assertFalse(step.verify())
        self.assertIn("未找到 hermes 可执行文件", step.failure_hint())

    def test_a_crashing_binary_counts_as_not_installed(self):
        step = HermesStep()
        with (
            mock.patch.object(os.path, "isfile", side_effect=_only_local_bin_exists),
            mock.patch.object(steps_mod.subprocess, "run", return_value=_completed(1, stderr="boom")),
        ):
            self.assertFalse(step.check())
        with (
            mock.patch.object(os.path, "isfile", side_effect=_only_local_bin_exists),
            mock.patch.object(steps_mod.subprocess, "run", return_value=_completed(0)),
        ):
            self.assertTrue(step.check())


if __name__ == "__main__":
    unittest.main()
