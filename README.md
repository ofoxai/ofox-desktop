# Ofox Desktop 测试安装包

这个分支只存放手动测试用的安装包，不是正式发布：不在 Releases 里，也不会通过应用内自动更新推送。

## 获取

```bash
git clone --branch test-builds --single-branch --depth 1 https://github.com/ofoxai/ofox-desktop.git ofox-test-builds
```

## 1.3.6-b54279e（所有可绑定工具都有用量统计 + 按 magpie 改版，仅 macOS，待验证）

源码：`feat/usage-all-tools` @ `b54279e9`。它等于正式版 1.3.6（main `963a728f`）加上还没合并的四个分支，依次是 `feat/magpie-ux` → `feat/session-subagents` → `fix/library-mcp-detection` → `feat/usage-all-tools`，共 314 个提交。不含 `fix/codex-mcp-type`。应用里显示的版本是 1.3.6，和正式版同号。

| 分支 | 内容 |
|---|---|
| `feat/usage-all-tools` | 用量统计覆盖所有可绑定的工具。Hermes 不再只有「Hermes 累计用量」弹窗，而是和其它工具一样进入概览、明细和会话分析：每个会话按模型的累计记在它最后活跃的那天（和 magpie 一致），项目取会话所在目录，活跃时长按消息时间计算。已绑定的工具都有用量标签，还没用过的显示 0；ChatGPT 的 Codex 模式记在 Codex 下。 |
| `fix/library-mcp-detection` | 资源库：MCP 改成表格，每个工具一列勾选框，识别各工具里已有的 MCP 时和 magpie 一致，应用自带的 MCP 不再列出；Skills 页按 magpie 改版（查找技能、按来源分组、每个工具一个开关，包括 Claude Desktop 的 Cowork）。 |
| `feat/session-subagents` | 会话：显示子代理（Codex、Claude、OpenCode），会话用量与 magpie 对齐，列表加载更快；工具页的列对齐；托盘弹窗改版（工具 / 用量两栏，Codex、Claude、Hermes 可调推理强度）。 |
| `feat/magpie-ux` | 按 magpie 改版：工具 / 会话 / 资源库 / 用量四个页面，会话管理带 Tokens 和费用，用量页带余额趋势，支持卸载工具。 |

| 文件 | 平台 | 说明 |
|---|---|---|
| `ofox_desktop_1.3.6_macos_aarch64.dmg` | macOS 12+，Apple Silicon | 公司证书（NICE TALK PTE. LTD.）签名，已通过 Apple 公证并附上公证票据，双击就能打开（离线也可以） |

这次没有 Windows 安装包。安装前先从菜单栏 / 托盘弹窗里点「退出 Ofox」。校验文件 `1.3.6-b54279e/SHA256SUMS.txt`。

用量统计请这样验：

1. **标签**：用量 → 会话分析，时段选「全部」。所有已绑定的工具都有标签，没用过的数值为 0；没有单独的 ChatGPT 标签。筛选行里原来的「Hermes 累计用量」按钮没有了。
2. **Hermes**：在某个项目目录里运行 `hermes -z "回复 OK"`，回到 Ofox 点刷新。Hermes 标签下出现这次会话，项目是刚才的目录，有活跃时长；Tokens 和 magpie「用量 → 会话」里 Hermes 的数字一致。明细里这条记录的来源是「本机会话导入」，鼠标悬停说明它是「一个会话在一个模型上的累计，不是单次请求」。
3. **会话继续**：在 Hermes 里继续同一个会话再发一句（交互模式 `hermes --resume <会话 ID>`），刷新后明细里还是那一条，Tokens 变大，不会多出一条。打包时在本机试过 `hermes --resume <会话 ID> -z …`，Ofox 返回 HTTP 405（上游服务商报错，和这次改动无关）；遇到就跳过这一步。
4. **OpenClaw、Pi**：绑定后各运行一句（`openclaw agent --local -m "回复 OK"`、`pi -p "回复 OK"`），刷新后对应标签有数字。
5. **重启**：退出再打开 Ofox，各项数字不变。

## 1.3.4-adfc4d6（退出登录停用 API Key，换账号后工具改用当前账号的额度；已随正式版 1.3.5 发布，macOS 已被 1.3.6-b54279e 取代）

源码：临时集成分支 `ci/test-installers` @ `adfc4d6a` = 1.3.4-22022d0 + PR #45。应用里显示的版本仍然是 1.3.4。

