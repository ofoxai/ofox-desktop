# Thin deck helpers for python-pptx (v2 port), run inside Pyodide before the model's code.
import io, re, base64, datetime, json, math, zipfile
from copy import deepcopy as _deepcopy
from lxml import etree
from pptx import Presentation
from pptx.util import Inches, Pt, Emu
from pptx.dml.color import RGBColor
from pptx.enum.shapes import MSO_SHAPE, MSO_CONNECTOR, PP_PLACEHOLDER
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

class _ThemeAliases(type):
    @property
    def card(cls): return cls.tint
    @property
    def textMuted(cls): return cls.faint

class T(metaclass=_ThemeAliases):
    primary = "1E3A5F"; primaryDark = "152B47"; accent = "D9822B"
    text = "1F2A37"; muted = "5B6672"; faint = "98A2AE"; line = "D5DDE6"
    tint = "EEF3F8"; accentTint = "FBEFE3"; white = "FFFFFF"; surface = "FFFFFF"
    font = "Microsoft YaHei"; mono = "Consolas"
    titleFont = font; dataFont = font; latinTitle = font; latinBody = font
    background = "FFFFFF"; onDark = "FFFFFF"; onDarkMuted = "DCE6F0"

class G:
    W = 13.333; H = 7.5; X = 0.72; R = 12.613; CW = 11.893; TOP = 1.7; BOTTOM = 6.85; GAP = 0.3

LINT = []
_state = {"footer": "", "page": 0}
TEMPLATE = "/template.pptx"
_LEGACY_THEME = {key: value for key, value in vars(T).items() if not key.startswith("_")}
with open("/styles.json", encoding="utf-8") as _style_file:
    _STYLES = {item["id"]: item["theme"] for item in json.load(_style_file)["styles"]
               if item.get("status") == "enabled" and item.get("enabled") is True
               and item.get("adapted") is True and item.get("licenseReviewed") is True
               and isinstance(item.get("theme"), dict)}
try:
    with open("/selection.json", encoding="utf-8") as _selection_file:
        _SELECTED_STYLE = json.load(_selection_file).get("selectedStyle")
except FileNotFoundError:
    _SELECTED_STYLE = None
if _SELECTED_STYLE is not None and (_SELECTED_STYLE != "navy" and _SELECTED_STYLE not in _STYLES):
    raise ValueError("unknown or disabled selected style")
try:
    with open("/media.json", encoding="utf-8") as _media_file:
        _media_manifest = json.load(_media_file)
        _MEDIA = {item["assetId"]: item for item in _media_manifest["assets"]}
        _MEDIA_PAGES = _media_manifest.get("pageAssetIds", [])
        _MEDIA_FALLBACKS = _media_manifest.get("pageFallbacks", [])
        _MEDIA_PLAN_PAGES = _media_manifest.get("planPageNumbers", list(range(1, len(_MEDIA_PAGES)+1)))
        _MEDIA_PLAN_ACTIVE = "planId" in _media_manifest
except FileNotFoundError:
    _MEDIA = {}
    _MEDIA_PAGES = []
    _MEDIA_FALLBACKS = []
    _MEDIA_PLAN_PAGES = []
    _MEDIA_PLAN_ACTIVE = False
_MEDIA_USED = {}
try:
    with open("/source-meta.json", encoding="utf-8") as _source_file:
        _SOURCE_META = json.load(_source_file)
except FileNotFoundError:
    _SOURCE_META = None
_SOURCE_PRS = None
_SOURCE_SLIDE_IDS = None

def _set_theme(theme, language):
    for key, value in _LEGACY_THEME.items(): setattr(T, key, value)
    if theme is None: return
    c = theme["colors"]; fonts = theme["fonts"]; latin = fonts["latin"]
    selected = fonts.get(language, latin)
    for key, source in (("background", "background"), ("primary", "primary"), ("primaryDark", "dark"),
                        ("accent", "accent"), ("text", "text"), ("muted", "textSecondary"),
                        ("faint", "textMuted"), ("tint", "card"), ("onDark", "onDark"),
                        ("onDarkMuted", "onDarkMuted")):
        setattr(T, key, c[source].lstrip("#"))
    T.line = c.get("line", c["textMuted"]).lstrip("#")
    T.accentTint = c.get("accentTint", c["card"]).lstrip("#")
    T.font = selected["body"]; T.titleFont = selected["title"]; T.dataFont = latin["data"]
    T.latinTitle = latin["title"]; T.latinBody = latin["body"]

def _activate(prs):
    """Restore a deck's style when a script works on more than one deck."""
    context = getattr(prs, "_ofox_context", None)
    if context is not None and _state.get("context") is not context:
        _state.clear(); _state["context"] = context
        _set_theme(context["theme"], context["language"])
    return context

def _using(slide):
    context = _activate(slide.part.package.presentation_part.presentation)
    if context is not None and _composition() == "spec":
        # Each page of a declarative composition keeps its own canvas colours.
        _apply_canvas(_canvas(context.get("tones", {}).get(slide.slide_id, "base")))
    elif context is not None and context.get("tones"):
        # A hand-written composition with whole dark pages: each page keeps its own colours.
        _set_theme(context["theme"], context["language"])
        if _dark(slide): _apply_canvas(_dark_canvas())
    return context

def _theme(): return _state.get("context", {}).get("theme")
def _composition(): return (_theme() or {}).get("decor", {}).get("composition", "legacy")
def _size(role, legacy):
    values = (_theme() or {}).get("sizes", {}).get(role, [])
    return values[-1] if values else legacy

# ---- declarative compositions: theme.decor.layout, composition "spec" ----
def _layout_spec(): return (_theme() or {}).get("decor", {}).get("layout") or {}
def _canvas(name):
    """The colour roles of one named page canvas; "base" is the theme itself."""
    c = _theme()["colors"]
    canvas = {"bg": c["background"], "title": c["primary"], "body": c["text"], "muted": c["textSecondary"],
              "faint": c["textMuted"], "accent": c["accent"], "line": c.get("line", c["textMuted"]), "card": c["card"],
              "surface": c.get("surface", "FFFFFF"), "accentTint": c.get("accentTint", c["card"])}
    canvas.update((_layout_spec().get("canvases") or {}).get(name, {}))
    return canvas
def _apply_canvas(cv):
    T.background = cv["bg"]; T.primary = cv["title"]; T.text = cv["body"]; T.muted = cv["muted"]; T.faint = cv["faint"]
    T.accent = cv["accent"]; T.line = cv["line"]; T.tint = cv["card"]; T.surface = cv["surface"]; T.accentTint = cv["accentTint"]
def _slide_canvas(slide):
    return _canvas(_using(slide).get("tones", {}).get(slide.slide_id, "base"))
def _ref(value, cv):
    """A decor colour: a canvas role, a theme colour name, palette:N or RRGGBB."""
    if value is None or value == "none": return None
    if value in cv: return cv[value]
    colors = _theme()["colors"]
    if value.startswith("palette:"):
        palette = colors.get("palette") or colors["chart"]
        return palette[int(value[8:]) % len(palette)]
    return colors[value] if isinstance(colors.get(value), str) else value
def _mix(a, b, t): return "".join(f"{round(int(a[i:i + 2], 16) * (1 - t) + int(b[i:i + 2], 16) * t):02X}" for i in (0, 2, 4))
def _dark_canvas():
    """The colour roles of a whole dark page in a hand-written composition."""
    c = _theme()["colors"]; bg = c["dark"]; ink = c["onDark"]; card = _mix(bg, ink, 0.1)
    return {"bg": bg, "title": ink, "body": ink, "muted": c["onDarkMuted"], "faint": _mix(bg, ink, 0.6), "accent": c["accent"],
            "line": _mix(bg, ink, 0.28), "card": card, "surface": card, "accentTint": _mix(bg, c["accent"], 0.3)}
def _dark(slide):
    return _composition() != "spec" and _state.get("context", {}).get("tones", {}).get(slide.slide_id) == "dark"
def _darken(slide):
    """Make this page of a hand-written composition a whole dark page with light text."""
    _state["context"].setdefault("tones", {})[slide.slide_id] = "dark"; _using(slide)
def _luminance(color): return sum(int(color[i:i + 2], 16) * k for i, k in ((0, 0.299), (2, 0.587), (4, 0.114))) / 255
def _ink(fill):
    """Readable text on a filled block: the theme's darkest or lightest colour."""
    c = _theme()["colors"]
    choices = [c["dark"], c["text"], c["onDark"], c["background"], "1A1A1A", "FFFFFF"]
    return (min if _luminance(fill) > 0.55 else max)(choices, key=_luminance)

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
def _number(slide):
    """The slide's place in its deck, from 1."""
    slides = slide.part.package.presentation_part.presentation.slides
    return next((n for n, s in enumerate(slides, 1) if s.slide_id == slide.slide_id), 0)
def _lint(slide, message): LINT.append(f"slide {_number(slide)}: {message}")
def need(what, needed, has, slide=None):
    """A lint line when the text needs more room than the box has; it names the slide when one is given."""
    if needed <= has + 0.04: return
    message = f"{what}: text needs {needed:.2f} in, box has {has:.2f} in"
    if slide is None: LINT.append(message)
    else: _lint(slide, message)

def rgb(h): return RGBColor.from_string(str(h).lstrip("#"))

def _font(run, size, color, bold=False, font=None, italic=False, role="body"):
    east_asian = T.titleFont if role == "title" else T.font
    latin_font = T.latinTitle if role == "title" else (T.dataFont if role == "data" else T.latinBody)
    if _theme() is None: east_asian = latin_font = T.font
    f = run.font; f.size = Pt(size); f.bold = bold; f.italic = italic; f.color.rgb = rgb(color); f.name = font or latin_font
    rPr = run._r.get_or_add_rPr()
    latin = rPr.find(qn("a:latin"))
    ea = rPr.find(qn("a:ea"))
    if ea is None:
        ea = etree.SubElement(rPr, qn("a:ea")); latin.addnext(ea)
    ea.set("typeface", font or (T.dataFont if role == "data" else east_asian))

_ALIGN = {"left": PP_ALIGN.LEFT, "center": PP_ALIGN.CENTER, "right": PP_ALIGN.RIGHT}
_ANCHOR = {"top": MSO_ANCHOR.TOP, "middle": MSO_ANCHOR.MIDDLE, "bottom": MSO_ANCHOR.BOTTOM}

def _plain_text(value, name):
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string, not a structured object")
    return value

def _text_items(content, name="text"):
    """Validate the complete payload before a caller adds any native objects."""
    if isinstance(content, str): return content.split("\n")
    if not isinstance(content, list):
        raise ValueError(f"{name} must be a string or a list of strings/rich-text items")
    for item in content:
        if isinstance(item, str): continue
        if not isinstance(item, dict) or not isinstance(item.get("text"), str):
            raise ValueError(f"{name} items must be strings or rich-text objects with a string text field")
    return content

def _structured_text_items(items, name, required):
    if not isinstance(items, (list, tuple)) or not items:
        raise ValueError(f"{name} requires a nonempty list of string fields")
    for item in items:
        if not isinstance(item, dict):
            raise ValueError(f"{name} items require string fields: {', '.join(required)}")
        for key in required: _plain_text(item.get(key), f"{name} {key}")
        _plain_text(item.get("body", ""), f"{name} body")

def text(slide, content, x, y, w, h, size=None, color=None, bold=False, align="left", valign="top", font=None, spacing=1.15, bullet=False, gap=0, role="body"):
    """content: str (\\n = new paragraph) or list of str / {"text", "bold", "color"}."""
    items = _text_items(content)
    _using(slide); size = _size("body", 13) if size is None else size
    tb = slide.shapes.add_textbox(Inches(x), Inches(y), Inches(w), Inches(h))
    tf = tb.text_frame; tf.word_wrap = True; tf.auto_size = None
    tf.margin_left = tf.margin_right = tf.margin_top = tf.margin_bottom = 0
    tf.vertical_anchor = _ANCHOR[valign]
    for i, it in enumerate(items):
        p = tf.paragraphs[0] if i == 0 else tf.add_paragraph()
        it = {"text": it} if isinstance(it, str) else it
        r = p.add_run(); r.text = it["text"]
        _font(r, size, it.get("color", color or T.text), it.get("bold", bold), font, role=role)
        p.alignment = _ALIGN[align]; p.line_spacing = spacing
        if gap: p.space_after = Pt(gap)
        if bullet:
            ind = int(Emu(Inches(size * 1.1 / 72)))
            pPr = p._p.get_or_add_pPr(); pPr.set("marL", str(ind)); pPr.set("indent", str(-ind))
            # The marker takes the text's colour explicitly: a viewer that defaults it to
            # black loses it on a dark page canvas.
            marker = etree.SubElement(etree.SubElement(pPr, qn("a:buClr")), qn("a:srgbClr"))
            marker.set("val", str(it.get("color", color or T.text)).lstrip("#"))
            bu = etree.SubElement(pPr, qn("a:buChar")); bu.set("char", "•")
    return tb

def shape(slide, kind, x, y, w, h, fill=None, line=None, line_w=0.75, radius=None):
    _using(slide)
    s = slide.shapes.add_shape(kind, Inches(x), Inches(y), Inches(w), Inches(h))
    if fill: s.fill.solid(); s.fill.fore_color.rgb = rgb(fill)
    else: s.fill.background()
    if line: s.line.color.rgb = rgb(line); s.line.width = Pt(line_w)
    else: s.line.fill.background()
    s.shadow.inherit = False
    if radius is not None and kind == MSO_SHAPE.ROUNDED_RECTANGLE: s.adjustments[0] = min(0.5, radius / min(w, h))
    return s

def arrow(slide, x1, y1, x2, y2, color=None, width=1.5):
    _using(slide)
    c = slide.shapes.add_connector(MSO_CONNECTOR.STRAIGHT, Inches(x1), Inches(y1), Inches(x2), Inches(y2))
    c.line.color.rgb = rgb(color or T.primary); c.line.width = Pt(width)
    ln = c.line._get_or_add_ln(); te = etree.SubElement(ln, qn("a:tailEnd")); te.set("type", "triangle")
    return c

