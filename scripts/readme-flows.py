#!/usr/bin/env python3
"""Generate explanatory animated README flow diagrams; never fabricated app UI.

No network, external assets, fonts, JavaScript, credentials or user configuration.
Run `python3 scripts/readme-flows.py` to rebuild dark and light flow diagrams.
"""

from html import escape
from pathlib import Path
import argparse
import xml.etree.ElementTree as ET

OUT = Path(__file__).resolve().parents[1] / "assets/readme"
PALETTES = {
    "dark": {
        "background": "#10131b", "end": "#151924", "text": "#eef1f7",
        "muted": "#a5aec0", "line": "#343b4d", "card": "#1a1f2c",
        "accent": "#ffab72", "active": "#33271f", "label": "#ffb684",
    },
    "light": {
        "background": "#fffdf9", "end": "#edf2f9", "text": "#1c293c",
        "muted": "#57677c", "line": "#c7d3e2", "card": "#ffffff",
        "accent": "#b24c20", "active": "#fff0e3", "label": "#a8461c",
    },
}

ICONS = {
    "signin": '<path d="M9 5H5v14h4m3-11 4 4-4 4m-4-4h12"/>',
    "discover": '<circle cx="10.5" cy="10.5" r="6"/><path d="m15 15 5 5M10.5 7v7M7 10.5h7"/>',
    "choose": '<rect x="4" y="4" width="16" height="16" rx="3"/><path d="m8 12 3 3 5-6"/>',
    "bind": '<path d="m9 15 6-6m-8 8-1 1a4 4 0 0 1-5-5l5-5a4 4 0 0 1 5 0m2-1 1-1a4 4 0 1 1 5 5l-5 5a4 4 0 0 1-5 0"/>',
    "start": '<path d="m8 4 12 8-12 8Z"/>',
    "problem": '<path d="m12 3 10 18H2Zm0 6v5m0 3v1"/>',
    "diagnose": '<path d="M4 12h4l3-7 3 14 3-7h3"/>',
    "review": '<path d="M6 3h9l4 4v14H6Zm8 0v5h5M9 12h7m-7 4h5"/>',
    "repair": '<path d="m5 19 6-6m2-2 6-6M14 4a6 6 0 0 0-7 8l-5 5 5 5 5-5a6 6 0 0 0 8-7l-4 4-6-6Z"/>',
    "verify": '<path d="m12 3 8 4v5c0 5-8 9-8 9s-8-4-8-9V7Zm-4 9 3 3 5-6"/>',
}

FLOWS = {
    "binding-flow": {
        "label": "LESS SETUP. MORE MOMENTUM.",
        "title": "From first launch to connected tools",
        "description": "Explanatory schematic: sign in once, discover installed tools, choose supported tools, bind selected tools, then start using them. This is not a recording or a guarantee that all tools need no additional setup.",
        "footnote": "Explanatory schematic · Ofox sign-in is required for binding; available actions depend on the tool and platform.",
        "steps": [
            ("signin", "Sign in once", "Use your Ofox account"),
            ("discover", "Discover tools", "See what is installed"),
            ("choose", "Choose tools", "Pick supported clients"),
            ("bind", "Bind selected", "Write native configuration"),
            ("start", "Start using", "Open your connected tools"),
        ],
    },
    "configuration-rescue": {
        "label": "A CLEAR PATH BACK TO WORK.",
        "title": "Understand a configuration problem before changing it",
        "description": "Explanatory schematic: identify a problem, diagnose the configuration, review and approve proposed changes, repair, then verify. Repairs require review and may need further work; no instant or universal success is promised.",
        "footnote": "Explanatory schematic · Review and approve changes first. Repair results vary; verify before continuing.",
        "steps": [
            ("problem", "Problem", "Notice a setup issue"),
            ("diagnose", "Diagnose", "Inspect the current state"),
            ("review", "Review / approve", "Check proposed changes"),
            ("repair", "Repair", "Apply approved changes"),
            ("verify", "Verify", "Check the result"),
        ],
    },
}


