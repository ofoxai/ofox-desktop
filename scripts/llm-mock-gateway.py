#!/usr/bin/env python3
"""极简 LLM mock gateway —— 验证 cc-switch 写盘是否真的让 CLI 把请求送到本机。

监听 :8088（默认；可用 --port 覆盖）。**会跟 ofox-gateway-traefik docker
容器抢端口**，所以测试前先停 traefik：

    docker stop ofox-gateway-traefik
    python3 scripts/llm-mock-gateway.py
    # 完事后：docker start ofox-gateway-traefik

支持 3 个协议的最小子集（cc-switch 6 工具都覆盖到了）：

| 协议       | cc-switch 工具                       | 路径 / 头           |
| ---------- | ------------------------------------ | ------------------- |
| OpenAI v1  | Codex / OpenCode / OpenClaw / Hermes | `/v1/...`, Bearer   |
| Anthropic  | Claude                               | `/anthropic/v1/...`, `x-api-key` |
| Gemini     | Gemini CLI                           | `/gemini/v1beta/...`, `x-goog-api-key` |

每个请求都会把 path / 关键 headers / body 打到 stdout（已识别字段加 `◀━ cc-switch`
标记，方便一眼定位）。响应返回一个**有效但简化**的 payload，让 CLI 不会因为
schema 不符而疯狂重试——足够走完一个 round-trip 验证「base URL 注入 + 凭据注入」
两件事。

约定：
- 不模拟 streaming（SSE）——cc-switch 的连通性测试和首屏请求都是 non-stream，
  足够验证。CLI 自己发流式请求看到 mock 返非流 JSON 会重试几次再放弃，
  这是已知现象，不代表 cc-switch 写盘错。
- 不校验鉴权——mock 不是鉴权服，**任何** Bearer / x-api-key / x-goog-api-key
  都接受。目的是确认 cc-switch 真的把凭据放进了正确的 header，不是验证 key 本身。
- 不持久化任何状态——退出即清空。

退出：Ctrl-C。
"""
import argparse
import json
import sys
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


# 一组识别为 "cc-switch 注入" 的 header 名（lowercased）。打日志时高亮，
# 方便排查"凭据是否落到了对的 header"——比如 OpenAI 走 Bearer、Anthropic 走
# x-api-key、Gemini 走 x-goog-api-key——任何一个工具写错 header 都会立刻看到。
INJECTED_HEADERS = {
    "authorization",
    "x-api-key",
    "x-goog-api-key",
    "anthropic-version",
}