def new_deck(title="", footer="", style="navy", language="zh"):
    """A native 16:9 deck. Public styles are experimental adaptations, not quality certifications.
    Only an enabled, packaged catalog entry with an adapted theme is selectable; navy preserves the legacy template."""
    if _SOURCE_META is not None:
        raise ValueError("source edit mode requires source_deck(), not new_deck()")
    if _SELECTED_STYLE is not None: style = _SELECTED_STYLE
    if style != "navy" and style not in _STYLES:
        raise ValueError(f"unknown or disabled style: {style}")
    if not isinstance(language, str): raise ValueError("language must be a string")
    prs = Presentation(TEMPLATE)
    prs._ofox_context = {"style": style, "theme": _STYLES.get(style) if style != "navy" else None,
                         "language": language, "footer": footer, "page": 0}
    _activate(prs)
    if style != "navy":
        # Keep the native outline/notes placeholders; remove the two navy-only
        # Content decorations rather than tinting every imported composition.
        for layout in prs.slide_layouts:
            for element in [item._element for item in layout.shapes if not item.is_placeholder]:
                element.getparent().remove(element)
            fill = layout.background.fill; fill.solid(); fill.fore_color.rgb = rgb(T.background)
    now = datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None)
    prs.core_properties.title = title; prs.core_properties.author = "Ofox"
    prs.core_properties.created = now; prs.core_properties.modified = now
    return prs

def _prepare_source():
    global _SOURCE_PRS, _SOURCE_SLIDE_IDS
    if _SOURCE_META is None or _SOURCE_PRS is not None: return
    with zipfile.ZipFile("/source.pptx") as archive:
        entries = archive.infolist()
        if len(entries) > 10000 or len({entry.filename for entry in entries}) != len(entries) or sum(entry.file_size for entry in entries) > 100 * 1024 * 1024:
            raise ValueError("source deck archive exceeds bounds")
        for entry in entries:
            if entry.filename.startswith("/") or "\\" in entry.filename or ".." in entry.filename.split("/") or ((entry.external_attr >> 16) & 0o170000) == 0o120000:
                raise ValueError("invalid source archive entry")
            if entry.filename.endswith((".xml", ".rels")):
                if entry.file_size > 16 * 1024 * 1024: raise ValueError("source XML exceeds bounds")
                data = archive.read(entry).upper()
                if b"<!DOCTYPE" in data or b"<!ENTITY" in data: raise ValueError("source DTD is unsupported")
    prs = Presentation("/source.pptx")
    if len(prs.slides) != _SOURCE_META["totalPages"]: raise ValueError("source page count mismatch")
    style = _SELECTED_STYLE or "navy"
    prs._ofox_context = {"style": style, "theme": _STYLES.get(style), "language": "zh", "footer": "", "page": len(prs.slides)}
    _SOURCE_PRS = prs
    _SOURCE_SLIDE_IDS = [slide.slide_id for slide in prs.slides]
    _activate(prs)

def source_deck():
    """Return the Host's source snapshot; no caller-supplied path."""
    if _SOURCE_META is None: raise ValueError("no source deck was authorized")
    _prepare_source()
    return _SOURCE_PRS

def target_slides(prs):
    """The authorized actual pages, in the frozen plan's order."""
    if _SOURCE_META is None or prs is not _SOURCE_PRS: raise ValueError("not the authorized source presentation")
    return [prs.slides[page - 1] for page in _SOURCE_META["pageNumbers"]]

def _validate_source(prs):
    if _SOURCE_META is None: return
    if prs is not _SOURCE_PRS or [slide.slide_id for slide in prs.slides] != _SOURCE_SLIDE_IDS:
        raise RuntimeError("source edit must preserve the original presentation, page count and slide IDs")

def _source_charts(prs):
    if _SOURCE_META is None: return None
    allowed, protected = set(), set()
    targets = set(_SOURCE_META["pageNumbers"])
    for page, slide in enumerate(prs.slides, 1):
        chart_parts = {rel.target_part for rel in slide.part.rels.values() if not rel.is_external and rel.reltype.endswith("/chart")}
        (allowed if page in targets else protected).update(chart_parts)
    return allowed - protected

def _new_slide(prs, layout):
    context = _activate(prs); context["page"] += 1
    slide = prs.slides.add_slide(_layout(prs, layout)); _using(slide)  # a new page starts from its deck's base colours
    return slide

def _layout(prs, name):
    for l in prs.slide_layouts:
        if l.name == name: return l
    raise RuntimeError(f"the template has no layout named {name}")

def _placeholder(slide, kind):
    """The slide's own copy of a layout placeholder python-pptx does not clone (footer, page number)."""
    for ph in slide.slide_layout.placeholders:
        if ph.placeholder_format.type == kind:
            slide.shapes.clone_placeholder(ph)
            return next(p for p in slide.placeholders if p.placeholder_format.idx == ph.placeholder_format.idx)
    raise RuntimeError(f"the layout {slide.slide_layout.name} has no placeholder {kind}")

def _flush(tf, anchor):
    """Zero insets and an explicit anchor: the preview does not read them from the layout."""
    tf.margin_left = tf.margin_right = tf.margin_top = tf.margin_bottom = 0
    tf.vertical_anchor = _ANCHOR[anchor]; tf.word_wrap = True

def _footer(slide, page):
    """The footer text and the page number, each in the layout's own placeholder.
    The number is plain text, not a slide-number field: the preview draws a field blank."""
    context = _using(slide)
    page_number = (context["theme"] or {}).get("decor", {}).get("pageNumber", True)
    for kind, content, align in ((PP_PLACEHOLDER.FOOTER, context["footer"], PP_ALIGN.LEFT), (PP_PLACEHOLDER.SLIDE_NUMBER, str(page) if page_number else "", PP_ALIGN.RIGHT)):
        if not content: continue
        tf = _placeholder(slide, kind).text_frame; _flush(tf, "top")
        p = tf.paragraphs[0]; p.alignment = align
        r = p.add_run(); r.text = content; _font(r, 9, T.faint)

def background(slide, color):
    """Colour the whole slide. The slide's own background is set for PowerPoint, and a full-slide rectangle goes at the back
    of the slide for the preview, which paints every page white and ignores that background. Text on it needs a light colour.
    Calling it again recolours: the slide keeps one Background rectangle (cover and section slides start with one)."""
    f = slide.background.fill; f.solid(); f.fore_color.rgb = rgb(color)
    tree = slide.shapes._spTree
    for old in [sh for sh in slide.shapes if sh.name == "Background"]: tree.remove(old._element)  # a cover starts with one: the latest colour wins
    pres = slide.part.package.presentation_part.presentation
    r = shape(slide, MSO_SHAPE.RECTANGLE, 0, 0, 1, 1, fill=color)
    r.left, r.top, r.width, r.height = 0, 0, pres.slide_width, pres.slide_height
    r.name = "Background"
    tree.remove(r._element); tree.insert(2, r._element)  # after nvGrpSpPr and grpSpPr
    return r

def _notes(slide, notes):
    if notes: slide.notes_slide.notes_text_frame.text = notes

def bring_title_to_front(slide):
    """Raise the same native title without changing its text, fonts, geometry or ID."""
    title_shape = slide.shapes.title
    if title_shape is None: raise ValueError("slide has no native title placeholder")
    tree = slide.shapes._spTree; tree.remove(title_shape._element)
    tree.insert_element_before(title_shape._element, "p:extLst")
    return title_shape

def _native_title(slide, title, x, y, w, h, size, color, align="left", bold=True):
    """Retain the native title placeholder (PowerPoint outline and accessibility)."""
    title_shape = slide.shapes.title
    title_shape.left, title_shape.top, title_shape.width, title_shape.height = map(Inches, (x, y, w, h))
    tf = title_shape.text_frame; tf.clear(); _flush(tf, "top")
    p = tf.paragraphs[0]; p.alignment = _ALIGN[align]; p.line_spacing = 1.05
    r = p.add_run(); r.text = str(title); _font(r, size, color, bold, role="title")
    need(f'title "{title}"', text_height(title, size, w, 1.1), h, slide)
    return bring_title_to_front(slide)

def _hero_size(title, width, height, role="hero", tidy=False):
    candidates = (_theme() or {}).get("sizes", {}).get(role, [44, 40, 36, 32])
    # tidy: a narrow column takes the largest size that adds no line over the smallest one.
    if tidy: candidates = [size for size in candidates if lines_for(title, size, width) == lines_for(title, candidates[-1], width)]
    return next((size for size in candidates if text_height(title, size, width, 1.1) <= height), candidates[-1])

def _alternating_panel(slide, side="auto", numeral=False):
    """The architectural source's 13 cm color panel, adapted to the 16:9 canvas.
    It starts on the right, as the source does; numeral draws the page index large at its head."""
    _using(slide)
    if side == "auto": side = "right" if _number(slide) % 2 else "left"
    if side not in ("left", "right"): raise ValueError("panel side must be auto, left or right")
    for old in [item for item in slide.shapes if item.name.startswith("Alternating panel") or item.name == "Panel navigation"]:
        old._element.getparent().remove(old._element)
    width = 5.12; left = 0 if side == "left" else G.W - width
    panel = shape(slide, MSO_SHAPE.RECTANGLE, left, 0, width, G.H, fill=T.tint)
    panel.name = f"Alternating panel {side}"
    tree = slide.shapes._spTree; tree.remove(panel._element); tree.insert(3, panel._element)
    navigation = text(slide, f'{_number(slide):02d}', left + 0.7, 0.7 if numeral else 6.42, 2.4 if numeral else 1.0,
                      1.0 if numeral else 0.38, size=54 if numeral else 16, color=T.primary, role="data")
    navigation.name = "Panel navigation"
    slide._ofox_regions = {
        "panel": {"x": left + 0.7, "y": G.TOP + 0.2, "w": width - 1.4, "h": 4.2},
        "main": {"x": width + 0.45 if side == "left" else G.X, "y": G.TOP + 0.2,
                 "w": G.W - width - G.X - 0.45, "h": 4.8},
    }
    return panel

_SPEC_ROLES = {"cover": "hero", "closing": "hero", "section": "statement", "statement": "statement", "content": "title"}
_SPEC_SHAPES = {"rect": MSO_SHAPE.RECTANGLE, "round": MSO_SHAPE.ROUNDED_RECTANGLE, "oval": MSO_SHAPE.OVAL}

def _spec_page(slide, kind, title, subtitle="", kicker="", meta="", number="", notes="", attribution="", art=None):
    """One page of a declarative composition: its canvas, decoration shapes and text blocks,
    all native. The title stays the layout's own title placeholder."""
    context = _state["context"]; spec = _layout_spec(); page = spec.get(kind) or {}
    tone = (spec.get("tones") or {}).get(kind, "base")
    if isinstance(tone, list): tone = tone[(context["page"] - 1) % len(tone)] if tone else "base"
    context.setdefault("tones", {})[slide.slide_id] = tone
    cv = _canvas(tone); _apply_canvas(cv); background(slide, cv["bg"])
    for item in page.get("shapes", []):
        # "repeat": {"count", "dx", "dy"} draws a run of the same shape (rules, stripes, dots).
        repeat = item.get("repeat") or {"count": 1}
        for step in range(min(60, repeat.get("count", 1))):
            drawn = shape(slide, _SPEC_SHAPES[item.get("kind", "rect")], item["x"] + step * repeat.get("dx", 0),
                          item["y"] + step * repeat.get("dy", 0), item["w"], item["h"], fill=_ref(item.get("fill"), cv),
                          line=_ref(item.get("line"), cv), line_w=item.get("lineW", 0.75), radius=item.get("radius"))
            if item.get("rot"): drawn.rotation = item["rot"]
            drawn.name = "Decor " + item.get("name", item.get("kind", "rect"))
    content = kind == "content"
    box = {"x": G.X, "y": 0.42 if content else 2.0, "w": G.CW, "h": 0.8 if content else 2.6, "align": "left", "anchor": "top"}
    box.update(page.get("title") or {})
    if art: box.update(x=min(box["x"], G.X), w=min(box["w"], 7.0), align="left")
    # An all-caps style leaves a formula or a call as written: r and n are not R and N.
    upper = lambda value: str(value).upper() if spec.get("upper") and box.get("upper", True) and not re.search(r"[=^]|\w\(", str(value)) else str(value)
    label = upper(title)
    sizes = box.get("sizes") or _theme()["sizes"][_SPEC_ROLES[kind]]
    if content: sizes = [size for size in sizes if size <= 36] or sizes[-1:]
    def block(key, value, default, size, color, bold=False, role="body"):
        """An optional secondary text block; returns the height it takes."""
        if not value: return 0
        b = dict(default, **(page.get(key) or {})); size = b.get("size", size)
        height = text_height(value, size, b["w"], 1.2) + 0.05
        if "h" in b: need(f"{kind} {key}", height, b["h"], slide)
        text(slide, value, b["x"], b["y"], b["w"], b.get("h", height), size=size, color=_ref(b.get("color", color), cv),
             bold=bold, align=b.get("align", box["align"]), role=role)
        return height
    below = {"w": min(box["w"], 9.6)}
    sub_size = min(15, _size("caption", 13)) if content else min(22, _size("subtitle", 20))
    floating = [(key, value, size) for key, value, size in (("subtitle", subtitle, sub_size), ("attribution", attribution, 14))
                if value and key not in page]
    reserve = sum(text_height(value, size, below["w"], 1.2) + 0.3 for _, value, size in floating)
    room = max(0.5, box["h"] - (reserve if box["anchor"] != "top" else 0))
    size = next((size for size in sizes if text_height(label, size, box["w"], 1.1) <= room), sizes[-1])
    height = text_height(label, size, box["w"], 1.1) + 0.1
    y = box["y"] + {"top": 0, "middle": (room - height) / 2, "bottom": room - height}[box["anchor"]]
    block("kicker", upper(kicker) if kicker else "", {"x": box["x"], "y": max(0.12, y - 0.5), "w": box["w"]}, 14, "accent", True)
    block("number", str(number) if number else "", {"x": box["x"], "y": max(0.3, y - 1.35), "w": box["w"]}, 64, "accent", True, "data")
    _native_title(slide, label, box["x"], y, box["w"], height, size, _ref(box.get("color", "title"), cv), box["align"],
                  bold=box.get("bold", spec.get("titleBold", True)))
    cursor = y + height + (0.02 if content else 0.25)
    for key, value, text_size in (("subtitle", subtitle, sub_size), ("attribution", attribution, 14)):
        taken = block(key, value, dict(below, x=box["x"], y=cursor), text_size, "muted")
        if key not in page and taken: cursor += taken + 0.2
    block("meta", meta, {"x": box["x"], "y": 6.6, "w": box["w"]}, 11, "faint")
    if art: figure(slide, art, 8.3, 1.7, 4.2, 4.5, alt="cover art")
    if content:
        _footer(slide, context["page"])
        if page.get("body"): slide._ofox_regions = {"main": dict(page["body"])}
    _notes(slide, notes); return slide

