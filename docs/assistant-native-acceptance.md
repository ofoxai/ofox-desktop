# 助手 macOS 原生验收（2026-10-28）

范围：Fizzy #1234。用户已明确要求本轮先不测试
Windows；下列结果仅证明 macOS。功能实现仍在 `feat/assistant-chat` 的未提交 S4
工作区，基线 `f493ce1324ee8f6ddcfd1e20a5cf0b19a30334fd`。

## 环境与证据边界

- macOS 26.5.1 arm64；真实 Tauri 窗口、React 页面、Rust 命令、文件存储和内置
  Node 22.23.4 / Pi SDK 1.4.0，使用独立 bundle ID `ai.ofox.desktop.qa1234`。
- 构建只覆盖 bundle ID/product name，并启用 `tauri/custom-protocol` 嵌入现有
  renderer；未修改产品实现。QA 可执行文件 SHA-256：
  `7bbfec61f7e39923db90459adbe61f1b47b547036fdf21d863f3db49aa9a90e5`。
- `CC_SWITCH_TEST_HOME` 和各 Agent 目录指向专用临时目录；调试凭据使用本地文件。
  未替换已安装应用，日常 Ofox 进程保持运行，测试未使用日常账号或工具配置写入。
- `OFOX_USE_LOCAL=1` 将授权、Key、聊天和搜索请求发送到 loopback 模拟服务。
  公共模型目录保留现有例外，从所选 `.ai` / `.io` 区域真实读取。UI 明示本地模拟
  网关。设备授权、搜索与模型响应是合成数据，文件对话框和内置运行时是真实执行。
- 本轮没有新增生产付费请求。此前真实 Key、双 apex 聊天/停止和搜索/代理实测记录
  分别见卡片 Fizzy #1234 上 S1、
  S2/S3 的进度评论。
  S0 的评论
  已确认搜索源、认证和价格口径；余额只有账号级视图，页面费用按保存的目录单价
  估算。逐请求余额对账不是额外完成门槛。

## 已完成的原生场景

| 场景 | 实际证据 |
| --- | --- |
| 聊天、流式、Markdown 与代码 | 页面提交后，Rust 驱动内置 Pi；可见格式化答案、代码块和 Token/估算费用 |
| 多轮上下文 | 下一次实际模型请求包含上一轮问题和答案 |
| 停止与重新生成 | 中途保留部分文本并显示“已停止”；重新生成可完成 |
| 模型切换 | 从 GPT-6.1 Sol 切到 GPT-5.4 Mini，模型请求与重启后的选择一致 |
| 受控失败、重启与重试 | 503 保存 `assistant_model_request_failed`；重启后仍显示具体本地化原因；重新生成成功 |
| 文本、PDF、图片 | 系统选择框导入；实际模型输入包含中文附件正文、PDF 中文标记和图像 data URL |
| DOCX/XLSX/PPTX | 三个真实中文文件通过系统选择框导入；文字进入模型请求，UI 保留纯文字/公式范围说明 |
| 内置搜索 MCP | 实际工具调用进入 MCP 和本地 Responses 接口；显示可操作来源控件及“含1次搜索计费” |
| 编辑并移除全部附件 | 三个 Office 附件逐个移除；最后一个移除后焦点回到编辑框；新问题无附件，原版本/原文件/草稿保留 |
| 历史搜索、重命名 | 无匹配提示、清空后恢复列表；标题持久化 |
| 回收站与恢复 | 只读会话可恢复，草稿和文件保留；删除确认默认 Cancel，Escape 返回触发按钮 |
| Markdown 导出 | 原生 Save 实际写出 0600 文件，包含来源、附件清单和未发送草稿；取消不新增错误或成功提示 |
| 完整 ZIP 备份 | 原生 Save 实际写出会话、历史版本、6个附件、native JSONL、7条 operations；未包含测试 Key |
| 原生恢复与重复导入 | 相同会话报告导入0/跳过1；退出并移走原临时 owner 目录后，空配置报告导入1/跳过0 |
| 永久删除成功 | 经用户确认删除专用两消息/一附件副本；session/native/attachments 目录及删除暂存目录均无残留，焦点回到回收站 |
| 恢复后续聊 | 原生重启、选会话、继续；`read_attachment` 从恢复目录重新读取旧附件，返回原中文正文 |
| 恢复数据一致性 | 6个附件与 native JSONL 字节相同；原 operations/usage 相同，续聊后旧 operations 不变 |
| 900×600 深色与键盘 | 原生截图 1800×1200（Retina 2×）；十附件输入区可滚动，Shift+Tab 到最后一个附件，发送和费用可见 |
| 区域变更与未登录 | `.ai` → `.io` 要求重新授权，旧助手视图清空；未登录仍能进入本机工具管理 |
| `.io` 重新授权 | 正常设备授权流程连接本地模拟服务后恢复账号；历史和十附件草稿保留，新会话请求成功 |

