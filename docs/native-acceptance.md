# 原生 Agent 验收运行说明

本文记录当前可用入口和证据边界。验收对象为 Claude Code、Codex、Gemini CLI、OpenCode、Hermes 的指令、MCP 和 Skills 消费行为；离线 fixture、配置写入成功、真实客户端消费是三种不同证据。遵守[开发规范](code-standards.md)和[接线说明](agent-wiring.md)，Windows 另按 [Windows 验收](windows-testing.md)执行。

## 当前状态

- [运行器](../scripts/native-acceptance.mjs)提供离线 fixture、隔离 profile 准备和五种固定客户端 driver。fixture 不调用模型或远端 MCP，不能作为原生通过结果。
- [测试专用导出器](../src-tauri/src/services/library/native_acceptance_profile.rs)通过真实资源库批量接口写入三类资源，导出文件哈希和操作收据；没有新增产品 IPC。
- 原生 CLI 默认返回 `blocked / serverCapVerifierUnavailable`，不启动客户端、不读取任何测试凭据。代码级 `runAcceptance()` 支持显式 operator 回调；没有自动生产预算 provider，也没有 manifest 或 CLI 解锁开关。
- 本轮人工已在官方控制台核验专用测试 Key：每日和累计上限均为 $10，累计限制启用 `notify_and_block`，核验时已用为零。这是真实人工核验，不能写成已有自动预算 provider。
- 公开文档仅明确 API Key 可读[账户余额](https://ofox.ai/zh/docs/api/openapi/balance)。控制台预算使用会话内 Server Action，消费查询使用会话内 analytics 接口；尚未取得专用 Key 自查询限额、原子预占或最终结算的公开契约。运行器不获取控制台 cookies 或生产 OAuth token。

## 1. 先跑离线证据链

在仓库根目录准备临时 manifest，所有输出留在私有临时目录。以下命令不运行 Agent，也不发送网络请求：

```sh
acceptanceDirectory=$(mktemp -d -t ofox-native-acceptance)
acceptanceSourceSha=$(git rev-parse HEAD)
node --input-type=module - "$acceptanceDirectory" "$acceptanceSourceSha" <<'NODE'
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
const [directory, sourceSha] = process.argv.slice(2);
const manifest = {
  schemaVersion: 1,
  sourceSha,
  mode: 'fixture',
  agents: ['claude', 'codex', 'gemini', 'opencode', 'hermes'],
  budgetLimitCents: 1000,
  maxRetries: 1,
  timeoutMs: 60000
};
writeFileSync(join(directory, 'fixture.json'), JSON.stringify(manifest), { mode: 0o600 });
NODE
node scripts/native-acceptance.mjs \
  "$acceptanceDirectory/fixture.json" "$acceptanceDirectory/fixture-report"
```

预期状态为 `fixturePassed`，`nativeAccepted=false`、`configurationWriterVerified=false`、费用 `notMeasured`。未提供真实产物证据时，`sourceSha` 为 null；`requestedSourceSha` 只表示输入值。

`summary.json` 记录结构化结果，`budget-ledger.json` 记录本次运行的预留与结算。SIGINT/SIGTERM 取消未开始的客户端并清理自建 profile。超时、陈旧 MCP 收据、输出超限和一次重试由[回归测试](../tests/guards/nativeAcceptance.test.ts)覆盖。

## 2. 准备由真实 writer 写入的 profile

复制 manifest 到临时目录，改为 `mode: "native"`，明确填写本次选择的各客户端 `models` 和 `ofoxApex`（仅 `ofox.ai` 或 `ofox.io`）。未填写模型会生成 `fixture-model`，只能用于准备测试，不能用于真实调用。manifest 不得包含 key、prompt、shell、command 或任意启动参数。

```sh
node scripts/native-acceptance.mjs prepare \
  "$acceptanceDirectory/native.json" "$acceptanceDirectory/prepared"
```

输出 `prepared/prepared-profiles.json`，其中各 `profiles[].root` 是系统临时目录下独立的根。准备阶段只生成 provider 配置、空工作目录及私有挑战材料，不伪造“已应用”的指令、技能或 MCP；挑战文本不是凭据，但不应分享。

待当前 Rust 大构建结束后，串行对每个准备根运行一次导出测试。`acceptanceProfileRoot` 必须取自本次 `prepared-profiles.json`，不能指向真实 home：

```sh
OFOX_NATIVE_PROFILE_ROOT="$acceptanceProfileRoot" \
  cargo test --manifest-path src-tauri/Cargo.toml --lib \
  services::library::native_acceptance_profile::export_native_acceptance_profile \
  -- --ignored --exact
```

该命令只在沙箱写入，不调用 Agent 或网络。它使用共享串行锁和 `test_sandbox`，实际调用资源库 preview/apply。导出器拒绝过期、根外软链接或被替换的准备材料。准备票据有效一小时；过期后重新准备，不能手改到期时间续签。

每个根生成 `writer-receipt.json`。将 root、收据路径及其实际 SHA-256 填入 manifest 的 `writerProfiles[agentId]`。profile 只能被原生运行消费一次；运行器不自动删除外部 writer 根，证据留存后由操作者清理这批临时目录。

## 3. 绑定源码、产物和客户端

真实模式还要求以下证据，不能用 fixture 字符串代替：

| 输入                   | 验证方式                                                                                                                                               |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `sourceSha`            | 与实际 Git HEAD 一致；checkout 必须干净，既有未跟踪 `HANDOFF-codex.md` 是唯一例外。                                                                    |
| `artifact`             | 显式指定产物绝对路径、SHA-256、provenance 路径及 SHA-256；provenance 的 `sourceCommit` / `sourceSha` 必须一致。使用真实构建证据，不手写假 provenance。 |
| writer 收据            | `sourceSha`、编译内嵌 `compiledSourceSha` 均匹配，`compiledSourceClean` 严格为 true；逐项核验三类操作及实际文件哈希。                                  |
| `executables[agentId]` | 固定客户端绝对路径及 SHA-256；不接受任意命令或 shell 包装器。                                                                                          |
| `versions[agentId]`    | 隔离环境执行固定 `--version`，与记录版本一致；Claude 使用 `--bare --version`。                                                                         |

[build.rs](../src-tauri/build.rs)自动从 Git 编译绑定源码身份。先提交最终代码、再从干净 checkout 构建导出器；脏源码编译出的收据不能通过，之后仅修改 manifest 的 SHA 也无效。源码归档缺少 Git 身份时不能声称获得相同等级的验证。

完整 `native.json` 包含 `schemaVersion`、`mode`、`sourceSha`、`agents`、`models`、`ofoxApex`、`artifact`、`executables`、`versions`、`writerProfiles`；路径和哈希仅留本机报告。当前直接运行仍安全地返回预算 blocked：

```sh
node scripts/native-acceptance.mjs \
  "$acceptanceDirectory/native.json" "$acceptanceDirectory/native-report"
```

## 4. 隔离与证据含义

运行器重建 HOME/USERPROFILE、各 Agent 目录、XDG、Windows AppData 和临时目录，剔除继承的 API key、proxy、SSH 和 Node 启动注入环境。只允许固定参数，禁用自动更新及已知遥测，不自动安装依赖。测试凭据只能来自显式专用测试存储，不自动查找生产 Keychain、生产配置或真实项目。

临时 HOME 不等于 macOS Keychain 隔离。Claude 当前使用 `--bare`，避免读取生产 Keychain；这会关闭默认 `CLAUDE.md` 自动发现，因此通过固定 CLI 文件注入并记录 `explicitCliFileNoKeychain`。其默认指令自动发现仍是未验证，需要独立 OS 身份或虚拟机验证，不能冒充通过。Hermes 若专用运行环境尚未就绪，应记录准确失败原因，不修复或重装真实用户环境。

Codex Skills 按产品实际 writer 目标写入 `.codex/skills`；不能把共享 `.agents/skills` 的结果替代该路径的原生消费。

| 证据                 | 可得结论                                                                             |
| -------------------- | ------------------------------------------------------------------------------------ |
| fixture 完成         | 固定离线 driver、取消、解析及本地 MCP 收据链工作；未验证真实 Agent。                 |
| writer 收据验证      | 此源码编译出的真实 OFox writer 在沙箱写入了对应文件；未证明 Agent 加载。             |
| 本地 `attest` 回执   | 原生进程确实通过 MCP 提交本次指令和 Skills 的随机挑战；不靠模型自述。                |
| Microsoft Learn 证据 | 固定公开 endpoint 的指定查询具有匹配的结构化 tool-use / result；与本地回执分别核验。 |
| 用量 / 费用          | 原生 Token 不是实际扣费证明；没有服务端费用证据时保持未知和预留。                    |

报告不保存原始 stdout/stderr、挑战全文或秘密。记录 OS/架构/Node、客户端实测版本、文件摘要、远端调用证据、失败原因和费用状态；macOS 结果不证明 Windows 通过。

## 5. 受监督官方控制台模式

本节是代码级入口，没有 CLI 开关。用户已授权本轮累计 $10，并已完成控制台金额限制设置；不重复询问相同授权，也不允许 manifest 的自填 quota JSON 解锁调用。官方 UI 的观察回调仍由受信任的操作者提供，不能把测试用伪造截图或合成回调用于真实执行。

入口为 `runAcceptance(manifest, { operator, outputDirectory, checkoutRoot })`。`operator` 必须是主线显式传入的函数对象，不能从 manifest 指定脚本、模块或 shell。默认没有 operator 时继续 blocked。它与要求自动服务端 reserve/settle/finalize 的 provider 分开，报告标记为 `supervisedOfficialConsole`。实现和完整类型见[监督模块](../scripts/native-acceptance-supervised.mjs)及[回调契约](../scripts/native-acceptance-supervised.d.mts)。

| 回调 / 配置                        | 实际约束                                                                                     |
| ---------------------------------- | -------------------------------------------------------------------------------------------- |
| `ledgerDirectory`                  | 显式私有绝对目录，跨 run 使用同一位置；每个 Key 独占 lease，不能通过新输出目录清空历史。     |
| `keyIdSha256` / `credentialSha256` | 分别绑定官方 Key ID 与专用测试凭据的摘要；从真实 UI 和本机专用存储获得，不能以名称匹配代替。 |
| `observe(context)`                 | 每次返回 context 全部字段原样，加入新鲜时间、官方截图及精确累计限额/已用 NanoDollar。        |
| `credentialFor(identity)`          | 仅从获授权的专用测试存储读取，并匹配摘要；不得自动读取生产 Keychain/OAuth/cookies。          |
| `close(context)`                   | 提供官方页面停用检查及截图，返回 `credentialDisabled`；不等于服务端最终结算证明。            |

context 包含 `phase`、`nonce`、`runId`、`callId`、`sourceSha`、`agentId`、`clientSha256`、两个 Key 摘要及 `ledgerSha256`。过期、乱序、另一客户端/源码/Key 或旧账本响应均拒绝。观察时间不得早于当前两分钟，有效期最多两分钟；截图使用显式 PNG 路径、实际 SHA-256 和无秘密查询参数的官方控制台 URL。截图是人工观察的支持材料，不是提供商签名。

1. **打开与绑定**：operator 通过官方页面核验同一专用 Key 的 ID、脱敏前缀、累计/每日上限、enabled、`notify_and_block` 与当前累计 used。截图摘要、来源 URL、核验时间与本次 SHA/run 绑定。专用 key 在本机私有存储计算摘要用于绑定，不写报告明文，不读取浏览器 cookies/OAuth。
2. **跨运行账本**：以专用 Key 摘要建立独占锁和持久账本；`runId + caseId` 唯一。记录 baseline、每次授权、启动、未知/已核费用和证据摘要。崩溃留下的 running 仍占用预留，不通过新目录或新 run 重置预算。
3. **每次授权一个 case**：`observe` 的 `beforeCase` 阶段接收新 nonce、run/case、源码/客户端/账本摘要。operator 刷新官方页面核验同一 Key 后，仅授权这一个固定 case；串行启动，下一个 case 必须取得下一份观察，不自动重试，不允许模型自选命令。
4. **核对实际结果**：`afterCase` 通过官方按 Key 过滤的日志核对实际费用和模型，返回 `caseActualNanoUsd` 与真实请求 ID 的摘要列表。费用须和本 case 前后的同 Key 累计差一致；旧请求摘要不能重复归入另一 case。无法确认则返回 null，保留剩余额度的预留并停止后续 case。监督账本使用精确 NanoDollar，本次运行的 cents 账本只作保守取整；本地预留不是服务端单次消费上界。
5. **收尾**：`close` 对当前已授权或历史未结项请求操作者停用专用 Key 并保存真实页面证据。每次新授权与旧关闭证据失效一起持久化，停用收据绑定最后一次授权；即使费用已记账后崩溃，重开也不能沿用旧的 verified。返回 `credentialDisabled=false` 或核验失败会明确报 `supervisedClosureUnverified` / 关闭核验失败，并在快照的 `closure` 中保留未确认状态；不能把中途保留启用说成已经关闭。仍不具备服务端在途为零/最终结算契约时，账单保持 `settlementPending`；两次稳定刷新也不等同最终性。不返回 `serverFinalized`，不因人工预算核验而设置整体 `nativeAccepted=true`。

该入口允许在已授权额度内推进真实行为证据，同时保留预算证据等级和结算边界。未知项目只能通过代码级 `openSupervisedAcceptance(...).reconcilePending()` 再次取得官方观察后核对；不得手改 ledger 清除预留。运行中断后的 authorizing/running 项提升为 pending。现存 lease 不会因超时自动解锁；先确认原进程已经停止并保留账本证据，再由维护者处理陈旧 lease，不能与仍在运行的验收并发。

当前入口已由[合成回归](../tests/guards/nativeAcceptanceOperator.test.ts)覆盖身份绑定、过期、重入、独占锁、跨 run 已知与未知费用、损坏账本、取消以及“未知费用只执行一个模拟 case”。回归不调用真实 Agent/模型；真实官方 GUI 回调接线、实际运行及独立评审结果应另记，不能直接把 `trustedBudgetProvider()` 替换成“返回 true”。

## 6. 验证与交付

维护这些入口后，先跑针对性回归，再执行仓库最终门禁：

```sh
pnpm exec vitest run tests/guards/nativeAcceptance.test.ts \
  tests/guards/nativeAcceptanceEvidence.test.ts \
  tests/guards/nativeAcceptanceOperator.test.ts \
  tests/guards/releaseArtifacts.test.ts tests/guards/releaseGate.test.ts
```

分享的报告只包含脱敏摘要和结果；私有 profile、测试 key、控制台身份和账本不进入源码包。交付逐客户端列出“配置写入、指令、Skills、本地 MCP、远端 MCP、实际费用”各自证据，注明 source SHA 与原生版本。Windows 源码验证包、Developer ID / 公证与升级结果另列，不用离线通过或 macOS 检查替代其验收。
