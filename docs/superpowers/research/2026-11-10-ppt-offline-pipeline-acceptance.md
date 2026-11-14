# PPT 离线文件链验收：自编案例与原稿局部编辑

日期：2026-11-10。工作树：`$HOME/code/ofox-desktop-1.5.2`。本记录覆盖三套独立编写的叙事案例（10 + 8 + 8，共 26 页），以及单独的 70 页分页夹具、其第 70 页编辑副本。它们用于检查真实 PPTX 文件链，不作为真实模型生成质量实验。

本次使用现有工具、私有 HOME 和禁止网络的子进程，没有安装依赖、访问凭据、调用模型或生图提供商。没有操作或改写用户原稿。本记录的案例产物均保存在忽略的私有目录 `.real-eval/ppt-completion-2026-11-10/cases/`，不是新增产品模板。

公开来源原稿的页数、许可及离线转换见[来源覆盖审计](2026-11-10-ppt-source-coverage-audit.md)；六种风格的统一八页预览见[自编风格示例预览](2026-11-10-ppt-full-preview.md)。这些资源类别、页数和本次自编案例分别计数。

## 已核实的工具与执行方式

| 项目               | 版本或来源                                                | 证据范围                                                       |
| ------------------ | --------------------------------------------------------- | -------------------------------------------------------------- |
| 平台               | macOS arm64，`darwin-arm64`                               | 两份 generation 收据记录的平台                                 |
| Node               | `v22.23.4`                                                | 每次运行实际执行 `--version`；使用既有临时签名副本，不改安装包 |
| OfficeCLI          | `1.0.160`                                                 | 只读使用安装应用内的可执行文件；实际执行 `--version`           |
| Pyodide            | `314.0.7`                                                 | 现有 Python 引擎 `assets.json`，runner 校验与依赖库版本一致    |
| python-pptx        | `1.0.2`                                                   | 现有锁定 wheel                                                 |
| Matplotlib / NumPy | `3.10.8` / `2.4.6`                                        | 现有锁定 wheel                                                 |
| lxml / Pillow      | `6.1.3` / `12.2.0`                                        | 现有锁定 wheel                                                 |
| 图像栅格化         | 现有 `@napi-rs/canvas 1.0.13`、runtime `thumbnailFromSvg` | 依赖版本核查；实际生成 JPEG                                    |

Python 使用当前工作树的 helper/runner 和安装包内的固定离线引擎资源；OfficeCLI 使用 `OFFICECLI_NO_AUTO_RESIDENT=1`、`OFFICECLI_SKIP_UPDATE=1`。每个子进程套用 `sandbox-exec` 的网络禁用规则，Node 另加网络调用拒绝 guard。没有真实 API 请求，也没有通过安装新引擎替代已有生成能力。

生成时对 helper、runner、规划模块、预览 worker、技能、catalog 等记录 SHA-256；其中 helper 为 `8d1ea8ab7a4227cbfbd26810dc0469c35a16128580ee899697f80d9faed6fab9`，runner 为 `cbc54ab573f63246f24800fe9ee32033c2d2878741a6145d33ff78b43a2bb84d`。完整列表在 `cases/generation.json` 与 `cases/pagination/generation.json`。后续原稿编辑、错误分类和目标页预览代码有另外的测试证据；没有把早先成品冒充为此后全部源码重新生成的结果。

## 三套独立叙事案例

| 案例与风格                   | 页数、内容来源                                                                                       | 文件与结构检查                                                                                                        | 已生成的预览                                 |
| ---------------------------- | ---------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| 学术方法 / Cartesian         | 10 页；自行编写研究设计、计算核对与结论边界；三条合成数据 x=1/2/3、y=2/4/6                           | `academic/source.pptx`，68,813 B；3 个 OMML 数学对象、3 个 PNG 兼容回退，原生表格、流程形状及连线                     | 10 页均有 SVG、1920×1080 JPEG、contact sheet |
| 经营复盘 / Blue Professional | 8 页；自行编写模拟 CSV。M1/M2/M3 收入 360/403/464，列示成本 298/326/365，示例结余 62/77/99，单位万元 | `business/source.pptx`，51,827 B；2 个原生 Chart part、2 个内嵌 XLSX，图表缓存及单元格含给定数值                      | 8 页均有 SVG、1920×1080 JPEG、contact sheet  |
| 虚构产品 / Product Launch    | 8 页；自行编写 Focus 产品概念，明确没有真实发布、用户调研或效率收益数据                              | `product/source.pptx`，84,065 B；本地自编 SVG→PNG 素材经受控 manifest ID 在第 1、3 页嵌入，保留原生流程、比较及时间线 | 8 页均有 SVG、1920×1080 JPEG、contact sheet  |

三份源码、CSV、素材来源和执行脚本均在各案例目录及其父目录。产品插画是本地自编素材，采用 user-image 路径模拟受控 ID 回填，没有生成式图片调用。

经营数据的“结余”仅为本例的收入减列示成本，不等同完整财务利润。28.9%、22.5%、59.7% 分别来自上述收入、成本、结余的首末期变化。学术案例的 L=0 仅是三条自编数据的计算结果，没有被写成泛化或因果证据。

