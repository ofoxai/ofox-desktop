# 网关探测（多参考图、并行工具调用）

`scripts/assistant-gateway-probe.sh` 用真实 key 向 Ofox 网关（OpenAI 兼容，
`https://api.<apex>/v1/...`）发 4 组请求，回答助手交付物依赖的三个问题：

- `/v1/images/edits` 是否接受多张参考图（多个 `image` 部分）。
- 流式 `chat/completions` 的并行工具调用：已知网关会给每个 tool call 不同的 `id`，
  但 `delta.tool_calls[].index` 全是 0，客户端按 index 拼装参数会串线。
- `/v1/responses` 的并行工具调用是否有同样的问题。

脚本只用 bash、curl 和 `python3 -I`，不装依赖。

## 探测什么

| 探测项 | 请求 | 判定 |
| --- | --- | --- |
| 1. 多参考图 | `POST /v1/images/edits`，multipart，两个 `image` 部分（脚本内生成的两张 64×64 纯色 PNG）加 `prompt`、`model`、`size=1024x1024` | 支持：2xx 且返回非空 `data[]`。不支持：400/413/415/422。其余（401、5xx、超时、非 JSON）：异常 |
| 1b. 多参考图（`image[]`） | 仅当 1 为"不支持"时再试一次，字段名改成 `image[]`（OpenAI 官方 SDK 的数组写法） | 同 1 |
| 2. chat 并行工具调用 | `POST /v1/chat/completions`，`stream:true`，一个 `get_weather(city)` 工具，提示词要求东京、大阪各调用一次 | 解析 SSE `data:` 行（忽略 `:` 注释行和 `[DONE]`），按 id 取首次出现时的 `index`。支持：至少两个不同 id，且各自 index 互不相同。不支持：不同 id 共用同一个 index。只出现一个 tool call：异常（无法下结论） |
| 3. 关闭并行 | 同 2，加 `parallel_tool_calls:false` | 支持：恰好一个 tool call，且流以 `[DONE]` 或 `finish_reason` 正常结束。不支持：仍返回多个 tool call，或网关用 400/422 拒绝该参数。没有 tool call、流没有正常结束：异常 |
| 4. responses 并行工具调用 | `POST /v1/responses`，`stream:true`，同一个工具（Responses 形状：`input`、`tools:[{type:"function",name,parameters}]`） | 收集 `response.output_item.added` 中 `type=function_call` 的条目。支持：至少两个条目且 `output_index` 互不相同。不支持：`output_index` 重复。少于两个条目：异常 |

结果只有三种：

- 支持：请求成功且行为符合预期。
- 不支持：请求成功，行为不符合预期（或网关明确拒绝）。
- 异常：无法下结论。HTTP 401/5xx、超时、响应不是预期格式、流里出错、模型没有发起足够的工具调用都算。
  响应被截断也算：HTTP 状态是 2xx，但 curl 以非零状态退出（超时、连接被重置）或 `.curl-err` 非空时，
  不管已收到多少内容一律判异常（"truncated response"）。"说明"列带 HTTP 状态和错误摘要。

## 运行

```bash
export OFOX_API_KEY=...        # 必填；没有则退出码 2。脚本不会自己找 key
bash scripts/assistant-gateway-probe.sh /tmp/probe
```

| 环境变量 | 默认值 | 说明 |
| --- | --- | --- |
| `OFOX_API_KEY` | 无 | 必填 |
| `OFOX_APEX` | `ofox.ai` | 请求发往 `https://api.<apex>/v1/...`，只允许域名字符 |
| `PROBE_MODEL` | `openai/gpt-6.1-terra` | 探测 2、3、4 使用的对话模型 |
| `PROBE_IMAGE_MODEL` | `openai/gpt-image-2` | 探测 1、1b 使用的图片模型 |

退出码：0 全部有结论（"不支持"也是结论）；1 至少一项异常；2 用法或环境错误。

每次运行会产生 1 次图片编辑（失败后重试则最多 2 次）和 3 次对话请求，图片编辑
按用量计费。每个请求 `--max-time 90`，总耗时最长约 8 分钟。

## 输出

`<outdir>` 下每个探测一组文件，`<id>` 为 `1-images-edits`、`1b-images-edits-array`、
`2-chat-parallel`、`3-chat-parallel-off`、`4-responses-parallel`：

- `<id>.request.json`：请求体（图片探测没有，两张参考图是 `ref-a.png`、`ref-b.png`）
- `<id>.body`：原始响应体（流式探测是完整 SSE 文本；图片响应含 base64 图片，体积较大）
- `<id>.status`：HTTP 状态码（`000` 表示没有拿到响应）
- `<id>.curl-err`：curl 的错误输出（正常为空）
- `summary.md`：结论表，同时打印到 stdout

key 的处理：脚本只通过 curl 的 `-K -` 从 stdin 传入，不进命令行参数；curl 以 `-q` 开头，
不读 `~/.curlrc`（里面的 `-v`、`-i`、`write-out` 会改变输出或打印请求头）；脚本不请求也不打印响应头。
脚本自己不会把 key 写进任何文件，stdout 和 `summary.md` 里的"说明"也会先把 key 和 `Bearer ...`
令牌遮成 `***`（在截断之前遮，避免截断留下半截 key）。但 `<id>.body`、`<id>.curl-err` 是网关
和 curl 的原样输出，不做遮蔽：如果某个网关在错误响应里回显 key，它会留在这两类文件里。
这些文件分享前先检查。

## 自检

```bash
bash scripts/assistant-gateway-probe.sh --self-test
```

不联网、不需要 key。用内嵌的 SSE/JSON 夹具（含 index 全为 0 的错误流、index 正确的流、
Responses 流、各种错误响应）校验分类，并用替身 curl 把整条流程跑一遍，同时检查 key
没有进入 curl 命令行、stdout 或 `summary.md`，curl 的第一个参数是 `-q`，网关回显 key 时说明列被遮蔽，
响应被截断（curl 退出 28）时判异常而不是支持。

## 结果

待用户拿真实 key 跑完后填写。

- 运行日期：
- 网关：`https://api.<apex>/v1`
- 对话模型 / 图片模型：

| 探测项 | 结果 | 说明 |
| --- | --- | --- |
| 1. images/edits 两张参考图（`image` x2） | | |
| 1b. images/edits 两张参考图（`image[]` x2，仅 1 不支持时运行） | | |
| 2. chat/completions 流式并行工具调用 | | |
| 3. chat/completions `parallel_tool_calls:false` | | |
| 4. responses 流式并行工具调用 | | |
