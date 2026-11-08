# Thin deck helpers for python-pptx (v2 port), run inside Pyodide before the model's code.
import io, re, base64
from lxml import etree
from pptx import Presentation
from pptx.util import Inches, Pt, Emu
from pptx.dml.color import RGBColor
from pptx.enum.shapes import MSO_SHAPE, MSO_CONNECTOR
from pptx.enum.text import PP_ALIGN, MSO_ANCHOR
from pptx.chart.data import CategoryChartData, XyChartData
from pptx.enum.chart import XL_CHART_TYPE, XL_LEGEND_POSITION, XL_LABEL_POSITION
from pptx.oxml.ns import qn
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib import font_manager
import numpy as np
import host

font_manager.fontManager.addfont("/fonts/NotoSansSC-Regular.otf")
plt.rcParams["font.family"] = ["Noto Sans SC", "DejaVu Sans"]
plt.rcParams["axes.unicode_minus"] = False

class T:
    primary = "1E3A5F"; primaryDark = "152B47"; accent = "D9822B"
    text = "1F2A37"; muted = "5B6672"; faint = "98A2AE"; line = "D5DDE6"
    tint = "EEF3F8"; accentTint = "FBEFE3"; white = "FFFFFF"
    font = "Microsoft YaHei"; mono = "Consolas"

class G:
    W = 13.333; H = 7.5; X = 0.72; R = 12.613; CW = 11.893; TOP = 1.7; BOTTOM = 6.85; GAP = 0.3

LINT = []
_state = {"footer": "", "page": 0}