def _spec_block(slide, x, y, w, h, style, highlight=False):
    """A container in a declarative composition's own card style.
    Returns the colour its text needs, or None when the page's own text colours apply."""
    context = _using(slide); cv = _slide_canvas(slide)
    kind = MSO_SHAPE.ROUNDED_RECTANGLE if style.get("shape") == "round" else MSO_SHAPE.RECTANGLE
    radius = style.get("radius", 0.12) if kind == MSO_SHAPE.ROUNDED_RECTANGLE else None
    fill = style.get("fill", "card")
    if fill == "palette":
        counts = context.setdefault("blocks", {}); index = counts.get(slide.slide_id, 0); counts[slide.slide_id] = index + 1
        fill = _ref(f"palette:{index}", cv)
    else:
        fill = _ref("accentTint" if highlight and fill == "card" else fill, cv)
    line = _ref(style.get("line", "line"), cv)
    if highlight and line and style.get("fill", "card") != "palette": line = cv["accent"]
    shadow = style.get("shadow")
    if shadow: shape(slide, kind, x + shadow[0], y + shadow[1], w, h, fill=_ref(shadow[2], cv), radius=radius)
    if fill or line: shape(slide, kind, x, y, w, h, fill=fill, line=line, line_w=style.get("lineW", 0.75), radius=radius)
    bar = style.get("bar"); bar_color = cv["accent"] if highlight else _ref(style.get("barColor", "title"), cv)
    if bar == "top": shape(slide, MSO_SHAPE.RECTANGLE, x, y, w, style.get("barW", 0.05), fill=bar_color)
    elif bar == "left": shape(slide, MSO_SHAPE.RECTANGLE, x, y, style.get("barW", 0.05), h, fill=bar_color)
    if not fill or fill == cv["bg"]: return None
    # A style may name the text colour its filled blocks carry; otherwise pick by contrast.
    return _ref(style["ink"], cv) if style.get("ink") else _ink(fill)

def _panel_content(slide, title, subtitle, notes, side="auto"):
    """An architectural content page: the page numeral, title and subtitle live in the
    panel, and the ruled main region is left whole for the body."""
    background(slide, T.background); _alternating_panel(slide, side, numeral=True)
    regions = slide._ofox_regions; main = regions["main"]; x = regions["panel"]["x"]; width = regions["panel"]["w"]
    shape(slide, MSO_SHAPE.RECTANGLE, main["x"], 0.72, main["w"], 0.012, fill=T.line)
    shape(slide, MSO_SHAPE.RECTANGLE, main["x"] + main["w"] - 0.15, 0.4, 0.15, 0.15, line=T.accent)
    shape(slide, MSO_SHAPE.RECTANGLE, x, 1.95, 0.9, 0.05, fill=T.accent)
    # The largest size that adds no line over the smallest, with a margin for bold glyphs.
    size = next(size for size in (32, 28, 24, 20) if lines_for(title, size, width - 0.4) == lines_for(title, 20, width - 0.4))
    height = text_height(title, size, width, 1.1) + 0.1; cursor = 2.25 + height + 0.2
    _native_title(slide, title, x, 2.25, width, height, size, T.primary)
    if subtitle:
        caption = min(15, _size("caption", 13)); taken = text_height(subtitle, caption, width, 1.2) + 0.05
        text(slide, subtitle, x, cursor, width, taken, size=caption, color=T.muted, spacing=1.2); cursor += taken + 0.2
    need(f'panel text of "{title}"', cursor, 6.4, slide)
    regions["panel"].update(y=cursor + 0.1, h=max(0.3, 6.3 - cursor))
    main.update(y=1.0, h=5.7)
    _footer(slide, _state["context"]["page"]); _notes(slide, notes)
    # What the page drew itself, and the panel's extent: the deck's own content is checked against it.
    band = next(item for item in slide.shapes if item.name.startswith("Alternating panel"))
    slide._ofox_panel_frame = {"left": band.left / Inches(1), "right": (band.left + band.width) / Inches(1),
                               "head": regions["panel"]["y"], "ids": {item.shape_id for item in slide.shapes}}
    return slide

def _styled_content(slide, title, subtitle, notes):
    if _composition() == "spec": return _spec_page(slide, "content", title, subtitle=subtitle, notes=notes)
    mode = _composition()
    if mode == "blueprint": return _panel_content(slide, title, subtitle, notes)
    # The launch source alternates light and dark pages: every fourth page is wholly dark.
    if mode == "launch" and _state["context"]["page"] % 4 == 0: _darken(slide)
    dark = _dark(slide); background(slide, T.background)
    x = G.X; color = T.primary; size = min(32, _size("title", 28))
    if mode == "launch" and not dark:
        shape(slide, MSO_SHAPE.RECTANGLE, 0, 0, G.W, 1.5, fill=T.primaryDark)
        color = T.onDark
    elif mode == "split":
        shape(slide, MSO_SHAPE.RECTANGLE, 0, 0, 0.28, G.H, fill=T.primary)
    elif mode == "course":
        shape(slide, MSO_SHAPE.RECTANGLE, G.X, 0.44, 0.11, 0.62, fill=T.accent); x += 0.28
    elif (_theme().get("decor", {}).get("titleRule") == "bar"):
        shape(slide, MSO_SHAPE.RECTANGLE, G.X, 1.42, 1.3, 0.035, fill=T.accent)
    _native_title(slide, title, x, 0.42, G.R - x, 0.7, size, color)
    if subtitle:
        caption = min(15, _size("caption", 13))
        need(f'subtitle of "{title}"', text_height(subtitle, caption, G.R - x, 1.15), 0.32, slide)
        text(slide, subtitle, x, 1.1, G.R - x, 0.32, size=caption, color=T.onDarkMuted if mode == "launch" and not dark else T.muted)
    _footer(slide, _state["context"]["page"]); _notes(slide, notes)
    return slide

def _styled_cover(slide, title, subtitle, kicker, meta, notes, art, closing=False):
    if _composition() == "spec":
        return _spec_page(slide, "cover", title, subtitle=subtitle, kicker=kicker, meta=meta, notes=notes, art=art)
    mode = _composition(); dark = mode == "launch"; paper = closing and mode == "split"
    if closing and mode == "blueprint": _darken(slide)  # the source closes on a whole dark page
    background(slide, T.primaryDark if dark else T.background)
    x, y, width, align = G.X, 2.0, 10.8, "left"
    color = T.onDark if dark else T.text
    if paper:
        # The source's closing is centred on plain paper, not on the cobalt panel.
        x, width, align = 1.6, G.W - 3.2, "center"
    elif mode == "split":
        shape(slide, MSO_SHAPE.RECTANGLE, 0, 0, 3.8, G.H, fill=T.primary)
        x, y, width = 4.55, 2.05, 7.9
        # Native Cartesian marks retain the source style's diagrammatic contrast.
        shape(slide, MSO_SHAPE.RECTANGLE, 1.15, 3.2, 1.5, 0.018, fill=T.onDark)
        shape(slide, MSO_SHAPE.RECTANGLE, 1.89, 2.46, 0.018, 1.5, fill=T.onDark)
    elif mode == "course":
        shape(slide, MSO_SHAPE.RECTANGLE, 0, 0, 3.1, G.H, fill=T.tint)
        shape(slide, MSO_SHAPE.RECTANGLE, 3.1, 0, 0.08, G.H, fill=T.accent)
        x, y, width = 3.85, 2.25, 8.7
    elif mode == "launch":
        x, y, width, align = 0.85, 1.9, G.W - 1.7, "center"
    elif mode == "blueprint":
        _alternating_panel(slide)
        # Title, kicker and rule stay wholly on the paper beside the panel.
        main = slide._ofox_regions["main"]; x, y, width = main["x"], 2.05, main["w"]
        shape(slide, MSO_SHAPE.RECTANGLE, x, 1.45, width, 0.015, fill=T.line)
    if art:
        width = min(width, 7.0 if mode != "split" else 3.2)
    size = _hero_size(title, width, 2.2, tidy=mode == "blueprint"); height = text_height(title, size, width, 1.1) + 0.1
    if paper:
        y = (G.H - height) / 2 - 0.5
        shape(slide, MSO_SHAPE.RECTANGLE, G.W / 2 - 0.3, y - 0.4, 0.6, 0.045, fill=T.primary)
    # The architectural accent is too pale for text on its paper; its kicker takes the title colour.
    if kicker: text(slide, kicker, x, 0.95, width, 0.4, size=14, color=T.primary if mode == "blueprint" else T.accent, bold=True, align=align)
    _native_title(slide, title, x, y, width, height, size, color, align)
    if subtitle:
        subtitle_size = min(22, _size("subtitle", 20)); sy = y + height + 0.28
        text(slide, subtitle, x, sy, width, text_height(subtitle, subtitle_size, width, 1.2) + 0.05,
             size=subtitle_size, color=T.onDarkMuted if dark else T.muted, align=align)
    if meta:
        meta_x, meta_width = x, width
        # The architectural page index sits in the panel and the metadata on the paper beside it.
        if mode == "blueprint": need("cover metadata", text_height(meta, 11, meta_width), 0.4, slide)
        text(slide, meta, meta_x, 6.45, meta_width, 0.4, size=11, color=T.onDarkMuted if dark else T.faint, align=align)
    if art: figure(slide, art, 8.3, 1.7, 4.2, 4.5, alt="cover art")
    _notes(slide, notes); return slide

def _styled_section(slide, title, number, subtitle, notes, kicker="", statement=False):
    if _composition() == "spec":
        return _spec_page(slide, "section", title, subtitle=subtitle, number=number, notes=notes)
    mode = _composition(); dark = mode == "launch"; paper = statement and mode == "split"
    background(slide, T.primaryDark if dark else T.background)
    x, y, width, align = G.X, 3.0, G.CW, "left"
    if paper:
        # The source's quotation is centred on plain paper, not beside the cobalt panel.
        x, width, align = 1.6, G.W - 3.2, "center"
    elif mode == "split":
        shape(slide, MSO_SHAPE.RECTANGLE, 0, 0, 3.8, G.H, fill=T.primary)
        if number: text(slide, str(number), 0.8, 2.65, 2.2, 1.6, size=72, color=T.onDark, role="data")
        x, y, width = 4.55, 2.75, 7.8
    else:
        if mode == "blueprint":
            _alternating_panel(slide); main = slide._ofox_regions["main"]; x, width = main["x"], main["w"]
        if mode == "course": shape(slide, MSO_SHAPE.RECTANGLE, 0, 0, 0.24, G.H, fill=T.primary)
        if mode == "launch": x, width, align = 0.85, G.W - 1.7, "center"
        if number: text(slide, str(number), x, 1.2, width, 1.45, size=64, color=T.primary if mode == "blueprint" else T.accent, align=align, role="data")
    size = _hero_size(title, width, 1.6, "statement", tidy=paper)
    height = text_height(title, size, width, 1.1) + 0.1
    if paper:
        y = (G.H - height) / 2 - 0.3
        shape(slide, MSO_SHAPE.RECTANGLE, G.W / 2 - 0.3, y - (0.95 if kicker else 0.4), 0.6, 0.045, fill=T.primary)
    # The kicker sits above the statement on the statement's own axis.
    if kicker: text(slide, kicker, x, max(0.3, y - 0.55), width, 0.4, size=14, color=T.accent, bold=True, align=align)
    _native_title(slide, title, x, y, width, height, size, T.onDark if dark else T.text, align)
    if subtitle: text(slide, subtitle, x, y + height + 0.3, width, 1.0, size=18, color=T.onDarkMuted if dark else T.muted, align=align)
    _notes(slide, notes); return slide

def panel_slide(prs, title, subtitle="", notes="", side="auto"):
    """A native two-region page. Architectural panels alternate left/right by page;
    use slide_regions() to place evidence in the panel and the argument in main."""
    if side not in ("auto", "left", "right"): raise ValueError("panel side must be auto, left or right")
    _activate(prs)
    if _composition() == "blueprint": return _panel_content(_new_slide(prs, "Content"), title, subtitle, notes, side)
    slide = content_slide(prs, title, subtitle=subtitle, notes=notes)
    _alternating_panel(slide, side)
    return slide

def lesson_slide(prs, title, objective, subtitle="", notes="", objective_title=None, current=None):
    """A native lesson page with an objective rail and a distinct main content region.
    objective: one sentence, or a list of objectives drawn with progress dots; current is the index of the one in hand."""
    listed = isinstance(objective, (list, tuple))
    if listed:
        if not objective or any(not isinstance(item, str) or not item.strip() for item in objective):
            raise ValueError("lesson objective must be a string or a nonempty list of strings")
        if current is not None and (isinstance(current, bool) or not isinstance(current, int) or not 0 <= current < len(objective)):
            raise ValueError("lesson current must be the index of a listed objective")
    else:
        _plain_text(objective, "lesson objective")
        if current is not None: raise ValueError("lesson current needs a list of objectives")
    if objective_title is not None: _plain_text(objective_title, "lesson objective title")
    slide = content_slide(prs, title, subtitle=subtitle, notes=notes); context = _using(slide)
    labels = {"zh": "学习目标", "ja": "学習目標"}
    label = objective_title or labels.get(context["language"], "Learning objective")
    panel = shape(slide, MSO_SHAPE.RECTANGLE, G.X, 1.85, 2.45, 4.85, fill=T.tint)
    panel.name = "Lesson objective region"
    shape(slide, MSO_SHAPE.RECTANGLE, G.X, 1.85, 2.45, 0.06, fill=T.accent)
    text(slide, label, G.X + 0.22, 2.15, 2.01, 0.6, size=18, bold=True, color=T.primary, role="title")
    if listed:
        # The source rail: one line per objective behind a progress dot; done filled, later ones open.
        cursor = 2.92; width = 2.01 - 0.26
        for index, item in enumerate(objective):
            here = index == current; done = current is None or index < current
            dot = shape(slide, MSO_SHAPE.OVAL, G.X + 0.22, cursor + 0.06, 0.12, 0.12, fill=T.accent if here else (T.primary if done else None),
                        line=None if here or done else T.faint, line_w=1)
            dot.name = "Lesson objective current" if here else "Lesson objective dot"
            taken = text_height(item, 13, width, 1.25)
            text(slide, item, G.X + 0.48, cursor, width, taken, size=13, bold=here, color=T.text if here or current is None else T.muted, spacing=1.2)
            cursor += taken + 0.16
        need("lesson objectives", cursor - 2.92 - 0.16, 3.35, slide)
    else:
        need("lesson objective", text_height(objective, 16, 2.01), 3.35, slide)
        text(slide, objective, G.X + 0.22, 2.9, 2.01, 3.35, size=16)
    slide._ofox_regions = {
        "objective": {"x": G.X + 0.22, "y": 2.9, "w": 2.01, "h": 3.35},
        "main": {"x": 3.65, "y": 1.85, "w": G.R - 3.65, "h": 4.85},
    }
    return slide

