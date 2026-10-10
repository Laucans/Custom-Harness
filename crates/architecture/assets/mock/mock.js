// The mock-up's loader: fills every `[data-component]` element from
// `components/<Name>.html` — the micro-frontend's markup and script — and
// every `[data-kit]` element from `kit/<Name>.html` — a component of the
// design system's kit, a visual primitive with no data of its own — so a
// page is static HTML plus the micro-frontends it injects, styled by the
// kit. A page under `pages/` and a preview under `__preview/` both find
// `components/`, `kit/` and `design/tokens.css` beside this file. A
// fragment's script finds its root through
// `document.currentScript.closest('[data-component]')` (or `[data-kit]`).
// Installed by `harness init-repo`.
(() => {
  'use strict';
  const base = new URL('./', document.currentScript.src);
  // The design system's tokens, once per document, when the page did not link them itself.
  if (!document.querySelector('link[href$="design/tokens.css"]')) {
    const link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = new URL('design/tokens.css', base);
    document.head.appendChild(link);
  }
  const fill = async (el, name, folder, what, mark) => {
    if (!name || el.dataset.loaded || el.dataset.inline) return;
    el.dataset.loaded = 'loading';
    try {
      const res = await fetch(new URL(`${folder}/${name}.html`, base));
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
      el.innerHTML = `<p class="${mark}">${what} ${name}: no fragment (${e.message || e})</p>`;
    }
  };
  // In order, one after the other: the first bundle starts the shared mock server.
  const fillAll = async (root) => {
    for (const el of root.querySelectorAll('[data-kit]')) await fill(el, el.dataset.kit, 'kit', 'kit component', 'kit-missing');
    for (const el of root.querySelectorAll('[data-component]')) await fill(el, el.dataset.component, 'components', 'micro-frontend', 'mf-missing');
  };
  fillAll(document);
  // A part added later — the factory view's edit mode drops a kit component on the page — is filled too.
  if (window.MutationObserver) {
    new MutationObserver((changes) => {
      for (const change of changes) {
        for (const node of change.addedNodes) {
          if (node.nodeType !== 1) continue;
          if (node.matches('[data-kit],[data-component]') && !node.dataset.loaded) fillAll({ querySelectorAll: (sel) => (node.matches(sel) ? [node] : []) });
          else fillAll(node);
        }
      }
    }).observe(document.documentElement, { childList: true, subtree: true });
  }
})();
