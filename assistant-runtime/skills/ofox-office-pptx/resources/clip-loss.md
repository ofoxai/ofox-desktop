# CLIP diagrams and loss

Planning uses `relationship="sequence"` for an ordered workflow; `process` is
its model-input alias. Python `relation` accepts `kind="sequence"` or
`kind="process"`. Parallel image/text encoders instead use independent branches:

```python
s = content_slide(prs, "Parallel encoders")
b = slide_regions(s)["main"]
fork_join(s, ["Image encoder", "Text encoder"], **b,
          source="Image/text pairs", merge="Similarity")
```

Never use `kind="spatial"` or connect independent branches sequentially. Use
`kind="comparison"` for independent items with no source/merge dependency.

Define score `s_{ij}` and temperature `\tau`. This symmetric loss includes both
image→text and text→image directions and normalizes over the batch:

```python
clip_loss = r"\mathcal{L}=-\frac{1}{2N}\sum_{i=1}^{N}{\left(\log\frac{\exp(s_{ii}/\tau)}{\sum_{j=1}^{N}{\exp(s_{ij}/\tau)}}+\log\frac{\exp(s_{ii}/\tau)}{\sum_{j=1}^{N}{\exp(s_{ji}/\tau)}}\right)}"
equation(slide, .75, 2, 11.8, 2.4, {"latex": clip_loss}, size=16)
```

Keep outer `\sum_i{...}` and inner `\sum_j{...}` bodies explicitly braced at
their respective scopes. Never write `\sum_i\log...` or an unbraced inner sum.
Do not double-escape Python raw strings or add unsupported macros such as
`\nobreak`. `\log` / `\exp` use upright native runs; `\mathcal{L}` is editable
OMML script text. Keep editable equations and size their boxes for the rendered
preview; split explanations across pages if needed.
