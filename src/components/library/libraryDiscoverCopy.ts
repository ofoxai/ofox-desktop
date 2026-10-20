/** Locale fragments merged into `library` by the page integration. */
export const LIBRARY_DISCOVER_COPY = {
  zh: {
    discover2: {
      search: "搜索精选资源",
      searchPlaceholder: "名称、用途或发布者",
      category: "用途分类",
      all: "全部用途",
      results: "{{count}} 项精选",
      empty: "没有符合条件的精选资源。",
      reset: "清除筛选",
      categories: {
        docs: "文档检索",
        browser: "浏览器",
        files: "文件与网页",
        development: "开发协作",
        design: "设计辅助",
        communication: "沟通与审查",
      },
      details: "配置详情",
      closeDetails: "收起详情",
      publisher: "发布者",
      runtimeLabel: "运行方式",
      preparationLabel: "准备事项",
      licenseLabel: "许可与条款",
      reviewedAt: "文档核对：{{date}}",
      reviewBoundary: "文档核对不代表实际连接或工具执行已通过验收。",
      selectedSkillNote:
        "仅下载所选技能目录，保留其中的许可文件，并固定实际提交版本。",
      configurable: "可配置",
      nativeAuth: "需凭据或原生授权",
      checkLabel: "检查方式",
      checkOfox: "可由 Ofox 发起连接检查",
      checkNative: "请在对应工具中检查",
      notTested: "尚未实测",
      notCollected: "尚未加入",
      collected: "已加入我的资源",
      nativeOther: "本机有其他配置",
      viewExisting: "查看已有配置",
      locateNative: "查看本机配置",
      locationUnavailable: "当前无法定位，请返回列表重新扫描。",
      readOnly: "仅提供只读工具集",
      credentialBoundary: "仍需凭据；实际访问权限由凭据决定。",
      githubChoice: "默认选择官方只读工具集；可在表单中按官方说明修改 URL。",
      compatibilityNote:
        "以下工具具有既有写入适配；实际加载、授权及服务兼容性需在对应版本确认。",
      runtime: {
        http: "远程 HTTP",
        node: "本地命令 · Node.js",
        uv: "本地命令 · uvx",
        skill: "按需下载技能文件",
        instructions: "可编辑 Markdown",
      },
      licenses: {
        mit: "MCP 源码：MIT；远程服务另受服务条款约束。",
        apache: "源码：Apache-2.0；以所用版本的许可文件为准。",
        reference:
          "官方参考示例；项目处于 MIT / Apache-2.0 许可过渡，以来源文件为准。",
        service: "远程服务适用其发布者的使用条款。",
        mitService: "服务端源码：MIT；托管服务另受 GitHub 条款约束。",
        skill: "下载时保留技能目录内许可文件；实际提交与许可在详情中记录。",
        skillFonts: "保留技能许可与字体的独立许可；使用字体前查看对应条款。",
        template: "Ofox 可编辑模板，可按项目需求修改。",
      },
      recipes: {
        "microsoft-learn": "检索微软官方文档与代码示例。",
        deepwiki: "理解公开 GitHub 仓库的结构与文档。",
        playwright: "通过浏览器操作网页，辅助交互测试。",
        github: "读取获授权的仓库、Issue 和 Pull Request。",
        "theme-factory": "为页面、演示和文档统一配色与字体。",
        "canvas-design": "设计海报、图片与静态视觉作品。",
      },
      preparation: {
        context7: "网络连接；可选 API Key 以提高额度",
        "chrome-devtools": "Node.js LTS、npm 与稳定版 Chrome",
        filesystem: "Node.js、npm 和明确允许的文件夹",
        fetch: "uv / uvx 与兼容 Python 环境",
        "microsoft-learn": "网络连接，无需认证",
        deepwiki: "网络连接，仅面向公开仓库",
        playwright: "Node.js 18+ 与浏览器环境",
        github: "自备凭据，或在目标工具中完成授权",
        "frontend-design": "项目所需的前端开发环境",
        "webapp-testing": "Python、Playwright 与浏览器",
        "mcp-builder": "Python 或 Node.js 开发环境",
        "skill-creator": "Python 与技能声明的验证依赖",
        "theme-factory": "可编辑的目标文档或页面",
        "canvas-design": "按技能说明准备 Python、绘图依赖和字体",
        coding: "保存后可继续编辑内容",
        chinese: "保存后可继续编辑内容",
        review: "保存后可继续编辑内容",
      },
      requirements: {
        publicHttp:
          "保持网络连接；使用目标工具支持的 Streamable HTTP，无需本地运行时或认证。",
        playwright:
          "Node.js 18 或更新版本；首次使用前按官方说明准备所需浏览器。Ofox 不会自动安装。",
        github:
          "在目标工具中完成受支持的原生授权，或在表单填写有效凭据；不要将凭据写入 URL。",
        "theme-factory":
          "先查看技能说明，准备需要应用主题的页面、文档或演示内容。",
        "canvas-design":
          "先查看 SKILL.md，准备其要求的 Python、绘图依赖及合法可用字体。",
      },
      permissions: {
        "microsoft-learn": "查询发送给 Microsoft Learn，只访问公开文档与示例。",
        deepwiki: "仓库名称与问题发送给 DeepWiki；此配方不提供私有仓库访问。",
        playwright:
          "可读取和操作浏览器中的页面与会话；只连接你允许使用的浏览器环境。",
        github:
          "只读工具集减少可调用的写入操作，但不能代替最小权限凭据和原生授权。",
      },
    },
  },
  en: {
    discover2: {
      search: "Search curated resources",
      searchPlaceholder: "Name, purpose, or publisher",
      category: "Purpose category",
      all: "All purposes",
      results: "{{count}} curated resources",
      empty: "No curated resources match these filters.",
      reset: "Clear filters",
      categories: {
        docs: "Documentation",
        browser: "Browser",
        files: "Files and web",
        development: "Development",
        design: "Design",
        communication: "Communication and review",
      },
      details: "Configuration details",
      closeDetails: "Hide details",
      publisher: "Publisher",
      runtimeLabel: "Run method",
      preparationLabel: "Preparation",
      licenseLabel: "License and terms",
      reviewedAt: "Documentation reviewed: {{date}}",
      reviewBoundary:
        "Reviewing documentation does not verify a live connection or execution in a tool.",
      selectedSkillNote:
        "Download only the selected skill directory, retain its license files, and pin the actual commit.",
      configurable: "Configurable",
      nativeAuth: "Credentials or native authorization required",
      checkLabel: "Check method",
      checkOfox: "Ofox can start a connection check",
      checkNative: "Check in the target tool",
      notTested: "Not live-tested",
      notCollected: "Not added yet",
      collected: "In My resources",
      nativeOther: "Other local configuration exists",
      viewExisting: "View existing configuration",
      locateNative: "View local configuration",
      locationUnavailable:
        "Cannot locate it now. Return to the list and scan again.",
      readOnly: "Read-only toolset only",
      credentialBoundary:
        "Credentials are still required; their scope determines actual access.",
      githubChoice:
        "The official read-only toolset is selected by default. You can edit the URL in the form using the official instructions.",
      compatibilityNote:
        "These tools have existing write adapters. Verify loading, authorization, and service compatibility in the actual target version.",
      runtime: {
        http: "Remote HTTP",
        node: "Local command · Node.js",
        uv: "Local command · uvx",
        skill: "Download selected skill files",
        instructions: "Editable Markdown",
      },
      licenses: {
        mit: "MCP source: MIT. The remote service has separate terms.",
        apache:
          "Source: Apache-2.0. Consult the license in the version you use.",
        reference:
          "Official reference example. The project is transitioning from MIT to Apache-2.0; consult the source files.",
        service: "The remote service is subject to its publisher's terms.",
        mitService:
          "Server source: MIT. GitHub terms apply separately to the hosted service.",
        skill:
          "License files within the selected skill directory are retained. Details record the actual commit and license.",
        skillFonts:
          "Retain the skill license and separate font licenses. Read their terms before using the fonts.",
        template:
          "An editable Ofox template that you can adapt to your project.",
      },
      recipes: {
        "microsoft-learn":
          "Search Microsoft's official documentation and code examples.",
        deepwiki:
          "Understand the structure and documentation of public GitHub repositories.",
        playwright:
          "Operate web pages in a browser to support interaction testing.",
        github: "Read authorized repositories, issues, and pull requests.",
        "theme-factory":
          "Apply consistent colors and typography to pages, slides, and documents.",
        "canvas-design": "Design posters, images, and static visual work.",
      },
      preparation: {
        context7: "Network access; optional API key for higher limits",
        "chrome-devtools": "Node.js LTS, npm, and stable Chrome",
        filesystem: "Node.js, npm, and explicitly allowed folders",
        fetch: "uv / uvx and a compatible Python environment",
        "microsoft-learn": "Network access; no authentication",
        deepwiki: "Network access; public repositories only",
        playwright: "Node.js 18+ and a browser environment",
        github: "Your own credentials or authorization in the target tool",
        "frontend-design": "Your project's frontend development environment",
        "webapp-testing": "Python, Playwright, and a browser",
        "mcp-builder": "Python or Node.js development environment",
        "skill-creator":
          "Python and the skill's declared validation dependencies",
        "theme-factory": "An editable target document or page",
        "canvas-design":
          "Python, drawing dependencies, and fonts required by the skill",
        coding: "You can edit the content after saving",
        chinese: "You can edit the content after saving",
        review: "You can edit the content after saving",
      },
      requirements: {
        publicHttp:
          "Network access and Streamable HTTP support in the target tool. No local runtime or authentication is required.",
        playwright:
          "Node.js 18 or newer. Prepare the required browser using the official instructions before first use. Ofox does not install it automatically.",
        github:
          "Complete supported native authorization in the target tool, or enter valid credentials in the form. Never put credentials in the URL.",
        "theme-factory":
          "Read the skill instructions and prepare the page, document, or presentation to style.",
        "canvas-design":
          "Read SKILL.md first. Prepare its required Python environment, drawing dependencies, and properly licensed fonts.",
      },
      permissions: {
        "microsoft-learn":
          "Queries are sent to Microsoft Learn and access public documentation and examples only.",
        deepwiki:
          "Repository names and questions are sent to DeepWiki. This recipe does not provide private repository access.",
        playwright:
          "Can read and operate pages and sessions in the browser. Connect only a browser environment you permit.",
        github:
          "The read-only toolset reduces available write operations, but does not replace least-privilege credentials or native authorization.",
      },
    },
  },
  ja: {
    discover2: {
      search: "厳選リソースを検索",
      searchPlaceholder: "名前、用途、公開元",
      category: "用途の分類",
      all: "すべての用途",
      results: "厳選リソース {{count}} 件",
      empty: "条件に一致する厳選リソースはありません。",
      reset: "絞り込みを解除",
      categories: {
        docs: "ドキュメント検索",
        browser: "ブラウザー",
        files: "ファイルとウェブ",
        development: "開発支援",
        design: "デザイン",
        communication: "対話とレビュー",
      },
      details: "設定の詳細",
      closeDetails: "詳細を閉じる",
      publisher: "公開元",
      runtimeLabel: "実行方式",
      preparationLabel: "準備するもの",
      licenseLabel: "ライセンスと利用条件",
      reviewedAt: "ドキュメント確認日：{{date}}",
      reviewBoundary:
        "ドキュメントの確認は、実際の接続やツールでの実行の検証を意味しません。",
      selectedSkillNote:
        "選択したスキルのディレクトリだけを取得し、その中のライセンスファイルを保持して実際のコミットを固定します。",
      configurable: "設定可能",
      nativeAuth: "認証情報またはツール側の認可が必要",
      checkLabel: "確認方法",
      checkOfox: "Ofox から接続確認を開始できます",
      checkNative: "対象ツールで確認してください",
      notTested: "実機での接続は未検証",
      notCollected: "未追加",
      collected: "マイリソースに追加済み",
      nativeOther: "別のローカル設定があります",
      viewExisting: "既存の設定を表示",
      locateNative: "ローカル設定を表示",
      locationUnavailable:
        "現在は場所を確認できません。一覧に戻って再スキャンしてください。",
      readOnly: "読み取り専用ツールセットのみ",
      credentialBoundary:
        "認証情報は別途必要です。実際のアクセス権は認証情報によって決まります。",
      githubChoice:
        "公式の読み取り専用ツールセットが初期値です。公式手順に従い、フォームで URL を変更できます。",
      compatibilityNote:
        "以下は既存の書き込みアダプターを持つツールです。読み込み、認可、サービスの互換性は対象バージョンで確認してください。",
      runtime: {
        http: "リモート HTTP",
        node: "ローカルコマンド · Node.js",
        uv: "ローカルコマンド · uvx",
        skill: "選択したスキルファイルを取得",
        instructions: "編集可能な Markdown",
      },
      licenses: {
        mit: "MCP ソース：MIT。リモートサービスには別途利用条件が適用されます。",
        apache:
          "ソース：Apache-2.0。使用するバージョンのライセンスファイルを確認してください。",
        reference:
          "公式の参考実装です。MIT から Apache-2.0 への移行中のため、ソースファイルの条件を確認してください。",
        service: "リモートサービスには公開元の利用条件が適用されます。",
        mitService:
          "サーバーソース：MIT。ホスト型サービスには別途 GitHub の利用条件が適用されます。",
        skill:
          "選択したスキル内のライセンスファイルを保持します。実際のコミットとライセンスは詳細に記録されます。",
        skillFonts:
          "スキルと各フォントの個別ライセンスを保持します。フォント使用前に条件を確認してください。",
        template: "プロジェクトに合わせて変更できる Ofox のテンプレートです。",
      },
      recipes: {
        "microsoft-learn":
          "Microsoft の公式ドキュメントとコード例を検索します。",
        deepwiki: "公開 GitHub リポジトリの構造とドキュメントを理解します。",
        playwright:
          "ブラウザーでウェブページを操作し、対話テストを支援します。",
        github: "権限のあるリポジトリ、Issue、Pull Request を読み取ります。",
        "theme-factory": "ページ、スライド、文書の配色とフォントを統一します。",
        "canvas-design": "ポスター、画像、静的なビジュアル作品を制作します。",
      },
      preparation: {
        context7: "ネット接続。API キーは上限拡張のため任意",
        "chrome-devtools": "Node.js LTS、npm、安定版 Chrome",
        filesystem: "Node.js、npm、明示的に許可するフォルダー",
        fetch: "uv / uvx と互換性のある Python 環境",
        "microsoft-learn": "ネット接続。認証は不要",
        deepwiki: "ネット接続。公開リポジトリのみ",
        playwright: "Node.js 18 以上とブラウザー環境",
        github: "自分の認証情報、または対象ツールでの認可",
        "frontend-design": "プロジェクトのフロントエンド開発環境",
        "webapp-testing": "Python、Playwright、ブラウザー",
        "mcp-builder": "Python または Node.js の開発環境",
        "skill-creator": "Python とスキルが指定する検証用の依存関係",
        "theme-factory": "編集可能な対象文書またはページ",
        "canvas-design": "スキルが指定する Python、描画用依存関係、フォント",
        coding: "保存後も内容を編集できます",
        chinese: "保存後も内容を編集できます",
        review: "保存後も内容を編集できます",
      },
      requirements: {
        publicHttp:
          "ネット接続と対象ツールの Streamable HTTP 対応が必要です。ローカル実行環境や認証は不要です。",
        playwright:
          "Node.js 18 以上。初回利用前に公式手順で必要なブラウザーを準備してください。Ofox は自動インストールしません。",
        github:
          "対象ツールで対応する認可を完了するか、フォームに有効な認証情報を入力してください。URL に秘密情報を含めないでください。",
        "theme-factory":
          "スキルの説明を読み、テーマを適用するページ、文書、プレゼンテーションを用意してください。",
        "canvas-design":
          "先に SKILL.md を確認し、必要な Python、描画用依存関係、利用許諾のあるフォントを準備してください。",
      },
      permissions: {
        "microsoft-learn":
          "問い合わせは Microsoft Learn に送信され、公開ドキュメントと例のみを参照します。",
        deepwiki:
          "リポジトリ名と質問は DeepWiki に送信されます。この設定では非公開リポジトリにアクセスできません。",
        playwright:
          "ブラウザーのページやセッションを読み取り、操作できます。利用を許可したブラウザー環境のみ接続してください。",
        github:
          "読み取り専用ツールセットは書き込み操作を減らしますが、最小権限の認証情報や対象ツールでの認可に代わるものではありません。",
      },
    },
  },
} as const;