三份成品的完整 SHA-256：

| 文件                   | SHA-256                                                            |
| ---------------------- | ------------------------------------------------------------------ |
| `academic/source.pptx` | `9f6ae32981da840b9f788d8e029ddc631579cf89ac6a67aaee8445d2c16bbed1` |
| `business/source.pptx` | `6825b1c2c21a2f202dbaf286ac9a0cd0903cbdc74e5053984ceb175a95715d9a` |
| `product/source.pptx`  | `8023dc4a7d3350e49c1c443073edb8c8c0e49f8f1a87286b249747a3c697cd25` |

`cases/structural-summary.json` 与各目录的 `structural-inspection.json` 保存页序、页面文字、数学结构、图片分类、图表缓存和内嵌工作簿单元格。原生结构存在证明文件包含对应对象与数据；实际应用中的编辑、保存重开另行验收。

## 耗时与费用

以下是一次本机离线运行的墙钟耗时，包含冷启动、文件读写；不是模型速度、平均性能或相对旧版的改善值。

| 案例          | Python 生成 | OfficeCLI schema | 本例完整文件链 |
| ------------- | ----------: | ---------------: | -------------: |
| 学术 10 页    |     4.628 s |          0.678 s |       11.884 s |
| 经营 8 页     |     5.110 s |          0.725 s |       11.822 s |
| 产品 8 页     |     5.203 s |          0.691 s |       11.163 s |
| 70 页分页夹具 |     5.045 s |          0.836 s |        9.201 s |

前三例完整链包含生成、lint、schema、私有兼容渲染副本、全部 26 页 SVG/JPEG 和 contact sheet。三例同次脚本总计 35.536 s，执行时间为 2026-11-10 22:31:11.107–22:31:46.643（UTC+8）。70 页夹具完整链只抽渲染第 1/60/61/70 页，其独立脚本总计 10.094 s；不与三例全页渲染耗时混比。准确命令与逐项毫秒值保存在两份 generation 收据。

本组模型调用数为 0、生图提供商调用数为 0，未发生这两类请求费用。模型 tokens、单价估算、真实账单核对均未执行/不适用；不能把这些离线结果写成某个收费模型已运行且“费用为零”。本机电力及设备成本没有估计。

## 预览与人工检查的实际范围

所有 26 页均完成文件渲染；三份 lint 都为空，OfficeCLI schema 错误数为零。为让 OfficeCLI 正确展示数学公式，学术例在私有预览副本中选择已有 PNG Fallback，源 PPTX 的 OMML Choice 与兼容回退保持不变。公式 JPEG 的可读性证明该兼容预览链工作，不证明 OfficeCLI 直接排版了 OMML 或目标应用已经编辑成功。

已查看三份可读 contact sheet，并详细检查学术第 5/6 页、经营第 3/4 页、产品第 3/7 页。另一位只读 reviewer 独立看过同样的三份 contact 与六张主图：

- 未见明显裁切或文字重叠；学术帽号、下标、1/n、求和及平方项完整。
- 业务图形与给定模拟数值、万元及有限结余口径一致。
- 产品图文、阶段顺序及非真实发布日期说明清楚。
- 经营第 3 页 R/C 图例在 OfficeCLI JPEG 中实际存在，但字号很小；正文中文说明可读。该问题保留为预览限制，未据 schema=0 宣称视觉全部通过。

检查范围与图像 SHA 分别记录在 `cases/visual-review.json` 和 `cases/independent-visual-review.json`。其余页面经过 contact 检查，没有写成每张主图都已独立放大审查。

## 70 页文件与只改第 70 页

`pagination/source.pptx` 有 70 个真实 sldId、每页唯一标题/哨兵。第 1/60/61/70 页的 OXML、SVG 页序与哨兵核对通过。文件为 192,692 B，SHA-256 为 `79c0e2254727fe5a0bda204ea5195d3a1ad3843eb82170113fd12b2552c26ba1`。

使用 Host-only source flags 的实际 Pyodide 局部编辑副本为 `pagination/edited-page-70.pptx`：192,688 B，SHA-256 为 `4627675d6a2666bf7c09d4091f2ee8d84830c26ace70220274969c1b5172c172`。结果保留 70 页及原有有序 slide IDs，逐项对比后仅 `ppt/slides/slide70.xml` 的解压后内容字节改变；其它所有 part 内容字节相同，原稿完整 SHA 未变。此证明绑定到这个具体夹具，不能外推为所有外来 PPTX 均可无损保存。

编辑副本 schema 为 0，实际第 70 页 SVG、1920×1080 JPEG 以及固定持久化 worker 的 768×432 JPEG 已生成，并看到 `Edited target page 70` 和 `PAGE_SENTINEL_070`。固定 worker 的尺寸是其既有缩略图策略，未冒充 1920 主图。结构、文件 hash、页码及渲染收据分别位于：

- `pagination/source-edit-inspection.json`
- `pagination/edited/receipt.json`
- `pagination/edited/runtime-raster/receipt.json`

