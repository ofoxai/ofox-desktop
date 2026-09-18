"""
步骤定义：5 个安装步骤，统一接口
每个步骤包含：检测、终端命令/内联执行、验证、失败提示
仅使用 Python 3.9 标准库

注意：Xcode CLT 安装由 init.sh (bash) 处理，不在此列。
"""
from __future__ import annotations

import os
import subprocess

from app.shell import login_shell_argv
from typing import Callable

# nvm 命令前缀：加载 nvm 环境
NVM_PREFIX = 'export NVM_DIR="$HOME/.nvm" && [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh" && '

# 全局区域设置，由 init.py 在启动时设置
REGION = "CN"


def set_region(region: str) -> None:
    global REGION
    REGION = region


def _cmd_exists(cmd: str) -> bool:
    """检测命令是否存在"""
    return _shell_check(f"command -v {cmd}")


def _shell_check(cmd: str) -> bool:
    """
    用 login shell 执行检测命令，返回是否成功。

    必须走 login shell：应用从 Finder/Dock 启动时 PATH 只有系统目录，不含
    nvm / fnm / volta / Homebrew 往用户 shell rc 里加的路径。用裸 shell 检测会把
    已装的工具判成未装，导致反复重装（fizzy #865）。详见 app/shell.py。
    """
    return subprocess.run(
        login_shell_argv(cmd),
        capture_output=True,
    ).returncode == 0


# ── 基类 ────────────────────────────────────────────────────────────────────

class Step:
    name: str = ""
    description: str = ""
    needs_terminal: bool = False
    timeout: int = 300          # 秒
    poll_interval: float = 3.0  # 秒

    def check(self) -> bool:
        """检测是否已安装（用于跳过和轮询）"""
        return False

    def terminal_command(self) -> str:
        """新终端窗口中执行的命令（needs_terminal=True 时使用）"""
        return ""

    def run_inline(self, log_fn: Callable[[str], None]) -> bool:
        """主进程中直接执行（needs_terminal=False 时使用）"""
        return True

    def verify(self) -> bool:
        """安装后验证"""
        return self.check()

    def failure_hint(self) -> str:
        """失败时的排障建议"""
        return "请检查网络连接后重试。"

    def should_skip(self) -> bool:
        """跳过条件"""
        return False


# ── Step 1: nvm ──────────────────────────────────────────────────────────────

class NvmStep(Step):
    name = "nvm"
    description = "Node.js 版本管理工具"
    needs_terminal = True
    timeout = 300  # 5 分钟
    poll_interval = 3.0

    def check(self) -> bool:
        return os.path.isfile(os.path.expanduser("~/.nvm/nvm.sh"))

    def terminal_command(self) -> str:
        if REGION == "CN":
            install_cmd = (
                'NVM_SOURCE=https://gitee.com/mirrors/nvm.git '
                'bash -c "$(curl -fsSL https://gitee.com/mirrors/nvm/raw/master/install.sh)"'
            )
            mirror_line = 'export NVM_NODEJS_ORG_MIRROR=https://npmmirror.com/mirrors/node'
        else:
            install_cmd = (
                'bash -c "$(curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh)"'
            )
            mirror_line = ""

        zshrc_block = """
# 确保 .zshrc 配置 nvm
[ -f ~/.zshrc ] || touch ~/.zshrc
if ! grep -q NVM_DIR ~/.zshrc 2>/dev/null; then
    cat >> ~/.zshrc << 'NVMEOF'

# nvm (installed by OpenClaw)
export NVM_DIR="$HOME/.nvm"
[ -s "$NVM_DIR/nvm.sh" ] && \\. "$NVM_DIR/nvm.sh"
""".strip()
        if mirror_line:
            zshrc_block += f"\n{mirror_line}"
        zshrc_block += "\nNVMEOF\nfi"

        return f"""
echo "安装 nvm..."
echo ""

{install_cmd}

{zshrc_block}

echo ""
echo "nvm 安装完成"
""".strip()

    def failure_hint(self) -> str:
        if REGION == "CN":
            return "nvm 安装失败。请手动访问 https://gitee.com/mirrors/nvm 安装。"
        return "nvm 安装失败。请手动访问 https://github.com/nvm-sh/nvm 安装。"


