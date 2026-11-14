# PPT 公开资源筛选与来源台账

日期：2026-11-10。本文记录公开资源入库时的状态快照；代表风格后续适配状态以 `assistant-runtime/skills/ofox-office-pptx/resources/catalog.json` 的独立字段及验证证据为准。

## 1. 已完成的范围

已建立 **24 个去重候选资源身份**：17 个 beautiful-html-templates 完整 HTML 样例、6 个 OpenDesign html-ppt 完整 HTML 稿件、1 个 OfficeCLI 风格与公开构建源码。5 个代表 ID 已固定为 `blue-professional`、`cartesian`、`course-module`、`product-launch`、`architectural-plan`。

这满足 20–30 套筛选目标，不能写成 24 套可用 PPTX。入库时：候选 24、项目自有代码/文本许可核对 24、第三方资产闭包核清 0、已适配 0、已通过视觉/PPTX 验收 0、已启用 0。后续实验可选择状态与视觉/PPTX 验收必须分别记录。

共引入 143 个公开原始文本/代码文件，合计 2,065,662 字节，保留上游目录层级及原字节。所有文件的 SHA-256 写入 catalog，并额外与固定版本 Git tree 的 blob SHA-1 对照通过。未执行上游脚本、启动源 HTML、安装浏览器、下载字体或图片、生成正式 PPTX、提交或推送。没有使用或复制闭源 ofox-agent 文件。

## 2. 固定公开来源与许可