def slide_regions(slide):
    """Suggested native content coordinates for panel_slide/lesson_slide (inches)."""
    return _deepcopy(getattr(slide, "_ofox_regions", {"main": {"x": G.X, "y": G.TOP, "w": G.CW, "h": G.BOTTOM - G.TOP}}))

def content_slide(prs, title, subtitle="", notes=""):
    s = _new_slide(prs, "Content")
    if _theme(): return _styled_content(s, title, subtitle, notes)
    need(f'title "{title}"', text_height(title, 26, G.CW - 0.2, 1.1), 0.62, s)
    _flush(s.shapes.title.text_frame, "middle")
    p = s.shapes.title.text_frame.paragraphs[0]; p.alignment = PP_ALIGN.LEFT; r = p.add_run(); r.text = title; _font(r, 26, T.primary, True)
    if subtitle:
        need(f'subtitle of "{title}"', text_height(subtitle, 13, G.CW - 0.2, 1.2), 0.38, s)
        text(s, subtitle, G.X + 0.2, 1.0, G.CW - 0.2, 0.36, size=13, color=T.muted, valign="middle")
    _footer(s, _state["context"]["page"])
    _notes(s, notes)
    return s

def cover_slide(prs, title, subtitle="", kicker="", meta="", notes="", art=None):
    s = _new_slide(prs, "Cover")
    if _theme(): return _styled_cover(s, title, subtitle, kicker, meta, notes, art)
    background(s, T.primaryDark)
    w = 7.4 if art else 10.5
    size = 44 if lines_for(title, 44, w) <= 2 else 36
    th = min(2, lines_for(title, size, w)) * size * 1.2 / 72
    y = 1.9
    if kicker: text(s, kicker, G.X, y - 0.55, w, 0.4, size=13, color=T.accent, bold=True)
    t = s.shapes.title; t.left, t.top, t.width, t.height = Inches(G.X), Inches(y), Inches(w), Inches(th)
    _flush(t.text_frame, "top")
    p = t.text_frame.paragraphs[0]; p.alignment = PP_ALIGN.LEFT; p.line_spacing = 1.0; r = p.add_run(); r.text = title; _font(r, size, T.white, True)
    y += th + 0.25
    if subtitle:
        sh = text_height(subtitle, 20, w, 1.4); text(s, subtitle, G.X, y, w, sh, size=20, color="DCE6F0"); y += sh + 0.3
    shape(s, MSO_SHAPE.RECTANGLE, G.X, y, 0.9, 0.05, fill=T.accent)
    if meta: text(s, meta, G.X, 6.3, 10, 0.5, size=11, color="AFC0D3")
    if art: figure(s, art, 8.3, 1.0, 4.6, 5.5, alt="cover art")
    _notes(s, notes)
    return s

def section_slide(prs, title, number="", subtitle="", notes=""):
    s = _new_slide(prs, "Section")
    if _theme(): return _styled_section(s, title, number, subtitle, notes)
    background(s, T.primaryDark)
    if number: text(s, str(number), G.X, 1.6, 3, 80 * 1.4 / 72, size=80, color=T.accent, bold=True)
    _flush(s.shapes.title.text_frame, "top")
    p = s.shapes.title.text_frame.paragraphs[0]; p.alignment = PP_ALIGN.LEFT; r = p.add_run(); r.text = title; _font(r, 36, T.white, True)
    if subtitle: text(s, subtitle, G.X, 4.2, 11, 1.0, size=18, color="DCE6F0")
    _notes(s, notes)
    return s

def statement_slide(prs, statement, kicker="", attribution="", notes=""):
    """One large claim or quotation on a page of its own; attribution names who said it."""
    for value, name in ((statement, "statement"), (kicker, "statement kicker"), (attribution, "statement attribution")):
        _plain_text(value, name)
    _activate(prs)
    if _composition() == "spec":
        return _spec_page(_new_slide(prs, "Section"), "statement", statement, kicker=kicker, attribution=attribution, notes=notes)
    if _theme(): return _styled_section(_new_slide(prs, "Section"), statement, "", attribution, notes, kicker=kicker, statement=True)
    s = section_slide(prs, statement, subtitle=attribution, notes=notes)
    if kicker:
        title = s.shapes.title
        text(s, kicker, title.left / Inches(1), max(0.3, title.top / Inches(1) - 0.55), title.width / Inches(1), 0.4,
             size=14, color=T.accent, bold=True)
    return s

def closing_slide(prs, title, subtitle="", meta="", notes=""):
    """The last page: a short sign-off, with contact or next-step text in subtitle/meta."""
    for value, name in ((title, "closing title"), (subtitle, "closing subtitle"), (meta, "closing meta")):
        _plain_text(value, name)
    _activate(prs)
    if _composition() == "spec":
        return _spec_page(_new_slide(prs, "Cover"), "closing", title, subtitle=subtitle, meta=meta, notes=notes)
    if _theme(): return _styled_cover(_new_slide(prs, "Cover"), title, subtitle, "", meta, notes, None, closing=True)
    return cover_slide(prs, title, subtitle=subtitle, meta=meta, notes=notes)

def bullets(slide, items, x, y, w, h, size=13, color=None, gap=6):
    _text_items(items, "bullets")
    if not isinstance(items, list) or not items: raise ValueError("bullets require a nonempty list of strings/rich-text items")
    _using(slide)
    first = items[0] if isinstance(items[0], str) else items[0]["text"]
    need(f'bullets "{first[:12]}…"', bullets_height(items, size, w, gap), h, slide)
    box = text(slide, items, x, y, w, h, size=size, color=color, bullet=True, gap=gap); box.name = "Bullets"
    return box

def card(slide, x, y, w, h, title, body="", points=None, highlight=False, size=None):
    _plain_text(title, "card title"); _plain_text(body, "card body")
    if points is not None:
        _text_items(points, "card points")
        if not isinstance(points, list): raise ValueError("card points require a list of strings/rich-text items")
    _using(slide); size = _size("card", 13) if size is None else size
    iw = w - 0.6; head_h = text_height(title, size + 2, iw, 1.4); top = y + 0.42 + head_h + 0.12
    body_h = bullets_height(points, size, iw) if points else (text_height(body, size, iw, 1.25) if body else 0)
    total = top - y + body_h + 0.22
    need(f'card "{title}"', total, h, slide)
    # Cards given the same top and height form a row. A row whose every card is more
    # than half empty is drawn at its tallest content, so a delivered page never keeps
    # hollow containers; a row with one fuller card keeps its height for all of them.
    row = _using(slide).setdefault("cardRows", {}).setdefault((slide.slide_id, round(y, 2), round(h, 2)), {"need": 0, "boxes": []})
    row["need"] = max(row["need"], total); asked = h
    if row["need"] < asked * 0.5:
        _lint(slide, f'card "{title}": the box is {asked:.2f} in tall but its text needs {total:.2f} in; drawn at {row["need"] + 0.25:.2f} in. Use the freed space for another zone, or add content')
        h = row["need"] + 0.25
    drawn_from = len(slide.shapes)
    mode = _composition(); ink = None
    if mode == "spec":
        style = _layout_spec().get("card") or {}
        ink = _spec_block(slide, x, y, w, h, style, highlight)
    elif mode in ("editorial", "split"):
        # Open editorial blocks and a leading rule retain whitespace and hierarchy.
        shape(slide, MSO_SHAPE.RECTANGLE, x, y, 0.035, h, fill=T.accent if highlight else T.primary)
    else:
        square = mode in ("course", "blueprint")
        shape(slide, MSO_SHAPE.RECTANGLE if square else MSO_SHAPE.ROUNDED_RECTANGLE, x, y, w, h,
              fill=None if mode == "blueprint" else (T.accentTint if highlight else T.tint),
              line=T.accent if highlight else T.line, radius=None if square else 0.08)
        if mode == "legacy": shape(slide, MSO_SHAPE.RECTANGLE, x + 0.3, y + 0.26, 0.5, 0.05, fill=T.accent if highlight else T.primary)
        elif mode == "course": shape(slide, MSO_SHAPE.RECTANGLE, x, y, w, 0.06, fill=T.accent if highlight else T.primary)
    # Full-height parts of this card, so the whole row can be resized together.
    row["boxes"].extend(item for item in list(slide.shapes)[drawn_from:] if abs(item.height / Inches(1) - h) < 0.01)
    for item in row["boxes"]: item.height = Inches(h)
    text(slide, title, x + 0.3, y + 0.4, iw, head_h, size=size + 2, bold=True, color=ink or (T.accent if highlight else T.primary), role="title").name = "Card"
    ih = max(0.3, y + h - top - 0.18)
    if points: text(slide, points, x + 0.3, top, iw, ih, size=size, color=ink, bullet=True, gap=6)
    elif body: text(slide, body, x + 0.3, top, iw, ih, size=size, color=ink, spacing=1.2)

def kpi(slide, x, y, w, h, value, label, note="", highlight=False):
    _plain_text(label, "kpi label"); _plain_text(note, "kpi note")
    _using(slide); mode = _composition(); ink = None; accent_value = highlight
    if mode == "spec":
        style = _layout_spec().get("kpi") or {"style": "box"}; accent_value = highlight or style.get("valueColor") == "accent"
        look = style.get("style", "box")
        if look == "rule": shape(slide, MSO_SHAPE.RECTANGLE, x, y + h - 0.02, w, 0.02, fill=T.accent if highlight else T.line)
        elif look != "bare":
            ink = _spec_block(slide, x, y, w, h, style if look == "block" else _layout_spec().get("card") or {}, highlight)
    elif mode in ("editorial", "split"):
        shape(slide, MSO_SHAPE.RECTANGLE, x, y + h - 0.02, w, 0.02, fill=T.accent if highlight else T.line)
    else:
        square = mode in ("course", "blueprint")
        shape(slide, MSO_SHAPE.RECTANGLE if square else MSO_SHAPE.ROUNDED_RECTANGLE, x, y, w, h,
              fill=None if mode == "blueprint" else (T.accentTint if highlight else T.tint),
              line=T.accent if highlight else T.line, radius=None if square else 0.08)
    size = _size("kpi", 44)
    while size > 28 and em_width(value) * size / 72 > w - 0.5: size -= 4
    vh = size * 1.4 / 72
    text(slide, str(value), x + 0.3, y + 0.25, w - 0.6, vh, size=size, bold=True, color=ink or (T.accent if accent_value else T.primary), role="data").name = "KPI"
    text(slide, label, x + 0.3, y + 0.3 + vh, w - 0.6, 0.4, size=14, bold=True, color=ink, role="title")
    if note:
        ny = y + 0.75 + vh; need(f'kpi note "{label}"', text_height(note, 12, w - 0.6), y + h - ny - 0.15, slide)
        text(slide, note, x + 0.3, ny, w - 0.6, max(0.3, y + h - ny - 0.15), size=12, color=ink or T.muted)

def _styled_steps(slide, items, x, y, w, h, highlight, size):
    """A sequence in a hand-written composition's own card language: open ruled columns
    (editorial, split), squared blocks (course, blueprint) or bold numbered blocks (launch)."""
    mode = _composition(); ruled = mode in ("editorial", "split"); bold = mode == "launch"; n = len(items)
    gap = 0.3 if ruled else 0.2; bw = (w - gap * (n - 1)) / n; pad = 0 if ruled else 0.22; top = 0.12 if ruled else pad
    iw = bw - 2 * pad - (0.15 if ruled else 0); numeral = 20 if ruled else 16; head = 0.44 if bold else numeral * 1.3 / 72
    rows = [(text_height(it["title"], size + 1, iw, 1.4), text_height(it["body"], size, iw) if it.get("body") else 0) for it in items]
    needed = max(top + head + 0.08 + th + (0.08 + bh if bh else 0) + pad for th, bh in rows)
    need("steps", needed, h, slide)
    if needed + 0.1 < h * 0.5:
        _lint(slide, f"steps: boxes are {h:.2f} in tall but the text needs {needed:.2f} in; drawn at {needed + 0.2:.2f} in. Use the freed space, or add a body line")
        h = needed + 0.2
    for i, it in enumerate(items):
        bx = x + i * (bw + gap); hi = i == highlight; th, bh = rows[i]; ink = None; tx = bx + pad; ty = y + top
        if ruled: shape(slide, MSO_SHAPE.RECTANGLE, bx, y, bw, 0.04 if hi else 0.015, fill=T.primary if hi else T.line)
        elif bold:
            fill = T.accent if hi else T.tint; ink = _ink(fill) if hi else None
            shape(slide, MSO_SHAPE.ROUNDED_RECTANGLE, bx, y, bw, h, fill=fill, radius=0.08)
            shape(slide, MSO_SHAPE.OVAL, tx, ty, head, head, fill=ink or T.accent)
        elif mode == "course":
            shape(slide, MSO_SHAPE.RECTANGLE, bx, y, bw, h, fill=T.tint, line=T.accent if hi else T.line)
            shape(slide, MSO_SHAPE.RECTANGLE, bx, y, bw, 0.06, fill=T.accent if hi else T.primary)
        else:
            shape(slide, MSO_SHAPE.RECTANGLE, bx, y, bw, h, fill=T.tint if hi else None, line=T.line)
            shape(slide, MSO_SHAPE.RECTANGLE, bx, y, 0.06, h, fill=T.accent)
        if bold: text(slide, str(i + 1), tx, ty, head, head, size=numeral, bold=True, color=T.accent if hi else _ink(T.accent), align="center", valign="middle", role="data")
        else: text(slide, f"{i + 1:02d}", tx, ty, iw, head, size=numeral, bold=not ruled, color=T.accent if hi and mode == "course" else T.primary, role="data")
        text(slide, it["title"], tx, ty + head + 0.08, iw, th, size=size + 1, bold=True, color=ink or T.primary, role="title").name = "Step"
        if it.get("body"): text(slide, it["body"], tx, ty + head + 0.16 + th, iw, bh + 0.05, size=size, color=ink or T.muted)