def flow_svg(name, theme):
    c, flow = PALETTES[theme], FLOWS[name]
    pieces = []
    for index in range(4):
        start, end = 242 + index * 230, 272 + index * 230
        pieces.append(f'''<path d="M{start} 210H{end}" fill="none" stroke="{c['line']}" stroke-width="2"/>
<path class="packet" d="M{start} 210H{end}" fill="none" stroke="{c['accent']}" stroke-width="3" stroke-dasharray="7 25" style="animation-delay:-{index * .7}s"/>
<path d="m{end-6} 205 6 5-6 5" fill="none" stroke="{c['accent']}" stroke-width="1.5"/>''')
    for index, (icon, title, detail) in enumerate(flow["steps"]):
        x = 42 + index * 230
        delay = -((15 - 3 * index) % 15)
        pieces.append(f'''<g>
<rect class="phase" x="{x}" y="137" width="200" height="147" rx="17" fill="{c['card']}" stroke="{c['line']}" stroke-width="1.2" style="animation-delay:{delay}s"/>
<circle cx="{x+169}" cy="161" r="11" fill="{c['background']}" stroke="{c['line']}"/>
<text x="{x+169}" y="165" font-size="11" text-anchor="middle" class="muted">{index+1}</text>
<g transform="translate({x+20} 157)" fill="none" stroke="{c['accent']}" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">{ICONS[icon]}</g>
<text x="{x+20}" y="221" font-size="20" font-weight="630">{escape(title)}</text>
<text x="{x+20}" y="252" font-size="12.5" class="muted">{escape(detail)}</text>
</g>''')
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="360" viewBox="0 0 1200 360" role="img" aria-labelledby="title desc">
<title id="title">{escape(flow['title'])}</title><desc id="desc">{escape(flow['description'])}</desc>
<defs><linearGradient id="canvas" x2="1" y2="1"><stop stop-color="{c['background']}"/><stop offset="1" stop-color="{c['end']}"/></linearGradient></defs>
<style>
text{{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Arial,sans-serif;fill:{c['text']}}}
.muted{{fill:{c['muted']}}}
.phase{{animation:phase 15s ease-in-out infinite}}
.packet{{animation:travel 1.8s linear infinite}}
@keyframes phase{{0%,16%{{fill:{c['active']};stroke:{c['accent']}}}22%,100%{{fill:{c['card']};stroke:{c['line']}}}}}
@keyframes travel{{to{{stroke-dashoffset:-64}}}}
@media(prefers-reduced-motion:reduce){{.phase,.packet{{animation:none}}}}
</style>
<rect width="1200" height="360" rx="24" fill="url(#canvas)"/>
<rect x=".5" y=".5" width="1199" height="359" rx="23.5" fill="none" stroke="{c['line']}"/>
<text x="42" y="42" font-size="12" font-weight="650" letter-spacing="2" style="fill:{c['label']}">{escape(flow['label'])}</text>
<text x="42" y="86" font-size="29" font-weight="650" letter-spacing="-.5">{escape(flow['title'])}</text>
{''.join(pieces)}
<text x="42" y="327" font-size="12.5" class="muted">{escape(flow['footnote'])}</text>
</svg>'''


def assistant_media_svg(theme):
    """An original capability illustration, separate from real UI screenshots."""
    c = PALETTES[theme]
    frames = []
    labels = [
        ("IMAGE", "Describe a visual", "From an idea to an image"),
        ("VIDEO", "Set an idea in motion", "Create a short clip"),
        ("SPEECH", "Give words a voice", "Generate spoken audio"),
    ]
    for i, (label, title, subtitle) in enumerate(labels):
        x = 38 + i * 380
        frames.append(f'''<rect x="{x}" y="116" width="364" height="288" rx="20" fill="{c['card']}" stroke="{c['line']}"/>
<text x="{x+24}" y="149" class="eyebrow">{label}</text>
<text x="{x+24}" y="346" font-size="23" font-weight="620">{title}</text>
<text x="{x+24}" y="377" class="muted" font-size="16">{subtitle}</text>''')
    image = f'''<g transform="translate(96 173)">
<rect width="246" height="130" rx="13" fill="{c['background']}" stroke="{c['line']}"/>
<circle class="sun" cx="175" cy="39" r="22" fill="{c['accent']}" opacity=".85"/>
<path d="m18 115 64-66 53 51 36-33 57 48Z" fill="{c['accent']}" opacity=".25"/>
<path d="m18 115 64-66 53 51 36-33 57 48" fill="none" stroke="{c['accent']}" stroke-width="2"/>
<path class="sweep" d="M1 1H245" stroke="{c['accent']}" stroke-width="2"/>
</g>'''
    video = f'''<g transform="translate(476 173)">