| 来源 ID                    | 固定版本                                                                                                                                           | 范围及许可                                         | 保留文件                                                        |
| -------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------- | --------------------------------------------------------------- |
| `beautiful-html-templates` | [e5e204fb1f3b06290846e7dcd7aceddabeceec8c](https://github.com/zarazhangrui/beautiful-html-templates/tree/e5e204fb1f3b06290846e7dcd7aceddabeceec8c) | 根目录自有代码/文档；MIT                           | `vendor/beautiful-html-templates/LICENSE`                       |
| `open-design-html-ppt`     | [07a86b1041531bfa0bec1006d9c0993985fee560](https://github.com/nexu-io/open-design/tree/07a86b1041531bfa0bec1006d9c0993985fee560)                   | `design-templates/html-ppt` 子包自有代码/文档；MIT | `vendor/open-design-html-ppt/design-templates/html-ppt/LICENSE` |
| `officecli`                | [c56512c8c7b76fe284f09940e4cd492470099d3e](https://github.com/iOfficeAI/OfficeCLI/tree/c56512c8c7b76fe284f09940e4cd492470099d3e)                   | 根目录自有代码/文档；Apache-2.0                    | `vendor/officecli/LICENSE`、`vendor/officecli/NOTICE`           |

OpenDesign html-ppt 的独立 MIT 声明 lewis 为版权人，SKILL 声明 `lewislulu/html-ppt-skill` 为上游；本次只固定 OpenDesign 快照，没有据此推断其对应某个已知上游提交。OfficeCLI 是 **Apache-2.0**，并非 MIT，故另保留 LICENSE 与 NOTICE。许可核对范围是所指自有文本/代码，不涵盖外部字体、图片、Logo、商标、截图或附带二进制。

## 3. 目录与数据接口

```text
assistant-runtime/skills/ofox-office-pptx/resources/
  catalog.json
  README.md
  vendor/beautiful-html-templates/…（保留原 sourcePath）
  vendor/open-design-html-ppt/…（保留原 sourcePath）
  vendor/officecli/…（保留原 sourcePath）
```

`sources[]` 保存 `id`、`repository`、完整 `revision`、`license`、`licenseFile`、`licenseFiles` 及共享文件。`styles[]` 保存 `id`、名称、场景、`sourceId`、`sourcePath`、`sourceUrl`、`resources[]` 和独立状态。每个资源条目包含相对 `path`、原始 `sourcePath`、原字节 `sha256`。素材原文保持不变，Ofox 的 theme/recipe/adapter 独立添加，不在原文件里改写。

状态字段独立：`licenseReviewed` 只表示所指代码/文本许可已读；`adapted` 表示接入适配状态；`verified` 表示有对应证据的验证状态；`enabled` 表示选择策略是否允许选择。`validation.rendered`、`validation.pptxAcceptance`、`platformsTested` 继续区分实际验收。`assetReview` 和 `externalDependencies.reviewStatus` 保持 `pending`，不能由代码许可字段自动提升。

上游 beautiful 的原 `index.json` 有 34 条，OpenDesign 的共享主题 CSS 也已作为关联源码保留；二者不计作更多 Ofox 候选。所有选择数量只按下表及 catalog 的 24 个 style ID。

## 4. 去重的 24 个候选

去重键为规范公开仓库与源目录；OpenDesign 的 `html-ppt-zhangzara-*` 和 frontend-slides 的 bold-template-pack 是 beautiful 的镜像/派生入口，不另计。不同布局系统按原说明、HTML/CSS 结构筛选；未运行渲染后视觉相似度检查，因此不声称 24 套视觉距离已经测量。

| ID                         | 场景                         | 结构差异筛选依据                       | 代表 |
| -------------------------- | ---------------------------- | -------------------------------------- | ---- |
| `blue-professional`        | consulting / business-review | 现代无衬线字阶、克制单强调色与整洁对比 | 是   |
| `cartesian`                | academic / research          | 衬线标题、低密留白与中性杂志式组织     | 是   |
| `editorial-forest`         | business-review / research   | 衬线/等宽对比、复盘式混合明暗节奏      | 否   |
| `monochrome`               | academic / consulting        | 纯黑白账簿感与高密阅读                 | 否   |
| `signal`                   | consulting / business-review | 海军蓝/纸色交替、机构型高密论证        | 否   |
| `studio`                   | product-launch / creative    | 黑底大字、展示型高对比                 | 否   |
| `vellum`                   | academic / research          | 暗底学术衬线与低密强调                 | 否   |
| `block-frame`              | teaching / product-launch    | 粗边框分区和模块化高密布局             | 否   |
| `capsule`                  | teaching / product-launch    | 胶囊形模块与多色分类                   | 否   |
| `cobalt-grid`              | academic / research          | 方格纸、细线与阶梯图形                 | 否   |
| `creative-mode`            | product-launch / creative    | 粗标题、多强调色与编辑式分区           | 否   |
| `daisy-days`               | teaching / workshop          | 手绘内嵌 SVG 装饰与教学语气            | 否   |
| `sakura-chroma`            | product-launch / creative    | 斜向彩带、压缩字阶与规格标注           | 否   |
| `scatterbrain`             | teaching / workshop          | 便签分组、手写/展示/衬线混排           | 否   |
| `bold-poster`              | product-launch / creative    | 超大海报字阶和稀疏构图                 | 否   |
| `broadside`                | product-launch / consulting  | 深色编辑稿、双语字阶与橙色强调         | 否   |
| `stencil-tablet`           | teaching / creative          | 模板镂空标题和考古式标注               | 否   |
| `course-module`            | teaching / technical         | 学习目标侧栏、代码/习题、知识检查序列  | 是   |
| `product-launch`           | product-launch               | 大英雄形状、明暗交替、产品功能与价目   | 是   |
| `knowledge-arch-blueprint` | technical / research         | 蓝图坐标、架构分区与关系链             | 否   |
| `graphify-dark-graph`      | technical / research         | 图网络节点、聚类证据与技术流程         | 否   |
| `hermes-cyber-terminal`    | technical / product-launch   | 终端视窗、控制台与拓扑叙事             | 否   |
| `weekly-report`            | business-review              | KPI、状态表、工作流进展和后续动作      | 否   |
| `architectural-plan`       | technical / consulting       | 交替左右面板、编号导航与星徽标注       | 是   |

入库时所有条目均为 `candidate`；许可为 beautiful/OpenDesign 的 23 个 MIT 候选和 OfficeCLI 的 1 个 Apache-2.0 候选。实际源路径、每文件哈希、字体 URL 与未引入资产逐条见 catalog。

## 5. 五套代表的适配证据

| ID                   | 原源颜色/字体与结构                                                                                                       | 适配时须保留/待核                                                                               |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `blue-professional`  | `template.json`：背景 `FDFAE7`、主色 `1E2BFA`；Space Grotesk 展示/Inter 正文                                              | 克制字阶与单强调色；CJK 字体映射及字体许可另核                                                  |
| `cartesian`          | `template.json`：`EDE8E0` / `E2DBD1`、文字 `1A1A1A`、强调 `8A8178`；Playfair Display / Inter；低密                        | 保留衬线与无衬线对比；原 HTML 有未锁版本 Chart.js CDN，不执行或依赖此 CDN 来证明原生图表        |
| `course-module`      | `style.css`：`FBFAF6`、`2D7D6E` 与 `D88A3A`；标题 Playfair Display，正文 Inter / Noto Sans SC；260px 目标栏与 56px 栏间距 | 保留教学目标侧栏、步骤/代码/习题关系；不将所有内容适配成同款卡片                                |
| `product-launch`     | `style.css`：白/`0A0A12` 交替，`FF5A36→FF8C5A→FFB36B`；Inter / Noto Sans SC；96/64px 主要字阶                             | 保留大字、英雄形状、明暗节奏；HTML CSS 效果与原生 PPTX 能力单列                                 |
| `architectural-plan` | `build.sh` 实际 token：`18293B`、`B5D5E3`、`F0BE3C`；13cm 左右交替面板、编号导航；脚本未指定 font family                  | `style.md` 概述色值与脚本近似但不相同，适配须注明取哪一份；字体是 Ofox 适配决策而非声称上游指定 |

OfficeCLI 的 `dark__architectural_plan.pptx` 二进制未引入，避免把其中潜在图片/字体/嵌入物直接当已核清资产。仅入库 style.md、build.sh 和 LICENSE/NOTICE；build.sh 只是静态参考，未执行。

## 6. 字体、外链与原始样例边界

每套的 `externalDependencies.fontUrls`、`otherUrls`、`fontFamiliesDeclared` 记录源文本中的声明；包括设计说明及共享包中未必实际使用的可选主题，因此是保守超集，不是“实际渲染使用字体”测量。Google Fonts URL 或字体名称不构成字体二进制分发许可证明。没有字体文件入库，CJK 字形覆盖、字重与替代后的版式仍待验证。

原 HTML 的外链保持原样，包括 Google Fonts、部分示例的 JavaScript CDN 或示例品牌链接；没有解析后下载这些外部资产。所有原样例仅作设计与适配参考，未作为离线已就绪 HTML 或安全执行入口。生成 benchmark 须使用独立自编的合成内容，不能把源样例的人名、品牌或性能数字继承为 Ofox 事实。

## 7. 本阶段检查与待办

- 通过：24 唯一 ID、5 个固定代表、143 份原字节 SHA-256 与固定 Git blob 对照、所选 HTML 的本地 src/href 依赖存在。
- 保留：MIT 版权/许可；OfficeCLI Apache-2.0 LICENSE/NOTICE；OpenDesign 上游说明及未知上游版本状态；所有原文件无修改。
- 未运行：源脚本、浏览器、字体/图片抓取、正式生成与视觉/PPTX 验收。本阶段没有改变运行时代码。
- 后续：5 代表适配、字体替代和许可闭包、固定合成样张、原生文字/图表/表格/公式编辑检查、macOS/Windows 目标 PowerPoint 复核及成本对照。
