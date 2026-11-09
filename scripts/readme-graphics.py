#!/usr/bin/env python3
"""Rebuild self-contained README SVGs from the repository's existing brand marks.

No network, fonts, credentials or user configuration are read. Run after capturing
assets/readme/tools-en.webp to embed that actual production-component screenshot.
"""

from base64 import b64encode
import argparse
from html import escape
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "assets/readme"
OUT.mkdir(parents=True, exist_ok=True)

# Explicit theme tokens keep embedded screenshots and original colored marks
# untouched. The default palette intentionally preserves the original dark SVGs.
PALETTES = {
    "dark": {
        "canvasStart": "#12141d", "canvasEnd": "#0a0b10", "grid": "#fff",
        "gridOpacity": ".025", "border": "#2b2d39", "text": "#eef0f6",
        "muted": "#a0a6b6", "label": "#ffad76", "accentStart": "#ffb26b",
        "accentEnd": "#ff713d", "glowOpacity": ".2", "icon": "#ffad76",
        "pillFill": "#242630", "pillStroke": "#383b49",
        "windowFill": "#202127", "windowStroke": "#4b3a33",
        "sweepBase": "#353744", "sweepAccent": "#ff9c64",
        "heroCardFill": "#171923", "heroCardStroke": "#2b2e3a",
        "wallCardFill": "#191b25", "wallCardStroke": "#303340",
        "flowFill": "#1b1d27", "flowStroke": "#47414b",
        "flowMiddleFill": "#211c1a", "flowMiddleStroke": "#715040",
        "flowLine": "#8b8fa0", "flowRequestBase": "#694b3a",
    },
    "light": {
        "canvasStart": "#fffdf9", "canvasEnd": "#eef2f8", "grid": "#364254",
        "gridOpacity": ".045", "border": "#d8dfe9", "text": "#192536",
        "muted": "#596578", "label": "#ae4617", "accentStart": "#db6829",
        "accentEnd": "#b53d16", "glowOpacity": ".1", "icon": "#b74b1c",
        "pillFill": "#ffffff", "pillStroke": "#cbd4e1",
        "windowFill": "#f3f5f8", "windowStroke": "#c9b7aa",
        "sweepBase": "#d0d9e5", "sweepAccent": "#ba4d1b",
        "heroCardFill": "#ffffff", "heroCardStroke": "#d1dae6",
        "wallCardFill": "#ffffff", "wallCardStroke": "#d1dae6",
        "flowFill": "#ffffff", "flowStroke": "#cbd5e2",
        "flowMiddleFill": "#fff3e8", "flowMiddleStroke": "#c8916d",
        "flowLine": "#748298", "flowRequestBase": "#d9b392",
    },
}


def output_name(name, theme):
    return f"{name}-light.svg" if theme == "light" else f"{name}.svg"


def image(path, white=False):
    data = (ROOT / path).read_bytes()
    mime = "image/svg+xml" if path.endswith(".svg") else "image/png"
    if path.endswith(".webp"):
        mime = "image/webp"
    if white:
        data = data.replace(b"currentColor", b"#f2f3f7")
    return f"data:{mime};base64,{b64encode(data).decode()}"


def svg(width, height, title, description, content, theme="dark"):
    c = PALETTES[theme]
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-labelledby="title desc">
<title id="title">{escape(title)}</title><desc id="desc">{escape(description)}</desc>
<defs>
 <linearGradient id="canvas" x2="1" y2="1"><stop stop-color="{c['canvasStart']}"/><stop offset="1" stop-color="{c['canvasEnd']}"/></linearGradient>
 <linearGradient id="accent"><stop stop-color="{c['accentStart']}"/><stop offset="1" stop-color="{c['accentEnd']}"/></linearGradient>
 <radialGradient id="glow"><stop stop-color="#ff793c" stop-opacity="{c['glowOpacity']}"/><stop offset="1" stop-color="#ff793c" stop-opacity="0"/></radialGradient>
 <pattern id="grid" width="40" height="40" patternUnits="userSpaceOnUse"><path d="M40 0H0V40" fill="none" stroke="{c['grid']}" stroke-opacity="{c['gridOpacity']}"/></pattern>
