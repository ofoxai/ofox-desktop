import copy
import json
from pathlib import Path
import tempfile
import unittest

from compare import compare
from run_reference import REFERENCE, verify_synthetic


class FrozenUsageComparison(unittest.TestCase):
    def setUp(self):
        self.report = {
            "referenceCommit": REFERENCE,
            "sourceSha256": {"fixture.jsonl": "frozen-input-hash"},
            "threadCount": 2,
            "tokens": {"input": 0, "output": 2},
            "costUsd": "0.0000010",
            "activeMs": 1201,
            "days": [{"date": "2026-10-02", "activeMs": 1201}],
            "threads": [{"id": "parity-a", "tokens": 1}, {"id": "parity-b", "tokens": 2}],
        }

    def test_equal_numeric_decimal_formats_and_reordered_threads_match(self):
        actual = copy.deepcopy(self.report)
        actual["costUsd"] = "1e-6"
        actual["threads"].reverse()
        self.assertEqual(compare(self.report, actual), [])

    def test_different_source_set_prevents_parity(self):
        actual = copy.deepcopy(self.report)
        actual["sourceSha256"]["fixture.jsonl"] = "changed"
        self.assertIn("sourceSha256", [row["metric"] for row in compare(self.report, actual)])

    def test_missing_input_provenance_cannot_match(self):
        actual = copy.deepcopy(self.report)
        del actual["sourceSha256"]
        reference = copy.deepcopy(actual)
        self.assertTrue(compare(reference, actual))

    def test_unknown_does_not_equal_a_genuine_zero(self):
        actual = copy.deepcopy(self.report)
        actual["tokens"]["input"] = None
        self.assertIn("tokens.input", [row["metric"] for row in compare(self.report, actual)])

    def test_rounding_seconds_cannot_hide_a_millisecond_difference(self):
        actual = copy.deepcopy(self.report)
        actual["activeMs"] = 1200
        self.assertIn("activeMs", [row["metric"] for row in compare(self.report, actual)])

    def test_duplicate_or_extra_thread_id_is_a_difference(self):
        for rows in [[{"id": "parity-a", "tokens": 1}] * 2, self.report["threads"] + [{"id": "parity-extra"}]]:
            actual = copy.deepcopy(self.report)
            actual["threads"] = rows
            self.assertTrue(compare(self.report, actual))

    def test_unknown_price_does_not_equal_a_known_amount(self):
        actual = copy.deepcopy(self.report)
        actual["costUsd"] = None
        self.assertIn("costUsd", [row["metric"] for row in compare(self.report, actual)])

    def test_source_safety_gate_rejects_body_fields(self):
        with tempfile.TemporaryDirectory(prefix="ofox-parity-check-") as directory:
            fixture = Path(directory)
            (fixture / "manifest.json").write_text(json.dumps({"synthetic": True, "referenceCommit": REFERENCE, "files": [{"path": "fixture.jsonl"}]}))
            (fixture / "fixture.jsonl").write_text(json.dumps({"payload": {"content": "synthetic sentinel"}}))
            with self.assertRaisesRegex(ValueError, "metadata/counters"):
                verify_synthetic(fixture)


if __name__ == "__main__":
    unittest.main()
