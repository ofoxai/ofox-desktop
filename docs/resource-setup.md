# MCP and Skills setup

Ofox Desktop 1.5.0 · [English: MCP](#en-mcp) / [Skills](#en-skills) · [中文：MCP](#zh-mcp) / [Skills](#zh-skills) · [日本語：MCP](#ja-mcp) / [Skills](#ja-skills)

<a id="en-mcp"></a>

## English · Add an MCP server

1. Open **Library → MCP → Add MCP**. Enter the server's command or URL and the settings supplied by its publisher. Existing definitions can also be collected with **Import from apps**.
2. In the add form, choose the available target tools. **Codex / ChatGPT** is one shared configuration target.
3. Review the server details and selected tools, then click **Save and enable**. Check the resulting states and resolve any conflicts or failures shown. For an imported definition, review it first, then use its tool checkboxes in the MCP table; those checkboxes apply changes immediately.

Saving a definition or writing it to a tool does not prove the server connects. Dependencies, client trust, and OAuth may require separate setup; **Check connection** is a separate explicit operation.

<a id="en-skills"></a>

## English · Add Skills

1. Open **Library → Skills**. Enter an `owner/repo`, GitHub link, local folder, or ZIP file in **Skill source**, then click **Find skills**. You can also use **Discover**, or **Bring in** a skill found in your existing agents.
2. Select the skills you want and choose target tools in the **for** row. Review the skill names, source, and selected tools before continuing.
3. Click **Install skills: N** and check **Installation results**. Resolve same-name differences or failed targets as shown; existing library rows also provide per-tool controls that apply changes immediately.

Installing here collects the selected skill and links or copies it to supported targets. It does not run the skill's scripts or install its dependencies, and it does not prove that a client has executed the skill.

<a id="zh-mcp"></a>

## 中文 · 添加 MCP

1. 打开 **资源库 → MCP → 添加 MCP**，按发布者说明填写命令或 URL 及所需设置。已有定义也可通过 **从应用导入** 收录。
2. 在添加表单中选择可用的目标工具。**Codex / ChatGPT** 共用一个配置目标。
3. 核对服务器配置与目标工具后，点击 **保存并启用**，再查看结果并处理提示的冲突或失败。对于导入的定义，先检查内容，再操作 MCP 表格中的工具复选框；这些复选框会立即应用变更。

保存定义或写入工具配置不等于服务器连接成功。依赖、客户端信任及 OAuth 可能需要单独设置；**检查连接** 是需要明确执行的独立操作。

<a id="zh-skills"></a>

## 中文 · 添加 Skills

1. 打开 **资源库 → Skills**，在 **技能来源** 填入 `owner/repo`、GitHub 链接、本机文件夹或 ZIP 文件，点击 **查找技能**。也可使用 **发现**，或从 **Agent 中已有** 的技能中选择 **导入**。
2. 勾选所需技能，在 **给** 一栏选择目标工具；继续前核对技能名称、来源和目标。
3. 点击 **安装 N 个技能**，查看 **安装结果**，按提示处理同名内容差异或失败目标。资源库中已有技能也提供工具开关，操作后会立即应用变更。

这里的安装是收录所选技能，并通过链接或副本分发到受支持目标，不会运行技能脚本或安装其依赖，也不代表客户端已经执行该技能。

<a id="ja-mcp"></a>

## 日本語 · MCP を追加する

1. **リソース → MCP → MCP を追加** を開き、提供元の説明に従ってコマンドまたは URL と必要な設定を入力します。既存の定義は **アプリから取り込む** でも収集できます。
2. 追加フォームで利用可能な対象ツールを選びます。**Codex / ChatGPT** は同じ設定先を共有します。
3. サーバーの設定と対象ツールを確認し、**保存して有効化** をクリックします。結果を確認し、競合や失敗の表示に従って対処してください。取り込んだ定義は内容を確認してから MCP 表のツール別チェックボックスを操作します。この操作は変更を即時に適用します。

定義の保存やツールへの設定書き込みだけでは、接続成功を意味しません。依存関係、クライアントの信頼設定、OAuth は別途設定が必要な場合があります。**接続を確認** は明示的に実行する別の操作です。

<a id="ja-skills"></a>

## 日本語 · Skills を追加する

1. **リソース → Skills** を開き、**スキルの取得元** に `owner/repo`、GitHub リンク、ローカルフォルダー、ZIP ファイルのいずれかを入力して **スキルを探す** をクリックします。**探す** から見つけたり、**Agent にあるもの** から **取り込む** こともできます。
2. 必要なスキルを選択し、**対象** の行でツールを選びます。続ける前に、スキル名、取得元、対象を確認してください。
3. **スキルを N 件インストール** をクリックして **インストール結果** を確認します。同名で内容が異なる場合や失敗した対象は、表示に従って対処してください。ライブラリ内のスキルにもツール別の切り替えがあり、操作すると変更が即時に適用されます。

ここでのインストールは、選択したスキルを収集し、リンクまたはコピーで対応ツールに配布する操作です。スクリプトの実行や依存関係のインストールは行わず、クライアントによる実行成功も保証しません。

## Screen references / 界面参考 / 画面例

Screenshots use fictional sample data. / 截图使用虚构示例数据。/ 画面は架空のサンプルデータを使用しています。

- MCP: [Light / 浅色 / ライト](../assets/readme/library-light.webp) · [Dark / 深色 / ダーク](../assets/readme/library.webp)
- Skills: [Light / 浅色 / ライト](../assets/readme/skills-light.webp) · [Dark / 深色 / ダーク](../assets/readme/skills.webp)

[Resource behavior and supported targets / 资源规则与目标范围 / リソースの動作と対応範囲](library-workbench.md)