# ── Step 2: Node.js LTS ─────────────────────────────────────────────────────

class NodejsStep(Step):
    name = "Node.js LTS"
    description = "通过 nvm 安装 Node.js 长期支持版"
    needs_terminal = True
    timeout = 600  # 10 分钟
    poll_interval = 3.0

    def check(self) -> bool:
        return _shell_check('node --version')

    def terminal_command(self) -> str:
        mirror_env = ""
        if REGION == "CN":
            mirror_env = 'export NVM_NODEJS_ORG_MIRROR=https://npmmirror.com/mirrors/node\n'

        return f"""
echo "通过 nvm 安装 Node.js LTS..."
echo ""

export NVM_DIR="$HOME/.nvm"
[ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"
{mirror_env}
nvm install --lts
nvm alias default "lts/*"

echo ""
echo "Node.js 版本: $(node --version)"
echo "npm 版本: $(npm --version)"
""".strip()

    def failure_hint(self) -> str:
        return "Node.js 安装失败。请确保 nvm 已安装，然后手动运行: nvm install --lts"


# ── Step 3: 国内镜像配置 ─────────────────────────────────────────────────────

class MirrorsStep(Step):
    name = "国内镜像配置"
    description = "配置 npm/pip 国内镜像"
    needs_terminal = False
    timeout = 0

    def should_skip(self) -> bool:
        return REGION != "CN"

    def check(self) -> bool:
        # 检查 npm 镜像
        npm_ok = _shell_check(
            'npm config get registry 2>/dev/null | grep -q npmmirror.com'
        )
        # 检查 pip 镜像
        pip_config = os.path.expanduser("~/.pip/config")
        pip_ok = False
        if os.path.isfile(pip_config):
            try:
                with open(pip_config) as f:
                    pip_ok = "tuna.tsinghua.edu.cn" in f.read()
            except OSError:
                pass
        return npm_ok and pip_ok

    def run_inline(self, log_fn: Callable[[str], None]) -> bool:
        from .executor import run_shell_command

        ok = True

        # npm 淘宝镜像
        log_fn("配置 npm 镜像: registry.npmmirror.com")
        if not run_shell_command(
            NVM_PREFIX + 'npm config set registry https://registry.npmmirror.com',
            log_fn=log_fn,
        ):
            ok = False

        # pip 清华镜像
        log_fn("配置 pip 镜像: tuna.tsinghua.edu.cn")
        pip_dir = os.path.expanduser("~/.pip")
        pip_config = os.path.join(pip_dir, "config")
        try:
            os.makedirs(pip_dir, exist_ok=True)
            need_write = True
            if os.path.isfile(pip_config):
                with open(pip_config) as f:
                    if "tuna.tsinghua.edu.cn" in f.read():
                        need_write = False
                        log_fn("pip 镜像已配置，跳过")

            if need_write:
                with open(pip_config, "w") as f:
                    f.write(
                        "[global]\n"
                        "index-url = https://mirrors.tuna.tsinghua.edu.cn/pypi/web/simple\n"
                        "trusted-host = mirrors.tuna.tsinghua.edu.cn\n"
                    )
                log_fn("pip 镜像配置完成")
        except OSError as e:
            log_fn(f"pip 配置失败: {e}")
            ok = False

        return ok

    def failure_hint(self) -> str:
        return "镜像配置失败。可手动运行: npm config set registry https://registry.npmmirror.com"


# ── AI 工具 Step 基类 ────────────────────────────────────────────────────────
#
# 两种安装模式，分别抽象成基类，让具体工具只需要 3-5 行配置：
#   - NpmGlobalStep     → npm install -g <pkg>          (claude/codex/gemini/openclaw/opencode)
#   - CurlInstallerStep → curl -fsSL <url> | bash       (hermes)