def line(msg: str) -> None:
    ts = datetime.now(timezone.utc).strftime("%H:%M:%S")
    print(f"[{ts}] {msg}", flush=True)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):  # silence default access log
        pass

    # -------- 请求 dump --------
    def _dump_request(self, body_bytes: bytes) -> None:
        line(f"━━━ {self.command} {self.path}")
        for k, v in self.headers.items():
            marker = " ◀━ cc-switch" if k.lower() in INJECTED_HEADERS else ""
            # 凭据类 header 截断显示，避免日志泄密
            if k.lower() in {"authorization", "x-api-key", "x-goog-api-key"}:
                prefix = v[:14] if len(v) > 14 else v
                v = f"{prefix}…(len={len(v)})"
            print(f"   {k}: {v}{marker}", flush=True)
        if body_bytes:
            try:
                pretty = json.dumps(json.loads(body_bytes), ensure_ascii=False, indent=2)
                # 长 body 截断——只看是否打过来、是否有 model 字段即可
                if len(pretty) > 800:
                    pretty = pretty[:800] + f"\n   …({len(body_bytes)}B total, truncated)"
                print("   body:", pretty, flush=True)
            except Exception:
                print(f"   body (raw, {len(body_bytes)}B): {body_bytes[:200]!r}", flush=True)

    def _reply(self, status: int, payload: dict) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    # -------- 路由 --------
    def do_GET(self):
        self._dump_request(b"")
        path = self.path

        # OpenAI: GET /v1/models —— cc-switch 用过这条做模型列表
        # （prod 走线上，dev 偶尔会打本地）
        if path.startswith("/v1/models"):
            self._reply(200, {
                "object": "list",
                "data": [
                    {"id": "mock-openai-mini", "object": "model", "owned_by": "mock"},
                    {"id": "mock-gpt-4", "object": "model", "owned_by": "mock"},
                ],
            })
            return

        # Anthropic: GET /anthropic/v1/models
        if path.startswith("/anthropic/v1/models"):
            self._reply(200, {
                "data": [
                    {"id": "mock-claude-sonnet", "type": "model"},
                    {"id": "mock-claude-haiku", "type": "model"},
                ],
                "has_more": False,
            })
            return

        # Gemini: GET /gemini/v1beta/models
        if path.startswith("/gemini/v1beta/models"):
            self._reply(200, {
                "models": [{
                    "name": "models/google/mock-gemini-flash",
                    "version": "001",
                    "displayName": "Mock Gemini Flash",
                    "supportedGenerationMethods": ["generateContent", "streamGenerateContent"],
                }],
            })
            return

        self._reply(404, {"error": {"message": "mock: GET not handled", "code": 404}})

    def do_POST(self):
        body_len = int(self.headers.get("content-length", 0) or 0)
        body = self.rfile.read(body_len) if body_len else b""
        self._dump_request(body)
        path = self.path

        try:
            payload = json.loads(body) if body else {}
        except Exception:
            payload = {}
        model_name = payload.get("model", "mock-model")
        ack_text = f"✅ mock 收到——cc-switch 注入的 base URL 和凭据都到位了。model={model_name}"

        # OpenAI: POST /v1/chat/completions  /v1/responses
        if path.startswith("/v1/chat/completions") or path.startswith("/v1/responses"):
            self._reply(200, {
                "id": "mock-chatcmpl-0001",
                "object": "chat.completion",
                "created": 0,
                "model": model_name,
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": ack_text},
                    "finish_reason": "stop",
                }],
                "usage": {"prompt_tokens": 4, "completion_tokens": 30, "total_tokens": 34},
            })
            return

        # Anthropic: POST /anthropic/v1/messages
        if path.startswith("/anthropic/v1/messages"):
            self._reply(200, {
                "id": "mock-msg-0001",
                "type": "message",
                "role": "assistant",
                "model": model_name,
                "content": [{"type": "text", "text": ack_text}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 4, "output_tokens": 30},
            })
            return

        # Gemini: POST /gemini/v1beta/models/{model}:generateContent|streamGenerateContent
        if path.startswith("/gemini/v1beta/models/"):
            if ":countTokens" in path:
                self._reply(200, {"totalTokens": 4})
                return
            # generateContent / streamGenerateContent —— mock 始终返非流 JSON。
            # 流式调用方会因为 schema 不匹配重试几次，已在文件 docstring 里说明。
            self._reply(200, {
                "candidates": [{
                    "content": {"parts": [{"text": ack_text}], "role": "model"},
                    "finishReason": "STOP",
                    "index": 0,
                }],
                "usageMetadata": {"promptTokenCount": 4, "candidatesTokenCount": 30, "totalTokenCount": 34},
                "modelVersion": model_name,
            })
            return

        self._reply(404, {"error": {"message": "mock: POST path not handled", "code": 404}})


def main():
    p = argparse.ArgumentParser(description="LLM mock gateway for cc-switch verification.")
    p.add_argument("--port", type=int, default=8088, help="TCP port to listen on (default 8088)")
    p.add_argument("--host", default="127.0.0.1", help="bind host (default 127.0.0.1)")
    args = p.parse_args()

    line(f"mock listening on http://{args.host}:{args.port}")
    line("→ Mocked routes:")
    line("  GET  /v1/models                              (OpenAI compat)")
    line("  POST /v1/chat/completions                    (Codex/OpenCode/OpenClaw/Hermes)")
    line("  POST /v1/responses                           (Codex responses API)")
    line("  GET  /anthropic/v1/models                    (Claude)")
    line("  POST /anthropic/v1/messages                  (Claude)")
    line("  GET  /gemini/v1beta/models                   (Gemini)")
    line("  POST /gemini/v1beta/models/{m}:generateContent|streamGenerateContent|countTokens")
    line("→ Ctrl-C to stop")
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        line("bye")
        server.server_close()


if __name__ == "__main__":
    sys.exit(main())