def steps(slide, items, x, y, w, h, highlight=-1, size=12):
    _structured_text_items(items, "steps", ("title",))
    _using(slide)
    n = len(items); gap = 0.35; bw = (w - gap * (n - 1)) / n
    blocks = []
    for it in items:
        th = text_height(it["title"], size + 1, bw - 0.3, 1.4); bh = text_height(it["body"], size, bw - 0.3) if it.get("body") else 0
        blocks.append((th, bh, th + (0.08 + bh if it.get("body") else 0)))
    tallest = max(b[2] for b in blocks)
    style = (_layout_spec().get("steps") or {}) if _composition() == "spec" else {}
    if style.get("style") in ("numbered", "block"):
        # The composition's own sequence: large numerals, or adjoining colour blocks.
        solid = style["style"] == "block"; numeral = 30; head = numeral * 1.25 / 72
        if solid: gap = style.get("gap", 0); bw = (w - gap * (n - 1)) / n
        need("steps", head + tallest + (0.45 if solid else 0.15), h, slide)
        for i, it in enumerate(items):
            bx = x + i * (bw + gap); hi = i == highlight; th, bh, total = blocks[i]; pad = 0.2 if solid else 0
            ink = _spec_block(slide, bx, y, bw, h, style, hi) if solid else None
            text(slide, f"{i + 1:02d}", bx + pad, y + pad, bw - 2 * pad, head, size=numeral, bold=True,
                 color=ink or (T.accent if hi or style.get("numberColor", "accent") == "accent" else T.primary), role="data")
            text(slide, it["title"], bx + pad, y + pad + head + 0.05, bw - 2 * pad - 0.1, th, size=size + 1, bold=True, color=ink or T.primary, role="title").name = "Step"
            if it.get("body"): text(slide, it["body"], bx + pad, y + pad + head + 0.13 + th, bw - 2 * pad - 0.1, bh + 0.05, size=size, color=ink or T.muted)
            if not solid and style.get("arrow", True) and i < n - 1:
                arrow(slide, bx + bw + 0.04, y + head / 2, bx + bw + gap - 0.04, y + head / 2, color=T.line)
        return
    if _composition() in ("editorial", "split", "course", "blueprint", "launch"):
        return _styled_steps(slide, items, x, y, w, h, highlight, size)
    need("steps", tallest + 0.3, h, slide)
    if tallest + 0.3 < h * 0.5:
        _lint(slide, f"steps: boxes are {h:.2f} in tall but the text needs {tallest + 0.3:.2f} in; drawn at {tallest + 0.6:.2f} in. Use the freed space, or add a body line")
        h = tallest + 0.6
    for i, it in enumerate(items):
        bx = x + i * (bw + gap); hi = i == highlight; th, bh, total = blocks[i]
        shape(slide, MSO_SHAPE.ROUNDED_RECTANGLE, bx, y, bw, h, fill=T.accentTint if hi else T.surface, line=T.accent if hi else T.primary, line_w=1, radius=0.08)
        ty = y + (h - total) / 2
        text(slide, it["title"], bx + 0.15, ty, bw - 0.3, th, size=size + 1, bold=True, color=T.accent if hi else T.primary, align="center").name = "Step"
        if it.get("body"): text(slide, it["body"], bx + 0.15, ty + th + 0.08, bw - 0.3, bh + 0.05, size=size, align="center")
        if i < n - 1: arrow(slide, bx + bw + 0.04, y + h / 2, bx + bw + gap - 0.04, y + h / 2)

def timeline(slide, items, x, y, w, h, highlight=-1, size=12):
    _structured_text_items(items, "timeline", ("label", "title"))
    _using(slide)
    n = len(items); line_y = y + 0.75; step = w / n
    ln = slide.shapes.add_connector(MSO_CONNECTOR.STRAIGHT, Inches(x), Inches(line_y), Inches(x + w), Inches(line_y)); ln.line.color.rgb = rgb(T.line); ln.line.width = Pt(2); ln.name = "Timeline"
    for i, it in enumerate(items):
        cx = x + step * (i + 0.5); hi = i == highlight
        text(slide, it["label"], cx - step / 2, y, step, 0.45, size=16, bold=True, color=T.accent if hi else T.primary, align="center")
        shape(slide, MSO_SHAPE.OVAL, cx - 0.11, line_y - 0.11, 0.22, 0.22, fill=T.accent if hi else T.primary, line=T.surface, line_w=1.5)
        text(slide, it["title"], cx - step / 2 + 0.1, line_y + 0.3, step - 0.2, 0.4, size=size + 1, bold=True, align="center")
        if it.get("body"):
            need(f'timeline "{it["title"]}"', 0.75 + text_height(it["body"], size, step - 0.2), y + h - line_y, slide)
            text(slide, it["body"], cx - step / 2 + 0.1, line_y + 0.75, step - 0.2, max(0.3, y + h - line_y - 0.75), size=size, color=T.muted, align="center")

def callout(slide, y, message, x=None, w=None, h=0.62):
    _plain_text(message, "callout message")
    _using(slide)
    x = G.X if x is None else x; w = G.CW if w is None else w
    need(f'callout "{message[:12]}…"', text_height(message, 13, w - 0.5, 1.2) + 0.1, h, slide)
    shape(slide, MSO_SHAPE.RECTANGLE, x, y, w, h, fill=T.tint)
    shape(slide, MSO_SHAPE.RECTANGLE, x, y, 0.06, h, fill=T.accent)
    text(slide, message, x + 0.3, y, w - 0.5, h, size=13, bold=True, color=T.primary, valign="middle")

def table(slide, headers, rows, x, y, w, h, size=12):
    """Native editable table: one header row and 1 to 12 rows of text cells."""
    if not isinstance(headers, (list, tuple)) or not 2 <= len(headers) <= 8 or any(not isinstance(value, str) for value in headers):
        raise ValueError("table needs 2 to 8 header strings")
    if (not isinstance(rows, (list, tuple)) or not 1 <= len(rows) <= 12
            or any(not isinstance(row, (list, tuple)) or len(row) != len(headers) or any(not isinstance(value, str) for value in row) for row in rows)):
        raise ValueError("table needs 1 to 12 rows of strings, one per header")
    _diagram_bounds(slide, x, y, w, h, "table"); _using(slide)
    row_h = h / (len(rows) + 1); col_w = w / len(headers)
    tallest = max(lines_for(value, size, col_w - 0.24) for row in [headers, *rows] for value in row)
    need("table row", tallest * size * 1.3 / 72 + 0.1, row_h, slide)
    frame = slide.shapes.add_table(len(rows) + 1, len(headers), Inches(x), Inches(y), Inches(w), Inches(h)); frame.name = "Table"
    grid = frame.table; head_ink = _ink(T.primary) if _theme() else "FFFFFF"
    for r, values in enumerate([headers, *rows]):
        grid.rows[r].height = Inches(row_h)
        for c, value in enumerate(values):
            cell = grid.cell(r, c); cell.fill.solid()
            cell.fill.fore_color.rgb = rgb(T.primary if r == 0 else (T.tint if r % 2 == 0 else T.background))
            cell.margin_left = cell.margin_right = Inches(0.12); cell.margin_top = cell.margin_bottom = Inches(0.04)
            cell.vertical_anchor = MSO_ANCHOR.MIDDLE
            run = cell.text_frame.paragraphs[0].add_run(); run.text = value
            _font(run, size, head_ink if r == 0 else T.text, r == 0 or c == 0, role="title" if r == 0 else "body")
    return frame

def pyramid(slide, levels, x, y, w, h, size=14):
    """Native hierarchy of 2 to 6 stacked bars, narrowest first. A level is a label or {"title", "body"}."""
    if not isinstance(levels, (list, tuple)) or not 2 <= len(levels) <= 6:
        raise ValueError("pyramid needs 2 to 6 levels")
    items = [{"title": level} if isinstance(level, str) else level for level in levels]
    _structured_text_items(items, "pyramid", ("title",))
    _diagram_bounds(slide, x, y, w, h, "pyramid"); _using(slide)
    n = len(items); gap = 0.07; bar_h = (h - gap * (n - 1)) / n; result = []
    for index, item in enumerate(items):
        width = w * (0.4 + 0.6 * index / (n - 1)); left = x + (w - width) / 2; top = y + index * (bar_h + gap)
        fill = T.accent if index == 0 else T.tint
        bar = shape(slide, MSO_SHAPE.RECTANGLE, left, top, width, bar_h, fill=fill, line=None if index == 0 else T.line)
        bar.name = f"Pyramid level {index + 1}"; result.append(bar)
        ink = (_ink(fill) if _theme() else "FFFFFF") if index == 0 else T.text
        body = item.get("body", ""); title_w = width - 0.4 if not body else width * 0.42
        need(f'pyramid "{item["title"]}"', text_height(item["title"], size, title_w, 1.15), bar_h - 0.08, slide)
        text(slide, item["title"], left + 0.2, top, title_w, bar_h, size=size, bold=True, color=ink,
             align="left" if body else "center", valign="middle", role="title")
        if body:
            need(f'pyramid note "{item["title"]}"', text_height(body, size - 2, width * 0.5, 1.15), bar_h - 0.08, slide)
            text(slide, body, left + width * 0.46, top, width * 0.5, bar_h, size=size - 2, color=ink if index == 0 else T.muted,
                 align="right", valign="middle")
    return result

def code(slide, source, x, y, w, h, size=12):
    """A code or terminal excerpt: native monospaced text on a dark panel, at most 40 lines."""
    _plain_text(source, "code source"); lines = source.split("\n")
    if not source.strip() or len(lines) > 40: raise ValueError("code needs 1 to 40 lines of text")
    _diagram_bounds(slide, x, y, w, h, "code"); _using(slide)
    colors = (_theme() or {}).get("colors", {})
    panel = colors.get("dark", T.primaryDark); ink = colors.get("onDark", "FFFFFF")
    block = shape(slide, MSO_SHAPE.RECTANGLE, x, y, w, h, fill=panel, line=T.line); block.name = "Code panel"
    need("code lines", len(lines) * size * 1.3 / 72 + 0.3, h, slide)
    widest = max(sum(1.0 if _CJK.match(ch) else 0.6 for ch in line) for line in lines) * size / 72
    need("code line width", widest + 0.4, w, slide)
    return text(slide, lines, x + 0.2, y + 0.15, w - 0.4, h - 0.3, size=size, color=ink,
                font=T.mono, spacing=1.2)

def formula(slide, x, y, w, h, lines, size=15):
    """Legacy monospaced text panel; use equation() for a native OMML math object."""
    _using(slide)
    _lint(slide, "legacy formula: ordinary text, not typeset mathematics; use equation with a structured or bounded LaTeX expression")
    shape(slide, MSO_SHAPE.ROUNDED_RECTANGLE, x, y, w, h, fill=T.tint, line=T.line, radius=0.06)
    text(slide, list(lines), x + 0.3, y, w - 0.6, h, size=size, color=T.primary, font=T.mono, valign="middle", spacing=1.3)

_MATH_NS = "http://schemas.openxmlformats.org/officeDocument/2006/math"
_A14_NS = "http://schemas.microsoft.com/office/drawing/2010/main"
_MC_NS = "http://schemas.openxmlformats.org/markup-compatibility/2006"