| PR | 内容 |
|---|---|
| #45 | 退出登录时，在服务器上吊销本账号在这台电脑上用的 API Key，已绑定的工具暂停使用；登录后（不管是不是同一个账号），已绑定的工具自动换成当前账号新签发的 key，只换 key、其它设置不动。别的账号的 key、已停用的 key 不再被复用（Fizzy #1041）。 |
| #40、#41、#42、#44 | 没有变化，说明见下面的各个版本。 |

| 文件 | 平台 | 说明 |
|---|---|---|
| `ofox_desktop_1.3.4_macos_aarch64.dmg` | macOS 12+，Apple Silicon | 公司证书（NICE TALK PTE. LTD.）签名，已通过 Apple 公证并附上公证票据，双击就能打开（离线也可以） |
| `ofox_desktop_1.3.4_windows_x64-setup.exe` | Windows 10/11 x64 | NSIS，未签名，默认按当前用户安装 |
| `ofox_desktop_1.3.4_windows_x64_en-US.msi` | Windows 10/11 x64 | MSI，仅用于验证能否安装，不会发布 |

安装前先从菜单栏 / 托盘弹窗里点「退出 Ofox」。校验文件 `1.3.4-adfc4d6/SHA256SUMS.txt`。

需要两个 Ofox 账号：A 有余额，B 没有余额（并且和 A 不在同一个组织——同一组织共用余额，换账号不会换 key）。macOS 和 Windows 都请按下面走一遍：

1. **A 登录并绑定**：用 A 登录，绑定 Claude Code、Codex 等工具，在终端里对话正常。记下主页工具行显示的 key 标签。
2. **退出登录**：设置 → 退出登录，提示「已退出登录。已绑定的工具暂停使用，重新登录后自动恢复。」。这时在终端里用 Claude Code 对话应提示鉴权失败（key 已在服务器上吊销）；在 A 的 Ofox 控制台里，这台电脑的这些 key 已经不在。
3. **B 登录**：进入主页后提示「Claude Code、Codex 已改用 b@… 的额度。」，工具行的 key 标签变了。在终端对话：B 没有余额，应提示余额不足——不再用 A 的钱。管理窗口里「测试连通性」显示「余额不足」。
4. **B 新加一个工具**：用的是 B 的 key（不会拿到以前 A 留下的 key）。
5. **再换回 A**：退出 B、用 A 登录，主页提示工具改用 A 的额度，对话恢复正常。
6. **配置被手动改过**：先在某个工具的配置里手动改一个 Ofox 管理的字段（例如 `~/.claude/settings.json` 里的 `ANTHROPIC_BASE_URL`），再退出、换账号登录：这个工具不会被覆盖，主页提示「xxx 还在用之前账号的 API Key：接入配置已被修改……」。其它工具照常换。
7. **断网退出**：断开网络后退出登录，额外提示「xxx 的 API Key 没能在服务器上停用（可能没联网）……」。联网后用同一个账号登录，工具换上新 key，旧 key 在服务器上被删掉。
8. **切换地区不受影响**：在设置里切换 ofox.ai / ofox.io 后重新登录同一个账号，工具照常可用，不会提示换 key。

其余检查项同下面的 1.3.4-22022d0。

## 1.3.4-22022d0（升级 Node.js 时按原版本装回全部全局工具，已被 1.3.4-adfc4d6 取代）

源码：临时集成分支 `ci/test-installers` @ `22022d0b` = 1.3.4-7ae801b + PR #41 的两个新提交（`b765425f`、`3be87a07`）。应用里显示的版本仍然是 1.3.4。

| PR | 和 1.3.4-7ae801b 相比 |
|---|---|
| #41 | 升级 Node.js 后，把旧版本下**所有**全局安装的工具按**原来的版本**装回新版本，不再只重装 Ofox 认识的 4 个、也不再顺手升级到最新版。只跳过 Node.js 自带的 npm、corepack。弹窗第 2 步写成「把原来的 N 个全局工具装到新版本（版本不变）」，下面列出全部工具。 |
| #40、#42、#44 | 没有变化，说明见下面的 1.3.4-7ae801b 和更早的版本。 |

| 文件 | 平台 | 说明 |
|---|---|---|
| `ofox_desktop_1.3.4_macos_aarch64.dmg` | macOS 12+，Apple Silicon | 公司证书（NICE TALK PTE. LTD.）签名，已通过 Apple 公证并附上公证票据，双击就能打开（离线也可以） |
| `ofox_desktop_1.3.4_windows_x64-setup.exe` | Windows 10/11 x64 | NSIS，未签名，默认按当前用户安装 |
| `ofox_desktop_1.3.4_windows_x64_en-US.msi` | Windows 10/11 x64 | MSI，仅用于验证能否安装，不会发布 |