停止后的该会话费用为“不确定”，不是已知费用丢失。已知历史费用和原始搜索单价
在双 apex 的真实 Node/Pi 恢复续聊集成测试中另有精确比较；见
[备份契约](assistant-backup.md)。本轮没有把未知费用补为零。

原生永久删除已获用户“可以删除”确认并执行。独立副本“临时删除验收 #1234”
（`8e54d7d5-2099-4926-bb2c-fee0a2ee5a5c`）原含两条合成消息、一个附件和真实
native JSONL。删除后目录、native、附件及 `.deleting-` 暂存目录均无残留；主验收
会话、十附件草稿、原始配置副本和 ZIP 备份保留，原始 operations 单价不变。原生
界面显示空回收站，焦点回到回收站按钮。本轮 macOS 验收完成。

## 截图

| 场景 | 证据 |
| --- | --- |
| 附件与上下文 | [原生聊天](screenshots/assistant-s4/native-text-chat-zh-light.jpg) |
| MCP 来源和搜索次数 | [原生搜索](screenshots/assistant-s4/native-search-zh-light.jpg) |
| Office 范围说明 | [原生 Office](screenshots/assistant-s4/native-office-zh-light.jpg) |
| 原生导出与备份 | [Markdown](screenshots/assistant-s4/native-export-zh-light.jpg)、[ZIP](screenshots/assistant-s4/native-backup-zh-light.jpg) |
| 相同内容跳过与空配置恢复 | [跳过](screenshots/assistant-s4/native-restore-identical-zh-light.jpg)、[恢复](screenshots/assistant-s4/native-restore-empty-zh-light.jpg) |
| 恢复后续聊与旧文件重读 | [继续聊天](screenshots/assistant-s4/native-resumed-chat-zh-light.jpg) |
| 重启保留失败原因 | [重启前](screenshots/assistant-s4/native-failure-before-restart-zh-light.jpg)、[重启后](screenshots/assistant-s4/native-failure-after-restart-zh-light.jpg) |
| 移除全部附件的编辑重发 | [纯文字重发](screenshots/assistant-s4/native-edit-empty-attachments-zh-light.jpg) |
| 900×600 十附件、深色 | [原生最小窗口](screenshots/assistant-s4/native-900-ten-attachments-dark.jpg) |
| 区域切换后的登录门禁 | [助手门禁](screenshots/assistant-s4/native-region-signout-900-dark.jpg)、[本机工具](screenshots/assistant-s4/native-logged-out-tools-900-dark.jpg) |
| 永久删除与主数据保留 | [空回收站](screenshots/assistant-s4/native-permanent-delete-empty-900-dark.jpg)、[主会话与草稿](screenshots/assistant-s4/native-main-preserved-after-delete-900-dark.jpg) |

中英日、浅深色、错误/空状态、取消、旧后端、账号和乱序回复、历史损坏、路径与
存储失败等回归由完整前端/Rust/运行时测试及既有 900×600 浏览器 fixture 补充。
该表不把浏览器模拟后端截图称为原生截图，也不把本地模拟服务称为生产网关。
