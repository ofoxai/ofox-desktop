#!/usr/bin/env bash
#
# assistant-gateway-probe.sh — 探测 Ofox 网关（OpenAI 兼容）的三个已知疑点。
#
#   1. POST /v1/images/edits   带两个 image 部分，网关是否接受多张参考图
#   2. POST /v1/chat/completions（流式）并行工具调用：同一轮两个 tool call 的
#      delta.tool_calls[].index 是否各不相同（已知 bug：id 不同但 index 全是 0）
#   3. 同上，带 parallel_tool_calls:false，是否只返回一个 tool call
#   4. POST /v1/responses（流式）并行工具调用：function_call 条目的 output_index
#      是否各不相同
#
# 用法：
#   OFOX_API_KEY=... bash scripts/assistant-gateway-probe.sh <outdir>
#   bash scripts/assistant-gateway-probe.sh --self-test     # 不联网、不需要 key
#
# 环境变量：
#   OFOX_API_KEY       必填。缺失时退出码 2。脚本不会去钥匙串或配置文件里找 key。
#   OFOX_APEX          默认 ofox.ai（请求发往 https://api.<apex>/v1/...）
#   PROBE_MODEL        默认 openai/gpt-6.1-terra（对话类探测）
#   PROBE_IMAGE_MODEL  默认 openai/gpt-image-2（图片编辑探测）
#
# 输出：<outdir> 下每个探测一组 <id>.request.json / .body / .status / .curl-err，
# 以及 summary.md；同一张表（支持 / 不支持 / 异常）也打印到 stdout。
# 退出码：0 全部有结论；1 至少一项异常；2 用法或环境错误。
#
# key 只经 curl 的 -K - 从 stdin 传入：不进命令行参数（ps 看不到）；curl 带 -q，
# 不读 ~/.curlrc；脚本不用 -i/-v，不打印响应头。stdout 和 summary.md 里的说明会把
# key 与 "Bearer ..." 遮成 ***；<id>.body / <id>.curl-err 是网关和 curl 的原样输出，
# 网关若在错误响应里回显 key，它会留在这两类文件里，分享前请先检查。
#
set -euo pipefail
umask 077

OFOX_APEX=${OFOX_APEX:-ofox.ai}
PROBE_MODEL=${PROBE_MODEL:-openai/gpt-6.1-terra}
PROBE_IMAGE_MODEL=${PROBE_IMAGE_MODEL:-openai/gpt-image-2}
SELF=${BASH_SOURCE[0]}
IMAGE_PROMPT="Blend the two reference images into one simple abstract picture."

# ── 内嵌 Python（只用标准库，python3 -I 隔离模式运行）──────────────────
# 子命令：
#   png <path> <r> <g> <b>     写一张 64x64 纯色 PNG
#   payload <chat|chat-off|responses> <model>   打印请求 JSON
#   analyze <kind> <body> <status> <errfile> [curl_rc]   打印 "OK|NO|BAD<TAB>detail"
#   check-gen                  自检 PNG 与请求体的生成
# 源码保持纯 ASCII：中文标签由 bash 侧映射，避免依赖 locale。
IFS= read -r -d '' PY <<'PYEOF' || true
import json
import os
import re
import struct
import sys
import zlib

PROMPT = "Get the weather for both Tokyo and Osaka using the tool, one call per city."
TOOL_DESC = "Get the current weather for a city."
TOOL_PARAMS = {
    "type": "object",
    "properties": {"city": {"type": "string", "description": "City name"}},
    "required": ["city"],
}


def png(size, rgb):
    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    row = b"\x00" + bytes(rgb) * size
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(row * size))
        + chunk(b"IEND", b"")
    )


def payload(which, model):
    if which == "responses":
        return {
            "model": model,
            "stream": True,
            "input": PROMPT,
            "tools": [{"type": "function", "name": "get_weather",
                       "description": TOOL_DESC, "parameters": TOOL_PARAMS}],
        }
    body = {
        "model": model,
        "stream": True,
        "messages": [{"role": "user", "content": PROMPT}],
        "tools": [{"type": "function", "function": {
            "name": "get_weather", "description": TOOL_DESC, "parameters": TOOL_PARAMS}}],
    }
    if which == "chat-off":
        body["parallel_tool_calls"] = False
    return body


def scrub(text):
    """Mask the API key and any Bearer token a gateway may echo back (before truncation)."""
    key = os.environ.get("OFOX_API_KEY", "")
    if key:
        text = text.replace(key, "***")
    return re.sub(r"(?i)\b(bearer)\s+[^\s\"',;]+", r"\1 ***", text)