安装前先从菜单栏 / 托盘弹窗里点「退出 Ofox」。校验文件 `1.3.4-22022d0/SHA256SUMS.txt`。

在 macOS 上请看（fnm 默认 Node.js 24.15.0）：

1. 先记下现在的版本：终端运行 `npm ls -g --depth=0`。
2. 在 Ofox 里安装或修复 OpenClaw，弹窗第 2 步是「把原来的 N 个全局工具装到新版本（版本不变）」，下面列出 Claude Code、Codex、Gemini、OpenCode 和其它全局工具（例如 @augmentcode/auggie、@larksuite/cli、pnpm），不含 npm、corepack。展开「查看要运行的命令」，每个包都写着具体版本号，例如 `'@anthropic-ai/claude-code@2.1.296'`，没有 `@latest`。
3. 点「升级并安装」，完成后新开终端：`node -v` 是新的 24.x；`npm ls -g --depth=0` 里原来的工具都在、版本和第 1 步一样，另外多了 openclaw；`auggie`、`lark-cli`、`pnpm` 等命令都还能用。
4. Claude Code、Codex 里配置的 MCP（例如 augment-context-engine）照常可用；Claude Code、Codex 的登录状态和配置不变。
5. 想退回原来的 Node.js：`fnm default 24.15.0`。

在 Windows 上请看：用 nvm-windows 或 fnm 管理的旧版 Node.js 时安装 OpenClaw，弹窗同样列出要装回去的全局工具，升级后它们都还在、版本不变。官方安装包装的 Node.js 换版本不影响全局工具，所以没有这一步。

其余检查项同下面的 1.3.4-7ae801b。

## 1.3.4-7ae801b（工具卡片对齐 + 模型列表自动加载 + 去掉兼容性确认 + 余额不足提示，已被 1.3.4-22022d0 取代）

源码：临时集成分支 `ci/test-installers` @ `7ae801be` = 1.3.4-cb770ee + PR #44。应用里显示的版本仍然是 1.3.4。

| PR | 内容 |
|---|---|
| #44 | 1. 添加工具窗口里同一行的卡片等高，「安装」按钮不再跑到卡片外面；桌面应用把「桌面应用」和版本号分两行显示。2. 管理窗口打开时自动加载模型列表，去掉刷新按钮；加载失败时显示「模型列表加载失败 · 重试」。3. 去掉「流式兼容性 · 暂时无法确定 · 我了解风险」：Codex / ChatGPT 换模型不再检测；其它工具只有确定不兼容才拦保存，查不出结果（余额、网络等）照常保存。4. 余额为 0 时连通性测试显示「余额不足」和「去充值」，不再显示 `HTTP 402` 和 JSON。 |
| #40、#41、#42 | 没有变化，说明见下面的 1.3.4-cb770ee 和 1.3.4-49b2dab。 |

| 文件 | 平台 | 说明 |
|---|---|---|
| `ofox_desktop_1.3.4_macos_aarch64.dmg` | macOS 12+，Apple Silicon | 公司证书（NICE TALK PTE. LTD.）签名，已通过 Apple 公证并附上公证票据，双击就能打开（离线也可以） |
| `ofox_desktop_1.3.4_windows_x64-setup.exe` | Windows 10/11 x64 | NSIS，未签名，默认按当前用户安装 |
| `ofox_desktop_1.3.4_windows_x64_en-US.msi` | Windows 10/11 x64 | MSI，仅用于验证能否安装，不会发布 |

安装前先从菜单栏 / 托盘弹窗里点「退出 Ofox」。校验文件 `1.3.4-7ae801b/SHA256SUMS.txt`。

macOS 和 Windows 都请看（两边是同一套代码）：

1. **添加工具的卡片**：主页点「添加」。同一行的卡片一样高；没安装的工具，「安装」按钮在卡片里面的右下角；ChatGPT 这类桌面应用显示「桌面应用」和「v版本号」两行。
2. **模型列表自动加载**：打开任意已绑定工具的「管理」。模型选择框右边没有刷新按钮；点开下拉框直接就是模型列表。断网后再打开管理窗口，选择框下面显示「模型列表加载失败 · 重试」；联网后点「重试」能加载出来。
3. **不再要求确认风险**：
   - ChatGPT / Codex 换一个模型：没有「流式兼容性」框，「保存」可以直接点；保存后 `~/.codex/config.toml`（Windows 是 `%USERPROFILE%\.codex\config.toml`）里的 `model` 是新模型，`wire_api` 仍是 `responses`。
   - OpenClaw、OpenCode、Claude Code 等换模型：最多短暂显示「正在确认这个模型能否使用…」，然后就能保存；没有「我了解风险」勾选框，也没有手动选协议。
   - 余额为 0 时也能正常保存模型。
