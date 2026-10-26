<div align="center">

<img src="src-tauri/icons/128x128@2x.png" alt="Ofox Desktop" width="96" height="96">

# Ofox Desktop

### 一个入口管理 AI 工具、会话、用量与资源

连接 Ofox 账号，管理本机 AI 工具的安装、接入和模型设置；
查看会话与用量，将 MCP、Skills 和指令分发到支持的应用。

[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey.svg)](https://github.com/ofoxai/ofox-desktop/releases/latest)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

[官网下载](https://ofox.ai/desktop) · [GitHub Releases](https://github.com/ofoxai/ofox-desktop/releases/latest) · [官网](https://ofox.ai)

</div>

---

## 主要入口

| 页面       | 可以做什么                                                     |
| ---------- | -------------------------------------------------------------- |
| **工具**   | 检测本机安装、安装或更新工具、绑定 Ofox、管理模型与接入状态    |
| **会话**   | 按项目查看本地会话，搜索、导出、续聊，使用可恢复的回收站       |
| **用量**   | 查看账户余额、本地请求和会话统计，按工具、项目、模型与日期筛选 |
| **资源库** | 管理指令、MCP 和 Skills，发现原生配置，预览并分发到多个应用    |

不同工具、版本和操作系统支持的操作有所不同，应用会显示当前可用动作和不可用原因。

## 工具与模型

支持 Claude Code、Codex、ChatGPT 桌面版、Gemini CLI、OpenCode、OpenClaw、
Hermes、WorkBuddy、DeepSeek Harness（dsh）和 Pi。

- **检测与安装管理**：区分已安装、未安装和检测异常，按实际安装来源提供更新、修复或手动处理入口。支持自动更新的工具可以逐个或批量更新；ChatGPT 等桌面客户端按其原生更新方式处理。
- **Ofox 接入**：登录后为绑定工具管理专属 API Key，并按各工具的原生格式配置接入。退出登录、切换账号、卸载工具和解除绑定是不同操作，详见[绑定生命周期](docs/tool-lifecycle.md)。
- **模型状态**：区分写入配置的默认模型与工具实际观察到的模型。支持保存和预览多工具模型方案；已有会话、启动参数及工具自己的模型选择仍可能影响实际使用的模型。
- **共享配置**：Codex 与 ChatGPT 的 Codex 模式共用配置目标。ChatGPT 内已有会话的模型由客户端决定；dsh 新会话默认模型与已有会话也分别处理。
- **恢复与解绑**：配置缺失时提供显式恢复；解绑先预览，再按 Ofox 管理的字段与当前文件状态恢复。能够识别的用户改动、共享配置持有者和其他资源会参与检查，冲突会说明原因。
- **区域选择**：支持自动探测和手动选择 `ofox.ai` / `ofox.io`。启动和区域切换时协调 Ofox 管理的地址，用户自定义地址出现冲突时保留并提示。

工具安装与绑定状态分别记录。卸载工具不会自动解除绑定，重新安装和恢复接入也分别提供操作。
实际配置目录支持应用设置和部分原生环境变量覆盖，见[各工具配置与数据来源](docs/agent-wiring.md)和[更新方式](docs/tool-updates.md)。

## 资源库

### MCP

以应用矩阵查看服务器的分发状态，支持表单、命令和 JSON 配置输入、原生配置发现、
导入比较、批量分发与撤回。修改会先预览并核对当前配置；同名冲突、来源变化和部分失败分别展示。

目标包括 Claude Code、Codex / ChatGPT、Gemini CLI、OpenCode、OpenClaw、Hermes、
WorkBuddy、dsh 和 Pi。Codex / ChatGPT 共用一个物理配置目标；各客户端的传输协议、
版本要求及配置范围由适配器检查。

收录到资源库、写入应用配置与连接成功分别显示。MCP 连接检查需要显式执行，
客户端的信任和 OAuth 授权仍在对应客户端完成。

### Skills

- 从 GitHub 仓库、skills.sh、ZIP、本地目录或应用中发现并收录 Skills。
- 在已收录技能与在线发现之间切换，按名称、描述、仓库和来源路径搜索。
- 选择多个应用分发，查看共享目录、已应用、待处理及内容不同的副本。
- 同名不同内容先比较；选择替换时保留备份，也可以保留应用自己的版本。
- 显式检查更新，处理备份、恢复和部分失败重试；本地修改不会因一次更新检查被自动覆盖。

### 指令与资源包

管理可编辑指令及模板，预览应用后的文件变化。支持选中资源的批量应用、撤回与资源包导入导出，
并提供本机批次历史和恢复检查。恢复会重新核对当前状态，过期计划不能直接重放。

完整的目标范围、配置格式与边界见[资源库说明](docs/library-workbench.md)。

## 会话与用量

**会话**提供项目与会话列表、Markdown 导出、续聊入口及可恢复的回收站。
操作能力取决于对应工具的本地记录；缺失、只支持元数据或无法确认的状态会明确显示。

**用量**将账户信息与本地记录分别呈现，提供概览、请求明细与会话分析。
请求面板包含汇总、分组图表、排名、日期选择、请求表格和 CSV 导出，支持按工具、
项目、模型等维度筛选。请求面板的 Token 合计包含输入、输出、缓存读取和缓存写入；
其他视图保留各自标明的统计口径。

本地统计依赖原生记录的覆盖范围。缺失证据、未知价格和部分覆盖分别标注；
价格估算与服务端账单分别展示。详细口径见[用量契约](docs/usage-analytics-contract.md)和[请求面板说明](docs/usage-requests-alignment.md)。

## 配置与凭据

- 模型请求由工具直接发送到其配置的服务端，Ofox Desktop 负责配置、管理与本地统计，不代理这些请求。
- Ofox 保存的登录令牌和工具密钥使用系统凭据存储，开发版与正式版隔离。向工具提供的 API Key 仍需按其原生格式写入本机配置文件。
- 配置改动使用文件事务与冲突检查，保留不属于本次操作的字段；跨文件操作可能部分完成，结果会逐项显示。
- 环境变量冲突会给出来源和处理入口。WebDAV 同步、数据库备份与恢复可在设置中管理。

具体行为与已知边界见[工具接线](docs/agent-wiring.md)和[已知限制](docs/known-limitations.md)。

## 安装与更新

从[官网下载页](https://ofox.ai/desktop)或 [GitHub Releases](https://github.com/ofoxai/ofox-desktop/releases/latest) 取得正式安装包。

| 平台                     | 安装包 | 说明                                                                        |
| ------------------------ | ------ | --------------------------------------------------------------------------- |
| macOS 12+，Apple Silicon | `.dmg` | 正式发布包含代码签名与 Apple 公证                                           |
| Windows 10/11，x64       | `.exe` | NSIS 安装包；暂未代码签名，SmartScreen 提示时可在“更多信息”中选择“仍要运行” |

应用顶栏会提示新版本，也可从设置检查更新。各 AI 客户端仍有自己的系统与版本要求。
GitHub Release 附带 `SHA256SUMS.txt`，可用于核对安装包；线上更新清单为：

```bash
curl -fsSL https://desktop.ofox.ai/latest.json
```

<details>
<summary>从早期版本升级后的 macOS 钥匙串授权</summary>

从旧签名版本升级时，macOS 可能要求新版 Ofox Desktop 重新获得已有钥匙串条目的读取权限。
这类提示涉及已保存的登录凭据和工具密钥，可能出现多次。

确认是当前 Ofox Desktop 的升级提示后，按系统要求输入密码并选择“始终允许”。
若之前拒绝了授权，可退出应用后重新打开再处理。后续是否再次提示取决于签名和系统授权状态。

</details>

## 开发

需要 Node.js 20.19+（20.x）或 22.12+、pnpm 10 和 Rust stable。
CI 使用 Node.js 20 与 pnpm 10.12.3。Windows 还需要 Microsoft C++ 生成工具和 WebView2；
平台依赖见 [Tauri 前置要求](https://tauri.app/start/prerequisites/)。

```bash
pnpm install --frozen-lockfile
pnpm dev                  # 启动完整 Tauri 开发应用
pnpm dev:renderer         # 仅启动 renderer，端口 3011
pnpm build:renderer       # 构建网页资源
pnpm build                # 构建桌面应用
```

检查命令：

```bash
pnpm typecheck
pnpm format:check
pnpm test:unit
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

测试配置、凭据与会话写入使用临时沙箱。平台专属行为需要在原生系统验证，
macOS 结果不能代替 Windows；开发约定见 [AGENTS.md](AGENTS.md)和[代码标准](docs/code-standards.md)。

### 发版

正式发布使用 [Release workflow](.github/workflows/release.yml)，产物上传到 Cloudflare R2 和 GitHub Releases，
再按平台合并更新 `latest.json`。

1. 同步 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` 中应用自身的版本，以及 `src-tauri/tauri.conf.json`。
2. 完成检查，并取得同一最终提交的独立 Windows Probe 成功记录。PR 合并引用的验证不能代替源提交的 push / 手动验证。
3. 在发布环境允许的分支上运行 `release.yml` 的 `dry_run=true`。确认提交 SHA 一致，等待完整检查、构建、签名、公证和安装包验证全部通过；遵守 GitHub 发布环境审批。
4. 在该已验证提交上创建 `<version>_YYYYMMDD_HHMM` 标签，只推送这一个标签，触发正式发布。
5. 核对 GitHub Release、两个平台的安装包校验值，以及线上 `latest.json`。

签名和公证也属于 dry-run 验证。普通分支的预览安装包只用于测试，不能作为正式公证包分发，
详见[分支安装包](docs/branch-packages.md)和[发布门禁](docs/code-standards.md#release-gate)。
`scripts/release.sh` 仅供 CI 不可用时应急使用。

## 许可

[MIT](LICENSE)

本项目基于 [cc-switch](https://github.com/farion1231/cc-switch)（MIT，Copyright © 2025 Jason Young）开发，
在其基础上增加了 Ofox 账号、工具管理、资源库和会话用量等能力。
Windows 命令行工具的检测、安装与升级移植自 [magpie](https://github.com/yetone/magpie)
（MIT，Copyright © 2026 yetone）。感谢原作者的工作。