def clean(value, limit=160):
    text = re.sub(r"\s+", " ", scrub(str(value))).strip().replace("|", "/")
    return text[:limit]


def emit(kind, detail):
    sys.stdout.buffer.write((kind + "\t" + clean(detail) + "\n").encode("utf-8"))


def read_text(path):
    try:
        with open(path, "rb") as handle:
            return handle.read().decode("utf-8", "replace")
    except OSError:
        return ""


def dicts(value):
    return [item for item in value if isinstance(item, dict)] if isinstance(value, list) else []


def error_text(obj):
    if not isinstance(obj, dict):
        return None
    err = obj.get("error")
    if isinstance(err, dict):
        return err.get("message") or json.dumps(err, ensure_ascii=False)
    if isinstance(err, str) and err:
        return err
    if isinstance(obj.get("message"), str):
        return obj["message"]
    return None


def http_problem(status, text, errfile):
    if status == 0:
        return "no HTTP response (curl: %s)" % (read_text(errfile) or "unknown error")
    try:
        message = error_text(json.loads(text))
    except ValueError:
        message = None
    return "HTTP %d: %s" % (status, message or text or "(empty body)")


def sse_events(text):
    """Yield (event_name, parsed_json) per data: line; skip comments, [DONE], non-JSON."""
    event = None
    for line in text.replace("\r\n", "\n").replace("\r", "\n").split("\n"):
        if line == "":
            event = None
        elif line.startswith(":"):
            continue
        elif line.startswith("event:"):
            event = line[6:].strip()
        elif line.startswith("data:"):
            data = line[5:].strip()
            if data in ("", "[DONE]"):
                continue
            try:
                yield event, json.loads(data)
            except ValueError:
                continue


def analyze_images(status, text):
    try:
        obj = json.loads(text)
    except ValueError:
        return "BAD", "HTTP %d but body is not JSON: %s" % (status, text)
    entries = [d for d in dicts(obj.get("data") if isinstance(obj, dict) else None)
               if d.get("b64_json") or d.get("url")]
    if entries:
        return "OK", "HTTP %d, data[%d] returned" % (status, len(entries))
    return "BAD", "HTTP %d but no data[]: %s" % (status, error_text(obj) or text)


def has_done(text):
    return re.search(r"^data:\s*\[DONE\]\s*$", text, re.M) is not None


def truncation(status, text, errfile, curl_rc):
    """A 2xx status with a curl error means the body was cut off (timeout, reset)."""
    err = read_text(errfile)
    try:
        code = int(curl_rc)
    except ValueError:
        code = 0
    if code == 0 and not err:
        return None
    return "truncated response: curl exit %d%s; HTTP %d, %d chars received" % (
        code, " (%s)" % err if err else "", status, len(text))


def analyze_chat(text, single):
    events, errors, ids, first_index, indexes = 0, [], [], {}, set()
    finished = False
    for _event, obj in sse_events(text):
        events += 1
        if not isinstance(obj, dict):
            continue
        if error_text(obj):
            errors.append(error_text(obj))
        for choice in dicts(obj.get("choices")):
            if choice.get("finish_reason"):
                finished = True
            delta = choice.get("delta")
            if not isinstance(delta, dict):
                continue
            for call in dicts(delta.get("tool_calls")):
                index = call.get("index")
                if index is not None:
                    indexes.add(index)
                call_id = call.get("id")
                if call_id and call_id not in first_index:
                    ids.append(call_id)
                    first_index[call_id] = index
    shown = "ids=%d, tool_calls index values=%s" % (len(ids), sorted(indexes, key=str))
    if events == 0:
        return "BAD", "HTTP 200 but no SSE data events: %s" % text
    if errors:
        return "BAD", "stream error: %s" % errors[0]
    if not ids:
        return "BAD", "no tool call id in stream (model may not have called the tool)"
    if single:
        if len(ids) == 1:
            if not (finished or has_done(text)):
                return "BAD", "stream ended without [DONE] or finish_reason (cut off?), inconclusive; " + shown
            return "OK", "exactly one tool call; " + shown
        return "NO", "%d tool calls despite parallel_tool_calls=false; %s" % (len(ids), shown)
    if len(ids) < 2:
        return "BAD", "only one tool call, inconclusive; " + shown
    if None in first_index.values():
        return "BAD", "tool_calls carry no index; " + shown
    if len(set(first_index.values())) == len(ids):
        return "OK", "distinct index per id; " + shown
    return "NO", "index collides across distinct ids (gateway bug); " + shown


