"""Trusted offline regression, including the failed CLIP tool inputs.

Run after the production deck helpers, inside the application Python engine.
No model, account, external files, network, or ordinary Python fallback.
"""

CLIP_DIRECTIONAL = r"\mathcal{L} = -\sum_{i}{\log\left(\frac{\exp(\operatorname{sim}(z_i,\hat{z}_i)/\tau)}{\sum_{j}{\exp(\operatorname{sim}(z_i,z_j)/\tau)}}\right)}"
CLIP_LOSS = r"\mathcal{L}=-\frac{1}{2N}\sum_{i=1}^{N}{\left(\log\frac{\exp(s_{ii}/\tau)}{\sum_{j=1}^{N}{\exp(s_{ij}/\tau)}}+\log\frac{\exp(s_{ii}/\tau)}{\sum_{j=1}^{N}{\exp(s_{ji}/\tau)}}\right)}"

_clip_ns = {"m": "http://schemas.openxmlformats.org/officeDocument/2006/math", "a": "http://schemas.openxmlformats.org/drawingml/2006/main", "p": "http://schemas.openxmlformats.org/presentationml/2006/main"}
_clip_failures = []


def clip_check(name, check):
    try:
        check()
    except (ValueError, AssertionError) as error:
        _clip_failures.append(name + ": " + str(error))


def clip_xml(slide):
    return etree.fromstring(etree.tostring(slide._element))


prs = new_deck(title="CLIP native contract", language="zh")
sequential = content_slide(prs, "训练流程：顺序连接")


def clip_sequence():
    boxes = relation(sequential, ["图文输入", "双编码器", "对比损失"], .8, 2, 11.6, 1.3, kind="sequence")
    assert [box.name for box in boxes] == ["Relation process 1", "Relation process 2", "Relation process 3"]
    connectors = [shape for shape in sequential.shapes if shape._element.tag == qn("p:cxnSp")]
    assert len(connectors) == 2
    for connector in connectors:
        assert connector.begin_x < connector.end_x, "a sequence must point from input to output"
        line = connector._element.find(".//" + qn("a:ln"))
        assert line.find(qn("a:tailEnd")).get("type") == "triangle"
        assert line.find(qn("a:headEnd")) is None


clip_check("sequence alias and arrow direction", clip_sequence)
comparison = content_slide(prs, "并列比较：没有顺序箭头")
relation(comparison, ["图像编码器", "文本编码器"], .8, 2, 11.6, 1.3, kind="comparison")
assert not clip_xml(comparison).findall(".//p:cxnSp", _clip_ns)
before = etree.tostring(comparison._element)
try:
    relation(comparison, ["图像", "文本"], .8, 4, 11.6, 1.2, kind="spatial")
except ValueError as error:
    if "comparison" not in str(error) or "fork_join()" not in str(error):
        _clip_failures.append("spatial must explain comparison or fork_join()")
else:
    _clip_failures.append("spatial must not silently become a sequential graph")
assert etree.tostring(comparison._element) == before

parallel = content_slide(prs, "双编码器：分叉与汇合")
graph = fork_join(parallel, ["图像编码器", "文本编码器"], .8, 2, 11.6, 3.8, source="图文对", merge="相似度")
assert len(graph["connectors"]) == 4
expected = {(graph["source"].shape_id, branch.shape_id) for branch in graph["branches"]}
expected |= {(branch.shape_id, graph["merge"].shape_id) for branch in graph["branches"]}
actual = set()
for line in graph["connectors"]:
    start = line._element.find(".//" + qn("a:stCxn"))
    end = line._element.find(".//" + qn("a:endCxn"))
    actual.add((int(start.get("id")), int(end.get("id"))))
assert actual == expected, "parallel branches must not point to one another"

directional = content_slide(prs, "实际失败输入：完整显式求和")
clip_check("recorded braced CLIP loss", lambda: equation(directional, .8, 2, 11.6, 2.4, {"latex": CLIP_DIRECTIONAL}, size=18))
symmetric = content_slide(prs, "对称 CLIP 损失：可编辑原生公式")


