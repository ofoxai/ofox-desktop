<div align="center">

<img src="src-tauri/icons/128x128@2x.png" alt="Ofox Desktop" width="96" height="96">

# Ofox Desktop

### 一个账号，打通所有 AI 编程工具

把 Claude Code、Codex、Gemini CLI、OpenCode、OpenClaw、Hermes 的供应商配置
收进一个桌面应用——登录一次，一键切换，不再手动改配置文件。

[![Platform](https://img.shields.io/badge/platform-macOS-lightgrey.svg)](https://desktop.ofox.ai/latest.json)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

[下载](https://ofox.ai/download) · [官网](https://ofox.ai)

</div>

---

## 这是什么

每个 AI 编程 CLI 都有自己的一套配置：Claude Code 读 `~/.claude/settings.json`，
Codex 读 `~/.codex/auth.json` 加环境变量，Gemini CLI 读 `.env`，OpenCode 和
OpenClaw 各有各的 JSON/JSON5 方言，Hermes 还要 YAML 加 dotenv 两个文件配合。

想换个供应商——比如从官方订阅切到中转服务，或者在几家之间比价——就得挨个
翻文档、找配置路径、改对字段名、重启工具。多装几个工具，这事就从"有点烦"
变成"每次都要查"。

Ofox Desktop 把这件事收敛成一次点击。它知道每个工具的配置文件在哪、字段叫
什么、要不要环境变量兜底，然后原子化地写进去。

## 核心能力

**一个账号覆盖所有工具**

用 Ofox 账号登录后，把任意工具"绑定"到 Ofox——应用会自动为该工具申请一把
专属 API key，写入它自己的配置格式。六个工具共用一个账号、一份额度，不用
在每个平台单独注册充值。

**原生供应商管理**

不想用 Ofox 也完全可以。应用本身就是个供应商配置管理器：为每个工具维护多套
配置（官方订阅、各家中转、自建网关），点一下切换，切换前自动快照当前配置，
随时能切回去。内置主流供应商预设，填个 key 就能用。

**配置直写，不做中间层**

凭据直接写进工具自己的配置文件，请求由工具直连供应商——Ofox Desktop 不代理
你的流量，关掉应用工具照常工作。这意味着没有额外延迟，也不存在"应用挂了工具
就不能用"的问题。

**凭据存系统钥匙串**

OAuth token 和 API key 存在 macOS Keychain 里，不落明文配置。debug 构建用
独立的 service 名，和正式版隔离。

**其他**

- 工具健康检查——一键探测各工具当前配置是否真的能通
- 低余额提醒
- 一键安装未安装的 CLI 工具
- MCP 服务器配置管理
- 用量统计与成本核算
- WebDAV 多设备同步
- 托盘常驻，快捷切换

## 支持的工具

| 工具 | 配置文件 | 格式 |
| --- | --- | --- |
| Claude Code | `~/.claude/settings.json` | JSON |
| Codex | `~/.codex/auth.json` + env | JSON |
| Gemini CLI | `~/.gemini/.env` | dotenv |
| OpenCode | `~/.config/opencode/config.json` | JSON |
| OpenClaw | `~/.openclaw/openclaw.json` | JSON5 |
| Hermes | `~/.hermes/config.yaml` + `.env` | YAML + dotenv |

## 安装

从 [ofox.ai/download](https://ofox.ai/download) 下载，或直接取最新版：

```bash
curl -s https://desktop.ofox.ai/latest.json
```

当前提供 **macOS (Apple Silicon)** 构建，已签名 + 公证，下载即用。
Intel Mac、Windows、Linux 构建在计划中。

应用内「关于」页可检查更新。

## 开发

```bash
pnpm install
pnpm tauri dev          # 开发模式
pnpm tauri build        # 打包
```

前置要求：Node 20+、pnpm 10+、Rust stable。

其他命令：

```bash
pnpm typecheck          # TypeScript 类型检查
pnpm test:unit          # 单元测试
pnpm format             # 格式化
```

技术栈：Tauri 2 + React 18 + TypeScript + Tailwind，后端 Rust。

### 发版

正式发版由 GitHub Actions 完成——打 tag 触发构建、签名、公证，产物发布到
Cloudflare R2：

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
账号体系、工具绑定与健康检查等功能。感谢原作者的工作。