4. **余额为 0 的提示**：在管理窗口点「测试连通性」，显示「余额不足 · OFOX 账户余额不足，充值后即可使用。」。owner / admin 账号有「去充值」，点了打开当前地区（ofox.ai 或 ofox.io）的钱包页；member 账号显示「请联系团队管理员充值。」。
5. 其余检查项（Node.js 升级弹窗、钥匙串、切回窗口）同下面的 1.3.4-cb770ee。

## 1.3.4-cb770ee（Node.js 升级弹窗新样式，已被 1.3.4-7ae801b 取代）

源码：临时集成分支 `ci/test-installers` @ `cb770ee8` = 1.3.4-49b2dab + PR #41 的新提交 `b505d230`。#40、#42 和 49b2dab 相同。应用里显示的版本仍然是 1.3.4。

| PR | 和 1.3.4-49b2dab 相比 |
|---|---|
| #41 | Node.js 升级弹窗改版（Fizzy #1021）：上面是「当前版本 → 最低要求」的对比，下面按顺序列出 Ofox 要做的几步（升级 Node.js → 重新安装其它工具 → 安装 OpenClaw），并说明为什么要重装其它工具。完整命令折叠在「查看要运行的命令」里，命令里也包含了重装步骤。Ofox 没法代为升级时，提供「打开 Node.js 官网」按钮。按钮改为 Ofox 的橙色。 |
| #40、#42 | 没有变化，说明见下面的 1.3.4-49b2dab。 |

| 文件 | 平台 | 说明 |
|---|---|---|
| `ofox_desktop_1.3.4_macos_aarch64.dmg` | macOS 12+，Apple Silicon | 公司证书（NICE TALK PTE. LTD.）签名，已通过 Apple 公证并附上公证票据，双击就能打开（离线也可以） |
| `ofox_desktop_1.3.4_windows_x64-setup.exe` | Windows 10/11 x64 | NSIS，未签名，默认按当前用户安装 |
| `ofox_desktop_1.3.4_windows_x64_en-US.msi` | Windows 10/11 x64 | MSI，仅用于验证能否安装，不会发布 |

安装前先从菜单栏 / 托盘弹窗里点「退出 Ofox」。校验文件 `1.3.4-cb770ee/SHA256SUMS.txt`。

在 macOS 上请看：

1. **确认装的是新包**：弹窗标题应该是「安装 OpenClaw 前需要升级 Node.js」，下面有版本对比卡片，按钮是橙色的。如果还是「OpenClaw 需要更新的 Node.js」和蓝色按钮，说明装的还是旧包。
2. **钥匙串不再弹（#1022）**：从 1.3.4-49b2dab 或正式版 1.3.4 换成这个包，都是同一张公司证书签名，打开后不应弹钥匙串授权，账号和已绑定的工具照常显示。
3. **Node.js 太旧时说清原因（#1021）**：fnm 默认 Node.js 是 24.15.0 时，在 Ofox 里安装或修复 OpenClaw，弹窗显示：当前版本 24.15.0（由 fnm 管理）→ 最低要求 24.16.0；三步分别是「用 fnm 升级 Node.js」「重新安装 Claude Code、Codex、Gemini、OpenCode」（附原因：配置和登录不受影响）「安装 OpenClaw」。展开「查看要运行的命令」，能看到完整命令和 OpenClaw 支持的版本范围 `>=24.16.0 <25 || >=26.1.0`。
4. 点「取消」：不安装，也不改 Node.js。
5. 点「升级并安装」：终端里依次升级 Node.js、重装上面那几个工具、安装 OpenClaw；完成后 Ofox 里 OpenClaw 显示已安装，Claude Code、Codex、Gemini CLI、OpenCode 仍能正常打开和对话。想退回原来的 Node.js，可以运行 `fnm default 24.15.0`。
6. 安装失败时终端窗口保留，Ofox 显示具体原因，不再出现「请手动运行 npm install」。
7. 拒绝钥匙串授权后的提示（#1022）和切回窗口（#40）：测法同下面 1.3.4-49b2dab 的第 2 条和第 7 条。

在 Windows 上请看：

