# CLI 切换模型误报冲突 + 缺 key 记录 实施计划

Fizzy: #1047 · 分支：`fix/cli-model-conflict`（从 origin/main 切）

**Goal**：在 CLI 里切换模型不再显示「配置冲突」，主页显示的模型与实际配置一致；Codex / Hermes 补上 key 记录，恢复「数据统计」。

**Architecture**：模型是用户的选择，不是接入配置的一部分——冲突检测只看服务商和 key，不看模型。但「修复绑定 / 换 key」会用桌面端存的模板整份重写配置，Codex 的受管字段里含 `model`，所以光去掉比对会让后续写入把用户在 CLI 选的模型悄悄改回旧值。因此在任何重写之前，先把配置文件里的实际模型**采纳**进桌面端的模板，两边始终一致。key 记录缺失的工具，在登录后和启动时的 key 对账里补一遍。

**Tech Stack**：Rust（`services/ofox_bind`、`services/account_keys`、`commands/manage_tool.rs`），Vitest（前端基本不动）。

## Global Constraints

- 冲突检测仍要抓到：服务商不是 Ofox、base_url / key 被改、文件缺失。只放开 `model`。
- 不碰用户的 `model_reasoning_effort`（已有规则：不比对、不覆盖）。
- 先写测试再实现；每个 Stage 独立可提交、`cargo test`、`cargo clippy -- -D warnings`、`cargo fmt --check` 通过。
- 用户可见文案走 i18n；本计划预计不新增文案。
- 沿用 #45 的 key 流程（签发、保存、旧 key 排队删除），不另起一套。

## Review Focus（测试里要覆盖的边界）

1. 服务商被改成非 Ofox 时，仍然判「配置冲突」（放开模型不能放开这个）。
2. CLI 里模型被改成带/不带 `openai/` 前缀的值、或整行删掉：不冲突，且不被改回。
3. 换 key（#45 对账）、启动修复在模型被改过时能成功，并且配置里的模型保持用户选的。
4. 有 key 没记录、但配置已被改成非 Ofox 的工具：不重新签发，报告「配置已被修改」。
5. 没登录（取不到账号）时对账直接返回，不因为补记录去签发 key。
6. Codex 与 ChatGPT 桌面端共用 `~/.codex`：采纳模型对两者都成立，不能互相覆盖。

---

## Stage 1: 模型不再算冲突（Codex / Claude / Gemini）

**Goal**：这三个工具在 CLI 里换模型，状态仍是「已配置」。
**Files**：`src-tauri/src/services/ofox_bind/status.rs`（`inspect_managed_fields`）、`src-tauri/src/services/ofox_bind/tests.rs`
**Success Criteria**：换模型 → `Configured`；换服务商 / 删文件 → 仍 `Modified` / `Missing`。
**Tests**（先写，应先失败）：
- `codex_model_changed_in_cli_is_still_configured`
- `codex_provider_changed_is_still_modified`
- `claude_model_changed_is_still_configured`、`gemini_model_changed_is_still_configured`
**Change**：从比对列表里去掉 Codex 的 `["model"]`、Claude 的 `["env","ANTHROPIC_MODEL"]`、Gemini 的 `GEMINI_MODEL`。
**实施中发现**：两个旧测试原本断言「模型被改就是冲突」，已按新约定更新——`status_and_restore_keep_modified_or_invalid_configuration_and_hide_secrets` 改成用 base_url 制造冲突；`startup_existing_repair_accepts_only_unchanged_saved_managed_fields` 暂时去掉「只改模型」用例，Stage 2 用新约定补回（修复成功且保留用户模型）。
**⚠ Stage 1 不能单独合并 / 发布（Stage 2 已补上）**：放开比对后，`replace_key` 和 `bind_existing` 会通过检查，然后用旧模板重写配置，把用户在 CLI 选的模型改回去。必须和 Stage 2 一起合并。
**Status**: Complete（分支 `fix/cli-model-conflict`，Rust 1454 测试通过；待 Stage 2）

## Stage 2: 重写前采纳配置里的实际模型，主页显示一致

