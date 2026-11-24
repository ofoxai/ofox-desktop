# 文档引擎客户端验收

## 一键离线引擎检查

从同一提交的 Windows Probe 下载验收包，完整解压到普通本地目录。先核对 Actions
运行的完整 SHA，再运行 `test-document-engine.ps1`（右键“使用 PowerShell 运行”，
或 `powershell -NoProfile -ExecutionPolicy Bypass -File .\test-document-engine.ps1`）。
脚本不需要管理员、系统 Python 或账户；只运行内置可信样例，验证包内文件摘要，
结果写入新建的临时目录，不修改真实代理配置。不可向探针传入任意模型代码。

Windows 10 22H2 和 Windows 11 分别保留报告。Windows Server CI 通过不能替代
这两份客户端结果；探测失败后降级可用也不能记为 Python 验收通过。

## 安装版检查

最终 SHA 的 Windows Probe 通过后，在同一提交上手动运行
[Branch Packages](../.github/workflows/branch-packages.yml)，下载
`branch-packages-windows-x86_64-<完整SHA>`。此 artifact 含完整 unsigned NSIS
桌面测试安装包、`build-provenance.json` 和 `SHA256SUMS.txt`，保留版本号，
不创建 tag、不发布 Release、不使用签名 secret。安装包使用正常应用标识和
用户数据；使用专用测试账户进行安装版验收。

用户明确选择当前账户时，先退出应用并备份旧安装、应用数据、WebView 数据、
Roaming 下的应用路径 Store，以及 HKCU 的卸载和协议注册项。使用私有
`CC_SWITCH_TEST_HOME`、`WEBVIEW2_USER_DATA_FOLDER` 和各 agent 目录 override；
Tauri Store 仍使用系统 Roaming 路径，不能仅靠 APPDATA 环境变量隔离。
登录通过正式 OAuth 流程，专用模型 API Key 不能替代登录。测试后按备份恢复
原先的 Store 状态，保留原用户配置与会话。

使用同 SHA 的桌面测试包，逐项记录系统 build、应用版本、SHA、
截图及结果，不复制 key、配置正文或私人会话：

- 新建 8 页文稿，预览和导出一致，附件能重新打开；文本和形状可编辑。
- 上传固定 8 页源稿，仅修改第 3 页；原文件摘要不变，其余页内容及顺序保留。
- 中文、公式、图表和字体正常；900px 浅深色、键盘焦点与取消操作正常。
- 超时或隔离探测失败时明确显示不可用，绝不改用普通 Python 执行。
- Windows 10 的模型输入重放须经应用同一隔离引擎执行，记录为重放，不能记录
  为新鲜模型测试；直接打开现成的输出文件不算重放通过。

探针报告仅代表引擎检查；`uiAcceptance: pending` 必须结合上述安装版证据人工
核验，不自动改成通过。真实模型任务使用统一预算账本及专用测试 key，总预算
US$10，费用未知或预算不足即停止；不在这个离线脚本中调用付费模型。

排查「助手运行环境意外中断」时，完全退出 OFox，再从设置了
`OFOX_ASSISTANT_RUNTIME_DIAGNOSTICS=1` 的 PowerShell 启动诊断包安装的客户端。
分别复现一条短消息和文稿操作，按时间查看应用数据目录下
`assistant-runtime-diagnostics` 中的 JSON。它记录请求是否写入 IPC、运行时最后
报告的阶段，以及主进程执行清理前观察到的退出状态。输出仅包含固定阶段、状态、
时间和系统错误号。记录证据后退出客户端，从普通快捷方式重新启动即可关闭诊断。

## 开发验收：真实模型与 Win10 Host 重放

从同一 Branch Packages 运行下载独立的
`assistant-parity-driver-windows-x86_64-<完整SHA>`，核对
`driver-provenance.json` 的源 SHA、实际编译 SHA、`compiledSourceClean: true`
及 exe 的 SHA-256。包内 `assistant_host-parity.exe` 运行真实 AssistantHost、
临时 TestEnv 和应用的 LPAC 引擎；它不运行已安装的桌面窗口，不能代替上述 UI
检查。此入口有开发环境前置，不属于一键离线探针。

打包前必须用实际 release driver 执行 `parity_eval::release_gateway_preflight`：
只访问固定本地假网关，使用合成凭据，验证两个区域的 Host→真实运行时接线，
不读取专用 Key、不请求付费模型。`driver-provenance.json` 的
`gatewayPreflight.status` 必须为 `passed`。只有临时 TestEnv 显式登记的严格
`http://localhost:<port>` 获得本地网关权限；正式安装版仍沿用原 HTTPS 策略。
这个前置检查不替代 LPAC、真实模型或安装版验收。

