# Ofox Desktop 测试安装包

这个分支只存放手动测试用的安装包，不是正式发布：不在 Releases 里，也不会通过应用内自动更新推送。

## 获取

```bash
git clone --branch test-builds --single-branch --depth 1 https://github.com/ofoxai/ofox-desktop.git ofox-test-builds
```

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