class NpmGlobalStep(Step):
    """通过 npm install -g 安装的工具——npm 工具的公共骨架。"""

    needs_terminal = True
    timeout = 300
    poll_interval = 3.0
    npm_pkg: str = ""       # e.g. "@anthropic-ai/claude-code"
    bin_name: str = ""      # e.g. "claude"

    def check(self) -> bool:
        # 用 `command -v` 而不是 `npm ls -g`——后者要全量读 npm 全局 tree，
        # 启动慢；只要 PATH 能找到 binary 就算装好（cc-switch 检测逻辑也是这个）。
        return _shell_check(f'command -v {self.bin_name}')

    def terminal_command(self) -> str:
        return f"""
echo "安装 {self.npm_pkg}..."
echo ""

export NVM_DIR="$HOME/.nvm"
[ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"

npm install -g {self.npm_pkg}

echo ""
echo "{self.bin_name} 版本: $({self.bin_name} --version 2>/dev/null || echo '?')"
""".strip()

    def failure_hint(self) -> str:
        return f"{self.name} 安装失败。请手动运行: npm install -g {self.npm_pkg}"


class CurlInstallerStep(Step):
    """通过 `curl -fsSL <url> | bash` 安装的工具——opencode/hermes 共用。"""

    needs_terminal = True
    timeout = 300
    poll_interval = 3.0
    installer_url: str = ""
    bin_name: str = ""
    # 额外的 binary 搜索路径——某些工具装到用户目录而非 PATH 里（OpenCode 落
    # 到 ~/.opencode/bin，这正是 cc-switch get_tool_versions 的扫描路径）。
    bin_search_paths: list[str] = []

    def check(self) -> bool:
        for p in self.bin_search_paths:
            if os.path.isfile(os.path.expanduser(p)):
                return True
        return _shell_check(f'command -v {self.bin_name}')

    def terminal_command(self) -> str:
        return f"""
echo "安装 {self.bin_name}..."
echo ""

curl -fsSL {self.installer_url} | bash

echo ""
echo "{self.bin_name} 安装完成"
""".strip()

    def failure_hint(self) -> str:
        return (
            f"{self.name} 安装失败。请手动运行: "
            f"curl -fsSL {self.installer_url} | bash"
        )


# ── 6 个 AI 工具 ─────────────────────────────────────────────────────────────


class ClaudeCodeStep(NpmGlobalStep):
    name = "Claude Code"
    description = "Anthropic 官方 CLI"
    npm_pkg = "@anthropic-ai/claude-code"
    bin_name = "claude"


class CodexStep(NpmGlobalStep):
    name = "Codex"
    description = "OpenAI Codex CLI"
    npm_pkg = "@openai/codex"
    bin_name = "codex"


class GeminiStep(NpmGlobalStep):
    name = "Gemini CLI"
    description = "Google Gemini CLI"
    npm_pkg = "@google/gemini-cli"
    bin_name = "gemini"


class OpenClawStep(NpmGlobalStep):
    name = "OpenClaw"
    description = "OpenClaw CLI"
    # 决策：永远装 latest，不迁 openclaw-launcher 的 version 锁定机制。
    npm_pkg = "openclaw"
    bin_name = "openclaw"


class OpenCodeStep(NpmGlobalStep):
    name = "OpenCode"
    description = "OpenCode CLI"
    # 与其他 npm 工具（claude / codex / gemini / openclaw）走同一条链路：nvm
    # 全局 bin 会被 NVM_PREFIX 加入 PATH，二进制名 `opencode` 通过 command -v
    # 检出，不需要 CurlInstallerStep 的 bin_search_paths 兜底。
    npm_pkg = "opencode-ai"
    bin_name = "opencode"


class HermesStep(CurlInstallerStep):
    name = "Hermes"
    description = "Hermes Agent CLI (Nous Research)"
    installer_url = "https://res1.hermesagent.org.cn/install.sh"
    bin_name = "hermes"
    bin_search_paths = [
        "~/.hermes/bin/hermes",
        "/usr/local/bin/hermes",
    ]