同一次 Branch Packages 还会并行生成
`assistant-parity-driver-early-windows-x86_64-<完整SHA>`，内容和 keyless
前置门禁相同，供开发验收与完整 CI/Windows Probe 并行。它不是安装包，也不代表
完整门禁已通过。正式桌面安装包仍等待 CI 和该 SHA 的独立 Windows Probe 成功；
最终验收必须保留这些结果，不能凭 early driver 关闭任务。

客户端需要 Git、pnpm 10.12.3 和现有 npm CLI。先 checkout 完整提交 SHA，
保持 tracked 和 untracked 都干净。要在客户端免装 Rust、MSVC 和系统 Python，
从该 SHA 的 Windows Probe 下载 `document-engine-acceptance-<完整SHA>`，
解压到 checkout 外的同级目录。在 checkout 里运行以下命令。
先让包内验收脚本校验完整 manifest 和文件摘要，再把已校验的原生探针与构建回执
复制到 checkout 忽略的资源目录。后续准备脚本会复用该产物。

```powershell
$sha = (git rev-parse HEAD).Trim()
$bundle = (Resolve-Path -LiteralPath "..\document-engine-acceptance-$sha").Path
$manifest = Get-Content -Raw -LiteralPath (Join-Path $bundle 'acceptance-manifest.json') | ConvertFrom-Json
if ($manifest.commit -cne $sha) { throw 'Windows Probe artifact SHA does not match checkout' }
powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $bundle 'test-document-engine.ps1')
if ($LASTEXITCODE -ne 0) { throw 'Offline artifact verification or native probe failed' }
$probes = Join-Path (Get-Location).Path 'assistant-runtime\resources\probes'
foreach ($name in @('windows-child-probe.node', 'windows-child-probe.source.json')) {
    if ($manifest.files.path -cnotcontains "runtime/probes/$name") { throw "Missing verified probe file $name" }
}
New-Item -ItemType Directory -Force -Path $probes | Out-Null
Copy-Item -LiteralPath (Join-Path $bundle 'runtime\probes\windows-child-probe.node'), (Join-Path $bundle 'runtime\probes\windows-child-probe.source.json') -Destination $probes

pnpm install --frozen-lockfile
pnpm assistant:install
pnpm assistant:prepare
pnpm assistant:smoke
git rev-parse HEAD
git status --porcelain --untracked-files=normal
```

这些命令只准备离线资源和验证工具。安装版界面与实模验收分别记录。

controller 通过 `pnpm assistant:install` 安装 sidecar 依赖。准备脚本使用现有 npm CLI
执行锁定的 `npm ci`，实际 controller 则必须使用包内 Node 22.23.4。
将 driver artifact 和 root 提供的唯一 campaign 目录放在 checkout 外；以下
示例假定它们是 checkout 的两个同级目录：

```powershell
$node = (Resolve-Path .\assistant-runtime\resources\node\node.exe).Path
$driver = (Resolve-Path ..\assistant-parity-driver\assistant_host-parity.exe).Path
$campaign = (Resolve-Path ..\assistant-1701-campaign).Path
Get-FileHash -LiteralPath $driver -Algorithm SHA256
```

Windows 11 的付费运行仅由 root 授权，并通过 `OFOX_API_KEY` 注入专用测试 key，
不借日常账户 key。root 准备、顺序移交同一个 campaign 和账本；不得在另一主机
新建 campaign 或复制出另一份 US$10 预算。确需付费运行时显式执行：

```powershell
& $node scripts/assistant-parity-eval.mjs --live --platform win11 --root-authorize-host win11 --out $campaign --driver $driver
```

在 Windows 10 客户端，拿到原六份 Win11 capture 及同一账本后执行免费 Host 重放：

```powershell
& $node scripts/assistant-parity-eval.mjs --replay-win10 --out $campaign --driver $driver
```

两个入口都严格检查原生 client OS、同 SHA 干净源码、实际编译身份和输入摘要。
不得直接双击 exe 或自行调用忽略的付费测试，不能把普通 Python 或打开现成产物
算作重放。缺 key、模型费用、原生客户端或手工页级／Office 保存重开证据时保持
pending。补齐证据后，在同 SHA checkout 使用 `--verify --out $campaign` 汇总；
12 个正式任务和六个 Win10 重放均需通过，不容忍失败项。
