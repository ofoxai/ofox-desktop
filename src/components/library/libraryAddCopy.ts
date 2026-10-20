/** Locale fragments merged into `library` by the page integration. */
export const LIBRARY_ADD_COPY = {
  zh: {
    batchLifecycle: {
      repreview: "重新预览剩余项",
      returnToSelection: "请返回资源列表，重新选择剩余项并预览。",
    },
    rowCheck: { last: "上次检查：{{state}} · {{time}}" },
    updateChecks: {
      description:
        "逐项检查所选技能，不会自动更新或应用；查看详情时会重新核验版本和差异。",
      start: "检查所选技能",
      stop: "停止检查",
      stopping: "正在停止…",
      open: "查看更新详情",
      progress: "已检查 {{completed}} / {{total}}",
      cleanupFailed: "临时候选未能清理，已停止后续检查。请关闭后重试。",
      state: {
        queued: "待检查",
        checking: "正在检查…",
        updateAvailable: "有更新可查看",
        upToDate: "已是最新版本",
        notTracked: "未跟踪远端版本，已跳过。",
        sourceMissing: "远端来源已不存在。",
        targetModified: "已跳过：工具中的副本有本地修改。",
        sourceModified: "已跳过：资源库源文件有本地修改或无法核验。",
        failed: "检查失败，可打开详情后重试。",
        cancelled: "已停止",
      },
    },
    instructionExperience: {
      view: "指令内容视图",
      edit: "编辑",
      preview: "预览",
      discard: "放弃修改",
      shortcut: "⌘ / Ctrl + S 保存",
      emptyPreview: "暂无内容，可切回编辑后添加。",
      ownSummary: "自有内容 {{lines}}",
      technicalDetails: "技术详情",
    },
    discover: {
      title: "精选资源",
      intro: "选择后可编辑配置、保存草稿，再预览应用到工具。",
      choose: "选择 {{name}}",
      source: "查看来源",
      reference: "官方参考示例",
      dependencies: "使用前准备",
      permissionsLabel: "使用范围",
      compatible: "可配置的工具",
      noExecution: "浏览和选择不会启动服务、运行脚本或安装依赖。",
      pinNote: "仅将所选技能导入资源库；下载仓库归档后固定实际提交版本。",
      templateSource: "Ofox 可编辑模板",
      templates: {
        coding: "通用编程",
        chinese: "中文沟通",
        review: "代码审查",
      },
      recipes: {
        context7: "为编程问题补充库和框架文档。",
        "chrome-devtools": "检查网页、排查浏览器问题与测试交互。",
        filesystem: "让工具访问你明确选择的本地文件夹。",
        fetch: "读取网页并提取正文内容。",
        "frontend-design": "帮助生成清晰、完整的前端界面。",
        "webapp-testing": "用浏览器测试本地网页及界面交互。",
        "mcp-builder": "帮助设计与实现 MCP 服务器。",
        "skill-creator": "帮助编写、改进和验证技能。",
        coding: "约定代码修改、测试与结果说明方式。",
        chinese: "使用中文清晰沟通，保留代码和专有名称。",
        review: "聚焦可验证的问题、影响和代码位置。",
      },
      requirements: {
        context7:
          "保持网络可用；建议在 Context7 创建 API Key，再填入 Authorization 请求头。",
        chrome: "先安装 Node.js LTS、npm 和当前稳定版 Chrome。",
        filesystem:
          "先安装 Node.js 和 npm，并填写允许访问的文件夹；不要直接填写整个磁盘。",
        fetch: "先安装 uv（提供 uvx）及兼容的 Python 环境。",
        "frontend-design": "使用前查看技能说明，并准备项目所需的前端环境。",
        "webapp-testing": "使用前按技能说明准备 Python、Playwright 和浏览器。",
        "mcp-builder": "使用前按技能说明准备 Python 或 Node.js 开发环境。",
        "skill-creator":
          "使用前查看技能说明；验证脚本可能需要 Python 及额外依赖。",
      },
      permissions: {
        remoteDocs: "查询内容会发送给 Context7；API Key 由你自行提供。",
        browser:
          "可读取并操作连接的浏览器内容；默认关闭该 MCP 的使用统计和 CrUX 查询。",
        filesystem: "可读取、修改或删除允许文件夹内的文件。",
        web: "可向网站发送请求并读取内容；访问范围以服务自身限制为准。",
        skillFiles:
          "技能可能包含脚本和依赖；导入不会执行它们，使用前可在详情中检查。",
      },
    },
    addFlow: {
      name: "名称",
      transport: "连接方式",
      stdio: "本地命令",
      http: "远程 HTTP",
      sse: "远程 SSE",
      command: "命令",
      arguments: "参数",
      argumentsHint: "每行一个参数；带空格的路径直接填写，不添加 shell 引号。",
      url: "服务器 URL",
      environment: "环境变量",
      headers: "请求头",
      addEnvironment: "添加环境变量",
      addHeader: "添加请求头",
      pairName: "名称 {{index}}",
      pairValue: "值 {{index}}",
      removeEnvironment: "移除环境变量 {{index}}",
      removeHeader: "移除请求头 {{index}}",
      directory: "允许访问的文件夹",
      directoryHint: "填写完整路径，仅向此服务开放你选择的目录。",
      save: "保存到我的资源",
      preview: "选择工具并预览",
      advanced: "高级字段",
      paste: "粘贴 JSON",
      jsonLabel: "MCP 配置 JSON",
      useJson: "填入表单",
      extraFields: "已保留 {{count}} 个高级字段，可在高级模式中检查。",
      secretsHint: "私密值仅用于保存配置，不会作为列表摘要展示。",
      draftNote: "保存草稿不会写入工具配置；应用前会显示变更预览。",
      errors: {
        missingName: "请填写名称。",
        missingCommand: "请填写可执行命令。",
        invalidUrl:
          "请填写不含用户名、密码或查询参数的 HTTP(S) URL；授权信息请放入请求头。",
        invalidPair: "请填写有效的变量或请求头名称。",
        duplicateEnvironment: "环境变量名称重复。",
        duplicateHeader: "请求头名称重复，大小写视为相同。",
        maskedValue: "不能保存脱敏占位符，请填写真实值或移除此字段。",
        invalidJson: "请粘贴有效的单个 MCP 配置。",
        oneServer: "一次只添加一个 MCP，请先选择其中一项。",
        advancedRequired: "此配置需要高级模式，无法安全转换为普通表单。",
        missingDirectory: "请填写允许访问的完整文件夹路径。",
        saveFailed: "草稿未保存，请重试。",
      },
    },
  },
  en: {
    batchLifecycle: {
      repreview: "Preview remaining changes again",
      returnToSelection:
        "Return to the resource list, select the remaining changes, and create a new preview.",
    },
    rowCheck: { last: "Last check: {{state}} · {{time}}" },
    updateChecks: {
      description:
        "Check selected skills one at a time without updating or applying them. Details recheck the version and differences.",
      start: "Check selected skills",
      stop: "Stop checks",
      stopping: "Stopping…",
      open: "View update details",
      progress: "Checked {{completed}} / {{total}}",
      cleanupFailed:
        "A temporary candidate could not be cleaned up, so further checks stopped. Close this view and retry.",
      state: {
        queued: "Waiting",
        checking: "Checking…",
        updateAvailable: "Update available for review",
        upToDate: "Up to date",
        notTracked: "No tracked remote version; skipped.",
        sourceMissing: "The remote source no longer exists.",
        targetModified: "Skipped: the tool copy has local changes.",
        sourceModified:
          "Skipped: the library source has local changes or could not be verified.",
        failed: "Check failed. Open details to retry.",
        cancelled: "Stopped",
      },
    },
    instructionExperience: {
      view: "Instruction content view",
      edit: "Edit",
      preview: "Preview",
      discard: "Discard changes",
      shortcut: "⌘ / Ctrl + S to save",
      emptyPreview: "No content yet. Switch to Edit to add instructions.",
      ownSummary: "Own content: {{lines}}",
      technicalDetails: "Technical details",
    },
    discover: {
      title: "Recommended resources",
      intro:
        "Choose a resource, edit its configuration, then save a draft or preview it for your tools.",
      choose: "Choose {{name}}",
      source: "View source",
      reference: "Official reference example",
      dependencies: "Before use",
      permissionsLabel: "Access",
      compatible: "Supported tools",
      noExecution:
        "Browsing and choosing do not start services, run scripts, or install dependencies.",
      pinNote:
        "Only the selected skill is imported; a repository archive is downloaded and the actual commit is pinned.",
      templateSource: "Editable Ofox template",
      templates: {
        coding: "General coding",
        chinese: "Chinese communication",
        review: "Code review",
      },
      recipes: {
        context7: "Look up library and framework documentation while coding.",
        "chrome-devtools":
          "Inspect websites, debug browser issues, and test interactions.",
        filesystem: "Give tools access to local folders you explicitly choose.",
        fetch: "Read web pages and extract their content.",
        "frontend-design": "Create complete, clear frontend interfaces.",
        "webapp-testing":
          "Test local web pages and interactions with a browser.",
        "mcp-builder": "Design and implement MCP servers.",
        "skill-creator": "Write, improve, and validate skills.",
        coding: "Set expectations for code changes, tests, and reports.",
        chinese:
          "Communicate clearly in Chinese while preserving code and proper names.",
        review: "Focus on verifiable findings, impact, and code locations.",
      },
      requirements: {
        context7:
          "Internet access is required. Create a Context7 API key if needed, then add it in the Authorization header.",
        chrome: "Install Node.js LTS, npm, and current stable Chrome first.",
        filesystem:
          "Install Node.js and npm, then enter the permitted folder. Avoid granting access to an entire disk.",
        fetch:
          "Install uv (which provides uvx) and a compatible Python environment first.",
        "frontend-design":
          "Read the skill instructions and prepare the frontend environment your project needs.",
        "webapp-testing":
          "Prepare Python, Playwright, and browsers as described by the skill.",
        "mcp-builder":
          "Prepare the Python or Node.js environment described by the skill.",
        "skill-creator":
          "Read the skill instructions; validation scripts may require Python and additional dependencies.",
      },
      permissions: {
        remoteDocs:
          "Queries are sent to Context7. You provide your own API key.",
        browser:
          "Can inspect and change the connected browser. This recipe disables MCP usage statistics and CrUX queries.",
        filesystem:
          "Can read, modify, or delete files within permitted folders.",
        web: "Can request and read websites, subject to the server's own access limits.",
        skillFiles:
          "Skills may contain scripts and dependencies. Import does not execute them; inspect them in the details before use.",
      },
    },
    addFlow: {
      name: "Name",
      transport: "Connection",
      stdio: "Local command",
      http: "Remote HTTP",
      sse: "Remote SSE",
      command: "Command",
      arguments: "Arguments",
      argumentsHint:
        "One argument per line. Enter paths with spaces directly, without shell quotes.",
      url: "Server URL",
      environment: "Environment variables",
      headers: "Headers",
      addEnvironment: "Add environment variable",
      addHeader: "Add header",
      pairName: "Name {{index}}",
      pairValue: "Value {{index}}",
      removeEnvironment: "Remove environment variable {{index}}",
      removeHeader: "Remove header {{index}}",
      directory: "Permitted folder",
      directoryHint:
        "Enter an absolute path. Grant this server access only to the folder you choose.",
      save: "Save to My resources",
      preview: "Choose tools and preview",
      advanced: "Advanced fields",
      paste: "Paste JSON",
      jsonLabel: "MCP configuration JSON",
      useJson: "Fill form",
      extraFields:
        "{{count}} advanced fields are preserved. Inspect them in advanced mode.",
      secretsHint:
        "Private values are used for the saved configuration and are excluded from list summaries.",
      draftNote:
        "Saving a draft does not write tool configuration. Changes are previewed before applying.",
      errors: {
        missingName: "Enter a name.",
        missingCommand: "Enter an executable command.",
        invalidUrl:
          "Enter an HTTP(S) URL without a username, password, or query parameters. Put authorization in headers.",
        invalidPair: "Enter a valid environment variable or header name.",
        duplicateEnvironment: "Environment variable names must be unique.",
        duplicateHeader: "Header names must be unique, ignoring letter case.",
        maskedValue:
          "Redacted placeholders cannot be saved. Enter the real value or remove the field.",
        invalidJson: "Paste one valid MCP configuration.",
        oneServer: "Add one MCP server at a time. Choose one definition first.",
        advancedRequired:
          "This configuration needs advanced mode and cannot be safely converted to the form.",
        missingDirectory: "Enter an absolute path for the permitted folder.",
        saveFailed: "The draft was not saved. Try again.",
      },
    },
  },
  ja: {
    batchLifecycle: {
      repreview: "残りの変更を再プレビュー",
      returnToSelection:
        "リソース一覧に戻り、残りの項目を選択して再度プレビューしてください。",
    },
    rowCheck: { last: "前回の確認：{{state}} · {{time}}" },
    updateChecks: {
      description:
        "選択したスキルを 1 件ずつ確認します。自動更新や適用は行わず、詳細を開くとバージョンと差分を再確認します。",
      start: "選択したスキルを確認",
      stop: "確認を停止",
      stopping: "停止中…",
      open: "更新の詳細を見る",
      progress: "確認済み {{completed}} / {{total}}",
      cleanupFailed:
        "一時候補を削除できないため、後続の確認を停止しました。画面を閉じて再試行してください。",
      state: {
        queued: "確認待ち",
        checking: "確認中…",
        updateAvailable: "確認できる更新があります",
        upToDate: "最新バージョンです",
        notTracked: "リモート版を追跡していないためスキップしました。",
        sourceMissing: "リモートの提供元が存在しません。",
        targetModified:
          "スキップ：ツール側のコピーにローカルの変更があります。",
        sourceModified:
          "スキップ：ライブラリの元ファイルに変更があるか、検証できません。",
        failed: "確認に失敗しました。詳細から再試行できます。",
        cancelled: "停止しました",
      },
    },
    instructionExperience: {
      view: "指示内容の表示",
      edit: "編集",
      preview: "プレビュー",
      discard: "変更を破棄",
      shortcut: "⌘ / Ctrl + S で保存",
      emptyPreview: "まだ内容がありません。編集に切り替えて追加してください。",
      ownSummary: "自分の内容：{{lines}}",
      technicalDetails: "技術的な詳細",
    },
    discover: {
      title: "おすすめのリソース",
      intro:
        "リソースを選び、設定を編集してから、下書きの保存またはツールへの適用をプレビューします。",
      choose: "{{name}} を選択",
      source: "提供元を見る",
      reference: "公式の参考実装",
      dependencies: "使用前の準備",
      permissionsLabel: "アクセス範囲",
      compatible: "対応ツール",
      noExecution:
        "閲覧と選択では、サービスの起動、スクリプトの実行、依存関係のインストールは行いません。",
      pinNote:
        "選択したスキルのみを取り込みます。リポジトリのアーカイブをダウンロードし、実際のコミットを固定します。",
      templateSource: "編集可能な Ofox テンプレート",
      templates: {
        coding: "基本のコーディング",
        chinese: "中国語での会話",
        review: "コードレビュー",
      },
      recipes: {
        context7:
          "コーディング中にライブラリやフレームワークのドキュメントを参照します。",
        "chrome-devtools":
          "ウェブページの確認、ブラウザーの問題調査、操作のテストを行います。",
        filesystem:
          "明示的に選んだローカルフォルダーへのアクセスを許可します。",
        fetch: "ウェブページを読み取り、本文を抽出します。",
        "frontend-design":
          "分かりやすく完成度の高いフロントエンドを作成します。",
        "webapp-testing":
          "ブラウザーでローカルページと画面操作をテストします。",
        "mcp-builder": "MCP サーバーの設計と実装を支援します。",
        "skill-creator": "スキルの作成、改善、検証を支援します。",
        coding: "コード変更、テスト、結果報告の方針を定めます。",
        chinese: "コードや固有名詞を保ちながら、中国語で分かりやすく伝えます。",
        review: "検証可能な問題、影響、コード位置に集中します。",
      },
      requirements: {
        context7:
          "ネット接続が必要です。必要に応じて Context7 の API キーを作成し、Authorization ヘッダーに追加してください。",
        chrome:
          "先に Node.js LTS、npm、安定版 Chrome をインストールしてください。",
        filesystem:
          "先に Node.js と npm を用意し、許可するフォルダーを入力してください。ディスク全体の指定は避けてください。",
        fetch:
          "先に uv（uvx を含む）と互換性のある Python 環境を用意してください。",
        "frontend-design":
          "スキルの説明を確認し、プロジェクトに必要なフロントエンド環境を用意してください。",
        "webapp-testing":
          "スキルの説明に従い、Python、Playwright、ブラウザーを用意してください。",
        "mcp-builder":
          "スキルの説明に従い、Python または Node.js の開発環境を用意してください。",
        "skill-creator":
          "スキルの説明を確認してください。検証スクリプトに Python や追加の依存関係が必要な場合があります。",
      },
      permissions: {
        remoteDocs:
          "問い合わせは Context7 に送信されます。API キーはご自身で用意します。",
        browser:
          "接続したブラウザーの内容を読み取り、操作できます。この設定では MCP の使用統計と CrUX 問い合わせを無効にします。",
        filesystem:
          "許可したフォルダー内のファイルを読み取り、変更、削除できます。",
        web: "サーバー自身の制限の範囲内で、ウェブサイトへのリクエストと読み取りを行います。",
        skillFiles:
          "スキルにスクリプトや依存関係が含まれる場合があります。インポートでは実行されず、使用前に詳細で確認できます。",
      },
    },
    addFlow: {
      name: "名前",
      transport: "接続方法",
      stdio: "ローカルコマンド",
      http: "リモート HTTP",
      sse: "リモート SSE",
      command: "コマンド",
      arguments: "引数",
      argumentsHint:
        "1 行に 1 つの引数を入力します。空白を含むパスにもシェルの引用符は付けません。",
      url: "サーバー URL",
      environment: "環境変数",
      headers: "リクエストヘッダー",
      addEnvironment: "環境変数を追加",
      addHeader: "ヘッダーを追加",
      pairName: "名前 {{index}}",
      pairValue: "値 {{index}}",
      removeEnvironment: "環境変数 {{index}} を削除",
      removeHeader: "ヘッダー {{index}} を削除",
      directory: "許可するフォルダー",
      directoryHint:
        "絶対パスを入力し、選択したフォルダーだけへのアクセスを許可します。",
      save: "マイリソースに保存",
      preview: "ツールを選んでプレビュー",
      advanced: "詳細フィールド",
      paste: "JSON を貼り付け",
      jsonLabel: "MCP 設定 JSON",
      useJson: "フォームに入力",
      extraFields:
        "{{count}} 個の詳細フィールドを保持しています。詳細モードで確認できます。",
      secretsHint:
        "秘密の値は設定の保存にのみ使い、一覧の概要には表示しません。",
      draftNote:
        "下書きの保存ではツールの設定を書き換えません。適用前に変更内容を確認できます。",
      errors: {
        missingName: "名前を入力してください。",
        missingCommand: "実行可能なコマンドを入力してください。",
        invalidUrl:
          "ユーザー名、パスワード、クエリを含まない HTTP(S) URL を入力してください。認証情報はヘッダーに指定します。",
        invalidPair: "有効な環境変数名またはヘッダー名を入力してください。",
        duplicateEnvironment: "環境変数名が重複しています。",
        duplicateHeader:
          "ヘッダー名が重複しています。大文字と小文字は区別しません。",
        maskedValue:
          "伏せ字は保存できません。実際の値を入力するか、フィールドを削除してください。",
        invalidJson: "有効な MCP 設定を 1 件貼り付けてください。",
        oneServer: "MCP は 1 件ずつ追加します。先に 1 件を選択してください。",
        advancedRequired:
          "この設定は安全にフォームへ変換できません。詳細モードを使用してください。",
        missingDirectory: "許可するフォルダーの絶対パスを入力してください。",
        saveFailed: "下書きを保存できませんでした。再試行してください。",
      },
    },
  },
} as const;