# ── 最终验证 ─────────────────────────────────────────────────────────────────

class VerifyStep(Step):
    """最终全局体检——仅在 onboarding 全装模式下执行（init.py 不传 --tool 时）。"""

    name = "最终验证"
    description = "检查所有工具的安装状态"
    needs_terminal = False
    timeout = 0

    def check(self) -> bool:
        return False  # 总是执行

    def run_inline(self, log_fn: Callable[[str], None]) -> bool:
        log_fn("══════════════════════════════════════")
        log_fn("   环境检查结果")
        log_fn("══════════════════════════════════════")

        # 公共环境层
        env_tools = [
            ("Xcode CLT", "xcode-select -p"),
            ("nvm", 'nvm --version'),
            ("Node.js", 'node --version'),
            ("npm", 'npm --version'),
        ]
        all_ok = True
        for name, cmd in env_tools:
            result = subprocess.run(
                cmd,
                shell=True,
                executable="/bin/bash",
                capture_output=True,
                text=True,
            )
            version = result.stdout.strip() if result.returncode == 0 else "未安装"
            icon = "✓" if result.returncode == 0 else "✕"
            log_fn(f"  {icon} {name}: {version}")
            if result.returncode != 0:
                all_ok = False

        # 6 个 AI 工具——通过 TOOL_STEPS 注册表迭代，新增工具时这里自动覆盖。
        for tid, cls in TOOL_STEPS.items():
            step = cls()
            installed = step.check()
            icon = "✓" if installed else "✕"
            log_fn(f"  {icon} {step.name}: {'已安装' if installed else '未安装'}")
            # 工具未装不计入 all_ok ── VerifyStep 只确保**公共环境**就绪。
            # 单个工具是否装好由各 Step 自己负责报。

        if REGION == "CN":
            log_fn("")
            npm_reg = subprocess.run(
                NVM_PREFIX + 'npm config get registry',
                shell=True, executable="/bin/bash",
                capture_output=True, text=True,
            )
            log_fn(f"  npm 镜像: {npm_reg.stdout.strip()}")

            pip_config = os.path.expanduser("~/.pip/config")
            if os.path.isfile(pip_config):
                try:
                    with open(pip_config) as f:
                        for line in f:
                            if "index-url" in line:
                                log_fn(f"  pip 镜像: {line.strip()}")
                                break
                except OSError:
                    pass

        log_fn("")
        return all_ok

    def verify(self) -> bool:
        # 验证仅看公共环境（nvm + node 必须在）。
        checks = [
            os.path.isfile(os.path.expanduser("~/.nvm/nvm.sh")),
            _shell_check('node --version'),
        ]
        return all(checks)

    def failure_hint(self) -> str:
        return "部分工具验证失败，请检查上方日志输出。"


# ── 入口函数 ─────────────────────────────────────────────────────────────────
#
# 区分「公共环境层」和「具体工具」，前者是 6 个工具共享的前置依赖，应该只装
# 一次；后者按 --tool 单独触发或全装模式一次跑完。


def get_env_steps() -> list[Step]:
    """公共环境层——所有 npm 工具都依赖的 nvm / Node / 镜像。Xcode CLT 由 init.sh
    在 Python 执行前就处理掉了，这里不出现。"""
    return [NvmStep(), NodejsStep(), MirrorsStep()]


# tool_id → Step class 映射。**TOOL_STEPS 是单一真源**——新增工具或调整可
# 安装列表都改这里，init.py / VerifyStep / Tauri 后端的白名单都从这里查。
TOOL_STEPS: dict[str, type[Step]] = {
    "claude":   ClaudeCodeStep,
    "codex":    CodexStep,
    "gemini":   GeminiStep,
    "opencode": OpenCodeStep,
    "openclaw": OpenClawStep,
    "hermes":   HermesStep,
}


def get_tool_step(tool_id: str) -> Step | None:
    """按 cc-switch 工具 id 返回对应的 Step；未知 id 返回 None。"""
    cls = TOOL_STEPS.get(tool_id)
    return cls() if cls is not None else None