def clip_loss():
    equation(symmetric, .75, 2, 11.8, 2.4, {"latex": CLIP_LOSS}, size=16)
    xml = clip_xml(symmetric)
    maths = xml.findall(".//m:oMath", _clip_ns)
    assert len(maths) == 1
    sums = maths[0].findall(".//m:nary", _clip_ns)
    assert len(sums) == 3, "one outer sum and both directional denominators"
    outer = sums[0]
    assert "".join(outer.find("m:sub", _clip_ns).itertext()) == "i=1"
    assert "".join(outer.find("m:sup", _clip_ns).itertext()) == "N"
    assert len(outer.find("m:e", _clip_ns).findall(".//m:nary", _clip_ns)) == 2
    denominators = maths[0].findall(".//m:den/m:nary", _clip_ns)
    assert len(denominators) == 2
    for inner in denominators:
        assert "".join(inner.find("m:sub", _clip_ns).itertext()) == "j=1"
        body = inner.find("m:e", _clip_ns)
        assert "log" not in "".join(body.itertext()), "the logarithm belongs outside the inner sum"
        assert "exp" in "".join(body.itertext())
        assert not body.findall(".//m:nary", _clip_ns)
    runs = maths[0].findall(".//m:r", _clip_ns)
    functions = [run for run in runs if "".join(run.findall("m:t", _clip_ns)[0].itertext()) in ("log", "exp")]
    assert len(functions) == 6
    for run in functions:
        assert run.find("m:rPr/m:sty", _clip_ns).get("{" + _clip_ns["m"] + "}val") == "p"
    letters = [run for run in runs if run.find("m:rPr/m:scr", _clip_ns) is not None]
    assert len(letters) == 1 and letters[0].find("m:t", _clip_ns).text == "L"
    assert letters[0].find("m:rPr/m:scr", _clip_ns).get("{" + _clip_ns["m"] + "}val") == "script"
    assert not any("\\" in (node.text or "") for node in maths[0].findall(".//m:t", _clip_ns))
    images = [symmetric.part.related_part(blip.get(qn("r:embed"))).blob for blip in symmetric._element.iter(qn("a:blip")) if blip.get(qn("r:embed"))]
    assert images, "native OMML must retain a rendered compatibility preview"
    from PIL import Image
    for blob in images:
        image = Image.open(io.BytesIO(blob)).convert("RGBA")
        assert image.getchannel("A").getbbox() is not None


clip_check("symmetric loss XML and rendered fallback", clip_loss)
operators = content_slide(prs, "正体函数与花体损失符号")
for index, expression in enumerate([r"\log(x)", r"\exp(x)", r"\mathcal{L}"]):
    clip_check(expression, lambda expression=expression, index=index: equation(operators, 1, 2 + index, 10, .8, {"latex": expression}))

rejections = content_slide(prs, "范围与拒绝：不猜求和边界")
for expression in [
    r"\sum_i\log(x)", r"\sum_i{\log\frac{x}{\sum_j x}}", r"\sum_i{}",
    r"\mathcal{}", r"\mathcal{\input{secret}}", r"\mathcal{L_i}",
    r"\logger(x)", r"\expandafter x", r"\nobreak x", r"\input{secret}",
]:
    before = etree.tostring(rejections._element)
    try:
        equation(rejections, 1, 2, 10, 2, {"latex": expression})
    except ValueError:
        assert etree.tostring(rejections._element) == before
    else:
        _clip_failures.append("unsupported or ambiguous math accepted: " + expression)
text(rejections, "内外求和分别使用显式花括号；不支持的宏保持拒绝。", 1, 2, 11, 1, size=18)
last = content_slide(prs, "验收范围")
text(last, "原生关系图、完整 CLIP 损失与逐页预览；PowerPoint 保存重开另验。", 1, 2, 11, 1.4, size=18)
assert len(prs.slides) == 8
assert not _clip_failures, "CLIP contract regression: " + "; ".join(_clip_failures)