</defs>
<style>
 text{{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Arial,sans-serif;fill:{c['text']}}}
 .muted{{fill:{c['muted']}}} .label{{font-size:15px;font-weight:600;letter-spacing:2px;fill:{c['label']}}}
 .packet{{stroke-dasharray:10 190;animation:travel 9s linear infinite}}
 .breathe{{animation:breathe 6s ease-in-out infinite}}
 @keyframes travel{{to{{stroke-dashoffset:-800}}}}
 @keyframes breathe{{0%,100%{{opacity:.45}}50%{{opacity:.95}}}}
 @media(prefers-reduced-motion:reduce){{.packet,.breathe{{animation:none}}}}
</style>
<rect width="{width}" height="{height}" rx="26" fill="url(#canvas)"/>
<rect width="{width}" height="{height}" rx="26" fill="url(#grid)"/>
<rect x=".5" y=".5" width="{width-1}" height="{height-1}" rx="25.5" fill="none" stroke="{c['border']}"/>
{content}
</svg>'''


icons = {
    "tools": '<path d="m6 6 5 5-5 5m8 0h5"/>',
    "sessions": '<path d="M5 4h14v13H9l-4 3Z"/><path d="M9 8h6m-6 4h4"/>',
    "library": '<path d="M4 5h4v14H4Zm6 0h4v14h-4Zm6 1 4-1 3 13-4 1Z"/>',
    "assistant": '<path d="m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5Z"/>',
    "usage": '<path d="M4 4v16h17M8 15v-4m5 4V7m5 8v-6"/>',
}


def icon(name, x, y, size=26, theme="dark"):
    return f'<g transform="translate({x} {y}) scale({size/24})" fill="none" stroke="{PALETTES[theme]["icon"]}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">{icons[name]}</g>'


def make_hero(theme="dark"):
    c = PALETTES[theme]
    screenshot_name = "tools-en-light.webp" if theme == "light" else "tools-en.webp"
    screenshot = OUT / screenshot_name
    if not screenshot.exists():
        print(f"Skipped {output_name('hero', theme)}: capture assets/readme/{screenshot_name} first; never substitute a drawn UI.")
        return False
    logo = image("src-tauri/icons/128x128@2x.png")
    screen = image(f"assets/readme/{screenshot_name}")
    content = f'''
