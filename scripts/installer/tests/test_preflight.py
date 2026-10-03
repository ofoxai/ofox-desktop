"""Disk preflight: a single tool needs far less room than a full install."""
from __future__ import annotations

import io
import os
import sys
import unittest
from collections import namedtuple
from contextlib import redirect_stderr, redirect_stdout
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import init as installer  # noqa: E402

Usage = namedtuple("Usage", "total used free")
GB = 1024 ** 3


class PreflightDiskTest(unittest.TestCase):
    def run_preflight(self, free_gb: float, min_free_gb: float) -> tuple[bool, str]:
        out = io.StringIO()
        with (
            mock.patch("platform.machine", return_value="arm64"),
            mock.patch("shutil.disk_usage", return_value=Usage(35 * GB, 10 * GB, int(free_gb * GB))),
            redirect_stdout(out),
            redirect_stderr(out),
        ):
            ok = installer.preflight_check(min_free_gb)
        return ok, out.getvalue()

    def test_single_tool_needs_two_gb(self) -> None:
        ok, _ = self.run_preflight(3.2, installer.MIN_FREE_GB_SINGLE_TOOL)
        self.assertTrue(ok)

    def test_full_install_still_needs_five_gb(self) -> None:
        ok, output = self.run_preflight(3.2, installer.MIN_FREE_GB_FULL_INSTALL)
        self.assertFalse(ok)
        self.assertIn("至少需要 5GB", output)

    def test_error_names_the_actual_threshold(self) -> None:
        ok, output = self.run_preflight(1.5, installer.MIN_FREE_GB_SINGLE_TOOL)
        self.assertFalse(ok)
        self.assertIn("1.5GB", output)
        self.assertIn("至少需要 2GB", output)

    def test_main_uses_the_single_tool_threshold_for_one_tool(self) -> None:
        seen: list[float] = []

        def fake_preflight(min_free_gb: float) -> bool:
            seen.append(min_free_gb)
            return False

        with (
            mock.patch.object(sys, "argv", ["init.py", "--tool", "hermes"]),
            mock.patch.object(installer, "print_banner"),
            mock.patch.object(installer, "preflight_check", side_effect=fake_preflight),
            self.assertRaises(SystemExit),
        ):
            installer.main()
        self.assertEqual(seen, [installer.MIN_FREE_GB_SINGLE_TOOL])


if __name__ == "__main__":
    unittest.main()