1. 用官方安装包装的旧版 Node.js（例如 22.x）时安装 OpenClaw：弹窗显示版本对比，第 1 步「用 winget 升级 Node.js」下面注明需要管理员授权。同意后 PowerShell 窗口里先升级 Node.js，再安装 OpenClaw。nvm-windows 管理的 Node.js 同样会提示需要管理员授权，并在第 2 步列出要重装的工具。
2. 切回窗口不再整页转圈。
3. 其余检查项同下面 1.3.4-3cf6d39 的清单。

## 1.3.4-49b2dab（Node.js 版本检查 + 钥匙串授权提示 + 切回窗口不再转圈，已被 1.3.4-cb770ee 取代）

源码：临时集成分支 `ci/test-installers` @ `49b2dab0` = 正式版 1.3.4 之后的 main（`b9e80851`）+ 三个 PR。应用里显示的版本仍然是 1.3.4。

| PR | 内容 |
|---|---|
| #40 | 切回 Ofox 窗口时在后台刷新工具状态，不再整页转圈、遮住工具列表；30 秒内反复切回不重复刷新。 |
| #41 | 安装 npm 类工具（OpenClaw、Gemini CLI、OpenCode 等）前先检查 Node.js 版本（Fizzy #1021）。没有 Node.js 时照旧自动安装；版本太旧时弹窗说明要求的版本、当前版本、Node.js 的安装方式、会在终端里运行的命令、是否需要管理员授权、哪些工具要一并重装，你同意后才升级。Ofox 没法代为升级的安装方式（asdf、mise 等）只给出手动升级方法。macOS 安装失败时保留终端窗口，不再提示「请手动运行 npm install」。 |
| #42 | 读取钥匙串被拒绝时，登录页说明原因并提供「重新授权」，允许后直接恢复原来的登录，不用重新登录（Fizzy #1022）。 |

| 文件 | 平台 | 说明 |
|---|---|---|
| `ofox_desktop_1.3.4_macos_aarch64.dmg` | macOS 12+，Apple Silicon | 公司证书（NICE TALK PTE. LTD.）签名，已提交 Apple 公证。公证通过前打开会被拦：在「系统设置 → 隐私与安全性」底部点「仍要打开」；通过后同一个文件可以直接打开 |
| `ofox_desktop_1.3.4_windows_x64-setup.exe` | Windows 10/11 x64 | NSIS，未签名，默认按当前用户安装 |
| `ofox_desktop_1.3.4_windows_x64_en-US.msi` | Windows 10/11 x64 | MSI，仅用于验证能否安装，不会发布 |

安装前先从菜单栏 / 托盘弹窗里点「退出 Ofox」。校验文件 `1.3.4-49b2dab/SHA256SUMS.txt`。

在 macOS 上请看：

1. **钥匙串只弹一次（#1022）**：从 1.3.2 或以前的 ad-hoc 测试包换过来，第一次打开会按条弹钥匙串授权，每次都选「始终允许」。之后再换成正式版 1.3.4 或以后用公司证书签名的测试包，都不应再弹。
2. **拒绝后的提示（#1022）**：在「钥匙串访问」里双击 `ai.ofox.desktop.oauth` 的任一条 →「访问控制」→ 把 Ofox Desktop 从列表里删掉 → 存储更改。重新打开 Ofox，在弹窗里点「拒绝」：登录页显示「读取钥匙串里的登录信息时被拒绝了……」和「重新授权」按钮，而不是普通的未登录页。点「重新授权」，输入开机密码并选「始终允许」：回到原来的账号和已绑定的工具，不需要重新登录。
3. **Node.js 太旧时说清原因（#1021）**：fnm 默认 Node.js 是 24.15.0 时，在 Ofox 里安装或修复 OpenClaw，弹出「OpenClaw 需要更新的 Node.js」：要求 `>=24.16.0 <25 || >=26.1.0`，当前 24.15.0（由 fnm 管理），会运行 `fnm install 24 && fnm default 24` 等命令，并说明 claude、codex、gemini、opencode 会一并重新安装。
4. 点「取消」：不安装，也不改 Node.js。
5. 点「升级 Node.js 并安装」：终端里先升级 Node.js、重装上面那几个工具，再装 OpenClaw；完成后 Ofox 里 OpenClaw 显示已安装，Claude Code、Codex、Gemini CLI、OpenCode 仍能正常打开和对话。想退回原来的 Node.js 可以运行 `fnm default 24.15.0`。
6. 安装失败时终端窗口保留，Ofox 显示具体原因，不再出现「请手动运行 npm install」。
7. **切回窗口（#40）**：切到别的应用再切回 Ofox，工具列表不再被整页转圈遮住，版本和状态在后台更新。

在 Windows 上请看：

