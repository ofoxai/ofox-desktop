# PPT 统一内容实验与离线验证

日期：2026-11-10。依据：[设计方案第 8 节](2026-11-09-ppt-design-system-research.md#8-可验证的实验与验收设计)。本记录区分冻结的真实模型实验计划、固定代码工具链冒烟、人工视觉/编辑验收，不能互相代替。

## 当前结论

- **已核实**：已有 staged Node、Python 和 OfficeCLI 可在当前 macOS 工作树运行；已有真实 API runner 支持所选 OFox apex、环境变量/Keychain 取密钥与日志脱敏。`--self-test` 的 14 项通过。当前进程未提供 `OFOX_API_KEY`；本次未访问钥匙串、未调用真实模型，也没有模型费用或质量提升得分。
- **已实现**：独立编写的六页统一内容、A/B/C/D 冻结清单与每次任务提示、真实结果收据导入检查，以及单独的固定代码样张生成/预览/OOXML 检查。5 个契约测试先失败（缺实现）后通过。
- **待验证**：A/B/C/D 真实模型收益、人工盲审、目标 PowerPoint 的编辑保存重开、Windows 本机路径、原风格构图保留、字体替代效果与真实账单成本。`enabled` 只是实验可选，不能替代 `verified` 或质量验收。

**对照 preflight 当前为 `blocked`。** 共享的系统/工具指导和技能已经包含公开风格选择、逐页计划，仅用不同用户提示不能把这些能力从 A/B 组隔离出去。脚本给每次运行、提示文件与报告写明该阻断。必须建立 A 的改造前完整运行时/技能/工具指导快照，以及只逐层增加相应能力的 B/C/D 隔离路径，才能作因果比较；导入原始收据也不会把 `comparability=blocked` 改成通过，当前不能计算或宣称 B−A、C−B、D−C 收益。未知引擎/资源版本或默认未冻结参数也仍在该阻断内。

## 内容与四条件

内容包：[`scripts/fixtures/ppt-benchmark.json`](../../../scripts/fixtures/ppt-benchmark.json)，版本 `2026-11-10.1`，SHA-256 为 `cb70e3190f7e7c03ba0f7ffe67a68d4bdf51dea562dc0510b7043080dec03a49`。中文、16:9、6 页、平衡阅读密度；正文下限 12pt，标签下限 10pt。内容全部为自编合成演示，不含闭源 ofox-agent 的代码、结构、提示、模板或示例，也不借用公开模板厂商的品牌和性能数字。

六页固定为：目标与合成声明；采集→去重→索引→检索；A 全量重建/B 仅处理变更及各自状态要求；三批完整合成数据；乙批次 `180÷200×100%=90%`；B 是待测候选且不能据本例宣称 B 优于 A。文字措辞可以调整，但所有组必须保留相同事实、数字与限定条件。

| 条件 | 新增变量                   | 必须保留/固定                                          |
| ---- | -------------------------- | ------------------------------------------------------ |
| A    | 当前 Desktop 基线          | 保留原有预览与检查，不能人为弱化基线                   |
| B    | 公开风格资源与必要适配     | 暂不添加新逐页计划或额外视觉复核                       |
| C    | 独立逐页设计计划           | 与 B 同模型/参数/风格版本/引擎/平台                    |
| D    | 当前导出成品视觉复核和修复 | 与 C 同生成模型，额外修复最多 2 轮，单记审稿调用与成本 |

每套代表风格每条件默认 3 次；A 在各风格配对中重复生成，不能把它们称为不同基线风格。先固定风格，再研究自动选择策略。Python、OfficeCLI、Node 路线分别建实验，HTML 与 PPTX 分开。本地脚本不会自动发起付费实验。

## 可执行入口

从仓库根目录执行，不安装依赖、不读取生产助手配置：

```sh
node --test assistant-runtime/test/ppt-eval.test.mjs

node scripts/assistant-ppt-eval.mjs \
  --out .real-eval/ppt-plan \
  --styles blue-professional,cartesian,architectural-plan \
  --repeats 3

node scripts/assistant-ppt-eval.mjs \
  --out .real-eval/ppt-fixture-smoke \
  --styles blue-professional,cartesian,architectural-plan \
  --repeats 3 --render
```

默认产物为 `benchmark.json`、`experiment.json`、`prompts/<runId>.md` 和 `report.md`。模型条件保持 `pending`。`--render` 使用自行编写的固定 Python 代码，复用 [`python-engine.mjs`](../../../assistant-runtime/test/fixtures/python-engine.mjs) 中的 staged runner、临时目录和 ZIP 读取器；OfficeCLI 使用隔离 HOME、关闭 resident/更新检查。它生成 `fixture-smoke/<style>/deck.pptx`、源代码、lint、6 页 SVG/PNG 与 contact sheet，并保留验证器原始结果。固定代码样张不属于模型 A/B/C/D，也不证明视觉收益。

真实执行前，指定 `--model ID --model-version VERSION --parameters parameters.json` 冻结真实生成参数。提示文件是未来隔离执行器的待接入任务稿，**不能把普通 Desktop 会话跑出的 A/B 直接称为有效对照**。脚本递归记录当前 runtime 源码、PPT 技能及资源、Rust assistant 源码的文件哈希与 `generationSnapshotHash`，另记录 helper/runner/template/assets/catalog/OfficeCLI 的引擎哈希；dirty 布尔与 HEAD 只作额外说明。当前 `scripts/assistant-real-eval.mjs` 的 Rust 接口固定跑四种旧任务（PPT 是 8 页 Transformer），不能直接声称已经跑过本六页实验；本轮未为它重写付费 host。

`--receipts FILE` 导入 JSON 数组，每项字段为 `runId`、完整 `frozen` 副本、`status`（`completed`/`failed`）、`artifacts`、`metrics`、`cost`，可另附有证据的 `manualReview`。未配置模型版本时拒绝导入；内容哈希、模型/参数、引擎/源码版本、平台与风格版本任何变化都拒绝混入同一次实验。收据只是带来源的结果声明；实际产物、调用记录与账单仍须保留以供复核。

```json
{
  "runId": "cartesian-r1-B",
  "frozen": "复制该实验 experiment.json 中对应 run 的完整 frozen 对象",
  "status": "completed",
  "artifacts": ["artifacts/cartesian-r1-B/deck.pptx"],
  "metrics": {
    "tokens": { "input": 1200, "output": 800 },
    "seconds": { "draft": 20, "final": 30 },
    "calls": { "generation": 1, "render": 1, "review": 0 },
    "retries": 0,
    "cache": "warm"
  },
  "cost": { "actual": null, "estimate": null }
}
```

以上数字仅解释收据格式，**不是实验结果**；实际文件中的 `frozen` 必须是对象。失败也保留，不以成功样本替换。运行与收据中禁止填入密钥。

## 费用、耗时与人工状态

| 项目     | 记录规则                                                                                                       |
| -------- | -------------------------------------------------------------------------------------------------------------- |
| Token    | 输入/输出分开；缓存读取/写入另记，未知为 `null`，不把缺数据写成 0                                              |
| 时间     | 初稿/最终、生成/渲染/审稿/失败重试分别记；cold/warm 分开，一次性适配工时单列                                   |
| 估算费用 | `cost.estimate` 含 `amount`、`currency`、`priceVersion`、`pricedAt`，不可冒充账单                              |
| 实际费用 | `cost.actual` 额外要求 `billingEvidence` 和 `billedAt`；未知为 `null`                                          |
| 人工审阅 | 默认 `pending`、分数 `null`；导入已复核状态需要 reviewer/evidence，分数限定 1–5                                |
| 可编辑性 | 原生 Chart、嵌入 XLSX、OMML 只检查结构存在；目标 PowerPoint 修改文字、图表数据和公式后保存重开仍独立 `pending` |

旧真实 runner 能记录 tokens/calls/seconds，但没有实际账单字段，不能据它的 tokens 自动宣称实际费用。离线固定样张明确 `modelExecuted=false`、`localModelCalls=0`、tokens 与两类费用均为 `null`；本机耗时不混入模型对照收益。

## 本轮执行证据

首次本地运行保留于 `.real-eval/ppt-validation-2026-11-10/`：基线六页生成成功；三个代表样本在 catalog 尚未启用时被正确拒绝为 `unknown or disabled style`。这是显式失败记录，不删除后重算通过率。

随后生成基线与 blue-professional 的全部六页 SVG/PNG，位于 `.real-eval/ppt-validation-2026-11-10-previews/`；两份样张均为 16:9、六页、必需文本未缺失、原生 Chart/嵌入 XLSX 与 OMML 结构存在、lint 为空。其余代表样本和验证器结果以最后一次运行的 `experiment.json` 为准。这里的 `generated` 只表示固定样张成功生成并取得六页预览。

技能指导和 catalog recipe 最后定稿后，重新完整运行全部五代表与默认样张；**上一轮离线快照（本轮修复前的历史证据）**位于 `.real-eval/ppt-validation-2026-11-10-accepted-snapshot/experiment.json` 和 `report.md`，生成平台 `darwin/arm64/25.5.0`。此前 `.real-eval/ppt-validation-2026-11-10-final/` 保留为历史证据，没有删除。本轮五代表各一份加默认样张共六份固定代码成品，**不是每条件三次的真实模型实验**；60 个 A/B/C/D 运行项全部 `pending` 且 `comparability=blocked`，未调用真实模型。

| 固定样张           | 页数/比例/事实检查    | lint | OfficeCLI validate | 原生结构            |
| ------------------ | --------------------- | ---- | ------------------ | ------------------- |
| desktop-current    | 6 / 16:9 / 未发现缺项 | 空   | success / 0 errors | Chart + XLSX + OMML |
| blue-professional  | 6 / 16:9 / 未发现缺项 | 空   | success / 0 errors | Chart + XLSX + OMML |
| cartesian          | 6 / 16:9 / 未发现缺项 | 空   | success / 0 errors | Chart + XLSX + OMML |
| course-module      | 6 / 16:9 / 未发现缺项 | 空   | success / 0 errors | Chart + XLSX + OMML |
| product-launch     | 6 / 16:9 / 未发现缺项 | 空   | success / 0 errors | Chart + XLSX + OMML |
| architectural-plan | 6 / 16:9 / 未发现缺项 | 空   | success / 0 errors | Chart + XLSX + OMML |

每份的 `deck.pptx`、六页 SVG/PNG、`contact-sheet.png`、固定代码和 lint 均保留在 `fixture-smoke/<style>/`。每份独立记录引擎/目录哈希、PPTX 和固定代码 SHA-256，运行前后引擎或 catalog 变化会标失败。六份共同绑定：

- helper `deck.py`：`f635e3a0ade5ab02d20c05f7888823654f69628605cf41926b7aa1fd6dabd19b`
- runner：`2b8345d5c48e8983b6d40f6d1c30a7c61a30b7cddcd75ac18da1d53a9bbbdfe8`
- catalog：`9fc88b92793c9e7c7041e89f2855b38eb06a3c461e9c680e2a1ad1c712e0c26c`
- PPT 技能 `SKILL.md`：`830f77e2949378acb2cd87c0f180e39610e8881c110d47b869f2834f6a40bf12`
- 180 个生成相关文件的快照：`605466674bb2e7cf0c676a206b1f6975335e12fb562e299753aa21a991693dca`
- 引擎文件集合摘要：`ddba02db2a258c05e44d2ab66c4ff29fc9628ff12959cbe68e9d1bcd89be538e`
- 评估器源码：`1a7ab2cbe1e105172607b92a2d77e575096ba653f396b8c4b3210749f382eb6e`

该轮结束时，180 个已记录文件、引擎文件和六份 PPTX 的 SHA-256 相符。**本轮审计发现快照漏项**：Rust 扫描路径误写为不存在的 `src-tauri/src/assistant`，实际目录是 `src-tauri/src/services/assistant`；因此该 180 文件快照不覆盖 Rust assistant，不能作为完整生成链身份。已修正路径并添加回归断言；历史记录保留，不追改为完整通过。此前 catalog 快照的差异仅为 recipe 对 architecture 的 Node 深色封面/Python 浅色封面作准确区分；以上哈希仅描述该历史快照，不能代表后续公式、预览和 UI 改动。

独立重跑 Python 风格测试 10/10 通过，包含五风格严格 validate、多 deck 上下文、原生关系图、目标侧栏和交替面板。实际 SVG 的流程连接线为向右 `marker-end`。该历史快照的课程与建筑 contact sheet 再次目视确认目标侧栏、左右交替色板及事实分区，尚无盲审评分；此前也观察到产品深色标题带与不同封面构图。这并不证明上游 HTML 所有特征完整映射。固定样张中的原生 Chart/Table 仍采用 python-pptx 默认配色，字体使用系统替代，实际字体匹配仍需目标机核验。历史 final 运行曾与其他回归任务并行；本次 accepted 快照也是每风格仅一份固定代码样张，`generationSeconds`、`renderSeconds` 和总秒数仅为运行日志，不能作稳定性能结论。

原生公式使用独立实现的 `equation()`，不调用 LaTeX 浏览器截图。初版 `a14:m` 遇到 leaf 校验问题后，按 [Microsoft 扩展规范](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-odrawxml/853b19c7-68a9-4f9a-a2ae-5e6cb0d02e62)改为 shape 级 `AlternateContent`（原生 Choice、明确为文本的 Fallback）；最终 OfficeCLI 1.0.160 严格校验六份均为 0 errors，原始验证结果完整保留，没有豁免 schema 错误。相关上游背景见 [Open XML SDK issue 1940](https://github.com/dotnet/Open-XML-SDK/issues/1940)。**该历史快照的 OfficeCLI 预览将样张分式画为连续的 `180200`，缺分数线**，属于已观察到的预览兼容缺陷；独立算例文本仍完整。OMML 结构与 schema 通过不能代替正确视觉呈现或 PowerPoint 编辑保存重开。

本机已安装 PowerPoint `16.109.1`（build `16.109.26051717`）。历史 final 样张曾为原生测试另复制到 `native-roundtrip/cartesian-roundtrip.pptx`，原始样张未修改。该次 CUA 文件选择器交互停在 GoToWindow，Return 和 AX 打开均未完成载入，未观察到文档实际打开；该轮 accepted 快照没有追加原生编辑成功证据。因此不能判定文件打不开、兼容失败或原生编辑成功；文字、图表数据和公式的编辑、保存、重开仍全部 `pending`。这次交互失败不能替代实际目标应用验收，也不能从安装版本推断验收结果。

## 下一次真实验收

先在同一真实模型/固定版本和相同运行路径执行代表风格四条件。比较内容和论证是否退化、构图是否保留来源特色、CJK/溢出、图表/公式实际编辑、时延和费用；报告全部失败、样本数、中位数和范围。为每份最终成品保留缺陷截图与修复记录，进行不知道实验条件的人工审阅；至少在 macOS 与 Windows 的目标 PowerPoint 保存重开。达到事实无误、无阻断阅读问题、核心对象可编辑、许可台账齐全并有可复核收益后，才逐批扩库。

## 公式与交互修复的后续证据

用户实际文件暴露了旧 `formula()` 仅为普通文本、但自动检查仍可能为零的问题。后续改动、WPS 样张与交互验证另见[修复记录](2026-11-10-ppt-math-ui-repair.md)。本页六风格实验仍是历史固定代码 smoke，不能据后续修复宣称 A/B/C/D 模型质量或 PowerPoint 编辑验收已完成。