class _LatexMath:
    """A bounded math subset, never a TeX interpreter or a file/network loader."""
    symbols = {"alpha": "α", "beta": "β", "gamma": "γ", "delta": "δ", "epsilon": "ε", "theta": "θ",
               "lambda": "λ", "mu": "μ", "pi": "π", "rho": "ρ", "sigma": "σ", "tau": "τ",
               "phi": "ϕ", "psi": "ψ", "omega": "ω", "Gamma": "Γ", "Delta": "Δ", "Theta": "Θ",
               "Lambda": "Λ", "Pi": "Π", "Sigma": "Σ", "Phi": "Φ", "Psi": "Ψ", "Omega": "Ω",
               "times": "×", "cdot": "⋅", "pm": "±", "infty": "∞", "leq": "≤", "geq": "≥", "neq": "≠",
               "top": "⊤", "dots": "…", "ldots": "…", "cdots": "⋯"}
    def __init__(self, source):
        if not isinstance(source, str) or not source.strip() or len(source) > 2048:
            raise ValueError("equation LaTeX must contain 1–2048 characters")
        self.source = source.strip()
        if self.source.startswith("$") and self.source.endswith("$"): self.source = self.source[1:-1]
        self.tokens = re.findall(r"\\[A-Za-z]+|\\.|[^\s]", self.source)
        if len(self.tokens) > 512: raise ValueError("equation LaTeX exceeds 512 tokens")
        self.position = 0; self.depth = 0
    def peek(self): return self.tokens[self.position] if self.position < len(self.tokens) else None
    def take(self):
        token = self.peek()
        if token is None: raise ValueError("incomplete equation LaTeX")
        self.position += 1; return token
    def argument(self):
        if self.peek() in (None, "}", "^", "_"): raise ValueError("missing equation LaTeX argument")
        return self.atom()
    def atom(self):
        self.depth += 1
        if self.depth > 24: raise ValueError("equation LaTeX nesting exceeds 24")
        try: return self._atom()
        finally: self.depth -= 1
    def _atom(self):
        token = self.take()
        if token == "{":
            value = self.sequence()
            if self.take() != "}": raise ValueError("unclosed equation LaTeX group")
            return value
        if token in ("}", "^", "_", "$", "\\"): raise ValueError("unexpected equation LaTeX token: " + token)
        if not token.startswith("\\"): return "−" if token == "-" else token
        command = token[1:]
        if command in self.symbols: return self.symbols[command]
        if command in (",", ";", "!", " "): return " "
        if command == "quad": return "\u2003"  # fixed em-space; glyph width is font-dependent
        if command in ("sin", "cos"): return {"roman": command}
        if command in ("{", "}", "_", "%"): return command
        if command == "frac": return {"fraction": [self.argument(), self.argument()]}
        if command == "sqrt":
            if self.peek() != "{": raise ValueError("equation sqrt requires a braced radicand; indexed roots are unsupported")
            return {"sqrt": self.argument()}
        if command in ("mathrm", "operatorname"):
            if self.take() != "{": raise ValueError("equation upright names require a braced literal name")
            letters = []
            while self.peek() != "}":
                letter = self.take()
                if not re.fullmatch(r"[A-Za-z0-9]", letter) or len(letters) >= 64:
                    raise ValueError("equation upright names accept only 1–64 literal ASCII letters/digits, without nested macros or scripts")
                letters.append(letter)
            self.take()
            if not letters: raise ValueError("equation upright names must not be empty")
            return {"roman": "".join(letters)}
        if command in ("hat", "widehat"): return {"hat": self.argument()}
        if command == "sum": return {"sum": ["", "", None]}
        if command in ("left", "right"):
            delimiter = self.take()
            if delimiter not in ("(", ")", "[", "]", "|", ".", r"\{", r"\}"):
                raise ValueError("unsupported equation LaTeX delimiter")
            return "" if delimiter == "." else delimiter.lstrip("\\")
        raise ValueError("unsupported equation LaTeX command: " + token)
    def scripted(self):
        base = self.atom(); scripts = {}
        while self.peek() in ("^", "_"):
            marker = self.take()
            if marker in scripts: raise ValueError("duplicate equation LaTeX script")
            first = self.peek()
            if first in ("-", "−", "+"): raise ValueError("signed equation scripts require braces, e.g. D^{-1/2}")
            scripts[marker] = self.argument()
            if first is not None and first.isdigit() and self.peek() is not None and self.peek().isdigit():
                raise ValueError("multi-digit equation scripts require braces, e.g. x^{12}")
        if isinstance(base, dict) and "sum" in base:
            if self.peek() != "{": raise ValueError("equation sum requires an explicitly braced summand: \\sum_t{...}")
            self.depth += 1
            if self.depth > 24: raise ValueError("equation LaTeX nesting exceeds 24")
            try: body = self.argument()
            finally: self.depth -= 1
            if not body["sequence"]: raise ValueError("equation sum needs a body")
            return {"sum": [scripts.get("_", ""), scripts.get("^", ""), body]}
        if "_" in scripts and "^" in scripts: return {"subsup": [base, scripts["_"], scripts["^"]]}
        if "_" in scripts: return {"subscript": [base, scripts["_"]]}
        if "^" in scripts: return {"superscript": [base, scripts["^"]]}
        return base
    def sequence(self):
        values = []
        while self.peek() not in (None, "}"):
            values.append(self.scripted())
        return {"sequence": values}
    def parse(self):
        result = self.sequence()
        if self.peek() is not None: raise ValueError("unmatched equation LaTeX closing brace")
        if not result["sequence"]: raise ValueError("empty equation LaTeX")
        return result

def _math_parts(expression, size):
    result = _math_node(expression, size)
    return result if isinstance(result, list) else [result]

def _math_node(expression, size):
    def node(name): return etree.Element(f"{{{_MATH_NS}}}{name}")
    if isinstance(expression, (str, int, float)):
        run = node("r")
        properties = etree.SubElement(run, qn("a:rPr")); properties.set("sz", str(int(size * 100)))
        fill = etree.SubElement(properties, qn("a:solidFill")); etree.SubElement(fill, qn("a:srgbClr")).set("val", T.text)
        etree.SubElement(properties, qn("a:latin")).set("typeface", "Cambria Math")
        etree.SubElement(properties, qn("a:ea")).set("typeface", T.font)
        content = etree.SubElement(run, f"{{{_MATH_NS}}}t"); content.text = str(expression)
        content.set("{http://www.w3.org/XML/1998/namespace}space", "preserve")
        return run
    if not isinstance(expression, dict) or len(expression) != 1:
        raise ValueError("equation expects text or one supported math structure")
    kind, values = next(iter(expression.items()))
    if kind == "sequence":
        if not isinstance(values, (list, tuple)) or not values: raise ValueError("equation sequence must be a nonempty list")
        return [part for value in values for part in _math_parts(value, size)]
    if kind == "roman":
        if not isinstance(values, str) or not re.fullmatch(r"[A-Za-z0-9]{1,64}", values):
            raise ValueError("equation upright name must contain 1–64 literal ASCII letters/digits")
        result = _math_node(values, size)
        properties = node("rPr"); style = node("sty"); style.set(f"{{{_MATH_NS}}}val", "p")
        properties.append(style); result.insert(0, properties)
        return result
    if kind == "sqrt":
        # OMML CT_Rad order: radPr, deg, e. A square root hides its degree.
        result = node("rad"); properties = node("radPr"); hide = node("degHide")
        hide.set(f"{{{_MATH_NS}}}val", "1"); properties.append(hide); result.append(properties)
        result.append(node("deg")); base = node("e"); base.extend(_math_parts(values, size)); result.append(base)
        return result
    if kind == "hat":
        result = node("acc"); properties = node("accPr"); mark = node("chr"); mark.set(f"{{{_MATH_NS}}}val", "\u0302")
        properties.append(mark); result.append(properties); base = node("e"); base.extend(_math_parts(values, size)); result.append(base)
        return result
    if kind == "sum":
        if not isinstance(values, (list, tuple)) or len(values) != 3: raise ValueError("equation sum expects lower, upper and body")
        result = node("nary"); properties = node("naryPr")
        for tag, value in (("chr", "∑"), ("limLoc", "undOvr"), ("subHide", "0" if values[0] != "" else "1"), ("supHide", "0" if values[1] != "" else "1")):
            prop = node(tag); prop.set(f"{{{_MATH_NS}}}val", value); properties.append(prop)
        result.append(properties)
        for tag, value in zip(("sub", "sup", "e"), values):
            child = node(tag); child.extend(_math_parts(value, size)); result.append(child)
        return result
    names = {"fraction": ("f", "num", "den"), "superscript": ("sSup", "e", "sup"), "subscript": ("sSub", "e", "sub"), "subsup": ("sSubSup", "e", "sub", "sup")}
    if kind not in names or not isinstance(values, (list, tuple)) or len(values) != len(names[kind]) - 1:
        raise ValueError("equation expects text or one supported math structure")
    result = node(names[kind][0])
    for tag, value in zip(names[kind][1:], values):
        child = node(tag); child.extend(_math_parts(value, size)); result.append(child)
    return result

def _math_linear(expression):
    if isinstance(expression, (str, int, float)): return str(expression)
    kind, values = next(iter(expression.items()))
    if kind == "sequence": return "".join(map(_math_linear, values))
    if kind == "roman": return values
    if kind == "sqrt": return "√(" + _math_linear(values) + ")"
    if kind == "hat": return _math_linear(values) + "\u0302"
    if kind == "sum":
        lower, upper, body = map(_math_linear, values)
        return f"∑[{lower};{upper}] {body}"
    if kind == "subsup":
        base, lower, upper = map(_math_linear, values)
        return f"{base}_({lower})^({upper})"
    first, second = map(_math_linear, values)
    if kind == "fraction": return f"({first}) / ({second})"
    return f"{first}{'^' if kind == 'superscript' else '_'}({second})"

def _math_unparsed(expression):
    if isinstance(expression, str): return re.search(r"\\[A-Za-z]+|[_^]", expression) is not None
    if isinstance(expression, dict): return any(_math_unparsed(value) for value in expression.values())
    if isinstance(expression, (list, tuple)): return any(map(_math_unparsed, expression))
    return False

def _math_image(source, size):
    """Render the supported expression offline using the already bundled Mathtext."""
    from matplotlib.mathtext import MathTextParser
    output = io.BytesIO(); prop = font_manager.FontProperties(size=size)
    with matplotlib.rc_context({"text.usetex": False, "mathtext.fontset": "stix", "savefig.transparent": True, "savefig.facecolor": "none"}):
        rendered = "$" + source + "$"
        bounds = MathTextParser("path").parse(rendered, dpi=72, prop=prop)
        # Mathtext's tight image can touch the canvas edge. Reserve 3pt on
        # every side so accents, numerator ascenders and antialiasing survive.
        padding = 3; width = bounds.width + 2 * padding; height = bounds.height + 2 * padding
        figure = plt.figure(figsize=(width / 72, height / 72), dpi=240)
        try:
            figure.text(padding / width, (bounds.depth + padding) / height, rendered, fontproperties=prop, color="#" + T.text)
            figure.savefig(output, dpi=240, format="png", transparent=True)
        finally: plt.close(figure)
    return output.getvalue()

def equation(slide, x, y, w, h, expression, size=22):
    """Native OMML. {'latex': source} supports a bounded offline math subset and
    supplies a rendered PNG compatibility fallback, which is not editable math.
    Plain text and the existing fraction/script structures remain compatible.
    Verify display and editing in the target Office application."""
    _using(slide)
    latex = None; png = None
    if isinstance(expression, dict) and set(expression) == {"latex"}:
        parser = _LatexMath(expression["latex"]); expression = parser.parse(); latex = parser.source
        png = _math_image(latex, size)  # validate and render before mutating the slide
    elif _math_unparsed(expression):
        _lint(slide, "equation contains unparsed math notation in literal text; use {'latex': source} or structured fractions/scripts")
    math = _math_parts(expression, size)
    box = slide.shapes.add_textbox(Inches(x), Inches(y), Inches(w), Inches(h)); box.name = "Equation (OMML)"
    tf = box.text_frame; _flush(tf, "middle")
    paragraph = tf.paragraphs[0]._p
    wrapper = etree.SubElement(paragraph, f"{{{_A14_NS}}}m", nsmap={"a14": _A14_NS, "m": _MATH_NS})
    para = etree.SubElement(wrapper, f"{{{_MATH_NS}}}oMathPara")
    etree.SubElement(para, f"{{{_MATH_NS}}}oMath").extend(math)
    # MS-ODRAWXML 2.2.5: Office 2010 math is a compatibility extension.
    # Legacy structures retain a plain-text fallback; bounded LaTeX uses a
    # typeset PNG. Neither fallback implies editable Office Math semantics.
    if png is not None:
        import struct
        pixel_width, pixel_height = struct.unpack(">II", png[16:24]); iw, ih = pixel_width / 240, pixel_height / 240
        factor = min(1, w / iw, h / ih)
        if factor < .95: _lint(slide, f"equation: typeset math needs {iw:.2f} × {ih:.2f} in at {size} pt; enlarge the box instead of shrinking the formula")
        picture = _picture(slide, png, x, y + (h - ih * factor) / 2, iw * factor, ih * factor, "Compatibility formula image; not editable math. LaTeX: " + latex)
        picture.name = "Equation fallback (rendered image, not editable math)"
        # MS-ODRAWXML Math's fallback is a p:sp with a picture fill, not p:pic.
        # https://learn.microsoft.com/en-us/openspecs/office_standards/ms-odrawxml/38b13e1f-1102-4bb7-819a-dd5d9abdb176
        fallback_shape = etree.Element(qn("p:sp"))
        nonvisual = etree.SubElement(fallback_shape, qn("p:nvSpPr"))
        nonvisual.append(_deepcopy(picture._element.nvPicPr.cNvPr))
        etree.SubElement(nonvisual, qn("p:cNvSpPr")); etree.SubElement(nonvisual, qn("p:nvPr"))
        properties = etree.SubElement(fallback_shape, qn("p:spPr"))
        properties.append(_deepcopy(picture._element.spPr.xfrm))
        properties.append(_deepcopy(picture._element.spPr.prstGeom))
        fill = _deepcopy(picture._element.blipFill); fill.tag = qn("a:blipFill"); properties.append(fill)
        etree.SubElement(etree.SubElement(properties, qn("a:ln")), qn("a:noFill"))
        picture._element.getparent().remove(picture._element)
        box._element.nvSpPr.cNvPr.set("descr", "Native OMML equation; rendered PNG compatibility fallback is not editable math. LaTeX: " + latex)
    else:
        fallback_shape = _deepcopy(box._element)
        fallback_paragraph = fallback_shape.find(f".//{qn('a:p')}")
        fallback_paragraph.remove(fallback_paragraph.find(f"{{{_A14_NS}}}m"))
        run = etree.SubElement(fallback_paragraph, qn("a:r"))
        props = etree.SubElement(run, qn("a:rPr")); props.set("sz", str(int(size * 100)))
        etree.SubElement(props, qn("a:latin")).set("typeface", "Cambria Math")
        etree.SubElement(props, qn("a:ea")).set("typeface", T.font)
        etree.SubElement(run, qn("a:t")).text = _math_linear(expression)
        fallback_shape.find(f".//{qn('p:cNvPr')}").set("name", "Equation fallback (text only)")
    tree = slide.shapes._spTree; index = list(tree).index(box._element)
    alternate = etree.Element(f"{{{_MC_NS}}}AlternateContent", nsmap={"mc": _MC_NS, "a14": _A14_NS})
    choice = etree.SubElement(alternate, f"{{{_MC_NS}}}Choice"); choice.set("Requires", "a14")
    choice.append(box._element)
    etree.SubElement(alternate, f"{{{_MC_NS}}}Fallback").append(fallback_shape)
    tree.insert(index, alternate)
    return box

def _diagram_bounds(slide, x, y, w, h, name):
    try:
        finite = all(isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) for value in (x, y, w, h))
    except (OverflowError, ValueError):
        finite = False
    prs = slide.part.package.presentation_part.presentation
    page_w = prs.slide_width / Inches(1); page_h = prs.slide_height / Inches(1)
    if not finite or x < 0 or y < 0 or w <= 0 or h <= 0 or x + w > page_w + 0.001 or y + h > page_h + 0.001:
        raise ValueError(f"{name} bounds must be finite, positive and inside the slide")