局部编辑首次生成本身没有单独计时，不补造耗时；其后只读 schema→SVG→JPEG 检查耗时 2.223 s，记录于 `pagination/edited/receipt.json`。这些离线验证没有点击真实应用的 70 页导航，也没有完成 WPS 编辑保存重开。

## 回归命令与签名环境边界

默认 `pnpm assistant:test` 本轮没有全绿：runtime 549 项中 537 通过、12 失败；独立 Search 102 项通过。`.real-eval/ppt-completion-2026-11-10/runtime-full.log` 保留原始失败。可复现的原因是 staged Developer ID Node 的 hardened library validation 拒绝工作树中不同 Team ID 的 canvas native binding；不是缺少依赖，也没有因此修改安装包或放宽生产签名。

随后用既有、同版 `v22.23.4` 私有 ad-hoc Node 完整执行 `--test test/*.test.mjs`，654 项中 653 通过、1 失败，日志 `runtime-private-node-full.log`。剩余失败来自 stdio fixture 仍默认启动 bundled Node，遇到同一 Team ID 限制。测试 fixture 新增显式 `OFOX_STDIO_TEST_NODE` 覆盖（绝对普通文件、拒 symlink；显式 `options.node` 仍优先，默认行为保留），对应 protocol/fixture 聚焦验证 23 项全部通过，0 跳过；日志为 `.real-eval/stdio-node-override-red.log`、`stdio-node-override-green.log`。没有把该聚焦复测写成默认命令或后续全量已经通过。

Python runner 的错误分类先取得 3 项行为失败回归，再以真实 runner/selection 两文件 22 项全绿收口：缺参数、未知风格、代码输入文件、素材清单等私有输入错误返回可恢复 exit 1；source 输入保留稳定 `assistant_presentation_source_unavailable`，引擎资源、Pyodide/依赖加载失败保留 exit 2。缺 wheel 的既有回归仍通过。日志为 `python-private-input-red.log`、`python-private-input-green.log`。

这些是工作树与私有执行器的测试证据，不能替代最终生产包的同 Team ID、签名、资源 hash 或安装后验收。本页不声明当前源码已经生产签名验收。

## 仍待验证与验收条件

1. 本组三套案例的 WPS/PowerPoint 原生文字、图表数据、公式编辑与保存重开未执行；Windows 未执行。需分别记录可编辑对象、实际动作、重开结果及版本。
2. 需求理解、自动候选设计判断、逐页计划、生图预算与失败回退尚需真实模型端到端验证；这组脚本计划是自行编写，不是模型决策。默认无预算禁止付费；真实运行的费用应区分授权上限、估算与账单。
3. 经营例 R/C 小图例、字体替代及各目标应用渲染仍需检查。结构、lint/schema=0、截图可读是不同的验收维度。
4. 70 页夹具只抽查四页渲染，完整应用中的分页、跳转、修订回退和当前目标页首图仍需运行态验证。任意外来原稿的关系图、共享 chart、主题和受保护 part 保留由 Host 逐文件后验核对。
5. 当前案例不是同内容 A/B/C/D 实验，没有模板数量或生成质量收益结论。扩大资源启用数量前，仍按[研究方案](2026-11-09-ppt-design-system-research.md)进行同内容比较、人工审查、耗时费用与实际可编辑性验收。

本记录不替代本轮全量前端、runtime、Rust、打包和签名门禁；它只描述已绑定产物与日志的离线文件链结果。

## 2026-11-11 补充：目标应用与真实 Host 预览

上文记录早先离线阶段的边界，后续补验不反写早期脚本收据。详见[工作区实施验收](2026-11-10-ppt-file-workspace-acceptance.md)。

- WPS 12.1.25895：学术第5页在公式工具中将w改a，保存、关闭、重开后保留；私有副本仍有两个原生OMML对象，原案例SHA未变。仅此符号编辑通过，不代表全部公式和字体一致性。
- 同版WPS：经营第3页原生图表改为样式2，保存、关闭、重开保留，六个柱图缓存值360/403/464与298/326/365以及两份内嵌工作簿仍在。数据编辑命令未得到可核实的数据编辑器，数据值修改仍未验证。PowerPoint16.109.1提示需要订阅才能编辑保存，未计通过。
- 当前源代码真实Host按需渲染70页夹具的全部1–70页及70页scale2，71次渲染164.48s通过；完整manifest、每页hash、尺寸和源SHA保存在私有receipt。不是逐页人工质量审阅。
- 新增Host下层make_python测试生成另一份70页私有种子，通过真实Python/OfficeCLI只改70页，仅slide70.xml变化，首图70.svg；make_python13.744s，整项27.29s。该下层验证不替代完整模型/账户/交付流程。
- 私有原生WKWebView实际检查了真实文件的70/70页、HTML脚本交互及隔离边界；这次界面中的70页来自真实文件，不再是8张图片重复的布局夹具。安装应用保持未变。

以上未发送收费模型/生图请求。真实模型设计质量、自动/手动同内容比较和生成式图片收益仍等待明确预算。