def analyze_responses(text):
    events, errors, items = 0, [], []
    for event, obj in sse_events(text):
        events += 1
        if not isinstance(obj, dict):
            continue
        kind = obj.get("type") or event
        if kind == "error":
            errors.append(error_text(obj) or "error event")
        elif kind == "response.failed":
            errors.append(error_text(obj.get("response")) or "response.failed")
        elif kind == "response.output_item.added":
            item = obj.get("item")
            if isinstance(item, dict) and item.get("type") == "function_call":
                items.append((obj.get("output_index"), item.get("id") or item.get("call_id")))
    indexes = [index for index, _ in items]
    shown = "function_call items=%d, output_index=%s, ids=%s" % (len(items), indexes, [i for _, i in items])
    if events == 0:
        return "BAD", "HTTP 200 but no SSE data events: %s" % text
    if errors:
        return "BAD", "stream error: %s" % errors[0]
    if len(items) < 2:
        return "BAD", "fewer than two function_call items, inconclusive; " + shown
    if None in indexes:
        return "BAD", "function_call items carry no output_index; " + shown
    if len(set(indexes)) == len(items):
        return "OK", "distinct output_index per item; " + shown
    return "NO", "output_index collides; " + shown


def analyze(kind, body_path, status_arg, errfile, curl_rc="0"):
    text = read_text(body_path)
    try:
        status = int(status_arg)
    except ValueError:
        status = 0
    success = 200 <= status < 300
    if success:
        cut = truncation(status, text, errfile, curl_rc)
        if cut:
            return "BAD", cut
    if kind == "images":
        if success:
            return analyze_images(status, text)
        if status in (400, 413, 415, 422):
            return "NO", http_problem(status, text, errfile)
        return "BAD", http_problem(status, text, errfile)
    if not success:
        if kind == "chat-single" and status in (400, 422) and "parallel_tool_calls" in text:
            return "NO", "parameter rejected: " + http_problem(status, text, errfile)
        return "BAD", http_problem(status, text, errfile)
    if kind == "responses":
        return analyze_responses(text)
    return analyze_chat(text, single=(kind == "chat-single"))


def check_gen():
    for size, rgb in ((64, (220, 40, 40)), (1, (0, 0, 0))):
        data = png(size, rgb)
        assert data[:8] == b"\x89PNG\r\n\x1a\n", "png signature"
        width, height = struct.unpack(">II", data[16:24])
        assert (width, height) == (size, size), "png size"
        assert data.endswith(b"IEND" + struct.pack(">I", zlib.crc32(b"IEND") & 0xFFFFFFFF)), "png tail"
        idat = data.index(b"IDAT")
        length = struct.unpack(">I", data[idat - 4:idat])[0]
        assert len(zlib.decompress(data[idat + 4:idat + 4 + length])) == (1 + 3 * size) * size, "png pixels"
    chat = payload("chat", "m")
    assert chat["stream"] is True and "parallel_tool_calls" not in chat, "chat payload"
    assert chat["tools"][0]["function"]["name"] == "get_weather", "chat tool"
    assert payload("chat-off", "m")["parallel_tool_calls"] is False, "chat-off payload"
    resp = payload("responses", "m")
    assert resp["input"] == PROMPT and resp["tools"][0]["name"] == "get_weather", "responses payload"
    assert resp["tools"][0]["type"] == "function" and "parameters" in resp["tools"][0], "responses tool shape"
    print("ok")


def main(argv):
    command = argv[0]
    if command == "png":
        with open(argv[1], "wb") as handle:
            handle.write(png(64, tuple(int(v) for v in argv[2:5])))
    elif command == "payload":
        print(json.dumps(payload(argv[1], argv[2])))
    elif command == "analyze":
        try:
            emit(*analyze(*argv[1:6]))
        except Exception as exc:  # parser must never crash the probe run
            emit("BAD", "parser error: %r" % (exc,))
    elif command == "check-gen":
        check_gen()
    else:
        sys.exit("unknown command: " + command)


main(sys.argv[1:])
PYEOF

py() { python3 -I -c "$PY" "$@"; }

# ── 公共函数 ───────────────────────────────────────────────────────────
die() { printf '%s\n' "$1" >&2; exit 2; }

# key 经 stdin 交给 curl 读配置（printf 是内建命令，key 不会出现在 argv）。
# -q 必须是第一个参数：不读 ~/.curlrc，免得里面的 -v/-i/write-out 改变输出或打印请求头。
curl_auth() {
  printf 'header = "Authorization: Bearer %s"\n' "$OFOX_API_KEY" | curl -q -K - "$@"
}

# classify <kind> <body> <status> <errfile> [curl_rc] → 打印 "支持|不支持|异常<TAB>说明"
classify() {
  local out label
  out=$(py analyze "$@" 2>&1) || out=$'BAD\t解析脚本失败: '"$out"
  case ${out%%$'\t'*} in
    OK) label=支持 ;;
    NO) label=不支持 ;;
    *) label=异常 ;;
  esac
  printf '%s\t%s\n' "$label" "${out#*$'\t'}"
}

