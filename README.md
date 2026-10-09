# Ofox Desktop 测试安装包

这个分支只存放手动测试用的安装包，不是正式发布：不在 Releases 里，也不会通过应用内自动更新推送。

## 获取

```bash
git clone --branch test-builds --single-branch --depth 1 https://github.com/ofoxai/ofox-desktop.git ofox-test-builds
```

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

- 暂无 Windows MSI：自定义 WiX 模板（`src-tauri/wix/per-user-main.wxs`）里，安装到用户目录的资源文件组件以文件作为 KeyPath，且目录缺少 RemoveFolder，`light.exe` 报 ICE38 / ICE64 而失败。待修复后再提供 MSI。