<ellipse cx="1180" cy="370" rx="600" ry="450" fill="url(#glow)" class="breathe"/>
<image href="{logo}" x="72" y="55" width="62" height="62"/>
<text x="150" y="98" font-size="34" font-weight="650" letter-spacing="-1">Ofox Desktop</text>
<text x="1470" y="91" class="label" text-anchor="end">THE OPEN-SOURCE AI WORKSPACE</text>
<text x="80" y="239" font-size="69" font-weight="730" letter-spacing="-3">Your AI tools.</text>
<text x="80" y="323" font-size="69" font-weight="730" letter-spacing="-3" style="fill:url(#accent)">One-click setup.</text>
<text x="83" y="390" class="muted" font-size="21">Sign in. Pick your tools. Let Ofox configure them.</text>
<text x="83" y="427" class="muted" font-size="21">Then explore sessions, resources, chat, and usage.</text>
<rect x="82" y="478" width="126" height="39" rx="19.5" fill="{c['pillFill']}" stroke="{c['pillStroke']}"/>
<text x="145" y="503" font-size="15" text-anchor="middle">macOS</text>
<rect x="220" y="478" width="126" height="39" rx="19.5" fill="{c['pillFill']}" stroke="{c['pillStroke']}"/>
<text x="283" y="503" font-size="15" text-anchor="middle">Windows</text>
<text x="83" y="565" class="muted" font-size="15">YOUR TOOLS · YOUR WORKSPACE · YOUR CHOICE</text>
<rect x="736" y="175" width="800" height="534" rx="18" fill="{c['windowFill']}" stroke="{c['windowStroke']}"/>
<circle cx="757" cy="193" r="4" fill="#ff7467"/><circle cx="772" cy="193" r="4" fill="#edc65b"/><circle cx="787" cy="193" r="4" fill="#76c78c"/>
<text x="1136" y="198" text-anchor="middle" font-size="11" class="muted">Ofox Desktop · sample workspace</text>
<svg x="737" y="212" width="798" height="496" viewBox="0 0 1440 900"><image href="{screen}" width="1440" height="900"/></svg>
<path d="M80 644H654" stroke="{c['sweepBase']}"/>
<path d="M80 644H654" stroke="{c['sweepAccent']}" stroke-width="2" class="packet"/>
<text x="83" y="688" font-size="16" class="muted">A native home for your entire AI stack.</text>
'''
    labels = [("tools", "Tools", "Connect & configure"), ("sessions", "Sessions", "Find & continue"), ("library", "Library", "Curate & distribute"), ("assistant", "Assistant", "Ask & create"), ("usage", "Usage", "Explore & understand")]
    for i, (name, label, sub) in enumerate(labels):
        x = 80 + i * 291
        content += f'<rect x="{x}" y="764" width="276" height="108" rx="14" fill="{c["heroCardFill"]}" stroke="{c["heroCardStroke"]}"/>'
        content += icon(name, x+20, 785, theme=theme)
        content += f'<text x="{x+61}" y="807" font-size="21" font-weight="600">{escape(label)}</text><text x="{x+21}" y="846" font-size="15" class="muted">{escape(sub)}</text>'
    (OUT / output_name("hero", theme)).write_text(svg(1600, 910, "Ofox Desktop — Your AI tools. One-click setup.", "Sign in, pick your tools and let Ofox configure them. Five connected areas: tools, sessions, library, assistant and usage. Contains an actual production-component screenshot with fictional sample data. Decorative paths animate; reduced motion is supported.", content, theme))
    return True


def make_wall(theme="dark"):
    c = PALETTES[theme]
    tools = [
        ("Claude Code", "src/assets/agent-logos/claudecode-color.svg", False),
        ("Codex", "src/assets/agent-logos/codex-color.svg", False),
        ("ChatGPT", "src/icons/extracted/openai.svg", True),
        ("OpenCode", "src/icons/extracted/opencode-logo-light.svg", False),
        ("OpenClaw", "src/icons/extracted/claw.svg", False),
        ("Hermes", "src/assets/agent-logos/hermes.svg", True),
        ("Gemini CLI", "src/assets/agent-logos/geminicli-color.svg", False),
        ("WorkBuddy", "src/icons/extracted/workbuddy.svg", False),
        ("DeepSeek Harness", "src/icons/extracted/deepseek.svg", False),
        ("Pi", "src/icons/extracted/pi.svg", True),
    ]
    content = '<text x="60" y="58" class="label">TEN TOOLS. ONE FAMILIAR WORKSPACE.</text><text x="60" y="93" class="muted" font-size="17">Native integrations. Capabilities vary by tool, version, and platform.</text>'
    for i, (name, path, white) in enumerate(tools):
        x, y = 60 + (i % 5) * 220, 127 + (i // 5) * 123
        content += f'<rect x="{x}" y="{y}" width="204" height="107" rx="14" fill="{c["wallCardFill"]}" stroke="{c["wallCardStroke"]}"/>'
        content += f'<image href="{image(path, white and theme == "dark")}" x="{x+84}" y="{y+15}" width="36" height="36"/>'
        content += f'<text x="{x+102}" y="{y+82}" font-size="16" font-weight="550" text-anchor="middle">{escape(name)}</text>'
    content += '<text x="60" y="395" class="muted" font-size="13">ChatGPT refers to its Codex mode. Product marks belong to their respective owners.</text>'
    (OUT / output_name("tool-wall", theme)).write_text(svg(1200, 430, "Supported tools in Ofox Desktop", "Claude Code, Codex, ChatGPT Codex mode, OpenCode, OpenClaw, Hermes, Gemini CLI, WorkBuddy, DeepSeek Harness and Pi. This is an integration list, not an endorsement or equal-feature claim.", content, theme))


def make_flow(theme="dark"):
    c = PALETTES[theme]
    content = f'''