ROWS=()
HAS_ABNORMAL=0
PROBE_VERDICT=

record() { # label verdict detail
  ROWS+=("$1"$'\t'"$2"$'\t'"$3")
  if [[ $2 == 异常 ]]; then HAS_ABNORMAL=1; fi
}

# probe <id> <label> <kind> <curl 参数...>：发请求、分类、记一行，结论放 PROBE_VERDICT。
probe() {
  local id=$1 label=$2 kind=$3
  shift 3
  local body="$OUTDIR/$id.body" errf="$OUTDIR/$id.curl-err" status result curl_rc=0
  : >"$body"
  : >"$errf"
  printf '[%s] %s ...\n' "$id" "$label" >&2
  status=$(curl_auth -sS --max-time 90 -o "$body" -w '%{http_code}' "$@" 2>"$errf") || curl_rc=$?
  [[ $status =~ ^[0-9]{3}$ ]] || status=000
  printf '%s\n' "$status" >"$OUTDIR/$id.status"
  result=$(classify "$kind" "$body" "$status" "$errf" "$curl_rc")
  PROBE_VERDICT=${result%%$'\t'*}
  record "$label" "$PROBE_VERDICT" "${result#*$'\t'}"
}

render_summary() {
  local row label verdict detail
  printf '# 网关探测结果\n\n'
  printf -- '- 时间: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf -- '- 网关: https://api.%s/v1\n' "$OFOX_APEX"
  printf -- '- 对话模型: %s\n' "$PROBE_MODEL"
  printf -- '- 图片模型: %s\n\n' "$PROBE_IMAGE_MODEL"
  printf '| 探测项 | 结果 | 说明 |\n| --- | --- | --- |\n'
  for row in "${ROWS[@]}"; do
    IFS=$'\t' read -r label verdict detail <<<"$row"
    printf '| %s | %s | %s |\n' "$label" "$verdict" "$detail"
  done
}

run_probes() {
  local base="https://api.$OFOX_APEX/v1"
  local json_headers=(-H 'Content-Type: application/json')

  py png "$OUTDIR/ref-a.png" 220 40 40
  py png "$OUTDIR/ref-b.png" 40 40 220

  # 1. images/edits：两个 image 部分。字段名不被接受时再试一次 image[]。
  local edit_args=(-X POST "$base/images/edits" -F "model=$PROBE_IMAGE_MODEL"
    -F "prompt=$IMAGE_PROMPT" -F "size=1024x1024")
  probe 1-images-edits "images/edits 两张参考图 (image x2)" images "${edit_args[@]}" \
    -F "image=@$OUTDIR/ref-a.png;type=image/png" -F "image=@$OUTDIR/ref-b.png;type=image/png"
  if [[ $PROBE_VERDICT == 不支持 ]]; then
    probe 1b-images-edits-array "images/edits 两张参考图 (image[] x2)" images "${edit_args[@]}" \
      -F "image[]=@$OUTDIR/ref-a.png;type=image/png" -F "image[]=@$OUTDIR/ref-b.png;type=image/png"
  fi

  # 2. chat/completions 流式并行工具调用
  py payload chat "$PROBE_MODEL" >"$OUTDIR/2-chat-parallel.request.json"
  probe 2-chat-parallel "chat/completions 流式并行工具调用" chat-parallel \
    -X POST "$base/chat/completions" "${json_headers[@]}" \
    --data-binary "@$OUTDIR/2-chat-parallel.request.json"

  # 3. 同上 + parallel_tool_calls:false
  py payload chat-off "$PROBE_MODEL" >"$OUTDIR/3-chat-parallel-off.request.json"
  probe 3-chat-parallel-off "chat/completions parallel_tool_calls:false" chat-single \
    -X POST "$base/chat/completions" "${json_headers[@]}" \
    --data-binary "@$OUTDIR/3-chat-parallel-off.request.json"

  # 4. responses 流式并行工具调用
  py payload responses "$PROBE_MODEL" >"$OUTDIR/4-responses-parallel.request.json"
  probe 4-responses-parallel "responses 流式并行工具调用" responses \
    -X POST "$base/responses" "${json_headers[@]}" \
    --data-binary "@$OUTDIR/4-responses-parallel.request.json"
}