def _lesson_body_bounds(slide, x, y, w, h, name):
    regions = getattr(slide, "_ofox_regions", {})
    if "objective" not in regions: return
    main = regions["main"]
    if x < main["x"] - 0.001 or y < main["y"] - 0.001 or x + w > main["x"] + main["w"] + 0.001 or y + h > main["y"] + main["h"] + 0.001:
        _lint(slide, f'{name} body outside main region may overlap objective rail; use slide_regions(slide)["main"]')

def relation(slide, items, x, y, w, h, kind="process", size=15):
    """Editable process or comparison: native shapes, text and connectors.
    Comparison uses side-by-side columns; a process uses directional links."""
    _using(slide)
    if kind not in ("process", "comparison"): raise ValueError("relation kind must be process or comparison")
    if not isinstance(items, (list, tuple)) or not 2 <= len(items) <= 6 or any(not isinstance(item, str) for item in items):
        raise ValueError("relation needs 2 to 6 string labels; use steps/timeline for structured title/body items")
    _diagram_bounds(slide, x, y, w, h, "relation")
    gap = 0.4 if kind == "process" else 0.25; width = (w - gap * (len(items) - 1)) / len(items)
    if width <= 0 or h <= 0: raise ValueError("relation needs positive boxes")
    _lesson_body_bounds(slide, x, y, w, h, "relation")
    result = []
    for index, item in enumerate(items):
        left = x + index * (width + gap); label = item
        box = shape(slide, MSO_SHAPE.RECTANGLE, left, y, width, h, fill=T.tint, line=T.primary)
        box.name = f"Relation {kind} {index + 1}"; result.append(box)
        need(f'relation "{label}"', text_height(label, size, width - 0.3, 1.15), h - 0.2, slide)
        text(slide, label, left + 0.15, y + 0.1, width - 0.3, h - 0.2, size=size, align="center", valign="middle", role="title")
        if kind == "process" and index < len(items) - 1:
            arrow(slide, left + width + 0.04, y + h / 2, left + width + gap - 0.04, y + h / 2)
    return result

def fork_join(slide, branches, x, y, w, h, source="", merge=""):
    """Native parallel branches, optionally connected from a source and to a merge.
    Branches never link to one another. Use slide_regions(slide)['main'] on lesson pages."""
    _using(slide)
    if not isinstance(branches, (list, tuple)) or not 2 <= len(branches) <= 6:
        raise ValueError("fork_join needs 2 to 6 branch labels")
    if any(not isinstance(label, str) or not label.strip() or len(label) > 160 for label in branches):
        raise ValueError("fork_join branches must be nonempty labels of at most 160 characters")
    if any(not isinstance(label, str) or len(label) > 160 or label and not label.strip() for label in (source, merge)):
        raise ValueError("fork_join endpoint labels must be empty or short nonempty text")
    _diagram_bounds(slide, x, y, w, h, "fork_join")
    gap = 0.4; row_gap = 0.18; columns = 1 + bool(source) + bool(merge)
    width = (w - gap * (columns - 1)) / columns
    height = (h - row_gap * (len(branches) - 1)) / len(branches)
    if width < 0.65 or height < 0.4:
        raise ValueError("fork_join bounds are too small for its branch rows and endpoint columns")
    _lesson_body_bounds(slide, x, y, w, h, "fork_join")
    size = min(15, _size("body", 15))
    def node(label, left, top, name, merged=False):
        box = shape(slide, MSO_SHAPE.RECTANGLE, left, top, width, height,
                    fill=T.accentTint if merged else T.tint, line=T.accent if merged else T.primary)
        box.name = name
        need(f'fork_join "{label}"', text_height(label, size, width - 0.24, 1.15), height - 0.2, slide)
        ink = _ink(T.accentTint) if merged and _composition() == "spec" else T.primary
        text(slide, label, left + 0.12, top + 0.1, width - 0.24, height - 0.2,
             size=size, color=ink, align="center", valign="middle", role="title")
        return box
    middle_y = y + (h - height) / 2
    first = node(source, x, middle_y, "Fork join source") if source else None
    branch_x = x + (width + gap if source else 0)
    nodes = [node(label, branch_x, y + index * (height + row_gap), f"Fork join branch {index + 1}")
             for index, label in enumerate(branches)]
    last = node(merge, x + w - width, middle_y, "Fork join merge", True) if merge else None
    connections = []
    def connect(start, end, name):
        line = arrow(slide, (start.left + start.width) / Inches(1), (start.top + start.height / 2) / Inches(1),
                     end.left / Inches(1), (end.top + end.height / 2) / Inches(1))
        line.name = name
        # Rectangles have top/left/bottom/right connection sites 0/1/2/3.
        line.begin_connect(start, 3); line.end_connect(end, 1)
        connections.append(line)
    for index, branch in enumerate(nodes):
        if first is not None: connect(first, branch, f"Fork join source to branch {index + 1}")
        if last is not None: connect(branch, last, f"Fork join branch {index + 1} to merge")
    return {"source": first, "branches": nodes, "merge": last, "connectors": connections}

def native_chart(slide, kind, categories, series, x, y, w, h, title=""):
    """Native chart with an embedded workbook; data must be supplied.
    Supports vertical bar, line and one-series pie, at most 100 values."""
    _using(slide)
    types = {"bar": XL_CHART_TYPE.COLUMN_CLUSTERED, "line": XL_CHART_TYPE.LINE_MARKERS, "pie": XL_CHART_TYPE.PIE}
    if kind not in types or not isinstance(categories, (list, tuple)) or not 1 <= len(categories) <= 100:
        raise ValueError("native_chart requires bar, line or pie and 1 to 100 categories")
    if any(not isinstance(label, str) or not label.strip() or len(label) > 200 for label in categories):
        raise ValueError("native_chart categories must be supplied nonempty text")
    if not isinstance(series, (list, tuple)) or not 1 <= len(series) <= 3 or len(series) * len(categories) > 100:
        raise ValueError("native_chart requires 1 to 3 series and at most 100 values")
    if not isinstance(title, str) or len(title) > 200:
        raise ValueError("native_chart title must be short text")
    if not all(isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) for value in (x, y, w, h)) or w <= 0 or h <= 0:
        raise ValueError("native_chart requires a finite positive box")
    cd = CategoryChartData(); cd.categories = list(categories)
    for item in series:
        if not isinstance(item, dict) or set(item) != {"name", "values"} or not isinstance(item["name"], str) or not item["name"].strip() or len(item["name"]) > 100:
            raise ValueError("native_chart series require a supplied name and values")
        values = item["values"]
        if not isinstance(values, (list, tuple)) or len(values) != len(categories):
            raise ValueError("native_chart values must match the categories")
        try:
            finite = all(isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) for value in values)
        except (OverflowError, ValueError):
            finite = False
        if not finite:
            raise ValueError("native_chart values must be supplied finite numbers")
        if kind == "pie" and (len(series) != 1 or len(categories) < 2 or any(value < 0 for value in values) or sum(values) <= 0):
            raise ValueError("native_chart pie requires one positive nonnegative series")
        cd.add_series(item["name"], list(values))
    chart = slide.shapes.add_chart(types[kind], Inches(x), Inches(y), Inches(w), Inches(h), cd).chart
    chart.has_title = bool(title)
    if title:
        chart.chart_title.text_frame.text = title
        for paragraph in chart.chart_title.text_frame.paragraphs:
            for run in paragraph.runs:
                _font(run, 14, T.text, True, role="title")
    chart.has_legend = len(series) > 1 or kind == "pie"
    if chart.has_legend:
        chart.legend.position = XL_LEGEND_POSITION.BOTTOM
        chart.legend.font.name = T.font; chart.legend.font.size = Pt(11)
    colors = (_theme() or {}).get("colors", {}).get("chart", [T.primary, T.accent, T.muted])
    if _composition() == "spec" or _dark(slide):
        # Series, labels and grid take the page canvas's own colours, so a chart
        # stays legible on a dark or saturated page in every application.
        colors = (None if _dark(slide) else _slide_canvas(slide).get("chart")) or [color for color in colors if abs(_luminance(color) - _luminance(T.background)) > 0.18] or [T.primary]
        if chart.has_legend: chart.legend.font.color.rgb = rgb(T.text)
        if kind != "pie":
            for axis in (chart.category_axis, chart.value_axis):
                axis.tick_labels.font.color.rgb = rgb(T.text)
                axis.format.line.color.rgb = rgb(T.line)
            chart.value_axis.major_gridlines.format.line.color.rgb = rgb(T.line)
    for index, native_series in enumerate(chart.series):
        if kind == "line":
            native_series.format.line.color.rgb = rgb(colors[index % len(colors)])
        elif kind == "pie":
            for point_index, point in enumerate(native_series.points):
                point.format.fill.solid(); point.format.fill.fore_color.rgb = rgb(colors[point_index % len(colors)])
        else:
            native_series.format.fill.solid(); native_series.format.fill.fore_color.rgb = rgb(colors[index % len(colors)])
    if kind != "pie":
        for axis in (chart.category_axis, chart.value_axis):
            axis.tick_labels.font.name = T.font; axis.tick_labels.font.size = Pt(11)
    return chart

def _picture(slide, png, x, y, w, h, alt):
    pic = slide.shapes.add_picture(io.BytesIO(png), Inches(x), Inches(y), Inches(w), Inches(h))
    pic._element.nvPicPr.cNvPr.set("descr", alt)
    return pic

def _text_extent(item):
    """The estimated area a text shape's words occupy inside its box (inches)."""
    left, top, width, height = (value / Inches(1) for value in (item.left, item.top, item.width, item.height))
    paragraphs = [p for p in item.text_frame.paragraphs if p.text.strip()]
    sizes = [run.font.size.pt for p in paragraphs for run in p.runs if run.font.size]
    if not paragraphs or not sizes: return left, top, width, height
    size = max(sizes); used = min(width, max(em_width(p.text) for p in paragraphs) * size / 72 + 0.1)
    tall = min(height, sum(lines_for(p.text, size, width) for p in paragraphs) * size * 1.2 / 72)
    align = paragraphs[0].alignment
    x = left + (width - used if align == PP_ALIGN.RIGHT else (width - used) / 2 if align == PP_ALIGN.CENTER else 0)
    anchor = item.text_frame.vertical_anchor
    y = top + (height - tall if anchor == MSO_ANCHOR.BOTTOM else (height - tall) / 2 if anchor == MSO_ANCHOR.MIDDLE else 0)
    return x, y, used, tall

def image(slide, asset_id, x, y, w, h, fit="contain", alt="", behind=False, veil=None):
    """Embed one Host-authorized image ID as a native picture, not a path.
    The image is raster artwork; it is not editable chart/diagram/math data.
    It may not cover text already on the page; behind=True places it under the
    page's text as a deliberate background, with the page's colour laid over it
    (veil, 0-0.9 opaque, 0.6 by default) so the text stays readable."""
    _using(slide)
    if not isinstance(asset_id, str) or asset_id not in _MEDIA:
        raise ValueError("unknown presentation assetId")
    page = _number(slide)
    if page not in _MEDIA_PLAN_PAGES or _MEDIA_PAGES[_MEDIA_PLAN_PAGES.index(page)] != asset_id:
        raise ValueError("presentation assetId does not match this page")
    if fit not in ("contain", "cover") or not all(isinstance(v, (int, float)) and math.isfinite(v) for v in (x, y, w, h)) or w <= 0 or h <= 0:
        raise ValueError("image requires a finite positive box and contain or cover fit")
    veil = 0.6 if veil is None else veil
    if isinstance(veil, bool) or not isinstance(veil, (int, float)) or not 0 <= veil <= 0.9:
        raise ValueError("veil is how opaque the page colour lies over a background image, from 0 to 0.9")
    from PIL import Image as _AssetImage
    data = base64.b64decode(_MEDIA[asset_id]["dataBase64"], validate=True)
    with _AssetImage.open(io.BytesIO(data)) as source:
        iw, ih = source.size
    if not behind:
        # The area the picture really covers: a contained picture is centred in its box.
        factor = min(w / iw, h / ih); dw, dh = (iw * factor, ih * factor) if fit == "contain" else (w, h)
        dx, dy = x + (w - dw) / 2, y + (h - dh) / 2
        for item in slide.shapes:
            if not item.has_text_frame or not item.text.strip(): continue
            # The page footer and page number are not content a picture must avoid.
            if item.is_placeholder and item.placeholder_format.type in (PP_PLACEHOLDER.FOOTER, PP_PLACEHOLDER.SLIDE_NUMBER, PP_PLACEHOLDER.DATE):
                continue
            tx, ty, tw, th = _text_extent(item)
            if min(dx + dw, tx + tw) - max(dx, tx) > 0.05 and min(dy + dh, ty + th) - max(dy, ty) > 0.05:
                raise ValueError(f'image covers existing text "{item.text.strip()[:40]}"; choose a region clear of the title '
                                 "and text, or pass behind=True for a deliberate full background")
    if fit == "contain":
        factor = min(w / iw, h / ih)
        actual_w, actual_h = iw * factor, ih * factor
        pic = _picture(slide, data, x + (w - actual_w) / 2, y + (h - actual_h) / 2, actual_w, actual_h, alt)
    else:
        pic = _picture(slide, data, x, y, w, h, alt)
        if iw / ih > w / h:
            pic.crop_left = pic.crop_right = (1 - (w / h) / (iw / ih)) / 2
        else:
            pic.crop_top = pic.crop_bottom = (1 - (iw / ih) / (w / h)) / 2
    pic.name = "Asset " + asset_id
    if behind:
        tree = slide.shapes._spTree; tree.remove(pic._element)
        back = next((item._element for item in slide.shapes if item.name == "Background"), None)
        if back is not None: back.addnext(pic._element)
        else: tree.insert(2, pic._element)
        if veil:
            cover = shape(slide, MSO_SHAPE.RECTANGLE, x, y, w, h, fill=_page_color(slide))
            cover.name = "Image veil"
            etree.SubElement(cover._element.xpath("./p:spPr/a:solidFill/a:srgbClr")[0], qn("a:alpha")).set("val", str(round(veil * 100000)))
            tree.remove(cover._element); pic._element.addnext(cover._element)
        else:
            _lint(slide, "background image under the page's text: check text contrast in the preview, or add a plain panel behind the text")
    _MEDIA_USED[id(slide)] = (asset_id, pic)
    return pic