**Goal**：任何会重写配置的路径都不改回用户的模型；主页显示的就是配置里的模型。
**Files**：
- 新增 `src-tauri/src/services/ofox_bind/live_model.rs`：`read_live_model(tool) -> Result<Option<String>, String>`、`adopt_live_model(db, tool) -> Result<bool, String>`（只在与模板不同时更新数据库，返回是否更新）
- 改 `ofox_bind/mod.rs`：`replace_key`、`bind_inner(require_existing=true)` 在读模板之前调用 `adopt_live_model`（都在 `BIND_LOCK` 内）
- 改 `commands/manage_tool.rs`：`get_tool_binding_status`、`get_active_ofox_model` 在状态为已配置时调用；`write_model_into_settings` 改 `pub(crate)` 供复用（不把逻辑搬进 services，避免反向依赖）
**Success Criteria**：Codex CLI 换成 `gpt-6-luna` 后：状态已配置；`get_active_ofox_model` 返回 `gpt-6-luna`；`replace_key` / `bind_existing` 之后配置里仍是 `gpt-6-luna`。
**Tests**（先写）：
- `replace_key_keeps_the_model_the_user_chose_in_the_cli`
- `bind_existing_keeps_the_model_the_user_chose_in_the_cli`
- `adopt_live_model_updates_the_template_only_when_it_differs`
- `adopt_live_model_ignores_a_missing_model_line`（整行被删：不改模板）
- `gemini_model_chosen_in_the_cli_is_adopted`、`sync_live_model_adopts_only_while_the_binding_is_configured`
- 补回 `startup_existing_repair_accepts_only_unchanged_saved_managed_fields` 的「只改模型」用例（修复成功、文件内容不变）
**实施中与计划不同**：主页显示的模型靠 `get_tool_binding_status` 里的 `sync_live_model` 同步（ConsolePage 先取绑定状态、再取模型，顺序已确认），没有再改 `get_active_ofox_model`。未做命令层测试（需要 Tauri State），由 `sync_live_model` 的测试和本机实测覆盖。
**突变检查**：临时去掉 `replace_key` 和 `bind_existing` 里的采纳调用，Codex 换 key、Claude 换 key、启动修复三个测试会失败，确认测试有效。
**Status**: Complete（分支 `fix/cli-model-conflict`，Rust 1460 测试通过；Stage 1 + 2 可以一起合并）

## Stage 3: 没有 key 记录的工具，对账时补上

**Goal**：Codex / Hermes（以及将来同样状态的工具）自动补回 key 记录，「数据统计」和 key 标签恢复。
**Files**：`src-tauri/src/services/account_keys.rs`（`reconcile`、`replace`）、其 `tests`
**Change**：
- `reconcile` 在遍历已有记录之后，再遍历所有已绑定、钥匙串里有 key 但没有记录的工具，走 `replace`（先用旧 key 核对配置是已配置 → 签发新 key → 写入配置 → 保存 key 和记录）。
- `replace` 的 `old` 改成 `Option<&ApiKeyMeta>`：没有记录就没有 key id，旧 key 无法排队删除。
- 成功计入 `switched`（前端已有刷新逻辑，会重新读 key 标签和统计链接）。
**Success Criteria**：构造「已绑定 + 钥匙串有 key + 无记录」→ 对账后有记录且配置里是新 key；配置被改成非 Ofox → 不签发、`failed` 里报原因；未登录 → 不动。
**Tests**（先写）：
- `bound_tool_with_a_key_but_no_record_gets_a_new_recorded_key`
- `unrecorded_key_is_not_replaced_when_the_binding_was_modified`
- `unrecorded_key_is_left_alone_when_signed_out`
- `tool_that_is_not_bound_is_not_rekeyed`
**实施中与计划不同**：补记录的逻辑放进 `rekey_unrecorded`（`pub(crate)`，签发 key 的函数由参数传入，测试里用假签发，不需要网络），`replace` 改成泛型接收 `issue`；测试放在 `ofox_bind/tests.rs`，复用那里的临时 HOME 和数据库。配置已被改成别的的工具静默跳过（只写日志），不进 `failed`，否则每次启动都会弹一次提示。
**突变检查**：去掉「已有记录就跳过」→ `a_tool_that_already_has_a_record_is_not_rekeyed_as_unrecorded` 失败；去掉「配置被改就跳过」→ `an_unrecorded_key_is_left_alone_when_the_binding_was_modified` 失败。
**已知限制**：旧的那把 key 在服务器上仍有效（没有 key id，无法删除），用户可在控制台按名称 `codex on <host>` 删除。**待后端确认**是否有列出 key 的接口（`GET /openapi/api-keys`），有的话可按前缀匹配后自动删。
**Status**: Complete（分支 `fix/cli-model-conflict`，Rust 1465 测试通过；真实环境未验证，需要测试包）

## Stage 4: 其余工具（OpenClaw / Hermes / OpenCode）+ 统一冲突原则【已实现，待真机验证】

