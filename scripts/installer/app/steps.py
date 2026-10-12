"""
步骤定义：5 个安装步骤，统一接口
每个步骤包含：检测、终端命令/内联执行、验证、失败提示
仅使用 Python 3.9 标准库

注意：Xcode CLT 安装由 init.sh (bash) 处理，不在此列。
"""
from __future__ import annotations

import os
import re
import subprocess

import shutil

from app.shell import login_shell_argv, login_shell_env, login_shell_path
from typing import Callable, Optional

# nvm 命令前缀：加载 nvm 环境
# 全局区域设置，由 init.py 在启动时设置
REGION = "CN"


def set_region(region: str) -> None:
    global REGION
    REGION = region


def _cmd_exists(cmd: str) -> bool:
    """
    命令是否存在。直接在缓存的 login PATH 里查，不起 shell —— 微秒级。

    比 `_shell_check(f"command -v {cmd}")` 快三个数量级，覆盖还更全：那个 PATH 取自
    interactive shell，含 `.zshrc` 里配的路径（pnpm 的 PNPM_HOME 就在那）。
    """
    return shutil.which(cmd, path=login_shell_path()) is not None


def _shell_check(cmd: str, *, refresh_path: bool = False) -> bool:
    """
    执行检测命令，返回是否成功。

    非交互 login shell（约 23ms）+ 注入完整 PATH：shell 起得快，PATH 又是全的。
    为什么需要这套：应用从 Finder/Dock 启动时 PATH 只有系统目录，不含
    nvm / fnm / volta / pnpm / Homebrew 往用户 shell rc 里加的路径，裸 shell 检测会
    把已装的工具判成未装、反复重装（fizzy #865）。详见 app/shell.py。
    """
    return subprocess.run(
        login_shell_argv(cmd),
        env=login_shell_env(refresh_path=refresh_path),
        capture_output=True,
        stdin=subprocess.DEVNULL,
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

    def should_skip(self) -> bool:
        """
        已经有可用的 node + npm 就不装 nvm。

        nvm 只是**手段**，目的是有个能用的 node —— fnm / volta / Homebrew 装的同样
        算达成。原来只看 `check()` 里那个 `~/.nvm/nvm.sh` 是否存在，于是所有非 nvm
        用户都被强行装一遍 nvm，还会往 `.zshrc` 追加 nvm 初始化代码，跟他现有的
        版本管理器抢 PATH（fizzy #874）。

        不做版本判断：node 太旧的话，`npm install -g` 会按包的 engines 字段报出
        准确的错误，比我们在这里猜一个下限可靠；而强行装 nvm 去覆盖用户环境的
        代价大得多。
        """
        return _cmd_exists("node") and _cmd_exists("npm")

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
        # nvm may have installed Node after the installer's first PATH lookup.
        # Refresh while polling so this same run can advance to the tool step.
        return _shell_check('node --version', refresh_path=True)

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


# ── 升级 Node.js（用户在 Ofox 里同意过）────────────────────────────────────────

def _node_version() -> Optional[tuple]:
    """新开终端里 `node --version` 的 (major, minor, patch)；没有 Node 时为 None。"""
    result = subprocess.run(
        login_shell_argv("node --version"),
        env=login_shell_env(refresh_path=True),
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
    )
    match = re.match(r"v?(\d+)\.(\d+)\.(\d+)", result.stdout.strip())
    if result.returncode != 0 or match is None:
        return None
    return tuple(int(part) for part in match.groups())


class NodeUpgradeStep(Step):
    """
    把 Node.js 升级到工具要求的版本，再继续装工具（fizzy #1021）。

    命令由 Ofox 后端按 Node 的管理方式生成（fnm / nvm / volta / Homebrew），已经
    包含「重新安装因切换版本而消失的工具」；这里只负责执行，并确认升级后的版本
    确实落在要求的范围里。
    """

    name = "升级 Node.js"
    description = "工具要求更新的 Node.js 版本"
    needs_terminal = True
    timeout = 1200
    poll_interval = 3.0

    def __init__(self, command: str, minimum: str, below: Optional[int], manual: str) -> None:
        self.command = command
        self.minimum = tuple(int(part) for part in minimum.split("."))
        self.minimum_text = minimum
        self.below = below
        self.manual = manual

    def check(self) -> bool:
        version = _node_version()
        if version is None:
            return False
        return version >= self.minimum and (self.below is None or version[0] < self.below)

    def terminal_command(self) -> str:
        return f"""
echo "升级 Node.js（Ofox 安装的工具要求 {self.minimum_text} 或更新）..."
echo ""

{self.command}

echo ""
echo "Node.js 版本: $(node --version)"
""".strip()

    def failure_hint(self) -> str:
        return f"Node.js 没能升级到 {self.minimum_text} 或更新。可以手动运行: {self.manual}"


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
            'npm config set registry https://registry.npmmirror.com',
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
        return _cmd_exists(self.bin_name)

    def terminal_command(self) -> str:
        return f"""
echo "安装 {self.npm_pkg}..."
echo ""

export NVM_DIR="$HOME/.nvm"
[ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"

npm install -g {self.npm_pkg}@latest

echo ""
echo "{self.bin_name} 版本: $({self.bin_name} --version 2>/dev/null || echo '?')"
""".strip()

    def failure_hint(self) -> str:
        # 原来提示「请手动运行 npm install -g …」：同一个 Node 下手动运行同样会失败。
        return (
            f"{self.name} 安装失败，原因见保留的终端窗口。常见原因：Node.js 版本低于"
            f"它的要求，或无法访问 npm。"
        )


class CurlInstallerStep(Step):
    """通过 `curl -fsSL <url> | bash` 安装的工具（目前只有 hermes）。

    安装脚本退出 0 不等于工具能用：hermes 的镜像脚本就装出过一个起不来的
    环境。所以 `check()` / `verify()` 不只看二进制在不在，还会按 `probe_args`
    真正启动一次；起不来的视为未安装，再点「安装」会重装而不是直接报已安装。
    """

    needs_terminal = True
    timeout = 300
    poll_interval = 3.0
    installer_url: str = ""
    bin_name: str = ""
    # 额外的 binary 搜索路径——某些工具装到用户目录而非 PATH 里。
    bin_search_paths: list[str] = []
    # 安装后要真正跑一遍的参数组，例如 [["--version"], ["--help"]]；空表示只看文件。
    probe_args: list[list[str]] = []
    probe_timeout = 60

    def __init__(self) -> None:
        self._failure_cause = ""

    def installed_path(self) -> Optional[str]:
        for p in self.bin_search_paths:
            expanded = os.path.expanduser(p)
            if os.path.isfile(expanded):
                return expanded
        # 刚装完的脚本可能改了 rc 里的 PATH，缓存的那份不算数。
        return shutil.which(self.bin_name, path=login_shell_path(refresh=True))

    def _probe(self, path: str, args: list[str]) -> Optional[str]:
        """跑一次 `path args`。成功返回 None，失败返回一行原因。"""
        shown = " ".join([self.bin_name, *args])
        try:
            result = subprocess.run(
                [path, *args],
                capture_output=True,
                text=True,
                stdin=subprocess.DEVNULL,
                # PATH 已在 installed_path() 里刷新过；这里复用缓存，不再起 shell。
                env=login_shell_env(),
                timeout=self.probe_timeout,
            )
        except subprocess.TimeoutExpired:
            return f"{shown} 超时（{self.probe_timeout}s）"
        except OSError as e:
            return f"{shown}: {e}"
        if result.returncode == 0:
            return None
        lines = [line.strip() for line in (result.stderr or "").splitlines() if line.strip()]
        if not lines:
            lines = [line.strip() for line in (result.stdout or "").splitlines() if line.strip()]
        return lines[-1] if lines else f"{shown} 退出码 {result.returncode}"

    def check(self) -> bool:
        path = self.installed_path()
        if path is None:
            return False
        return not self.probe_args or self._probe(path, self.probe_args[0]) is None

    def verify(self) -> bool:
        self._failure_cause = ""
        path = self.installed_path()
        if path is None:
            self._failure_cause = f"未找到 {self.bin_name} 可执行文件"
            return False
        for args in self.probe_args:
            cause = self._probe(path, args)
            if cause is not None:
                self._failure_cause = cause
                return False
        return True

    def terminal_command(self) -> str:
        return f"""
echo "安装 {self.bin_name}..."
echo ""

curl -fsSL {self.installer_url} | bash

echo ""
echo "{self.bin_name} 安装完成"
""".strip()

    def failure_hint(self) -> str:
        if self._failure_cause:
            # 必须是一行：界面只取最后一条「提示:」。
            cause = " ".join(self._failure_cause.split())
            return (
                f"{self.name} 已安装但无法启动: {cause}。"
                f"请在终端运行 {self.bin_name} --version 查看详情。"
            )
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
    # 与其他 npm 工具（claude / codex / gemini / openclaw）走同一条链路：检测走
    # login shell（见 app/shell.py），npm 全局 bin 自然在用户 PATH 里，二进制名
    # `opencode` 通过 command -v 检出，不需要 CurlInstallerStep 的
    # bin_search_paths 兜底。
    npm_pkg = "opencode-ai"
    bin_name = "opencode"


HERMES_MIRROR_URL = "https://res1.hermesagent.org.cn/install.sh"
HERMES_OFFICIAL_URL = "https://hermes-agent.nousresearch.com/install.sh"


def parse_upstream_commit(script_text: str) -> Optional[str]:
    """镜像安装脚本头部声明的、与它配套的上游 commit。"""
    match = re.search(r'^UPSTREAM_COMMIT="([0-9a-f]{7,40})"', script_text, re.M)
    return match.group(1) if match else None


class HermesStep(CurlInstallerStep):
    """Hermes Agent CLI。

    国内走镜像脚本，并把代码锁到脚本自己声明的 `UPSTREAM_COMMIT`：镜像脚本写死
    Python 3.11，却默认 clone 镜像仓库的最新代码（已要求 3.14），装出来的环境
    一个运行时依赖都没有。锁到配套的 commit 后脚本和代码一致、全部下载走镜像；
    commit 在运行时从脚本里解析，镜像更新脚本后自动跟上。装到的不是最新版，
    之后由「升级」（hermes update）前进；更新器把代码拉到新版后 3.11 环境会
    再次坏掉，那时 `check()` 会判成未安装，再点「安装」即重装修复。

    海外走官方脚本（非交互、跳过浏览器组件）。
    """

    name = "Hermes"
    description = "Hermes Agent CLI (Nous Research)"
    installer_url = HERMES_MIRROR_URL
    bin_name = "hermes"
    timeout = 900
    bin_search_paths = [
        "~/.local/bin/hermes",
        "~/.hermes/bin/hermes",
        "/usr/local/bin/hermes",
    ]
    # `--version` 和 Rust 侧探测一致；`--help` 会导入完整的 CLI 和配置模块，
    # 依赖缺失正是在这一步暴露的。
    probe_args = [["--version"], ["--help"]]

    def _fetch_mirror_script(self) -> Optional[str]:
        # 用 curl 取，和终端里真正执行安装的是同一条网络路径。urllib 会跟随 macOS
        # 的系统代理设置，代理失效时这里会误判成「镜像不可用」。
        try:
            result = subprocess.run(
                ["curl", "-fsSL", "--max-time", "20", HERMES_MIRROR_URL],
                capture_output=True,
                stdin=subprocess.DEVNULL,
                timeout=30,
            )
        except (OSError, subprocess.TimeoutExpired):
            return None
        if result.returncode != 0 or not result.stdout:
            return None
        return result.stdout.decode("utf-8", "replace")

    def _official_command(self) -> str:
        self.installer_url = HERMES_OFFICIAL_URL
        return (
            f"curl -fsSL {HERMES_OFFICIAL_URL} | "
            "bash -s -- --non-interactive --skip-setup --skip-browser"
        )

    def _install_command(self) -> str:
        if REGION != "CN":
            return self._official_command()
        script = self._fetch_mirror_script()
        if script is None:
            return (
                'echo "镜像不可用，改用官方安装脚本"\n'
                + self._official_command()
            )
        self.installer_url = HERMES_MIRROR_URL
        commit = parse_upstream_commit(script)
        if commit is None:
            return f"curl -fsSL {HERMES_MIRROR_URL} | bash -s -- --skip-setup"
        return (
            f'echo "锁定上游提交 {commit}（与镜像安装脚本配套）"\n'
            f"curl -fsSL {HERMES_MIRROR_URL} | bash -s -- --commit {commit} --skip-setup"
        )

    def terminal_command(self) -> str:
        return f"""
echo "安装 {self.bin_name}..."
echo ""

{self._install_command()}

echo ""
echo "{self.bin_name} 安装完成"
""".strip()


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
                login_shell_argv('npm config get registry'),
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
            # 判据是"node / npm 能用"，不是"nvm 装了"——fnm / volta / Homebrew 装的
            # node 同样算环境就绪。原来要求 ~/.nvm/nvm.sh 存在，会让所有非 nvm 用户
            # 的验证永远失败、被告知"环境未就绪"，而他的环境明明是好的（#874）。
            _shell_check('node --version'),
            _cmd_exists("npm"),
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
