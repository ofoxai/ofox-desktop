"""
进度事件输出的单元测试。

安装器的 stdout 会被 Rust 侧 install_tool.rs 逐行转发成 install-tool-log 事件，
前端据此还原安装进度。所以这里验证的是"行格式"这个契约：必须是单行合法 JSON、
以换行结尾（Rust 用 BufReader::lines() 按 \n 切分，\r 覆写的内容到不了前端）。

只用标准库，与 scripts/installer 其余部分保持一致。
运行：python3 -m unittest discover -s scripts/installer/tests
"""
from __future__ import annotations

import io
import json
import os
import sys
import unittest
from contextlib import redirect_stdout

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app.progress import (  # noqa: E402
    PROGRESS_TYPE,
    emit_progress,
    make_poll_callback,
)


class EmitProgressTest(unittest.TestCase):
    def _capture(self, **kwargs) -> str:
        buf = io.StringIO()
        with redirect_stdout(buf):
            emit_progress(**kwargs)
        return buf.getvalue()

    def test_emits_single_json_line_ending_with_newline(self):
        out = self._capture(step=2, total=4, name="Node.js LTS", phase="start")

        self.assertTrue(out.endswith("\n"), "必须以 \\n 结尾，否则 Rust 侧读不到这一行")
        self.assertEqual(out.count("\n"), 1, "必须是单行，多行会被切成多个事件")
        self.assertNotIn("\r", out, "不能含 \\r，覆写式输出到不了前端")

    def test_payload_carries_step_context(self):
        out = self._capture(step=2, total=4, name="Node.js LTS", phase="start")
        payload = json.loads(out)

        self.assertEqual(payload["type"], PROGRESS_TYPE)
        self.assertEqual(payload["step"], 2)
        self.assertEqual(payload["total"], 4)
        self.assertEqual(payload["name"], "Node.js LTS")
        self.assertEqual(payload["phase"], "start")

    def test_waiting_phase_carries_elapsed_and_timeout(self):
        out = self._capture(
            step=2, total=4, name="Node.js LTS", phase="waiting", elapsed=18, timeout=300
        )
        payload = json.loads(out)

        self.assertEqual(payload["elapsed"], 18)
        self.assertEqual(payload["timeout"], 300)

    def test_optional_fields_omitted_when_not_given(self):
        out = self._capture(step=1, total=4, name="nvm", phase="start")
        payload = json.loads(out)

        self.assertNotIn("elapsed", payload)
        self.assertNotIn("timeout", payload)

    def test_step_name_with_quotes_stays_parseable(self):
        # 步骤名来自 steps.py，将来可能含引号/中文；JSON 编码必须扛得住
        out = self._capture(step=1, total=2, name='foo "bar" 中文', phase="done")
        payload = json.loads(out)

        self.assertEqual(payload["name"], 'foo "bar" 中文')


class MakePollCallbackTest(unittest.TestCase):
    """
    wait_for_condition 的 progress_callback 只收到 (elapsed, timeout)，没有"这是第几步"
    的上下文。这个工厂把步骤信息闭包进去，同时保留原有的人类可读输出。
    """

    def _run(self, cb, *args) -> str:
        buf = io.StringIO()
        with redirect_stdout(buf):
            cb(*args)
        return buf.getvalue()

    def _progress_payloads(self, out: str):
        return [
            json.loads(line)
            for line in out.splitlines()
            if line.strip().startswith("{")
        ]

    def test_emits_waiting_with_step_context(self):
        cb = make_poll_callback(step=2, total=4, name="Node.js LTS")

        payloads = self._progress_payloads(self._run(cb, 18, 300))

        self.assertEqual(len(payloads), 1)
        self.assertEqual(payloads[0]["phase"], "waiting")
        self.assertEqual(payloads[0]["step"], 2)
        self.assertEqual(payloads[0]["name"], "Node.js LTS")
        self.assertEqual(payloads[0]["elapsed"], 18)
        self.assertEqual(payloads[0]["timeout"], 300)

    def test_still_calls_the_human_readable_tick(self):
        seen = []
        cb = make_poll_callback(
            step=1, total=2, name="nvm", tick_fn=lambda e, t: seen.append((e, t))
        )

        self._run(cb, 5, 60)

        self.assertEqual(seen, [(5, 60)], "原有的终端进度条输出不能被顶掉")

    def test_json_stays_on_its_own_line_after_an_overwriting_tick(self):
        """
        真实的 display.print_poll_tick 用 \\r 原地覆写进度条且不换行，光标停在行尾。
        如果进度 JSON 紧接着打印，两者会挤成一行——Rust 侧 BufReader::lines() 按 \\n
        切分，前端 parseProgressLine 又要求行首是 '{'，这样的行会被整条丢弃。
        """

        def overwriting_tick(elapsed: int, timeout: int) -> None:
            print(f"\r       等待中 {elapsed:02d}", end="", flush=True)

        cb = make_poll_callback(
            step=1, total=2, name="Codex", tick_fn=overwriting_tick
        )

        out = self._run(cb, 3, 300)

        json_lines = [l for l in out.splitlines() if PROGRESS_TYPE in l]
        self.assertEqual(len(json_lines), 1)
        self.assertTrue(
            json_lines[0].lstrip().startswith("{"),
            f"进度 JSON 必须独占一行，实际是：{json_lines[0]!r}",
        )

    def test_works_without_tick_fn(self):
        cb = make_poll_callback(step=1, total=2, name="nvm")

        payloads = self._progress_payloads(self._run(cb, 1, 60))

        self.assertEqual(len(payloads), 1)


if __name__ == "__main__":
    unittest.main()