# ── 自检：夹具 + 替身 curl，不联网 ─────────────────────────────────────
self_test() {
  local pass=0 fail=0
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/ofox-probe-selftest.XXXXXX")
  trap 'rm -rf "$tmp"' EXIT
  local fx="$tmp/fx"
  mkdir -p "$fx" "$tmp/bin" "$tmp/log"

  write_fixtures "$fx"
  local secret="sk-selftest-SECRET-0123456789" out rc
  # 网关把 key 原样回显（含恰好跨过 160 字符截断点的情况）和回显别的 Bearer token
  printf '{"error":{"message":"Incorrect API key provided: %s"}}\n' "$secret" >"$fx/images-401-echo.json"
  printf '%s %s\n' "$(printf 'x%.0s' $(seq 1 130))" "$secret" >"$fx/key-at-truncation-edge.txt"
  printf '{"error":{"message":"bad header Authorization: Bearer sk-other-token-XYZ123"}}\n' >"$fx/bearer-echo.json"

  check() { # name cmd...
    local name=$1
    shift
    if "$@"; then pass=$((pass + 1)); printf 'PASS  %s\n' "$name"
    else fail=$((fail + 1)); printf 'FAIL  %s\n' "$name"; fi
  }
  # expect <name> <期望结论> <kind> <fixture> <status> [说明里应包含的子串] [curl-err 夹具] [curl 退出码]
  expect() {
    local name=$1 want=$2 kind=$3 file=$4 status=$5 needle=${6:-} errfile=${7:-empty.txt} rc=${8:-0} out got detail
    out=$(classify "$kind" "$fx/$file" "$status" "$fx/$errfile" "$rc")
    got=${out%%$'\t'*}
    detail=${out#*$'\t'}
    if [[ $got == "$want" && $detail == *"$needle"* ]]; then
      pass=$((pass + 1)); printf 'PASS  %-44s -> %s\n' "$name" "$got"
    else
      fail=$((fail + 1)); printf 'FAIL  %-44s want %s%s, got %s (%s)\n' "$name" "$want" "${needle:+ with \"$needle\"}" "$got" "$detail"
    fi
  }

  # scrubbed <fixture> <status> <不得出现> <应出现>：网关回显的 key / Bearer token 在说明里被遮掉
  scrubbed() {
    local result
    result=$(OFOX_API_KEY=$secret classify images "$fx/$1" "$2" "$fx/empty.txt" 0)
    [[ $result != *"$3"* && $result == *"$4"* ]]
  }

  echo "== 解析器 =="
  expect "images 200 with data[]"              支持   images images-ok.json 200
  expect "images 400 (second image rejected)"  不支持 images images-400.json 400 "single file"
  expect "images 401 is not a verdict"         异常   images images-401.json 401 "401"
  expect "images 200 but html body"            异常   images html-502.html 200 "not JSON"
  expect "images 502 html"                     异常   images html-502.html 502 "502"
  expect "images curl failure (status 000)"    异常   images empty.txt 000 "timed out" curl-err.txt 28
  expect "chat index=0 on distinct ids (bug)"  不支持 chat-parallel chat-buggy.sse 200 "[0]"
  expect "chat distinct index per id"          支持   chat-parallel chat-ok.sse 200 "[0, 1]"
  expect "chat distinct index, CRLF stream"    支持   chat-parallel chat-ok-crlf.sse 200
  expect "chat one call only is inconclusive"  异常   chat-parallel chat-single.sse 200 "inconclusive"
  expect "chat 200 but text only"              异常   chat-parallel chat-text.sse 200 "no tool call"
  expect "chat 200 but html body"              异常   chat-parallel html-502.html 200 "no SSE"
  expect "chat 502 html"                       异常   chat-parallel html-502.html 502 "502"
  expect "chat stream error event"             异常   chat-parallel chat-error.sse 200 "overloaded"
  expect "chat off: exactly one call"          支持   chat-single chat-single.sse 200
  expect "chat off ignored: two calls"         不支持 chat-single chat-ok.sse 200 "despite"
  expect "chat off rejected by gateway"        不支持 chat-single param-400.json 400 "parallel_tool_calls"
  expect "chat off 400 unrelated is abnormal"  异常   chat-single images-400.json 400 "400"
  expect "chat off no tool call"               异常   chat-single chat-text.sse 200
  expect "chat off: one call, finish_reason only" 支持 chat-single chat-single-finish.sse 200
  expect "chat off: one call, [DONE] only"     支持   chat-single chat-single-done.sse 200
  expect "chat off: one call, no end marker"   异常   chat-single chat-truncated.sse 200 "without [DONE]"
  expect "chat off: cut off, curl exit 28"     异常   chat-single chat-truncated.sse 200 "truncated" curl-err.txt 28
  expect "chat off: cut off, only .curl-err"   异常   chat-single chat-truncated.sse 200 "truncated" curl-err.txt 0
  expect "chat parallel: cut off, curl exit 28" 异常  chat-parallel chat-ok.sse 200 "truncated" curl-err.txt 28
  expect "responses: cut off, curl exit 28"    异常   responses responses-ok.sse 200 "truncated" curl-err.txt 28
  expect "images: cut off, curl exit 28"       异常   images images-ok.json 200 "truncated" curl-err.txt 28
  expect "responses distinct output_index"     支持   responses responses-ok.sse 200 "[1, 2]"
  expect "responses same output_index"         不支持 responses responses-bug.sse 200 "collides"
  expect "responses one function_call"         异常   responses responses-one.sse 200 "inconclusive"
  expect "responses response.failed event"     异常   responses responses-failed.sse 200 "quota"
  expect "responses 502 html"                  异常   responses html-502.html 502 "502"

  echo "== 密钥遮蔽 =="
  check "echoed key is masked in the detail"     scrubbed images-401-echo.json 401 "$secret" '***'
  check "echoed key is masked before truncation" scrubbed key-at-truncation-edge.txt 502 "sk-selft" '***'
  check "echoed Bearer token is masked"          scrubbed bearer-echo.json 401 "sk-other-token-XYZ123" 'Bearer ***'

  echo "== 生成 =="
  check "png and request payloads are valid" test "$(py check-gen)" = ok

  echo "== 端到端（替身 curl） =="
  write_fake_curl "$tmp/bin/curl"
  local summary="$tmp/out/summary.md"
  has() { grep -qF -- "$1" "$2"; }
  argv_has_no_secret() { [[ -s $tmp/log/argv.log ]] && ! grep -qF -- "$secret" "$tmp/log/argv.log"; }
  outputs_have_no_secret() { [[ $1 != *"$secret"* ]] && ! grep -rqF -- "$secret" "$tmp/out"; }
  no_header_flags() { ! grep -qE '(^| )(-i|-v|--include|--verbose|-D|--dump-header)( |$)' "$tmp/log/argv.log"; }
  exits_with() { [[ $1 == "$2" ]]; }
  not_in_file() { ! grep -qF -- "$1" "$2"; }
  argv_all_start_with_q() { [[ -s $tmp/log/argv.log ]] && ! grep -qv '^-q -K - ' "$tmp/log/argv.log"; }

  rc=0
  out=$(env -u OFOX_API_KEY bash "$SELF" "$tmp/out-nokey" 2>&1) || rc=$?
  check "missing OFOX_API_KEY exits 2" exits_with "$rc" 2
  check "missing OFOX_API_KEY prints a hint" test -n "$out"

  rc=0
  out=$(PATH="$tmp/bin:$PATH" FAKE_FIXTURES="$fx" FAKE_LOG="$tmp/log" OFOX_API_KEY="$secret" \
    bash "$SELF" "$tmp/out" 2>&1) || rc=$?
  check "run exits 0 when every probe has a verdict" exits_with "$rc" 0
  check "images: field image rejected"        has '(image x2) | 不支持 |' "$summary"
  check "images: image[] retry accepted"      has '(image[] x2) | 支持 |' "$summary"
  check "chat parallel: index bug detected"   has 'chat/completions 流式并行工具调用 | 不支持 |' "$summary"
  check "chat parallel_tool_calls:false ok"   has 'parallel_tool_calls:false | 支持 |' "$summary"
  check "responses parallel ok"               has 'responses 流式并行工具调用 | 支持 |' "$summary"
  check "summary is also printed to stdout"   has '| 探测项 | 结果 | 说明 |' <(printf '%s\n' "$out")
  check "request and status files are written" test -s "$tmp/out/2-chat-parallel.request.json" -a -s "$tmp/out/4-responses-parallel.status"
  check "parallel:false request carries flag"  has '"parallel_tool_calls": false' "$tmp/out/3-chat-parallel-off.request.json"
  check "key is not on the curl command line"  argv_has_no_secret
  check "key is delivered via -K - on stdin"   has "Authorization: Bearer $secret" "$tmp/log/stdin-config.log"
  check "key is not echoed or written to disk" outputs_have_no_secret "$out"
  check "no response headers requested"        no_header_flags
  check "curl runs with -q first (no ~/.curlrc)" argv_all_start_with_q

  rc=0
  out=$(PATH="$tmp/bin:$PATH" FAKE_FIXTURES="$fx" FAKE_LOG="$tmp/log" FAKE_MODE=down OFOX_API_KEY="$secret" \
    bash "$SELF" "$tmp/out-down" 2>&1) || rc=$?
  check "gateway down: exits 1" exits_with "$rc" 1
  check "gateway down: all four rows 异常 with HTTP 502" exits_with "$(grep -c '| 异常 | HTTP 502' "$tmp/out-down/summary.md")" 4

  rc=0
  out=$(PATH="$tmp/bin:$PATH" FAKE_FIXTURES="$fx" FAKE_LOG="$tmp/log" FAKE_MODE=cut OFOX_API_KEY="$secret" \
    bash "$SELF" "$tmp/out-cut" 2>&1) || rc=$?
  check "cut-off chat streams: exits 1" exits_with "$rc" 1
  check "cut-off probe 3 is 异常, not 支持" has 'parallel_tool_calls:false | 异常 | truncated response: curl exit 28' "$tmp/out-cut/summary.md"
  check "cut-off probe 2 is 异常" has 'chat/completions 流式并行工具调用 | 异常 | truncated response' "$tmp/out-cut/summary.md"
  check "cut-off chat leaves responses intact" has 'responses 流式并行工具调用 | 支持 |' "$tmp/out-cut/summary.md"

  rc=0
  out=$(PATH="$tmp/bin:$PATH" FAKE_FIXTURES="$fx" FAKE_LOG="$tmp/log" FAKE_MODE=echo OFOX_API_KEY="$secret" \
    bash "$SELF" "$tmp/out-echo" 2>&1) || rc=$?
  check "gateway echoes key: summary shows HTTP 401" has 'HTTP 401: Incorrect API key provided: ***' "$tmp/out-echo/summary.md"
  check "gateway echoes key: summary has no key" not_in_file "$secret" "$tmp/out-echo/summary.md"
  check "gateway echoes key: stdout has no key" test "${out#*"$secret"}" = "$out"

  printf '\nself-test: %d passed, %d failed\n' "$pass" "$fail"
  [[ $fail -eq 0 ]]
}

write_fixtures() {
  local d=$1
  printf '%s\n' ': keep-alive' '' \
    'data: {"choices":[{"index":0,"delta":{"role":"assistant","content":null},"finish_reason":null}]}' '' \
    'data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_tokyo","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}' '' \
    'data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":\"Tokyo\"}"}}]}}]}' '' \
    'data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_osaka","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}' '' \
    'data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":\"Osaka\"}"}}]}}]}' '' \
    'data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}' '' \
    'data: [DONE]' '' >"$d/chat-buggy.sse"
  # 同样的流，第二个 tool call 的 tool_calls[].index 改成 1（正确行为）；
  # choices[].index 恒为 0，解析器不能把它当成 tool call 的 index
  sed '/[Oo]saka/s/"tool_calls":\[{"index":0,/"tool_calls":[{"index":1,/' "$d/chat-buggy.sse" >"$d/chat-ok.sse"
  grep -q '"index":1,"id":"call_osaka"' "$d/chat-ok.sse" || die "fixture build failed"
  sed 's/$/\r/' "$d/chat-ok.sse" >"$d/chat-ok-crlf.sse"
  printf '%s\n' \
    'data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_tokyo","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Tokyo\"}"}}]}}]}' '' \
    'data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}' '' 'data: [DONE]' '' >"$d/chat-single.sse"
  printf '%s\n' \
    'data: {"choices":[{"index":0,"delta":{"content":"It is sunny."},"finish_reason":null}]}' '' 'data: [DONE]' '' >"$d/chat-text.sse"
  grep -v 'DONE' "$d/chat-single.sse" >"$d/chat-single-finish.sse"
  grep -v 'finish_reason' "$d/chat-single.sse" >"$d/chat-single-done.sse"
  printf '%s\n' \
    'data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_tokyo","type":"function","function":{"name":"get_weather","arguments":"{\"city\""}}]}}]}' '' >"$d/chat-truncated.sse"
  printf '%s\n' 'data: {"error":{"message":"upstream overloaded","type":"server_error"}}' '' >"$d/chat-error.sse"
  printf '%s\n' \
    'event: response.created' \
    'data: {"type":"response.created","response":{"id":"resp_1","status":"in_progress"}}' '' \
    'event: response.output_item.added' \
    'data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_1"}}' '' \
    'event: response.output_item.added' \
    'data: {"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","id":"fc_1","call_id":"call_tokyo","name":"get_weather","arguments":""}}' '' \
    'event: response.function_call_arguments.delta' \
    'data: {"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":1,"delta":"{\"city\""}' '' \
    'event: response.output_item.added' \
    'data: {"type":"response.output_item.added","output_index":2,"item":{"type":"function_call","id":"fc_2","call_id":"call_osaka","name":"get_weather","arguments":""}}' '' \
    'event: response.completed' \
    'data: {"type":"response.completed","response":{"id":"resp_1","status":"completed"}}' '' >"$d/responses-ok.sse"
  sed 's/"output_index":2/"output_index":1/' "$d/responses-ok.sse" >"$d/responses-bug.sse"
  printf '%s\n' \
    'event: response.output_item.added' \
    'data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_tokyo","name":"get_weather","arguments":""}}' '' >"$d/responses-one.sse"
  printf '%s\n' \
    'event: response.failed' \
    'data: {"type":"response.failed","response":{"id":"resp_1","status":"failed","error":{"code":"rate_limit","message":"quota exceeded"}}}' '' >"$d/responses-failed.sse"
  printf '%s\n' '{"created":1,"data":[{"b64_json":"iVBORw0KGgo="}]}' >"$d/images-ok.json"
  printf '%s\n' '{"error":{"message":"expected a single file for image","type":"invalid_request_error"}}' >"$d/images-400.json"
  printf '%s\n' '{"error":{"message":"invalid api key"}}' >"$d/images-401.json"
  printf '%s\n' '{"error":{"message":"unknown parameter: parallel_tool_calls","type":"invalid_request_error"}}' >"$d/param-400.json"
  printf '%s\n' '<html><head><title>502 Bad Gateway</title></head><body>bad gateway</body></html>' >"$d/html-502.html"
  : >"$d/empty.txt"
  printf '%s\n' 'curl: (28) Operation timed out after 90001 milliseconds with 141 bytes received' >"$d/curl-err.txt"
}

# 替身 curl：记录 argv 与 stdin 配置，按 URL 返回夹具，行为模拟"image 字段被拒、
# image[] 被接受、chat 有 index=0 bug、parallel_tool_calls:false 被遵守、responses 正常"。
# FAKE_MODE：down（全部 502）、echo（401 并回显 key）、cut（chat 流被截断，curl 退出 28）。
write_fake_curl() {
  cat >"$1" <<'FAKE_END'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$FAKE_LOG/argv.log"
cat >>"$FAKE_LOG/stdin-config.log"
out="" url="" data="" field=image
while (($#)); do
  case $1 in
    -o) out=$2; shift ;;
    --data-binary) data=${2#@}; shift ;;
    -F) if [[ $2 == "image[]="* ]]; then field=array; fi; shift ;;
    https://*) url=$1 ;;
  esac
  shift
done
status=200 rc=0
case ${FAKE_MODE:-} in
  down) src=html-502.html status=502 ;;
  echo) printf '{"error":{"message":"Incorrect API key provided: %s"}}\n' "$OFOX_API_KEY" >"$out"; printf '401'; exit 0 ;;
  *)
    case $url in
      */images/edits) if [[ $field == array ]]; then src=images-ok.json; else src=images-400.json status=400; fi ;;
      */chat/completions)
        if [[ ${FAKE_MODE:-} == cut ]]; then
          src=chat-truncated.sse rc=28
          echo "curl: (28) Operation timed out after 90001 milliseconds with 141 bytes received" >&2
        elif grep -q parallel_tool_calls "$data"; then src=chat-single.sse
        else src=chat-buggy.sse; fi ;;
      */responses) src=responses-ok.sse ;;
      *) src=empty.txt status=404 ;;
    esac ;;