**原则（用户已确认）**：只要还在用 Ofox 的 API，在 agent 里做的事（换模型、改 effort、加 MCP / skills / 插件）都不算冲突。冲突 = 接入路径不再经 Ofox 或 key 不是绑定的那把。切到别的服务商的模型算冲突；用户把自己的 key 填进 Ofox 条目算冲突。采用方案 B：检测只看接入不变量 + 写入改成合并而不是覆盖。

**实测结论（隔离 HOME + 合成种子，2026-10-13；CLI 子命令已实测，TUI 操作只从程序字符串推断，标「未验证」）**
- 所有 agent 的 CLI 命令（加删 MCP、插件、feature、`config set`、`doctor --fix`、`config migrate` 等）都**没有改写 Ofox 条目本身**，也没有补默认值。Ofox 条目只会被用户主动编辑。
- 会变的只有「选中的路由」：Codex `model`（TUI `/model`）、Claude 顶层 `model`（未验证）、OpenClaw `agents.defaults.model.primary`（`models set`）、Hermes `model.default` / `model.provider`（`config set`）、OpenCode 顶层 `model`（`PATCH /global/config`）；OpenCode TUI 的选择存在 state/model.json，不在配置里（未验证）。
- 格式会被 agent 重排：Claude / Gemini / OpenCode（PATCH）整份重写缩进；OpenClaw 任何写入都转成严格 JSON 并**删除所有注释**，加 `meta.*`、留 `.bak`。所以比对必须按解析后的值。
- OpenClaw 校验要求 `models[].name` 必填，缺了会让 `config validate`、`models set`、`doctor --fix` 全部拒绝运行。
- `models set <新 id>` 不会把新 id 加进 provider 的 `models[]`；`agents.defaults.model` 字符串形式会被规整成 `{primary}`。
- 只有 `/auth`（Gemini）能合法改 `selectedType`，那是用户真的换了认证方式，仍算冲突。

**现状写入路径的问题（读代码）**
- OpenClaw `bound_config`：整条替换 `models.providers.ofox-openclaw`（丢掉用户加在条目里的字段），并**无条件**把 `agents.defaults.model.primary` 设成模板第一个模型（改回去）。
- Hermes `bound_config`：写条目并调用 `switch_defaults`，会把 `model.default` 改回模板的默认模型。
- OpenCode `bound_config`：整条替换 `provider.ofox-opencode`（丢掉用户加的模型和参数），不碰顶层 `model`。
- 所以 Stage 4 必须同时改检测和写入，否则放开检测后「换 key / 启动修复」会把用户选的模型改回去（和 Stage 1 的风险同类）。

**Stage 4a 检测（每个工具只比接入不变量）**
- Codex：`model_provider == ofox`；`[model_providers.ofox]` 的 `base_url`、`wire_api`、`requires_openai_auth`、key；其余字段（`http_headers` 等）不比。
- Claude / Gemini：不变（已是 base_url / key / selectedType）。
- OpenClaw：条目 `baseUrl`、`apiKey`、`api`；`agents.defaults.model`（字符串或 `{primary}` 两种写法都认）的服务商部分必须是 `ofox-openclaw`，模型 id 部分不比。
- Hermes：条目 `api`、`api_key`、`transport`；`model.provider == ofox-hermes`；`model.default`、条目里的 `models` / `default_model` 不比。
- OpenCode：条目 `options.baseURL`、`options.apiKey`；顶层 `model` 若存在，服务商部分必须是 `ofox-opencode`；不存在不算冲突。
**Stage 4b 写入（合并而不是覆盖）**
- 条目：只写接入字段，合并进现有条目，保留用户加的其它字段和模型。
- 选中的路由：已经指向 Ofox 时不动（保留用户选的模型）；没指向 Ofox（首次绑定、或用户确认重新绑定）才按模板设置。
- 主页显示的模型：OpenClaw / Hermes 读实际配置里选中的模型（不是模板列表第一个）。
**Status**: Complete（分支 fix/cli-model-conflict-stage4，叠在 Stage 1–3 上）。与设计的差异：OpenCode 不检查顶层 `model`（当前模型在 TUI 状态里，见 Fizzy #1048）；Codex 的 provider 表仍整表比较。

## 验证（全部 Stage 完成后）

- `pnpm typecheck`、`pnpm format:check`、`pnpm test:unit`
- `cargo test`、`cargo fmt --check`、`cargo clippy -- -D warnings`（均带 `--manifest-path src-tauri/Cargo.toml`）
- 本机实测：Codex 里 `/model` 换模型 → 主页无冲突、模型显示一致、「打开」按钮还在；重新登录后 Codex / Hermes 出现「数据统计」；两种 apex（ofox.ai / ofox.io）下新签发的 key 域名正确。