<text x="58" y="57" class="label">CONFIGURATION YOU CAN UNDERSTAND</text>
<text x="58" y="93" class="muted" font-size="17">Bound tools send model requests directly to their configured service.</text>
<rect x="58" y="145" width="266" height="145" rx="18" fill="{c['flowFill']}" stroke="{c['flowStroke']}"/>
<text x="84" y="185" font-size="24" font-weight="600">Ofox Desktop</text>
<text x="84" y="222" class="muted" font-size="16">Manages native configuration</text>
<text x="84" y="251" class="muted" font-size="16">Checks changes &amp; conflicts</text>
<path d="M326 217H444" fill="none" stroke="{c['flowLine']}" stroke-width="2" stroke-dasharray="4 7"/>
<path d="m435 211 9 6-9 6" fill="none" stroke="{c['flowLine']}" stroke-width="2"/>
<text x="384" y="194" class="muted" font-size="13" text-anchor="middle">configure</text>
<rect x="462" y="145" width="270" height="145" rx="18" fill="{c['flowMiddleFill']}" stroke="{c['flowMiddleStroke']}"/>
<text x="488" y="185" font-size="24" font-weight="600">Your AI tool</text>
<text x="488" y="222" class="muted" font-size="16">Its native configuration file</text>
<text x="488" y="251" class="muted" font-size="16">Its dedicated API key</text>
<path d="M734 217H875" fill="none" stroke="{c['flowRequestBase']}" stroke-width="2"/>
<path d="M734 217H875" fill="none" stroke="{c['icon']}" stroke-width="2" class="packet"/>
<path d="m866 211 9 6-9 6" fill="none" stroke="{c['icon']}" stroke-width="2"/>
<text x="804" y="194" class="muted" font-size="13" text-anchor="middle">model requests</text>
<rect x="892" y="145" width="250" height="145" rx="18" fill="{c['flowFill']}" stroke="{c['flowStroke']}"/>
<text x="918" y="185" font-size="24" font-weight="600">Configured service</text>
<text x="918" y="222" class="muted" font-size="16">Ofox or another provider</text>
<text x="918" y="251" class="muted" font-size="16">Selected endpoint &amp; model</text>
<text x="58" y="341" font-size="15" class="muted">Ofox login credentials use system credential storage. Tool API keys are also written to native tool configuration.</text>
<text x="58" y="373" font-size="15" class="muted">This diagram describes bound external tools. The built-in assistant has its own model and tool requests.</text>
'''
    (OUT / output_name("data-flow", theme)).write_text(svg(1200, 410, "How bound-tool configuration and requests flow", "Desktop manages native tool configuration. The tool sends requests directly to the configured service. Login credentials use system storage, while dedicated tool API keys are written into the tool's native configuration. This is not a no-cloud or zero-data-sharing claim.", content, theme))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--theme", choices=("dark", "light", "all"), default="dark")
    args = parser.parse_args()
    themes = ("dark", "light") if args.theme == "all" else (args.theme,)
    for theme in themes:
        make_wall(theme)
        make_flow(theme)
        hero = make_hero(theme)
        generated = [output_name("tool-wall", theme), output_name("data-flow", theme)]
        if hero:
            generated.insert(0, output_name("hero", theme))
        print(f"Generated {', '.join(generated)} (self-contained, no external resources).")