esac
cp "$FAKE_FIXTURES/$src" "$out"
printf '%s' "$status"
exit "$rc"
FAKE_END
  chmod +x "$1"
}

# ── 入口 ───────────────────────────────────────────────────────────────
main() {
  if [[ ${1:-} == --self-test ]]; then
    self_test
    return
  fi
  [[ $# -eq 1 && $1 != -* ]] || die "用法: OFOX_API_KEY=... bash $0 <outdir>   （或 --self-test）"
  [[ -n ${OFOX_API_KEY:-} ]] || die "缺少 OFOX_API_KEY：请先 export OFOX_API_KEY=...（脚本不会自行查找密钥）"
  [[ $OFOX_API_KEY != *[[:space:][:cntrl:]\"\\]* ]] || die "OFOX_API_KEY 含有空白、引号或反斜杠，请检查是否复制完整"
  [[ $OFOX_APEX =~ ^[A-Za-z0-9.-]+$ ]] || die "OFOX_APEX 只能是域名（如 ofox.ai）"
  [[ $1 != *[\;,\"]* ]] || die "outdir 不能包含 ; , \" 字符（curl -F 无法安全引用）"
  command -v curl >/dev/null || die "需要 curl"
  command -v python3 >/dev/null || die "需要 python3"

  OUTDIR=${1%/}
  mkdir -p "$OUTDIR"
  run_probes
  render_summary | tee "$OUTDIR/summary.md"
  if [[ $HAS_ABNORMAL -eq 1 ]]; then return 1; fi
}

main "$@"
