#!/usr/bin/env python3
"""Compare normalized, body-free Rust and Magpie frozen-source reports."""

import argparse
from decimal import Decimal
import json
from pathlib import Path
import sys


def compare(reference, actual):
    differences = []

    def visit(expected, observed, path):
        if isinstance(expected, dict):
            if not isinstance(observed, dict):
                differences.append({"metric": path, "reference": expected, "actual": observed})
                return
            for key, value in expected.items():
                if key in {"referenceActiveSeconds", "sourceSha256", "callCount", "callTokens", "referenceLimitations"}:
                    continue
                visit(value, observed.get(key), f"{path}.{key}" if path else key)
        elif isinstance(expected, list):
            if not isinstance(observed, list):
                differences.append({"metric": path, "reference": expected, "actual": observed})
                return
            key = "date" if path.endswith("days") else "id"
            left = {item[key]: item for item in expected}
            right = {item[key]: item for item in observed}
            if len(left) != len(expected) or len(right) != len(observed):
                differences.append({"metric": path, "reason": "duplicate normalized identities"})
                return
            for identity in sorted(left.keys() | right.keys()):
                visit(left.get(identity), right.get(identity), f"{path}[{identity}]")
        elif path.endswith("costUsd") and observed is not None:
            if Decimal(str(expected)) != Decimal(str(observed)):
                differences.append({"metric": path, "reference": expected, "actual": observed})
        elif expected != observed:
            differences.append({"metric": path, "reference": expected, "actual": observed})

    if not isinstance(reference.get("sourceSha256"), dict) or not reference.get("sourceSha256") or reference.get("sourceSha256") != actual.get("sourceSha256"):
        differences.append({"metric": "sourceSha256", "reason": "inputs are not the same frozen source set"})
    visit(reference, actual, "")
    return differences


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", required=True, type=Path)
    parser.add_argument("--actual", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    differences = compare(json.loads(args.reference.read_text()), json.loads(args.actual.read_text()))
    result = {"state": "different" if differences else "matched", "differences": differences}
    text = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        with args.output.open("x") as stream:
            stream.write(text)
    print(text, end="")
    return int(bool(differences))


if __name__ == "__main__":
    sys.exit(main())
