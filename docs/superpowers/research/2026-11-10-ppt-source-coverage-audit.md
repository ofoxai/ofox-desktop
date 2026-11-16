# PPT 真实上游页面覆盖审计

本文记录 `$HOME/code/ofox-desktop-1.5.2` 既有工作树中的资源审计、核实脚本、真实源预览和对应证据。原始 vendor 字节及固定 catalog 来源未改变；后续 UI 和文件工作区接入由本轮实施验收记录另外描述。没有提交、推送、发布或替换安装应用，未发送模型请求。

## 四类对象

`resources/source-coverage.json`（路径前缀为 `assistant-runtime/skills/ofox-office-pptx/`）通过 `artifactKinds` 区分：固定上游原始 deck、Ofox 原生适配、自编展示示例、用户实际生成文件。第四类是会话授权的运行时产物，不在静态资源中登记任何用户文件。`availability` 独立表示 enabled/candidate。

现有八页展示应称“自编风格示例”，不能证明上游完整转换。五套公开风格的实际源页面共 **41 页**；navy 的 `template.pptx` 有 **0 个内容页、3 个布局（Cover/Section/Content）、1 个母版**。24 个公开候选中 5 个实验启用、19 个未开放，另加内置 navy 形成界面的六种风格。

| ID                 | 原始内容页 | 原始版式顺序                                                             | 当前原生适配边界                                            |
| ------------------ | ---------: | ------------------------------------------------------------------------ | ----------------------------------------------------------- |
| blue-professional  |         10 | 封面、目录、指标、仪表盘、左右拆分、横条图、引言、时间线、详细分析、结束 | 颜色、封面/标题、部分构图；不是十页逐页映射                 |
| cartesian          |         10 | 标题、议程、宣言、柱图、双栏、能力卡、折线图、时间线、团队、结束         | 编辑式标题/留白与部分面板；未将原 HTML 交互完整转成 PPTX    |
| course-module      |          7 | 封面、目标、概念、例题、练习、知识检查、总结                             | 目标侧栏与课程构件；不是七页所有交互/版式的等价转换         |
| product-launch     |          8 | 发布封面、介绍、三页特性、流程、价格、证言+CTA                           | 原生 hero/header/panels；不是原始渐变/动画/整套发布流程转换 |
| architectural-plan |          6 | 标题、服务、愿景使命、业务基础、业务详情、结束                           | 原生编号/面板等部分适配；不是六页完整坐标和工作流转换       |

机器清单保留每页的原文件路径、行号、class/layout、DOM selector/index。HTML 使用无脚本、无网络的 jsdom 解析；优先直接 `.deck/.presentation > .slide`，避免运行时 overview 克隆被当成原页；custom element 源使用 `deck-stage > section`。架构按六条实际顶层 `sl --prop background` 调用及 SLIDE 标题识别；不按 style.md 的推荐页号计数。

## 来源和许可

固定来源及许可证未改变：beautiful `e5e204fb1f3b06290846e7dcd7aceddabeceec8c`（MIT），OpenDesign html-ppt 子包 `07a86b1041531bfa0bec1006d9c0993985fee560`（MIT），OfficeCLI `c56512c8c7b76fe284f09940e4cd492470099d3e`（Apache-2.0，保留 LICENSE+NOTICE）。143 个原始文本/代码文件共 2,065,662 字节，逐文件 SHA-256 全部匹配；核实脚本另冻结整个原始路径+SHA 清单的 digest，防止同时改源文件和 catalog 后仍冒充原固定版本。

字体、图片和品牌/示例内容分别记录，不能由代码 MIT/Apache 许可推导为已核清。vendor 未引入字体或图片二进制。Blue/Cartesian 请求 Google Fonts；course/product 依赖本地 base/style/animations/runtime，fonts.css 有八项远程字体声明。原 HTML 都有浏览器脚本，不能直接挂在 App 同源执行。