# ---- fit estimates (CJK 1em, Latin 0.55em, line height 1.3) ----
_CJK = re.compile(r"[⺀-鿿가-힯豈-﫿＀-￯　-〿]")
def em_width(s): return sum(1.0 if _CJK.match(ch) else 0.55 for ch in str(s))
def lines_for(text, pt, width_in):
    per = max(1.0, width_in * 72 / pt)
    return sum(max(1, -(-em_width(p) // per)) for p in str(text).split("\n"))
def text_height(text, pt, width_in, spacing=1.3): return lines_for(text, pt, width_in) * pt * spacing / 72
def bullets_height(items, pt, width_in, gap_pt=6):
    indent = pt * 1.1 / 72
    return sum(text_height(it if isinstance(it, str) else it["text"], pt, width_in - indent, 1.25) + gap_pt / 72 for it in items)
def need(what, needed, has):
    if needed > has + 0.04: LINT.append(f"{what}: text needs {needed:.2f} in, box has {has:.2f} in")

def rgb(h): return RGBColor.from_string(str(h).lstrip("#"))

def _font(run, size, color, bold=False, font=None, italic=False):
    f = run.font; f.size = Pt(size); f.bold = bold; f.italic = italic; f.color.rgb = rgb(color); f.name = font or T.font
    rPr = run._r.get_or_add_rPr()
    latin = rPr.find(qn("a:latin"))
    ea = rPr.find(qn("a:ea"))
    if ea is None:
        ea = etree.SubElement(rPr, qn("a:ea")); latin.addnext(ea)
    ea.set("typeface", font or T.font)

_ALIGN = {"left": PP_ALIGN.LEFT, "center": PP_ALIGN.CENTER, "right": PP_ALIGN.RIGHT}
_ANCHOR = {"top": MSO_ANCHOR.TOP, "middle": MSO_ANCHOR.MIDDLE, "bottom": MSO_ANCHOR.BOTTOM}

def text(slide, content, x, y, w, h, size=13, color=None, bold=False, align="left", valign="top", font=None, spacing=1.15, bullet=False, gap=0):
    """content: str (\\n = new paragraph) or list of str / {"text", "bold", "color"}."""
    tb = slide.shapes.add_textbox(Inches(x), Inches(y), Inches(w), Inches(h))
    tf = tb.text_frame; tf.word_wrap = True; tf.auto_size = None
    tf.margin_left = tf.margin_right = tf.margin_top = tf.margin_bottom = 0
    tf.vertical_anchor = _ANCHOR[valign]
    items = content if isinstance(content, list) else str(content).split("\n")
    for i, it in enumerate(items):
        p = tf.paragraphs[0] if i == 0 else tf.add_paragraph()
        it = {"text": it} if isinstance(it, str) else it
        r = p.add_run(); r.text = str(it["text"])
        _font(r, size, it.get("color", color or T.text), it.get("bold", bold), font)
        p.alignment = _ALIGN[align]; p.line_spacing = spacing
        if gap: p.space_after = Pt(gap)
        if bullet:
            ind = int(Emu(Inches(size * 1.1 / 72)))
            pPr = p._p.get_or_add_pPr(); pPr.set("marL", str(ind)); pPr.set("indent", str(-ind))
            bu = etree.SubElement(pPr, qn("a:buChar")); bu.set("char", "•")
    return tb

def shape(slide, kind, x, y, w, h, fill=None, line=None, line_w=0.75, radius=None):
    s = slide.shapes.add_shape(kind, Inches(x), Inches(y), Inches(w), Inches(h))
    if fill: s.fill.solid(); s.fill.fore_color.rgb = rgb(fill)
    else: s.fill.background()
    if line: s.line.color.rgb = rgb(line); s.line.width = Pt(line_w)
    else: s.line.fill.background()
    s.shadow.inherit = False
    if radius is not None and kind == MSO_SHAPE.ROUNDED_RECTANGLE: s.adjustments[0] = min(0.5, radius / min(w, h))
    return s

def arrow(slide, x1, y1, x2, y2, color=None, width=1.5):
    c = slide.shapes.add_connector(MSO_CONNECTOR.STRAIGHT, Inches(x1), Inches(y1), Inches(x2), Inches(y2))
    c.line.color.rgb = rgb(color or T.primary); c.line.width = Pt(width)
    ln = c.line._get_or_add_ln(); te = etree.SubElement(ln, qn("a:tailEnd")); te.set("type", "triangle")
    return c

def new_deck(title="", footer=""):
    prs = Presentation(); prs.slide_width = Inches(G.W); prs.slide_height = Inches(G.H)
    prs.core_properties.title = title; prs.core_properties.author = "Ofox"
    _state["footer"] = footer; _state["page"] = 0
    return prs

def _layout(prs, name):
    for l in prs.slide_layouts:
        if l.name == name: return l
    return prs.slide_layouts[6]

def _bg(slide, color):
    f = slide.background.fill; f.solid(); f.fore_color.rgb = rgb(color)

def _notes(slide, notes):
    if notes: slide.notes_slide.notes_text_frame.text = notes

def content_slide(prs, title, subtitle="", notes=""):
    s = prs.slides.add_slide(_layout(prs, "Title Only")); _state["page"] += 1
    need(f'title "{title}"', text_height(title, 26, G.CW - 0.2, 1.1), 0.62)
    t = s.shapes.title; t.left, t.top, t.width, t.height = Inches(G.X + 0.2), Inches(0.4), Inches(G.CW - 0.2), Inches(0.62)
    tf = t.text_frame; tf.margin_left = tf.margin_right = tf.margin_top = tf.margin_bottom = 0; tf.vertical_anchor = MSO_ANCHOR.MIDDLE; tf.word_wrap = True
    p = tf.paragraphs[0]; p.alignment = PP_ALIGN.LEFT; r = p.add_run(); r.text = title; _font(r, 26, T.primary, True)
    shape(s, MSO_SHAPE.RECTANGLE, G.X, 0.5, 0.07, 0.5, fill=T.accent)
    ln = s.shapes.add_connector(MSO_CONNECTOR.STRAIGHT, Inches(G.X), Inches(1.42), Inches(G.R), Inches(1.42)); ln.line.color.rgb = rgb(T.line); ln.line.width = Pt(0.75)
    if subtitle:
        need(f'subtitle of "{title}"', text_height(subtitle, 13, G.CW - 0.2, 1.2), 0.38)
        text(s, subtitle, G.X + 0.2, 1.0, G.CW - 0.2, 0.36, size=13, color=T.muted, valign="middle")
    text(s, _state["footer"], G.X, 7.0, 8, 0.3, size=9, color=T.faint)
    text(s, str(_state["page"]), 12.0, 7.0, 0.6, 0.3, size=9, color=T.faint, align="right")
    _notes(s, notes)
    return s

def cover_slide(prs, title, subtitle="", kicker="", meta="", notes="", art=None):
    s = prs.slides.add_slide(_layout(prs, "Title Only")); _state["page"] += 1; _bg(s, T.primaryDark)
    w = 7.4 if art else 10.5
    size = 44 if lines_for(title, 44, w) <= 2 else 36
    th = min(2, lines_for(title, size, w)) * size * 1.15 / 72
    y = 1.9
    if kicker: text(s, kicker, G.X, y - 0.55, w, 0.4, size=13, color=T.accent, bold=True)
    t = s.shapes.title; t.left, t.top, t.width, t.height = Inches(G.X), Inches(y), Inches(w), Inches(th)
    tf = t.text_frame; tf.margin_left = tf.margin_right = tf.margin_top = tf.margin_bottom = 0; tf.vertical_anchor = MSO_ANCHOR.TOP; tf.word_wrap = True
    p = tf.paragraphs[0]; p.alignment = PP_ALIGN.LEFT; p.line_spacing = 1.0; r = p.add_run(); r.text = title; _font(r, size, T.white, True)
    y += th + 0.25
    if subtitle:
        sh = text_height(subtitle, 20, w, 1.4); text(s, subtitle, G.X, y, w, sh, size=20, color="DCE6F0"); y += sh + 0.3
    shape(s, MSO_SHAPE.RECTANGLE, G.X, y, 0.9, 0.05, fill=T.accent)
    if meta: text(s, meta, G.X, 6.3, 10, 0.5, size=11, color="AFC0D3")
    if art: figure(s, art, 8.3, 1.0, 4.6, 5.5, alt="cover art")
    _notes(s, notes)
    return s

def section_slide(prs, title, number="", subtitle="", notes=""):
    s = prs.slides.add_slide(_layout(prs, "Title Only")); _state["page"] += 1; _bg(s, T.primaryDark)
    if number: text(s, str(number), G.X, 1.6, 3, 1.4, size=80, color=T.accent, bold=True)
    t = s.shapes.title; t.left, t.top, t.width, t.height = Inches(G.X), Inches(3.1), Inches(11), Inches(1.0)
    tf = t.text_frame; tf.margin_left = tf.margin_right = tf.margin_top = tf.margin_bottom = 0; tf.vertical_anchor = MSO_ANCHOR.TOP
    p = tf.paragraphs[0]; p.alignment = PP_ALIGN.LEFT; r = p.add_run(); r.text = title; _font(r, 36, T.white, True)
    if subtitle: text(s, subtitle, G.X, 4.2, 11, 1.0, size=18, color="DCE6F0")
    _notes(s, notes)
    return s

def bullets(slide, items, x, y, w, h, size=13, color=None, gap=6):
    first = items[0] if isinstance(items[0], str) else items[0]["text"]
    need(f'bullets "{first[:12]}…"', bullets_height(items, size, w, gap), h)
    return text(slide, items, x, y, w, h, size=size, color=color, bullet=True, gap=gap)

def card(slide, x, y, w, h, title, body="", points=None, highlight=False, size=13):
    iw = w - 0.6; head_h = text_height(title, size + 2, iw, 1.4); top = y + 0.42 + head_h + 0.12
    body_h = bullets_height(points, size, iw) if points else (text_height(body, size, iw, 1.25) if body else 0)
    total = top - y + body_h + 0.22
    need(f'card "{title}"', total, h)
    if total < h * 0.5: LINT.append(f'card "{title}": the box is {h:.2f} in tall but its text needs {total:.2f} in; lower h or add content')
    shape(slide, MSO_SHAPE.ROUNDED_RECTANGLE, x, y, w, h, fill=T.accentTint if highlight else T.tint, line=T.accent if highlight else T.line, radius=0.08)
    shape(slide, MSO_SHAPE.RECTANGLE, x + 0.3, y + 0.26, 0.5, 0.05, fill=T.accent if highlight else T.primary)
    text(slide, title, x + 0.3, y + 0.4, iw, head_h, size=size + 2, bold=True, color=T.accent if highlight else T.primary)
    ih = max(0.3, y + h - top - 0.18)
    if points: text(slide, points, x + 0.3, top, iw, ih, size=size, bullet=True, gap=6)
    elif body: text(slide, body, x + 0.3, top, iw, ih, size=size, spacing=1.2)

def kpi(slide, x, y, w, h, value, label, note="", highlight=False):
    shape(slide, MSO_SHAPE.ROUNDED_RECTANGLE, x, y, w, h, fill=T.accentTint if highlight else T.tint, line=T.accent if highlight else T.line, radius=0.08)
    size = 44
    while size > 28 and em_width(value) * size / 72 > w - 0.5: size -= 4
    vh = size * 1.4 / 72
    text(slide, str(value), x + 0.3, y + 0.25, w - 0.6, vh, size=size, bold=True, color=T.accent if highlight else T.primary)
    text(slide, label, x + 0.3, y + 0.3 + vh, w - 0.6, 0.4, size=14, bold=True)
    if note:
        ny = y + 0.75 + vh; need(f'kpi note "{label}"', text_height(note, 12, w - 0.6), y + h - ny - 0.15)
        text(slide, note, x + 0.3, ny, w - 0.6, max(0.3, y + h - ny - 0.15), size=12, color=T.muted)

def steps(slide, items, x, y, w, h, highlight=-1, size=12):
    n = len(items); gap = 0.35; bw = (w - gap * (n - 1)) / n
    blocks = []
    for it in items:
        th = text_height(it["title"], size + 1, bw - 0.3, 1.4); bh = text_height(it["body"], size, bw - 0.3) if it.get("body") else 0
        blocks.append((th, bh, th + (0.08 + bh if it.get("body") else 0)))
    tallest = max(b[2] for b in blocks); need("steps", tallest + 0.3, h)
    if tallest + 0.3 < h * 0.5: LINT.append(f"steps: boxes are {h:.2f} in tall but the text needs {tallest + 0.3:.2f} in; lower h or add a body line")
    for i, it in enumerate(items):
        bx = x + i * (bw + gap); hi = i == highlight; th, bh, total = blocks[i]
        shape(slide, MSO_SHAPE.ROUNDED_RECTANGLE, bx, y, bw, h, fill=T.accentTint if hi else T.white, line=T.accent if hi else T.primary, line_w=1, radius=0.08)
        ty = y + (h - total) / 2
        text(slide, it["title"], bx + 0.15, ty, bw - 0.3, th, size=size + 1, bold=True, color=T.accent if hi else T.primary, align="center")
        if it.get("body"): text(slide, it["body"], bx + 0.15, ty + th + 0.08, bw - 0.3, bh + 0.05, size=size, align="center")
        if i < n - 1: arrow(slide, bx + bw + 0.04, y + h / 2, bx + bw + gap - 0.04, y + h / 2)

def timeline(slide, items, x, y, w, h, highlight=-1, size=12):
    n = len(items); line_y = y + 0.75; step = w / n
    ln = slide.shapes.add_connector(MSO_CONNECTOR.STRAIGHT, Inches(x), Inches(line_y), Inches(x + w), Inches(line_y)); ln.line.color.rgb = rgb(T.line); ln.line.width = Pt(2)
    for i, it in enumerate(items):
        cx = x + step * (i + 0.5); hi = i == highlight
        text(slide, it["label"], cx - step / 2, y, step, 0.45, size=16, bold=True, color=T.accent if hi else T.primary, align="center")
        shape(slide, MSO_SHAPE.OVAL, cx - 0.11, line_y - 0.11, 0.22, 0.22, fill=T.accent if hi else T.primary, line=T.white, line_w=1.5)
        text(slide, it["title"], cx - step / 2 + 0.1, line_y + 0.3, step - 0.2, 0.4, size=size + 1, bold=True, align="center")
        if it.get("body"):
            need(f'timeline "{it["title"]}"', 0.75 + text_height(it["body"], size, step - 0.2), y + h - line_y)
            text(slide, it["body"], cx - step / 2 + 0.1, line_y + 0.75, step - 0.2, max(0.3, y + h - line_y - 0.75), size=size, color=T.muted, align="center")

def callout(slide, y, message, x=None, w=None, h=0.62):
    x = G.X if x is None else x; w = G.CW if w is None else w
    need(f'callout "{message[:12]}…"', text_height(message, 13, w - 0.5, 1.2) + 0.1, h)
    shape(slide, MSO_SHAPE.RECTANGLE, x, y, w, h, fill=T.tint)
    shape(slide, MSO_SHAPE.RECTANGLE, x, y, 0.06, h, fill=T.accent)
    text(slide, message, x + 0.3, y, w - 0.5, h, size=13, bold=True, color=T.primary, valign="middle")

def formula(slide, x, y, w, h, lines, size=15):
    shape(slide, MSO_SHAPE.ROUNDED_RECTANGLE, x, y, w, h, fill=T.tint, line=T.line, radius=0.06)
    text(slide, list(lines), x + 0.3, y, w - 0.6, h, size=size, color=T.primary, font=T.mono, valign="middle", spacing=1.3)

def _picture(slide, png, x, y, w, h, alt):
    pic = slide.shapes.add_picture(io.BytesIO(png), Inches(x), Inches(y), Inches(w), Inches(h))
    pic._element.nvPicPr.cNvPr.set("descr", alt)
    return pic

def figure(slide, svg, x, y, w, h, alt="figure"):
    """An SVG figure, rasterised by the host at 2x."""
    svg = re.sub(r"((?:fill|stroke|stop-color|color)\s*=\s*[\"'])([0-9A-Fa-f]{6})([\"'])", r"\1#\2\3", str(svg))
    m = re.search(r"viewBox\s*=\s*[\"']\s*[-\d.]+[\s,]+[-\d.]+[\s,]+([\d.]+)[\s,]+([\d.]+)", svg)
    if m:
        vw, vh = float(m.group(1)), float(m.group(2)); pt_per_px = w * 72 / vw
        if abs(vw / vh - w / h) / (w / h) > 0.04: LINT.append(f'figure "{alt}": viewBox {vw:g}x{vh:g} does not match the box {w}x{h} in; it will be stretched')
        sizes = [float(v) for v in re.findall(r"font-size\s*=\s*[\"']([\d.]+)", svg)]
        if sizes and min(sizes) * pt_per_px < 10: LINT.append(f'figure "{alt}": smallest text is {min(sizes) * pt_per_px:.1f} pt on the slide; use at least 10 pt')
    png = base64.b64decode(host.svg_to_png(svg, round(w * 192), round(h * 192)))
    return _picture(slide, png, x, y, w, h, alt)

def plot(slide, fig, x, y, w, h, alt="chart"):
    """A matplotlib figure placed at x, y, w, h (inches); it is resized to the box."""
    fig.set_size_inches(w, h)
    for ax in fig.axes:
        for t in [ax.title, ax.xaxis.label, ax.yaxis.label] + ax.get_xticklabels() + ax.get_yticklabels():
            if t.get_text() and t.get_fontsize() < 10: LINT.append(f'plot "{alt}": text "{t.get_text()[:10]}" is {t.get_fontsize():.0f} pt; use at least 10')
    buf = io.BytesIO(); fig.savefig(buf, format="png", dpi=200, facecolor="white"); plt.close(fig)
    return _picture(slide, buf.getvalue(), x, y, w, h, alt)
