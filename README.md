<div align="center">

<img src="src-tauri/icons/128x128@2x.png" alt="Ofox Desktop" width="96" height="96">

# Ofox Desktop

### 一个账号，打通所有 AI 编程工具

把 Claude Code、Codex、ChatGPT 桌面版、Gemini CLI、OpenCode、OpenClaw、Hermes、
WorkBuddy 的供应商配置收进一个桌面应用——登录一次，一键接入，不再手动改配置文件。

[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey.svg)](https://desktop.ofox.ai/latest.json)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

[下载](https://ofox.ai/desktop) · [官网](https://ofox.ai)

</div>

---

## 这是什么

每个 AI 编程工具都有自己的一套配置：Claude Code 读 `~/.claude/settings.json`，
Codex 和 ChatGPT 桌面版共用 `~/.codex/config.toml`，Gemini CLI 读 `.env` 加
`settings.json`，OpenCode 和 OpenClaw 各有各的 JSON/JSON5 方言，Hermes 用 YAML，
WorkBuddy 又是另一份模型列表。

想换个供应商——比如从官方订阅切到中转服务，或者在几家之间比价——就得挨个
翻文档、找配置路径、改对字段名、重启工具。多装几个工具，这事就从"有点烦"
变成"每次都要查"。工具本身还要装、要升级，有的还得先装 Node.js。

Ofox Desktop 把这些收敛成一次点击。它知道每个工具装在哪、配置文件在哪、字段
叫什么，然后原子化地写进去；没装的帮你装，有新版本帮你升。

## 核心能力

**一个账号覆盖所有工具**

用 Ofox 账号登录后，把任意工具"绑定"到 Ofox——应用会自动为该工具申请一把
专属 API key，写入它自己的配置格式。所有工具共用一个账号、一份额度，不用
在每个平台单独注册充值。

**自动检测，一键安装和升级**

自动发现本机已安装的工具。没装的在工具列表里一键安装，缺 Node.js 时先帮你
装好（macOS 通过 nvm，Windows 通过 winget）。有新版本时列表里直接显示
「升级到 vX」，按工具原来的安装方式升级（Homebrew、npm、pnpm 或工具自带的
更新命令）；多个工具可以一键全部升级。认不出安装来源的（如 Volta）只给手动
升级入口，不会另装一份。

**模型先测再换**

切换模型前先用一次流式请求实测兼容性，不兼容的模型不让选。OpenCode 会自动
选用实测能用的 Chat Completions 或 Responses 协议。

**解绑原样还原**

解绑时把接入方式（provider、base URL、key、认证方式、模型）还原成绑定前的
样子；绑定期间加的 MCP、技能、插件都保留，JSON/JSON5 里的注释不会丢。解绑前
先预览要改什么，解绑后展示结果。Codex 绑定不改 `auth.json`，ChatGPT 登录令牌
不会发往 Ofox。

**卸载不丢绑定**

工具被删掉后，绑定记录仍然保留，工具折叠到「未安装」分组，重新装上后绑定还在。
接入配置丢了可以一键恢复；你改过的字段不会被覆盖。

**环境变量冲突提醒**

发现会绕过 Ofox 配置的环境变量（例如系统级的 `GEMINI_API_KEY`），告诉你它设在
哪里、怎么删除。只显示变量名和来源，不显示值。

**区域自动选择**

每次启动按出口 IP 自动选用 `ofox.ai` 或 `ofox.io`，已绑定工具里由 Ofox 写入的
地址随之更新；你自己改过的地址不会被覆盖，而是提示冲突。手动选过区域后就固定
下来。

**原生供应商管理**

不想用 Ofox 也完全可以。应用本身就是个供应商配置管理器：为每个工具维护多套
配置（官方订阅、各家中转、自建网关），点一下切换，切换前自动快照当前配置，
随时能切回去。内置主流供应商预设，填个 key 就能用。

**配置直写，不做中间层**

凭据直接写进工具自己的配置文件，请求由工具直连供应商——Ofox Desktop 不代理
你的流量，关掉应用工具照常工作。这意味着没有额外延迟，也不存在"应用挂了工具
就不能用"的问题。

**凭据存系统钥匙串**

OAuth token 和 API key 存在 macOS 钥匙串或 Windows 凭据管理器里，不落明文
配置。debug 构建用独立的存储，和正式版隔离。

**其他**

- 工具健康检查——一键探测各工具当前配置是否真的能通
- 低余额提醒
- MCP 服务器配置管理
- 用量统计与成本核算
- WebDAV 多设备同步
- 托盘常驻，快捷切换

## 支持的工具

| 工具 | 配置文件 | 格式 |
| --- | --- | --- |
| Claude Code | `~/.claude/settings.json` | JSON |
| Codex | `~/.codex/config.toml` | TOML |
| ChatGPT 桌面版 | 与 Codex 共用 `~/.codex/config.toml` | TOML |
| Gemini CLI | `~/.gemini/.env` + `~/.gemini/settings.json` | dotenv + JSON |
| OpenCode | `~/.config/opencode/opencode.json` | JSON |
| OpenClaw | `~/.openclaw/openclaw.json` | JSON5 |
| Hermes | `~/.hermes/config.yaml`（Windows：`%LOCALAPPDATA%\hermes\config.yaml`） | YAML |
| WorkBuddy | `~/.workbuddy/models.json` | JSON |

Windows 上 `~` 指用户目录（`%USERPROFILE%`）。

## 安装

从 [ofox.ai/desktop](https://ofox.ai/desktop) 下载，或直接取最新版：

```bash
curl -s https://desktop.ofox.ai/latest.json
```

| 平台 | 安装包 | 说明 |
| --- | --- | --- |
| macOS 12+（Apple Silicon） | `.dmg` | 已签名 + 公证，下载即用 |
| Windows 10/11（x64） | `.exe` | 暂未代码签名，SmartScreen 提示时点「更多信息 → 仍要运行」 |

应用内「关于」页可检查更新。

## 开发

```bash
pnpm install
pnpm tauri dev          # 开发模式
pnpm tauri build        # 打包
```

前置要求：Node 20+、pnpm 10+、Rust stable。Windows 上另需 Microsoft C++
生成工具和 WebView2，见 [Tauri 前置要求](https://tauri.app/start/prerequisites/)。

其他命令：

```bash
pnpm typecheck          # TypeScript 类型检查
pnpm test:unit          # 前端单元测试
pnpm format             # 格式化
cargo test --manifest-path src-tauri/Cargo.toml    # Rust 测试
```

技术栈：Tauri 2 + React 18 + TypeScript + Tailwind，后端 Rust。

更多说明：

- [工具更新](docs/tool-updates.md)——各安装来源的检测与升级方式
- [绑定生命周期](docs/tool-lifecycle.md)——卸载、恢复与解绑的状态规则
- [Windows 测试](docs/windows-testing.md)——Windows 验证入口与手动验收清单

### 发版

正式发版由 GitHub Actions 完成——打 tag 触发构建：macOS 产出签名并公证的
`.dmg`，Windows 产出 `.exe` 安装包（暂未签名）。产物发布到 Cloudflare R2，
`latest.json` 按平台合并更新：

```bash
git tag <version>_$(date +%Y%m%d_%H%M)
git push origin --tags
```

版本号需在 `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`
三处保持一致，CI 会校验并在不一致时直接失败。

`scripts/release.sh` 是本地应急发版脚本，常规情况请用 CI。

## 许可

[MIT](LICENSE)

本项目基于 [cc-switch](https://github.com/farion1231/cc-switch)（MIT，
Copyright © 2025 Jason Young）开发，在其供应商切换能力之上增加了 Ofox
账号体系、工具绑定与健康检查等功能。Windows 上的命令行工具检测、安装与升级
移植自 [magpie](https://github.com/yetone/magpie)（MIT，Copyright © 2026
yetone）。感谢原作者的工作。
