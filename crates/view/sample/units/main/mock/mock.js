// The mock-up's loader: fills every `[data-component]` element from
// `components/<Name>.html` — the micro-frontend's markup and script — so a
// page is static HTML plus the micro-frontends it injects. Relative to the
// page: a page under `pages/` and a preview under `__preview/` both find
// `components/` beside the loader. A fragment's script finds its root through
// `document.currentScript.closest('[data-component]')`.
(() => {
  'use strict';
  const base = new URL('./', document.currentScript.src);
  document.querySelectorAll('[data-component]').forEach(async (el) => {
    const name = el.dataset.component;
    // An inlined micro-frontend — forked on a page by the factory view — carries its own markup.
    if (!name || el.dataset.loaded || el.dataset.inline) return;
    el.dataset.loaded = 'loading';
    try {
      const res = await fetch(new URL(`components/${name}.html`, base));
      if (!res.ok) throw new Error(res.status);
      const html = await res.text();
      const tpl = document.createElement('template');
      tpl.innerHTML = html;
      el.replaceChildren(tpl.content);
      // A script set by innerHTML does not run: it is made again.
      el.querySelectorAll('script').forEach((old) => {
        const script = document.createElement('script');
        [...old.attributes].forEach((a) => script.setAttribute(a.name, a.value));
        script.textContent = old.textContent;
        old.replaceWith(script);
      });
      el.dataset.loaded = 'yes';
    } catch (e) {
      el.dataset.loaded = 'no';
      el.innerHTML = `<p style="color:#7a2e2e;font:12px monospace">micro-frontend ${name}: no fragment (${e.message || e})</p>`;
    }
  });
})();