def _validate_media(prs):
    """Check normal helper output against the Host's frozen page bindings."""
    if not _MEDIA_PLAN_ACTIVE: return
    expected_count = _SOURCE_META["totalPages"] if _SOURCE_META else len(_MEDIA_PAGES)
    if len(prs.slides) != expected_count:
        raise RuntimeError("presentation page count does not match the frozen plan")
    for index, page in enumerate(_MEDIA_PLAN_PAGES):
        slide = prs.slides[page - 1]
        expected = _MEDIA_PAGES[index]
        if expected is None: continue
        used = _MEDIA_USED.get(id(slide))
        if used and used[0] == expected and used[1]._element.getparent() is slide.shapes._spTree:
            continue
        fallback = _MEDIA_FALLBACKS[index]
        if fallback == "stop":
            raise RuntimeError("required presentation image is missing from its page")
        if expected in _MEDIA:
            _lint(slide, f'the plan\'s image "{expected}" is ready but was not drawn: add image(slide, "{expected}", x, y, w, h) on this page '
                         f'(behind=True with fit="cover" for a full-page background); {fallback} fallback used')
        else:
            _lint(slide, "planned image unavailable; " + fallback + " fallback used")

def _validate_density(prs):
    """Advisory: a content page whose content stops in the upper half reads as unfinished.
    Decoration, the title block, footers and full-height panels are not content."""
    if _SOURCE_META: return
    for slide in prs.slides:
        if slide.slide_layout.name != "Content": continue
        bottoms = []
        for item in slide.shapes:
            if item.is_placeholder or item.name == "Background" or item.name.startswith(("Decor", "Alternating panel", "Panel navigation")): continue
            top, height = item.top / Inches(1), item.height / Inches(1)
            if height > 6: continue
            bottoms.append(top + height)
        if bottoms and max(bottoms) < 4.4:
            _lint(slide, f"content ends {max(bottoms):.1f} in down a 7.5 in page; the lower part is empty. Enlarge the type and blocks to fill the body region, or add a supporting zone")

def _validate_panel_regions(prs):
    """Refuse, not just report: on a page with the architectural side panel, the deck's own content
    stays in its regions, the paper beside the panel or the panel below its title. Content across
    the panel edge or over its title was delivered again and again under a warning (2026-11-13)."""
    problems = []
    for index, slide in enumerate(prs.slides, 1):
        frame = getattr(slide, "_ofox_panel_frame", None)
        if not frame: continue
        left, right, head = frame["left"], frame["right"], frame["head"]
        found = []
        for item in slide.shapes:
            if item.shape_id in frame["ids"] or item.is_placeholder or item.name == "Background": continue
            x, y, w, h = (value / Inches(1) for value in (item.left, item.top, item.width, item.height))
            if w * h > G.W * G.H * 0.6: continue  # a deliberate full-page background
            if min(x + w, right) - max(x, left) <= 0.05: continue  # on the paper beside the panel
            if x >= left - 0.05 and x + w <= right + 0.05:
                if y >= head - 0.05: continue  # in the panel, below its title
                reason = "covers the side panel's title"
            else:
                reason = "crosses the side panel's edge"
            words = item.has_text_frame and item.text.strip()
            label = item.text.strip().split("\n")[0][:20] if words else item.name
            # Text names the content best: it goes first, before unnamed frames and lines.
            found.append((not words, f'"{label}" (x {x:.2f}–{x + w:.2f}, y {y:.2f}) {reason}'))
        found = [entry for _, entry in sorted(found, key=lambda pair: pair[0])]
        if found:
            regions = slide_regions(slide); main = regions["main"]; panel = regions["panel"]
            more = f" and {len(found) - 2} more" if len(found) > 2 else ""
            problems.append(f'slide {index}: ' + "; ".join(found[:2]) + more +
                            f' (main x {main["x"]:.2f}–{main["x"] + main["w"]:.2f}, y {main["y"]:.2f}–{main["y"] + main["h"]:.2f};'
                            f' panel x {panel["x"]:.2f}–{panel["x"] + panel["w"]:.2f} from y {panel["y"]:.2f})')
    if problems:
        raise RuntimeError("on the architectural style a page's content goes in slide_regions(s): \"main\" beside the side panel, "
                           "or \"panel\" below its title; explicit coordinates are not moved for you.\n" + "\n".join(problems))

def _validate_title_cover(prs):
    """Advisory: a box or text drawn over a page's title hides it in some applications.
    Background, decoration, panels and the title's own rule are drawn behind on purpose."""
    page_area = prs.slide_width * prs.slide_height
    for slide in prs.slides:
        title = slide.shapes.title
        if title is None or not title.has_text_frame or not title.text.strip(): continue
        tx, ty, tw, th = _text_extent(title)
        order = list(slide.shapes)
        for item in order[order.index(title) + 1:]:
            if item.is_placeholder or item.name == "Background" or item.name.startswith(("Decor", "Alternating panel", "Panel navigation", "Asset ")):
                continue
            if item.width * item.height > page_area * 0.6: continue
            x, y, w, h = (value / Inches(1) for value in (item.left, item.top, item.width, item.height))
            if min(x + w, tx + tw) - max(x, tx) > 0.1 and min(y + h, ty + th) - max(y, ty) > 0.1:
                _lint(slide, f'a shape covers the title "{title.text.strip()[:30]}"; move the content below or beside the title region')
                break

def _plan_receipt(prs):
    """One compact line saying what each page was built with, for the runtime to
    compare with the frozen page plan. Written only for a planned deck."""
    if not _MEDIA_PLAN_ACTIVE or _SOURCE_META: return
    names = (("Code panel", "cc"), ("Pyramid level", "py"), ("Fork join", "fj"), ("Relation ", "rl"),
             ("Lesson objective region", "ls"), ("Asset ", "im"), ("Step", "st"), ("Timeline", "tl"), ("Bullets", "bu"),
             ("Figure", "fg"), ("Plot", "pl"))
    pages = []
    for number, slide in enumerate(prs.slides, 1):
        tags = []; cards = kpis = 0
        layout = slide.slide_layout.name
        if layout in ("Cover", "Section"): tags.append(layout[0])
        for item in slide.shapes:
            if item.has_chart:
                kind = str(item.chart.chart_type)
                tags.append("cp" if "PIE" in kind else "cl" if "LINE" in kind else "cb")
            elif getattr(item, "has_table", False) and item.has_table: tags.append("tb")
            elif item.name == "Card": cards += 1
            elif item.name == "KPI": kpis += 1
            else: tags.extend(code for prefix, code in names if item.name.startswith(prefix))
        if b"oMath" in etree.tostring(slide.shapes._spTree): tags.append("eq")
        if cards: tags.append(f"k{cards}")
        if kpis: tags.append(f"n{kpis}")
        pages.append(f"{number}=" + ",".join(dict.fromkeys(tags)))
    LINT.insert(0, ("plan-receipt:" + ";".join(pages))[:480])

def _validate_layout(prs):
    """Advisory axis-aligned, top-level chart/text frame checks, not visual certification.
    Groups, rotated objects, chart internals and actual glyph bounds are outside this check.
    Source edits inspect authorized pages only; nothing is repositioned or rewritten."""
    pages = ((page, prs.slides[page - 1]) for page in _SOURCE_META["pageNumbers"]) if _SOURCE_META else enumerate(prs.slides, 1)
    findings = 0; pairs = 0
    def report(slide, message):
        nonlocal findings
        _lint(slide, message); findings += 1
        if findings >= 40:
            _lint(slide, "chart/text layout check reached its findings limit; review remaining frames")
            return False
        return True
    for page, slide in pages:
        if page > 1000:
            report(slide, "chart/text layout check reached its page limit; review remaining pages")
            return
        charts = []; texts = []
        for index, item in enumerate(slide.shapes):
            if index >= 500:
                if not report(slide, "chart/text layout check reached its object limit; review remaining objects"): return
                break
            props = item._element.find(".//" + qn("p:cNvPr"))
            if props is not None and props.get("hidden") in ("1", "true"): continue
            if item.rotation % 360: continue
            if item.has_chart: charts.append(item)
            elif item.has_text_frame and item.text.strip(): texts.append(item)
        for chart in charts:
            for label in texts:
                if pairs >= 10000:
                    report(slide, "chart/text layout check reached its pair limit; review remaining frames")
                    return
                pairs += 1
                width = min(chart.left + chart.width, label.left + label.width) - max(chart.left, label.left)
                height = min(chart.top + chart.height, label.top + label.height) - max(chart.top, label.top)
                # Ignore shared edges and sub-0.01in coordinate rounding.
                if width <= Inches(0.01) or height <= Inches(0.01): continue
                if not report(slide, f"native chart/text overlap: chart shape {chart.shape_id} and text shape {label.shape_id} intersect {width / Inches(1):.2f} x {height / Inches(1):.2f} in; separate external text from the chart frame"):
                    return

def figure(slide, svg, x, y, w, h, alt="figure"):
    """An SVG figure, rasterised by the host at 2x."""
    svg = re.sub(r"((?:fill|stroke|stop-color|color)\s*=\s*[\"'])([0-9A-Fa-f]{6})([\"'])", r"\1#\2\3", str(svg))
    m = re.search(r"viewBox\s*=\s*[\"']\s*[-\d.]+[\s,]+[-\d.]+[\s,]+([\d.]+)[\s,]+([\d.]+)", svg)
    if m:
        vw, vh = float(m.group(1)), float(m.group(2)); pt_per_px = w * 72 / vw
        if abs(vw / vh - w / h) / (w / h) > 0.04: _lint(slide, f'figure "{alt}": viewBox {vw:g}x{vh:g} does not match the box {w}x{h} in; it will be stretched')
        sizes = [float(v) for v in re.findall(r"font-size\s*=\s*[\"']([\d.]+)", svg)]
        if sizes and min(sizes) * pt_per_px < 10: _lint(slide, f'figure "{alt}": smallest text is {min(sizes) * pt_per_px:.1f} pt on the slide; use at least 10 pt')
    png = base64.b64decode(host.svg_to_png(svg, round(w * 192), round(h * 192)))
    pic = _picture(slide, png, x, y, w, h, alt)
    pic.name = "Figure"
    return pic

# The lint line for each group of plot text: how it reads, and how to fix it.
_PLOT_TEXT = {
    "title": ("title is", "ax.set_title(text, fontsize=12)"),
    "axis labels": ("axis labels are", "ax.set_xlabel(text, fontsize=10)"),
    "tick labels": ("tick labels are", "ax.tick_params(labelsize=10)"),
}

def _plot_text(fig):
    """The text a plot draws, by group. Tick labels are those inside the axis view limits: the others are never drawn."""
    groups = {name: [] for name in _PLOT_TEXT}
    for ax in fig.axes:
        groups["title"].append(ax.title)
        groups["axis labels"] += [ax.xaxis.label, ax.yaxis.label]
        for axis in (ax.xaxis, ax.yaxis):
            axis.get_majorticklabels()  # refreshes the labels' text
            lo, hi = sorted(axis.get_view_interval()); eps = (hi - lo) * 1e-9
            groups["tick labels"] += [tk.label1 for tk in axis.get_major_ticks() if lo - eps <= tk.get_loc() <= hi + eps]
    return groups

def _page_color(slide):
    """The colour a picture sits on: the slide's own solid fill, else the page colour."""
    found = slide._element.xpath("./p:cSld/p:bg//a:srgbClr/@val")
    return str(found[0]).upper() if found else T.background.upper()

def _theme_plot(fig, page):
    """matplotlib draws black on white; on a coloured or dark page that reads as a pasted
    white box. Only the frame still at those defaults takes the page's colours: the
    background, spines, ticks, titles, axis labels and legends. Text inside the data
    area, such as numbers in heatmap cells, keeps its colour."""
    from matplotlib.colors import same_color, to_rgb
    is_ = lambda color, name: same_color(to_rgb(color), name)
    paper = "#" + page
    # The default style has no theme colours to choose from.
    ink = "#" + (_ink(page) if _theme() else ("1A1A1A" if _luminance(page) > 0.55 else "FFFFFF"))
    if is_(fig.get_facecolor(), "white"): fig.set_facecolor(paper)
    frame_text = [t for t in (fig._suptitle, fig._supxlabel, fig._supylabel) if t is not None]
    for ax in fig.axes:
        if is_(ax.get_facecolor(), "white"): ax.set_facecolor(paper)
        for spine in ax.spines.values():
            if is_(spine.get_edgecolor(), "black"): spine.set_edgecolor(ink)
        for axis in (ax.xaxis, ax.yaxis):
            ticks = axis.get_major_ticks()
            if ticks and is_(ticks[0].tick1line.get_color(), "black"): axis.set_tick_params(which="both", color=ink)
            if ticks and is_(ticks[0].label1.get_color(), "black"): axis.set_tick_params(which="both", labelcolor=ink)
        frame_text += [ax.title, ax._left_title, ax._right_title, ax.xaxis.label, ax.yaxis.label]
        legend = ax.get_legend()
        if legend is not None:
            if is_(legend.get_frame().get_facecolor(), "white"): legend.get_frame().set_facecolor(paper)
            frame_text += [legend.get_title(), *legend.get_texts()]
    for item in frame_text:
        if is_(item.get_color(), "black"): item.set_color(ink)

def plot(slide, fig, x, y, w, h, alt="chart"):
    """A matplotlib figure placed at x, y, w, h (inches); it is resized to the box."""
    fig.set_size_inches(w, h)
    page = _page_color(slide)
    if page != "FFFFFF": _theme_plot(fig, page)
    for group, texts in _plot_text(fig).items():
        sizes = [t.get_fontsize() for t in texts if t.get_text() and t.get_visible() and t.get_fontsize() < 10]
        if sizes:
            what, fix = _PLOT_TEXT[group]; small, large = min(sizes), max(sizes)
            size = f"{small:g}" if small == large else f"{small:g}-{large:g}"
            _lint(slide, f'plot "{alt}": {what} {size} pt; use at least 10 ({fix})')
    buf = io.BytesIO(); fig.savefig(buf, format="png", dpi=200, facecolor=fig.get_facecolor()); plt.close(fig)
    pic = _picture(slide, buf.getvalue(), x, y, w, h, alt)
    pic.name = "Plot"
    return pic