1. 用官方安装包装的旧版 Node.js（例如 22.x）时安装 OpenClaw：弹窗说明版本差距，并提示需要管理员授权（通过 winget 升级 Node.js LTS）。同意后 PowerShell 窗口里先升级 Node.js，再安装 OpenClaw。nvm-windows 管理的 Node.js 同样会提示需要管理员授权，并列出要重装的工具。
2. 切回窗口不再整页转圈。
3. 其余检查项同下面 1.3.4-3cf6d39 的清单。

## 1.3.4-3cf6d39（环境变量提示 + 托盘首次点击 + 终端目录，待验证）

源码：临时集成分支 `ci/test-installers` @ `3cf6d391` = 1.3.4-a7e6d00 + 下面这些修复：

| 分支 | 内容 |
|---|---|
| `feat/env-override-warning` | 系统环境变量会盖过 Ofox 配置时给出提示：Gemini 查 `GEMINI_API_KEY`、`GOOGLE_GEMINI_BASE_URL`、`GEMINI_MODEL`，Claude Code 查 `ANTHROPIC_API_KEY`。工具行显示「环境变量冲突」并计入「需处理」，「管理」里列出变量名、来源（Windows 用户 / 系统环境变量、shell 配置文件第几行）和删除方法。只显示变量名，不显示值。 |
| `fix/windows-window-layout` | Windows 托盘第一次点击不再「闪一下就消失」：启动后在后台预先建好弹窗。另外，保存设置、切换服务商后右键不会再变成旧的原生菜单。 |
| `feat/windows-launch` | Windows 上「打开」的终端改在用户主目录启动，不再在 Ofox 安装目录（如 `E:\Ofox Desktop`）里启动。 |

| 文件 | 平台 | 说明 |
|---|---|---|
| `ofox_desktop_1.3.4_macos_aarch64.dmg` | macOS 12+，Apple Silicon | ad-hoc 签名，未公证 |
| `ofox_desktop_1.3.4_windows_x64-setup.exe` | Windows 10/11 x64 | NSIS，未签名，默认按当前用户安装 |
| `ofox_desktop_1.3.4_windows_x64_en-US.msi` | Windows 10/11 x64 | MSI，仅用于验证能否安装，不会发布 |

安装前先在托盘弹窗里点「退出 Ofox」，确认 `Get-Process ofox-desktop` 没有输出再安装，否则运行的还是旧版本。校验文件 `1.3.4-3cf6d39/SHA256SUMS.txt`。

在 Windows 上请看：

1. 启动 Ofox 后等几秒，第一次左键或右键点托盘图标，弹窗直接出现，不再闪一下就消失（包括图标收在「^」溢出区里的情况）。
2. 在设置里保存一次，或切换一次模型，再右键托盘图标：仍然是 Ofox 弹窗，不是旧的原生菜单。
3. 系统环境变量 `GEMINI_API_KEY` 还在时：Gemini 一行显示「环境变量冲突」；「管理」里列出 `GEMINI_API_KEY` · Windows 系统环境变量，并说明删除方法。
4. 删除这个变量，从托盘退出 Ofox 再打开：提示消失，`gemini` 能正常对话。如果只删了变量、没重启 Ofox，提示会写「已从系统删除，但 Ofox 启动时还带着」。
5. 点 Gemini、OpenCode 等的「打开」，终端的当前目录是 `C:\Users\<你的用户名>`，不是 Ofox 安装目录。
6. 其余检查项同下面 1.3.4-c554707 的清单。

## 1.3.4-a7e6d00（托盘实体窗口 + 配置冲突误报修复，待验证）

源码：临时集成分支 `ci/test-installers` @ `a7e6d004` = 1.3.4-c554707 + 两个修复：

| 分支 | 内容 |
|---|---|
| `fix/windows-window-layout` | Windows 托盘弹窗改成实体窗口：不透明、铺满，外面不再多一圈虚线框；大小和原来的卡片一样。macOS 不变。 |
| `fix/binding-conflict-fields` | 在 ChatGPT / Codex 里改推理强度后不再显示「配置冲突」，Ofox 切换模型时也保留你选的强度；真正有冲突时，「管理」里会列出是哪个文件的哪个字段不一致（只有字段名，不显示 key 或其他值）。macOS 和 Windows 都包含这个修复。 |

