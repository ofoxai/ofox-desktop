# Magpie 功能迁移测试说明

本轮开发在 `codex/magpie-tool-experience` 分支完成，包含模型快捷选择、模型 Profiles、会话中心及 Pi / DeepSeek Harness 接入。先完成用户验收，再决定正式发布；本分支不创建 release tag，也不更新正式更新源。

## 启动开发版本

在本分支 checkout 中执行：

```sh
pnpm install --frozen-lockfile
pnpm dev
```

开发版本使用 debug 凭据存储。工具配置仍是当前选择的真实配置目录；测试绑定、切模型、区域切换和解绑会按界面操作修改相应工具配置。需要隔离时，使用测试用户目录与独立工具配置目录，并检查设置页的应用数据目录。测试真实配置时，先退出已安装版本，避免两个进程同时管理同一配置。

macOS 本地测试包通过 `pnpm tauri build --debug --bundles app --ci --config '{"productName":"Ofox Desktop Test","bundle":{"macOS":{"signingIdentity":"-"}}}'` 生成，位于 `src-tauri/target/debug/bundle/macos/Ofox Desktop Test.app`。这只生成本地产物，不上传发行文件。

## 验收顺序

1. **快捷选择**：从工具行、托盘和管理弹窗修改模型，确认三处同步更新。单模型选择后保存；OpenCode / WorkBuddy 保留多选及顺序。取消选择器不写入。配置被 CLI 修改后，旧选择器保存应要求刷新。
2. **模型 Profiles**：保存当前组合、编辑名称和模型、预览、应用、撤销。确认未指定工具不改变，未安装或未绑定工具显示跳过，配置冲突阻止应用。OpenCode 的部分保存应显示具体跳过模型。仅安装 ChatGPT 时，共享的 Codex 默认仍可应用；ChatGPT 最近使用模型与配置默认分开显示。
3. **撤销边界**：应用后正常撤销只恢复模型。再应用一次，在 CLI 中修改模型或受管接入字段后撤销，应停止并保留后续更改。切换账号、区域或重新启动后，旧临时撤销令牌不可继续使用。
4. **Pi**：检测、安装或更新、绑定、启动、切模型、替换 Key、解绑。确认 `PI_CODING_AGENT_DIR` 的配置目录生效，已有 provider、扩展、enabledModels 的 glob / thinking 后缀和其他用户字段保留。老 npm 安装渠道应按原渠道更新。
5. **dsh**：检查 `DSH_HOME`、home / profile 的配置层和多 profile 范围。不同 profile 的模型应显示“多个配置”。应用 Profile 后，各受管 profile 一致；撤销恢复各自原模型。后来新增的 profile 不自动接管。`.env` 与 `.credentials.yaml` 中无关字段在模型、Key、区域操作后保留。
6. **两个 OFox 区域**：分别测试 `ofox.ai` / `ofox.io`、首次检测、切换区域及重启。直接检查外部工具实际写入的 API URL，再运行一个真实请求。自定义或用户修改的接入字段应显示冲突。
7. **会话中心**：切换工具 / 项目分组、元数据搜索、正文、Markdown 导出和续聊。Pi 读取当前 JSONL 分支，并从原项目目录 `pi --session`；旧会话使用会话内模型。dsh 首版显示元数据，正文 / 导出 / 通用续聊保持禁用。
8. **回收站**：对 Claude、Codex、Gemini、Pi、dsh 的非活跃会话移入、恢复及永久清理。确认附属文件一并恢复；目标已占用时不覆盖。Gemini 同时覆盖 JSON / JSONL 与子 Agent。dsh 覆盖全部 generation、子 Agent 和独立 projection 行。共享 SQLite、索引和其他会话保持原状。

macOS 和 Windows 使用相同验收顺序。真实请求与桌面 CLI 续聊需要已安装工具和可用账号；自动化 fixture 通过不代替这些实测。

## 自动化检查

```sh
pnpm typecheck
pnpm format:check
pnpm test:unit
pnpm build:renderer
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

后端测试使用临时目录和内存数据库，覆盖配置保留、协议、两个 apex、Key 失败回滚、模型提交版本、分段应用与撤销、会话分支、回收站中断恢复及禁止覆盖。Windows CI 另外运行原有 `scripts/test-windows.ps1` 验证入口。

Magpie 的具体源码和 fixture 映射见 [实施计划](superpowers/plans/2026-10-14-magpie-tool-experience.md)。移植的原生适配器和会话模块各保留 MIT 许可文件。
