---
name: ofox-office-html
description: Create responsive interactive HTML with create_document, working controls, keyboard access, visible states and scoped validation.
---

# Interactive HTML pages

Use create_document with format html and the complete document source. It is saved as one file for an external browser. Do not replace this with Markdown or claim the app has run the page. No browser engine is bundled into this workflow.

Before code, name the audience, primary task, information hierarchy and interactions. For each visible control, name its input, state change and visible result. A travel guide needs finding/comparing places; a calculator needs editable assumptions and recalculation; an explainer needs steps and reset. Avoid a decorative dashboard of identical cards.

- Use one coherent typography/spacing/color system suited to the task, a clear primary action and restrained secondary controls. Preserve the user's language, data, qualifications and design. Label synthetic examples; never invent live metrics.
- Deliver a complete HTML document with lang, UTF-8, viewport, semantic headings and local CSS/JS. Prefer system fonts and inline/local artwork. Do not require remote fonts, a CDN, installation, credentials or a backend for a self-contained page.
- Buttons must do what their labels promise. Do not use empty links, fake loading timers, placeholder alerts, nonfunctional export/login/save buttons or an unavailable backend disguised as success. If a function needs a service, explain its boundary instead of simulating a completed transaction.
- Give inputs visible labels. Use native buttons and form controls, visible focus, selected states such as aria-pressed, keyboard operation, readable result/error/empty states and a reset path. Announce updated results without moving focus. Do not rely on hover or color alone.
- At 375px and 900px, check wrapping, content order, touch targets and overflow. Respect reduced motion; ensure readable contrast in the actual palette. Do not claim a conformance level without a corresponding audit.
- Before delivery, exercise every control, invalid/empty inputs, clear/reset and keyboard paths in a browser if a browser tool is available. Otherwise say browser behavior is unverified. Saving HTML or seeing no engine issues is not an interaction test.

The following independently written example demonstrates working behavior, not a mandatory visual template. Adapt its information structure, labels, styles and data to the user's task. Search and category combine; reset restores both; empty results stay visible and announced. It needs no network.

```html
<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Explore options</title>
<style>
  :root{font:16px/1.55 system-ui,sans-serif;color:#1e293b;background:#f7f8fa;color-scheme:light}
  *{box-sizing:border-box}body{margin:0}main{max-width:64rem;margin:auto;padding:clamp(1rem,4vw,3rem)}
  h1{font-size:clamp(2rem,5vw,3.5rem);line-height:1.1;max-width:16ch}p{max-width:60ch}
  form{display:flex;flex-wrap:wrap;align-items:end;gap:1rem;margin:2rem 0 1rem}label{display:grid;gap:.35rem;flex:1 1 15rem}
  fieldset{display:flex;flex-wrap:wrap;gap:.5rem;border:0;padding:0;margin:0}legend{margin-bottom:.35rem}
  button,input{font:inherit;border:1px solid #64748b;border-radius:.4rem;padding:.65rem .9rem;min-height:2.75rem}
  input{min-width:0;width:100%;background:white;color:inherit}button{cursor:pointer;color:inherit;background:white}
  button[aria-pressed=true]{background:#24475c;color:white;border-color:#24475c}:focus-visible{outline:3px solid #936215;outline-offset:3px}
  ul{list-style:none;padding:0;display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,15rem),1fr));gap:1rem}
  li{padding:1.3rem;border-top:3px solid #24475c;background:white}li[hidden],[hidden]{display:none!important}
  h2{margin-top:0;font-size:1.25rem}#empty{padding:1.2rem;border:1px dashed #64748b}
  @media(prefers-reduced-motion:reduce){*{scroll-behavior:auto!important;transition:none!important}}
</style>
<main>
  <p>Independent sample data</p><h1>Find an option that fits</h1>
  <p>Compare the available items by category and name.</p>
  <form id="filters" role="search">
    <label>Search options<input id="query" type="search" autocomplete="off" aria-controls="results"></label>
    <fieldset><legend>Category</legend>
      <button type="button" data-filter="all" aria-pressed="true">All</button>
      <button type="button" data-filter="learn" aria-pressed="false">Learn</button>
      <button type="button" data-filter="build" aria-pressed="false">Build</button>
    </fieldset><button id="reset" type="button">Reset</button>
  </form>
  <p id="summary" role="status" aria-live="polite" aria-atomic="true"></p>
  <p id="empty" hidden>No matching options. Try another term or reset the filters.</p>
  <ul id="results" aria-label="Matching options">
    <li data-category="learn"><h2>Explore the basics</h2><p>A short introduction.</p></li>
    <li data-category="learn"><h2>Work through an example</h2><p>A guided exercise.</p></li>
    <li data-category="build"><h2>Build a prototype</h2><p>A local project.</p></li>
  </ul>
</main>
<script>
  (() => {
    const form=document.querySelector('#filters'), query=document.querySelector('#query');
    const buttons=[...form.querySelectorAll('[data-filter]')], items=[...document.querySelectorAll('#results>li')];
    const summary=document.querySelector('#summary'), empty=document.querySelector('#empty');
    let category='all';
    function update(){
      const term=query.value.trim().toLocaleLowerCase();let count=0;
      items.forEach(item=>{item.hidden=!((category==='all'||item.dataset.category===category)&&item.textContent.toLocaleLowerCase().includes(term));if(!item.hidden)count++});
      buttons.forEach(button=>button.setAttribute('aria-pressed',String(button.dataset.filter===category)));
      empty.hidden=count!==0;summary.textContent=`${count} of ${items.length} options shown`;
    }
    buttons.forEach(button=>button.addEventListener('click',()=>{category=button.dataset.filter;update()}));
    query.addEventListener('input',update);form.addEventListener('submit',event=>{event.preventDefault();update()});
    document.querySelector('#reset').addEventListener('click',()=>{category='all';query.value='';update();query.focus()});
    update();
  })();
</script>
</html>
```

When reporting, distinguish saved output, source inspection, browser interactions actually exercised and unresolved behavior. Name missing functionality and dependencies plainly. The code and preview are evidence only for what was actually tested.