公开本地参考 `AionCore@503a87199eb978790ca77675b39c84b241535bf3` 中，architectural-plan 的 build.sh/style.md 与 vendor 相同字节；另有六页 PPTX，但其来源只可登记为 AionCore 该提交，不能冒充 OfficeCLI 固定提交的原始二进制。本轮没有用它替代固定脚本。其实际 style.md/build.sh/PPTX 数量为 51/27/27；SKILL 中“52”不是本地实际数量，INDEX 还明确提示部分脚本存在重叠、不能照搬坐标。未读取或复制闭源 ofox-agent；本地没有发现可核对的 PPTMaster 源码。

## 真实源渲染

`resources/source-previews/` 保留五套 **41/41** 原始内容页的 1920×1080 JPEG，而非八页自编样张。HTML 使用现成 Chrome、全新私有 headless profile 和只服务公开资源白名单的独立 loopback server；CDP 阻断外部网络，保留原始 HTML/CSS 字节，只切换原页并暂停动画。字体采用本机已有 fallback，不代表原字体或 Windows 显示已经验证。

初次 Cartesian 只渲染 8/10，因第 4/7 页实际依赖未固定版本的 CDN Chart.js，明确没有发布空白或自绘替代。新增真实缺页断言先 RED 后，把预览专用依赖固定为 [Chart.js 4.6.1](https://github.com/chartjs/Chart.js/releases/tag/v4.6.1)，提交 `76d8cf895a7485dc838c90ef37e2bc9d9e388fcb`，UMD SHA-256 `87364cc2b2a76a6684929e7d6b80b18a897794db38400caff1fcf1791c6925d2`。保留其 [MIT](https://github.com/chartjs/Chart.js/blob/76d8cf895a7485dc838c90ef37e2bc9d9e388fcb/LICENSE.md)，也核对并保留内嵌 [@kurkle/color 0.3.2 MIT](https://github.com/kurkle/color/blob/eb0d1db26a35704553c93782bad8c3ac2d971597/LICENSE.md)。没有安装 npm 包；原 unversioned URL 只在隔离预览中映射本地固定字节，其余外部请求仍阻断。这是当前固定依赖环境，不声称知道原作者当时版本。

两页真实图已查看；证据还记录 Chart.version、原始两组 dataset 和 canvas 非透明像素，防止用自编 30/45/25 样张代替。架构六页则复制固定 build.sh 到新任务目录，以 OfficeCLI 1.0.160 真实重建，再转 SVG/JPEG；子进程使用独立 HOME、禁用自动 resident。重建 PPTX SHA 为 `b24e98d30cd03f7c2c211b4e174e3e64188ee18046fcc716f2c54f36a0ef8bd9`，不称原作者预编译二进制。

**原始架构渲染有真实问题**：2/4/5 页卡片序号/标题裁切，2–6 页底部统计/页脚挤压或裁切。已查看六页总览及第 2/5 页主图；没有改源文件“美容”，也没有因 schema 无错误而宣称视觉通过。`sourceDeck.preview.visualFindings` 保留稳定 code、severity、pages、message，供 UI 显示。五套总览和部分主图已看，尚不能称 41 页主图全部逐页视觉验收、完整原生适配或目标软件编辑验收。

## 检查和交接

首项回归真实 RED：缺少机器原始覆盖清单，断言 `A machine-readable original-deck coverage record is required` 失败。第二项真实 RED：Cartesian 原始第 4 页没有渲染，明确断言缺页失败。实现后四项聚焦测试 GREEN，覆盖 41 原始页、navy 0/3、十九候选、四类隔离、源哈希/版本/假等价漂移拒绝、CSS/脚本/许可边界及两页真实 canvas 证明。

```sh
node scripts/presentation-resource-coverage.mjs
node --test assistant-runtime/test/ppt-source-coverage.test.mjs
```

`--write` 只用于审阅后更新机器清单；默认核实命令只读。两个离线渲染脚本不安装依赖，分别用于实际 HTML 和唯一已审查的固定 OfficeCLI 演示脚本。UI 可消费 `sourceDeck.pageCount/pages/preview.pages/preview.visualFindings`，并保留 `nativeAdaptation` 与 `selfAuthoredPreview.sourceEquivalent:false` 的区别。用户实际文件仍由会话受权的 file workspace 提供，不来自此静态目录。最终全量 gates、UI 集成、平台验收和安装状态由根代理另行记录。