| 文件 | 平台 | 说明 |
|---|---|---|
| `ofox_desktop_1.3.4_macos_aarch64.dmg` | macOS 12+，Apple Silicon | ad-hoc 签名，未公证 |
| `ofox_desktop_1.3.4_windows_x64-setup.exe` | Windows 10/11 x64 | NSIS，未签名，默认按当前用户安装 |
| `ofox_desktop_1.3.4_windows_x64_en-US.msi` | Windows 10/11 x64 | MSI，仅用于验证能否安装，不会发布 |

安装、校验方法同下面的 1.3.4-9274f52（校验文件 `1.3.4-a7e6d00/SHA256SUMS.txt`）。

在 Windows 上请看：

1. 点击托盘图标，弹窗是一个完整的实体窗口：没有外圈框，背景不透明，Windows 11 上四角是系统圆角；浅色、深色模式都正常，点弹窗外面会收起。
2. Codex、ChatGPT 不再显示「配置冲突」，Codex 一行重新出现「打开」。在 ChatGPT 里把推理强度换成「高」「低」等，刷新后仍然正常。
3. 在 Ofox 里给 Codex 换一个模型并保存，再看 ChatGPT / Codex 里的推理强度，还是你之前选的那个。
4. 如果 Gemini 仍显示「配置冲突」：点 Gemini 的「管理」，把橙色提示框里列出的字段截图发过来（例如 `~/.gemini/.env · GEMINI_API_KEY`，或「当前服务商已切换为其它服务商」）。
5. 其余检查项同下面 1.3.4-c554707 的清单。

## 1.3.4-c554707（Windows 功能对齐 macOS，待验证）

源码：临时集成分支 `ci/test-installers` @ `c5547071`，在 main（`a642c00b`）上合并了下面这些分支，验证通过后再逐个提 PR 合并：

| 分支 | 内容 |
|---|---|
| `fix/windows-window-layout` | 托盘弹窗不再被挡；主窗口 1000×650 居中、标题 Ofox Desktop；引导页可滚动（同 1.3.4-eb42785） |
| `fix/windows-bundle-resources` | Windows 安装包不再带 macOS 安装脚本，MSI 能打出来 |
| `fix/update-manifest-fields` | 「检查更新」能读到新版的发布时间和 Windows 下载链接 |
| `ci/release-windows-nsis` | 正式发布流程会同时出 Windows setup.exe（未签名） |
| `feat/windows-update`（含 tools / launch / install） | Windows 上的 CLI 检测、打开、一键安装、一键升级（移植自 magpie） |
| `feat/windows-ui` | Windows 不再在系统标题栏下面重复画一条标题；托盘「Ctrl+O」、「任务栏通知区域」「打开所在文件夹」等文案 |

文件：

| 文件 | 平台 | 说明 |
|---|---|---|
| `ofox_desktop_1.3.4_macos_aarch64.dmg` | macOS 12+，Apple Silicon | ad-hoc 签名，未公证 |
| `ofox_desktop_1.3.4_windows_x64-setup.exe` | Windows 10/11 x64 | NSIS，未签名，默认按当前用户安装 |
| `ofox_desktop_1.3.4_windows_x64_en-US.msi` | Windows 10/11 x64 | MSI，仅用于验证能否安装，不会发布 |

安装、校验方法同下面的 1.3.4-9274f52（校验文件 `1.3.4-c554707/SHA256SUMS.txt`）。

### Windows 检查清单

**检测**

1. 已装的 Claude Code / Codex / Gemini / OpenCode / Hermes 都显示版本号，不再是「未安装」。
2. Ofox 开着的时候，在另一个终端里装或升级一个 CLI（例如 `npm i -g @google/gemini-cli`，或 `claude update`），回到 Ofox 刷新后能识别新版本，不用重启 Ofox。
3. 只装了 ChatGPT 桌面版（没有 Codex CLI）时，Codex 一行显示桌面 App。

**打开**

4. 在工具列表点「打开」，弹出的命令行窗口里 CLI 正常启动；退出 CLI 后出现「按任意键」，按键后窗口关闭。
5. 打开后检查 `%TEMP%`，不残留 `cc_switch_*.bat`；「服务商终端」打开 Claude 后退出，不残留含 API Key 的 `.json`。
6. 没装的工具点「打开」，提示「未找到 XXX，请先安装或检查安装路径」。

**一键安装**（建议在没装过这些工具的 Windows 用户下测）

7. 点「安装」会弹出一个 PowerShell 窗口执行安装；成功后窗口约 5 秒自动关闭，列表里出现版本号。
8. 装 Gemini / OpenCode 时如果电脑没有 Node.js：会用 winget 装 Node LTS（可能弹 UAC），再继续装 CLI。
9. 安装失败时窗口停住等按键，Ofox 里显示「错误: …」原因；中途关掉安装窗口，Ofox 几秒内结束「安装中」并给出结果。
10. 区域切到 ofox.io 后装 Claude Code / Codex，走 npm + npmmirror 镜像。

