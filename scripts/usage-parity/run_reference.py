#!/usr/bin/env python3
"""Run only synthetic usage fixtures against one exact Magpie revision."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

REFERENCE = "bfaedf18301d02a423bd5422d22e41432d231ab8"
HERE = Path(__file__).resolve().parent


def command(args, cwd, env=None):
    return subprocess.run(args, cwd=cwd, env=env, text=True, capture_output=True, check=True).stdout.strip()


def verify_synthetic(fixtures):
    manifest = json.loads((fixtures / "manifest.json").read_text())
    if manifest.get("synthetic") is not True or manifest.get("referenceCommit") != REFERENCE:
        raise ValueError("expected the pinned synthetic fixture manifest")
    allowed = {"timestamp", "type", "payload", "id", "session_id", "cwd", "source", "turn_id", "model", "thread_id", "response_id", "usage", "thread_token_usage", "input_tokens", "output_tokens", "cached_input_tokens", "cache_write_input_tokens", "reasoning_output_tokens", "info", "total_token_usage", "last_token_usage", "compaction_response_id", "latest_token_usage_record"}
    identifiers = {"id", "session_id", "turn_id", "model", "thread_id", "response_id", "compaction_response_id"}

    def check(value):
        if isinstance(value, dict):
            if value.keys() - allowed:
                raise ValueError("fixtures permit only whitelisted metadata/counters, without conversation or secrets")
            for key, item in value.items():
                if key == "cwd" and not str(item).startswith("/fixture/"):
                    raise ValueError("fixture cwd must be explicitly fictional")
                if key in identifiers and not str(item).startswith("parity-"):
                    raise ValueError("fixture identities must be explicitly fictional")
                check(item)
        elif isinstance(value, list):
            for item in value:
                check(item)
        elif isinstance(value, str) and ("https://" in value or "http://" in value or value.startswith("sk-")):
            raise ValueError("fixture must not include endpoints or credentials")

    for item in manifest["files"]:
        path = fixtures / item["path"]
        if Path(item["path"]).name != item["path"] or path.is_symlink():
            raise ValueError("only regular synthetic fixture basenames are permitted")
        for line in path.read_text().splitlines():
            check(json.loads(line))
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--magpie", required=True, type=Path)
    parser.add_argument("--output", type=Path, help="a new or empty report directory; defaults to a temporary directory")
    parser.add_argument("--go", default="go", help="Go executable; an already installed Go 1.26.8 toolchain is required offline")
    args = parser.parse_args()
    reference = args.magpie.resolve(strict=True)
    if command(["git", "rev-parse", "HEAD"], reference) != REFERENCE:
        raise ValueError("Magpie must be exactly the pinned reference commit")
    if command(["git", "status", "--porcelain"], reference):
        raise ValueError("the reference checkout must be clean; no files will be changed")
    verify_synthetic(HERE / "fixtures")
    output = args.output.resolve() if args.output else Path(tempfile.mkdtemp(prefix="ofox-usage-parity-"))
    if output.exists() and any(output.iterdir()):
        raise ValueError("the report directory must be empty")
    output.mkdir(parents=True, exist_ok=True)
    base_env = dict(os.environ, GOWORK="off")
    base_env.pop("GOROOT", None)
    root = Path(command([args.go, "env", "GOROOT"], reference, base_env))
    executable = root / "bin" / ("go.exe" if os.name == "nt" else "go")
    if not executable.is_file():
        raise ValueError("cannot resolve the pinned Go toolchain")
    env = dict(base_env, GOROOT=str(root), GOTOOLCHAIN="local", GOPROXY="off", GOSUMDB="off")
    go_version = command([str(executable), "version"], reference, env)
    env.update(OFOX_PARITY_FIXTURES=str(HERE / "fixtures"), OFOX_PARITY_REFERENCE_OUTPUT=str(output / "reference.json"))
    overlay = {"Replace": {str(reference / "internal/sessions/ofox_frozen_parity_test.go"): str(HERE / "reference_overlay_test.go")}}
    (output / "overlay.json").write_text(json.dumps(overlay, indent=2) + "\n")
    invocation = [str(executable), "test", "-mod=readonly", "-overlay", str(output / "overlay.json"), "./internal/sessions", "-run", "^TestOfoxFrozenUsageParity$", "-count=1", "-parallel=1", "-v"]
    result = subprocess.run(invocation, cwd=reference, env=env, text=True, capture_output=True)
    (output / "reference-test.log").write_text(result.stdout + result.stderr)
    clean = not command(["git", "status", "--porcelain"], reference)
    if not clean:
        raise ValueError("reference checkout changed during validation; inspect it without resetting user work")
    provenance = {
        "referenceCommit": REFERENCE,
        "referenceTreeCleanBeforeAndAfter": clean,
        "goVersion": go_version,
        "overlayTestSha256": hashlib.sha256((HERE / "reference_overlay_test.go").read_bytes()).hexdigest(),
        "manifestSha256": hashlib.sha256((HERE / "fixtures/manifest.json").read_bytes()).hexdigest(),
        "exitCode": result.returncode,
        "runtimeSources": "synthetic metadata and counters only",
        "paidCalls": 0,
        "runtimeNetwork": "external HTTP blocked by Magpie testenv.Offline",
    }
    (output / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    print(f"Reference report: {output}")
    if result.returncode:
        print(result.stdout + result.stderr, file=sys.stderr)
    return result.returncode


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Usage parity validation failed: {error}", file=sys.stderr)
        sys.exit(1)
