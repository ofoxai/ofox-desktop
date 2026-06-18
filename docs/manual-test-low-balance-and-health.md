# 人工验收 checklist：低余额提醒 + 工具健康检查

> 本文档**不随代码合并入主分支**，仅作为合并前必跑的人工验证脚本。  
> 配套自动化测试见 `src-tauri/src/services/{low_balance,tool_health,sleeper}.rs::tests` 与 `tests/hooks/useToolHealth.test.tsx` 等。
>
> 自动化覆盖了纯逻辑（边缘触发、阈值变更、cooldown 时间窗、cache 行为、事件分发、settings 序列化兼容）；本 checklist 覆盖**端到端**和**操作系统侧**的网络/通知能力——这些不可能在 CI 里桩出来。

---

## 准备

1. 确认在 `feature/low-balance-and-health-check` 分支：`git branch --show-current`。
2. 启动开发模式：`pnpm tauri dev`。
3. 启动日志确认：
   - `tauri-plugin-notification` 注册成功（无 warn）。
   - 见到 `[tool_health] round complete: ...` 或 `[low-balance] skipping: not authenticated`（auth 未就绪正常）。
4. 登录 OFox。
5. 至少绑定 Claude + Codex 两个工具，并各选一个真实 model。

> 提示：测试 24h 冷却之类的长时间逻辑时，可手动改 `~/.ofox-switch/settings.json` 的 `lowBalanceLastAlertAt` 字段后**重启应用**生效（重启会重新加载 settings.json 到内存缓存）。

---

## A. 低余额提醒（OS 级系统通知）

> ⚠️ 关键交付：**关掉主窗口、仅托盘运行时仍能弹出**。这是本次工作的核心动机。

| # | 步骤 | 期望 |
|---|------|------|
| A1 | 在"设置 → 偏好"将阈值设为大于当前余额（例：余额 $5、阈值 $10），保存 | **30 分钟内**收到 macOS / Windows / Linux 系统通知 `OFox 余额提醒`，正文 `当前余额 $X.XX，低于阈值 $Y.YY。` |
| A2 | 上一轮触发后，再等下一轮 30 分钟 | **不再**重复弹（边缘触发 + 24h 冷却生效） |
| A3 | 阈值降到 $3（低于余额），保存；再升到 $7（高于余额，假设余额 $5），保存 | 下轮 30 分钟内**重新弹**（阈值变更清空了闩锁） |
| A4 | 手动改 `~/.ofox-switch/settings.json` 的 `lowBalanceLastAlertAt` 为**1 小时前**的 unix-ms 后重启 | 下轮**不弹**（24h 内）|
| A5 | 同上，改成**25 小时前** | 下轮**弹** |
| A6 | 退出登录 | 循环跳过本轮（日志：`skipping: not authenticated`），无通知 |
| A7 | 关掉低余额开关 → 重新打开 | 不立即误触发；如仍低于阈值，下一轮 30 分钟内重新触发（开关 false→true 也会清闩锁）|
| A8 | **关掉主窗口（仅托盘）**，触发条件下等 30 分钟 | OS 系统通知**仍然弹出** ← **核心交付** |
| A9 | 系统设置 → 通知 → ofox-switch / Ofox Switch → 关闭通知权限；重新触发 | 应用**不崩**；日志 `[low-balance] notify failed (continuing): ...`；闩锁仍写入；循环继续 |
| A10 | 关 + 不退出应用，再开通知权限 → 改阈值触发新一轮 | 通知正常弹出 |

### 文案验证（必看一次）
- 标题：`OFox 余额提醒`
- 正文格式：`当前余额 $X.XX，低于阈值 $Y.YY。`（金额两位小数）

---

## B. 工具健康检查（Console 卡片胶囊）

> 健康检查使用 `max_tokens=1` 真实 completion，每次会消耗用户极少配额。设置对话框文案已告知。

| # | 步骤 | 期望 |
|---|------|------|
| B1 | 间隔设 `1h`，至少 Claude + Codex 两个工具健康；浏览器 DevTools `__TAURI__.core.invoke('trigger_tool_health_check_now')` | Console 工具卡片状态行**右侧出现绿点 + "延迟 234ms · 刚刚"**；`checkedAt` 实时更新 |
| B2 | 把 Claude active model 改成不存在的字符串（`"nonexistent-model"`），再 trigger | Claude 卡片**红点 + "连接失败"**；hover tooltip 见 `HTTP 404 ... 未找到该模型...` |
| B3 | 把 Claude active model 字段清空（`ANTHROPIC_MODEL=""`），再 trigger | Claude 卡片**灰点 + "未配置模型"**；**Codex 仍绿**（Codex seed 默认有 `bailian/qwen3-coder-plus`） |
| B4 | 间隔设 `off` | 循环阻塞（日志：`interval was off, woken by prefs-updated`，仅在前端再保存一次时出现）；ManageToolDialog 内手工 ping 仍正常工作 |
| B5 | 间隔从 `6h` → `1h`（保存触发 `ofox-prefs-updated`） | 日志 `[tool_health] woken by ofox-prefs-updated`；下轮按 1h 间隔 |
| B6 | 在 Console 添加 OpenCode 绑定 | 下一轮（或 trigger）OpenCode 出现在 cache（依赖 `bindTools` 镜像 + `ofox-prefs-updated` wake） |
| B7 | 退出登录 | `getSnapshot()` 返回空 map；卡片不再显示胶囊；重新登录后下轮重建 |
| B8 | dev StrictMode 双挂载 | 浏览器 console 不应出现重复 `[useToolHealth]` 错误；快速来回切换 ConsolePage 也无重复订阅警告 |
| B9 | **关闭主窗口（仅托盘）一段时间**，再打开 | 重开后 ConsolePage `getSnapshot()` 立即恢复胶囊；后台日志显示循环未停 `[tool_health] round complete: ...` |

---

## C. 配额成本预算（健康检查的副作用）

健康检查会消耗用户少量 OFox 额度（每次 max_tokens=1 完整 completion）：

- **默认 6h × 4 工具 = 16 次/天 ≈ 480 次/月**（每次极小）
- **用户调到 1h × 4 工具 = 96 次/天 ≈ 2880 次/月**

- [ ] UI 文案已在"工具健康检查"hint 处告知（实测确认 hint 显示"消耗极少额度，max_tokens=1"字样）

---

## D. 跨平台（按发布目标勾选）

- [ ] **macOS**：`.app` bundle 内通知图标正确（不显示 cargo "终端" 图标）；通知 banner 进入通知中心
- [ ] **Windows**：通知卡片样式正常；点击不导致应用崩溃
- [ ] **Linux**（如发布）：notify-osd / dunst 兼容性正常

---

## E. 退路 / 兼容性

- [ ] 旧版本 settings.json（缺新字段）首次启动正常，不报错；未启用低余额时不打扰
- [ ] 新字段在老版本前端打开"设置"对话框时不会丢失（前端 read-modify-write 测试已覆盖；这里再人工确认一次：保存任意一个其他字段后，新字段值不变）

---

## 通过标准

只有当上面所有勾选项都通过、且 `cargo test` + `pnpm test:unit` 全绿时，本 PR 方可合入主分支。

如有任意一项**关键**项（A8、A9、B1～B3、B7）失败，**必须**修复后重跑整套，不能"已知问题"绕过。
