# 调研：要不要给助手加 Python（2026-11-08）

起因：PPT 走“模型写代码”路线后，后面还要做 Word、表格、视频。问题是 Python 加不加、怎么加。数字都在本机
（macOS arm64）实测。

## 参照：dsh（deepseek-harness）怎么做

- CLI 是 Node 程序，全局安装约 244 MB，其中 LibreOffice Kit 148 MB（渲染和转格式）；`dsh --version` 峰值内存 58 MB。
- Office 技能用 Python 库（python-docx、python-pptx、openpyxl）。Python 不在 CLI 里，由桌面版统一打包：
  CPython 3.12（python-build-standalone）+ Node 24 + numpy、pandas、python-docx、python-pptx、openpyxl、Pillow、lxml、XlsxWriter
  （`scripts/primary-runtime/lock.json`）。模型通过 `load_workspace_dependencies` 拿到这个 Python 的路径，直接在本机执行。

## 三种做法实测

| 做法 | 磁盘 | 内存 | 速度 | 隔离 | 备注 |
|---|---|---|---|---|---|
| QuickJS + pptxgenjs（已验证） | 约 2 MB | 小（上限设 256 MB） | 8 页 7 图约 1 秒 | WASM，无文件 / 网络 / 进程 | 只能新建 PPT，不能打开已有文件；pptxgenjs 输出有一处 schema 顺序问题 |
| 原生 CPython 3.12 + numpy、pandas、matplotlib、python-docx / pptx、openpyxl、Pillow、lxml | 解释器 73 MB + 库 150 MB ≈ 223 MB / 平台（不含 matplotlib 约 180 MB） | 全部导入并画一张图 147 MB | 快；首次导入 57 秒（字体缓存、编译） | 无，跑在用户本机；Windows 上没有现成沙箱 | macOS 公证要给每个 .so 签名；各平台各打一份 |
| Pyodide 314.0.7（CPython 3.14 编译成 WASM，跑在现有 Node 里） | 核心 13 MB + numpy / pandas / matplotlib / lxml / Pillow 19 MB + 纯 Python 的 Office 库 9 MB ≈ 41 MB，各平台同一份 | 空载 186 MB；只用 Office 库 250 MB；加 pandas + matplotlib 峰值 626 MB | 启动 1 秒、加载包 0.8 秒（已缓存）、生成 docx + xlsx（含图表）+ pptx + matplotlib 图 3.6 秒 | WASM：读本机文件 `FileNotFoundError`，`subprocess` 报 `OSError` | 三份产物 officecli `validate` 全部通过；包要随应用打包或放自家 CDN（jsDelivr 主站在本机只有 15 KB/s，PyPI 在沙箱里取不到） |

## 各场景要不要 Python

| 场景 | 不用 Python | 用 Python |
|---|---|---|
| 新建 PPT | pptxgenjs 已够用 | python-pptx 也行，且能打开已有文件 |
| 改用户的 Word / PPT / Excel（M3b） | officecli 能改；JS 库大多只能新建 | python-docx / python-pptx / openpyxl 能读能改，最成熟 |
| 表格分析、出图 | JS 生态弱，模型也不擅长 | pandas + matplotlib，模型最熟 |
| 视频 | ffmpeg 二进制（宿主按固定参数调用）+ Ofox 视频 API | 不需要；moviepy 只是包 ffmpeg，Pyodide 也调不了子进程 |

## 建议

1. 加 Python，用 Pyodide，按需加载：任务需要时才起一个 Pyodide 进程，用完就退，释放内存；约 41 MB 随应用发布
   （或首次使用时从 Ofox 自己的 CDN 下载）。不需要额外的系统沙箱，Windows 也一样安全。
2. 主要用在 Word、表格和数据图；PPT 是否也统一到 python-pptx，用同一套“规则 + 辅助库 + 写时检查 + 看图自查”再跑一轮对比后定。
   统一的好处是一种语言、一套技能，并且能改已有文件；代价是内存比 QuickJS 高。
3. 原生 Python 暂不做：体积大、要逐个签名、Windows 上没有隔离。等出现 Pyodide 没有的包或大数据量性能问题再评估。
4. 视频走 ffmpeg，与 Python 无关。