<rect width="246" height="130" rx="13" fill="{c['background']}" stroke="{c['line']}"/>
<circle cx="123" cy="58" r="33" fill="{c['active']}"/>
<path d="m116 41 25 17-25 17Z" fill="{c['accent']}"/>
<path d="M24 110H222" stroke="{c['line']}" stroke-width="4" stroke-linecap="round"/>
<path class="timeline" d="M24 110H222" stroke="{c['accent']}" stroke-width="4" stroke-linecap="round"/>
</g>'''
    waves = []
    for i in range(21):
        h = [18, 30, 52, 80, 104, 72, 46][i % 7]
        waves.append(f'<rect class="wave" x="{22+i*10}" y="{65-h/2}" width="4" height="{h}" rx="2" fill="{c["accent"]}" style="animation-delay:-{i*.12}s"/>')
    audio = f'<g transform="translate(856 173)"><rect width="246" height="130" rx="13" fill="{c["background"]}" stroke="{c["line"]}"/>{"".join(waves)}</g>'
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="460" viewBox="0 0 1200 460" role="img" aria-labelledby="title desc">
<title id="title">One assistant. More ways to create.</title><desc id="desc">Original animated capability illustration: image, video and speech generation. These are conceptual symbols, not generated media results or application UI. Availability depends on the selected model and account.</desc>
<defs><linearGradient id="canvas" x2="1" y2="1"><stop stop-color="{c['background']}"/><stop offset="1" stop-color="{c['end']}"/></linearGradient></defs>
<style>
text{{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Arial,sans-serif;fill:{c['text']}}}
.muted{{fill:{c['muted']}}}.eyebrow{{font-size:12px;letter-spacing:2px;font-weight:650;fill:{c['label']}}}
.wave{{transform-box:fill-box;transform-origin:center;animation:voice 1.8s ease-in-out infinite}}
.timeline{{stroke-dasharray:198;animation:play 4s ease-in-out infinite}}
.sun{{animation:glow 4s ease-in-out infinite}}.sweep{{animation:scan 4s ease-in-out infinite}}
@keyframes voice{{0%,100%{{transform:scaleY(.45);opacity:.55}}50%{{transform:scaleY(1);opacity:1}}}}
@keyframes play{{0%{{stroke-dashoffset:198}}80%,100%{{stroke-dashoffset:0}}}}
@keyframes glow{{0%,100%{{opacity:.5}}50%{{opacity:1}}}}
@keyframes scan{{0%,100%{{transform:translateY(0);opacity:0}}20%,80%{{opacity:.8}}90%{{transform:translateY(128px);opacity:0}}}}
@media(prefers-reduced-motion:reduce){{.wave,.timeline,.sun,.sweep{{animation:none}}}}
</style>
<rect width="1200" height="460" rx="24" fill="url(#canvas)"/>
<rect x=".5" y=".5" width="1199" height="459" rx="23.5" fill="none" stroke="{c['line']}"/>
<text x="38" y="40" class="eyebrow">CREATE WITH OFOX ASSISTANT</text>
<text x="38" y="83" font-size="32" font-weight="650" letter-spacing="-.5">One assistant. More ways to create.</text>
{''.join(frames)}{image}{video}{audio}
<text x="38" y="437" font-size="13" class="muted">Capability illustration · Available generation options depend on your selected model and account.</text>
</svg>'''


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--theme", choices=("dark", "light", "all"), default="all")
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    themes = ("dark", "light") if args.theme == "all" else (args.theme,)
    for theme in themes:
        for name in FLOWS:
            suffix = "-light" if theme == "light" else ""
            output = OUT / f"{name}{suffix}.svg"
            content = flow_svg(name, theme)
            ET.fromstring(content)
            output.write_text(content)
            print(f"Generated {output.name}: explanatory schematic, CSS motion, reduced-motion support.")
        suffix = "-light" if theme == "light" else ""
        output = OUT / f"assistant-media{suffix}.svg"
        content = assistant_media_svg(theme)
        ET.fromstring(content)
        output.write_text(content)
        print(f"Generated {output.name}: original media-capability illustration.")