**一键升级**

11. 先降级一个工具（例如 `npm i -g @google/gemini-cli@0.60.0`），Ofox 显示「升级到 vX」，点击后不弹外链、不弹黑窗口，直接升级完成并提示新版本。
12. 原生安装的 Claude Code（`irm https://claude.ai/install.ps1 | iex`）、Codex、OpenCode、Hermes 都能一键升级。
13. 两个以上工具有新版本时出现「全部升级」，能依次完成。
14. 工具正在运行时去升级：如果失败，要给出错误提示，不能一直转圈。

**界面**

15. 主窗口只有系统标题栏，下面没有第二条「Ofox Desktop」标题。
16. 引导完成页写「任务栏通知区域的 Ofox 图标」；管理工具里的按钮是「打开所在文件夹」，点击后打开资源管理器。
17. 托盘弹窗显示「Ctrl+O」，按 Ctrl+O 打开主窗口。

**之前的界面修复**（同 1.3.4-eb42785）

18. 托盘弹窗完整显示在任务栏上方；窗口居中、标题 Ofox Desktop；引导页可以滚动。
19. MSI 能装上并正常启动（只验证，不发布）。

### macOS 回归

macOS 包含同样的改动，检测/升级的公共代码有调整，请顺手确认：工具列表的版本检测、「升级到 vX」、「全部升级」、打开工具都和之前一致；窗口顶部仍有 Ofox Desktop 标题条且可以拖动。

## 1.3.4-eb42785（Windows 界面修复，待验证）

源码：分支 `fix/windows-window-layout` @ `eb427854`，在 main（`a642c00b`）基础上增加 3 个修复，验证通过后再合并：

- 托盘弹窗：任务栏在底部时向上弹出，并限制在屏幕可用区域内（之前总是往下弹，大部分落在屏幕外）。
- Windows 主窗口：恢复 1000×650、居中，标题改为「Ofox Desktop」（之前是 900×600、不居中、标题「CC Switch」）。
- 引导页：内容比窗口高时可以滚动，不再上下截断。

文件、安装方式与下面的 1.3.4-9274f52 相同；校验用 `1.3.4-eb42785/SHA256SUMS.txt`。

在 Windows 上请重点看：

1. 点击托盘图标（包括收在「^」溢出区里的情况），弹窗完整显示在任务栏上方。
2. 首次打开窗口居中、标题为 Ofox Desktop，「已发现 N 个 AI 工具」页的标题和「开始绑定」按钮都能看到；把窗口缩到最小时可以滚动。

## 1.3.4-9274f52

源码：`main` @ `9274f526`，包含 #22（未安装 CLI 不再误报检测失败）、#23（支持 Hermes 迁移后的 `providers` 配置）、#24（在工具列表里直接升级工具）。之后合并的 #25 只改了 dependabot 配置，不影响安装包。

| 文件 | 平台 | 说明 |
|---|---|---|
| `ofox_desktop_1.3.4_macos_aarch64.dmg` | macOS 12+，Apple Silicon | ad-hoc 签名，未公证 |
| `ofox_desktop_1.3.4_windows_x64-setup.exe` | Windows 10/11 x64 | NSIS 安装包，未签名，默认按当前用户安装 |

校验：

```bash
# macOS
cd 1.3.4-9274f52 && shasum -a 256 -c SHA256SUMS.txt
```

```powershell
# Windows
Get-FileHash .\1.3.4-9274f52\ofox_desktop_1.3.4_windows_x64-setup.exe -Algorithm SHA256
```

### 安装

- **macOS**：打开 DMG，把 Ofox Desktop 拖进「应用程序」。通过 git clone 得到的文件不带隔离属性，一般可以直接打开；如果提示「已损坏」或「无法验证开发者」，执行：

  ```bash
  xattr -dr com.apple.quarantine "/Applications/Ofox Desktop.app"
  ```

- **Windows**：安装包未签名，SmartScreen 会提示「Windows 已保护你的电脑」，点「更多信息」→「仍要运行」。

### 已知问题

- （1.3.4-c554707 起已修复）暂无 Windows MSI：自定义 WiX 模板（`src-tauri/wix/per-user-main.wxs`）里，安装到用户目录的资源文件组件以文件作为 KeyPath，且目录缺少 RemoveFolder，`light.exe` 报 ICE38 / ICE64 而失败。待修复后再提供 MSI。
