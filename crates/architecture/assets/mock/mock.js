// The mock-up's loader: fills every `[data-component]` element from
// `components/<Name>.html` — the micro-frontend's markup and script — so a
// page is static HTML plus the micro-frontends it injects. A page under
// `pages/` and a preview under `__preview/` both find `components/` beside
// this file. A fragment's script finds its root through
// `document.currentScript.closest('[data-component]')`. Installed by
// `harness init-repo`.
(() => {
  'use strict';
  const base = new URL('./', document.currentScript.src);
  const load = async (el) => {
    const name = el.dataset.component;
    if (!name || el.dataset.loaded) return;
    el.dataset.loaded = 'loading';
    try {
      const res = await fetch(new URL(`components/${name}.html`, base));
      if (!res.ok) throw new Error(res.status);
      const tpl = document.createElement('template');
      tpl.innerHTML = await res.text();
      el.replaceChildren(tpl.content);
      // A script set by innerHTML does not run: it is made again, in order.
      for (const old of [...el.querySelectorAll('script')]) {
        const script = document.createElement('script');
        [...old.attributes].forEach((a) => script.setAttribute(a.name, a.value));
        script.textContent = old.textContent;
        await new Promise((done) => { script.onload = done; script.onerror = done; old.replaceWith(script); if (!script.src) done(); });
      }
      el.dataset.loaded = 'yes';
    } catch (e) {
      el.dataset.loaded = 'no';
      el.innerHTML = `<p class="mf-missing">micro-frontend ${name}: no fragment (${e.message || e})</p>`;
    }
  };
  // In order, one after the other: the first bundle starts the shared mock server.
  (async () => { for (const el of document.querySelectorAll('[data-component]')) await load(el); })();
})();
