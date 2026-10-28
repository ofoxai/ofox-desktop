"""Regenerate synthetic Office fixtures; production never needs these Python libraries."""
from pathlib import Path
from datetime import datetime
from io import BytesIO
from zipfile import ZipFile, ZIP_DEFLATED
from docx import Document
from openpyxl import Workbook
from pptx import Presentation
from pptx.util import Inches

root = Path(__file__).parent
stamp = datetime(2000, 1, 1)
doc = Document()
doc.core_properties.author = "Ofox test fixture"
doc.core_properties.created = doc.core_properties.modified = stamp
doc.add_heading("星河项目季度报告", 0)
doc.add_paragraph("中文正文：本季度完成验收。日本語の確認。")
table = doc.add_table(rows=2, cols=2)
for cell, value in zip(table.rows[0].cells, ["城市", "数量"]): cell.text = value
for cell, value in zip(table.rows[1].cells, ["上海", "42"]): cell.text = value
doc.add_paragraph("正文结束标记：星河归档。")
doc.save(root / "chinese.docx")

book = Workbook()
book.properties.creator = "Ofox test fixture"
book.properties.created = book.properties.modified = stamp
sheet = book.active
sheet.title = "中文收入"
sheet.append(["城市", "金额"])
sheet.append(["上海", 12345.67])
sheet["D4"] = "=SUM(B2:B3)"
book.create_sheet("日文成本")["B3"] = "東京支出"
buf = BytesIO(); book.save(buf)
with ZipFile(buf) as source, ZipFile(root / "chinese.xlsx", "w", ZIP_DEFLATED) as dest:
    for item in source.infolist():
        data = source.read(item.filename)
        if item.filename == "xl/worksheets/sheet1.xml":
            data = data.replace(b"<f>SUM(B2:B3)</f><v></v>", b"<f>SUM(B2:B3)</f><v>12345.67</v>")
        dest.writestr(item, data)

slides = Presentation()
slides.core_properties.author = "Ofox test fixture"
slides.core_properties.created = slides.core_properties.modified = stamp
for title, body in [("中文第一页", "本月目标：提升质量"), ("中文第二页", "验收结论：全部通过")]:
    slide = slides.slides.add_slide(slides.slide_layouts[1])
    slide.shapes.title.text = title
    slide.placeholders[1].text = body
    slide.notes_slide.notes_text_frame.text = "讲者备注：现场演示"
table = slides.slides[1].shapes.add_table(2, 2, Inches(1), Inches(4), Inches(6), Inches(1)).table
for row, values in enumerate([["城市", "数量"], ["深圳", "88"]]):
    for col, value in enumerate(values): table.cell(row, col).text = value
slides.save(root / "chinese.pptx")
