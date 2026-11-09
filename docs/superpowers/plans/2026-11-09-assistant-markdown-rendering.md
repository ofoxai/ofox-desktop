# 回复渲染修复（阶段 1、2）

日期：2026-11-09　分支：`fix/assistant-markdown-render`（自 `feat/assistant-1.5.1` de8f9cc1）

## 为什么做

用户真机截图里两类问题，已用本机原回复复现：

1. **中文加粗失效**：`**说明：**正文`、`**多头注意力（MHA）**后面`、`卡在**「推送镜像」**这一步`。CommonMark 规定收尾的 `**`
   前面是标点、后面紧跟文字时不能收尾（开头同理），中文天天这么写。本机 107 条助手回复 5 条中招，抽样 4000 条 Claude Code 回复 6 条，全是中文。
2. **公式吞掉后文**：`$$公式…⏎…公式$$` 的开头、收尾 `$$` 都没单独占行，remark-math 只丢掉首行然后一直吞到回复末尾，
   之后的正文、表格、标题全被当公式，KaTeX 失败，整段红字。

同类问题（顺带修）：

3. `**http://localhost:3000**，打开即可`：GFM 网址识别到空白才停，`**，打开即可` 被吞进网址，加粗失效。现有的网址尾巴修正在解析之后，救不回来。
4. 单独一行的 `$$E=mc^2$$` 被当成行内小公式（不居中）。
5. 表格里的 `$|x|$`：竖线把单元格切开。
6. 公式出错时整段红色（KaTeX 默认 `#cc0000`）。
7. 没有收尾的 `$$`（模型漏写）同样会吞掉后文。

参考：dsh 在解析层处理中文加粗、给网址识别打补丁（遇非 ASCII 标点即止）、公式多种写法做成分隔符扩展；
AionUi 只做了 `\[ \]` 改写。我们保持 react-markdown，借用思路，不抄代码。

## 规则

| # | 改动 | 位置 |
|---|---|---|
| A | 加 `remark-cjk-friendly@2.3.4`、`remark-cjk-friendly-gfm-strikethrough@2.3.3`（锁版本，MIT），所有 SessionMarkdown 都用（助手、会话页、资源库） | `SessionMarkdown.tsx` |
| B | 解析前切断网址：正文里（代码外、链接目标外）的裸网址遇到全角标点即止，去掉其前的 `*_~` 与 ASCII 句末标点，写成 `<url>`（`www.` 写成 `[www…](http://www…)`）；没有全角标点紧跟的网址不动 | `markdownNormalize.ts` |
| C | `$$` 整理：开头 `$$` 在行首（前面只有空白、`>`）且公式跨行、或收尾 `$$` 后到行尾只有空白 → 改成开头、收尾各占一行的公式块（保留引用/缩进前缀）；收尾后还有文字时，文字另起一段；行中的 `$$…$$` 仍是行内公式 | `markdownMath.ts` |
| D | 找不到收尾的 `$$` 原样显示为文字（`\$\$`），不吞后文 | `markdownMath.ts` |
| E | GFM 表格行里公式中的 `\|` 写成 `\Vert `、`|` 写成 `\vert ` | `markdownMath.ts` |
| F | KaTeX 出错显示为灰色等宽源码，悬停看原因（`errorColor` 用正文弱色，不用红） | `markdownMath.ts`、`SessionMarkdown.tsx` |

不做（本轮）：流式冻结段落与“正在写的段落先不渲染公式”（阶段 3，按 dsh 设计，用户验收后做）；`<br>`、中文软换行、`<think>`（本机样本未出现）。

## 测试

- `tests/components/MarkdownRenderRegressions.test.tsx`：每类一条以上真实形态的样例（合成文本，不含用户内容），在 jsdom 里渲染 SessionMarkdown，断言：
  正文里没有残留的 `**`/`~~`、加粗数量、公式块数量、无 `.katex-error`、公式后的标题/表格/段落仍在、网址 `href` 不含 `*` 和全角标点。
- `tests/lib/markdownMath.test.ts`、`tests/lib/markdownNormalize.test.ts`：每条规则的边界（代码里不改、引用块、列表缩进、货币 `$`、已规范的输入不变）。
- `tests/markdown/renderAudit.test.tsx` + `pnpm render:audit`：设 `RENDER_AUDIT_DIR` 时扫描本机助手会话逐条渲染并报告，修复后应为零。

## 验收

1. 上述测试 + `pnpm test:unit`、`pnpm typecheck`、`pnpm format:check` 全过。
2. `RENDER_AUDIT_DIR=~/.ofox-desktop/assistant pnpm render:audit` 归零（基线：加粗 5 条、吞公式 1 条）。
3. 一次独立审查。
4. 签名打包装到本机，用户看两张截图对应的对话。
