// The mock-up on its easel: the product's frontend, mocked — the static pages
// under `mock/` of the branch shown — run in an iframe on a board, so a
// human pins notes on its parts, draws zones around several, links a page
// to another (or creates one from an objective), drops requests an AI
// applies, and drops micro-frontends on it. One board per branch and page:
// pins stay where they were left.
//
// Two views. **frontend**: the page, with a switch that outlines every
// micro-frontend in its own colour (pins then land on micro-frontends, not
// their parts), a strip of the branch's micro-frontends to drag onto the
// page, and an **edit** mode: parts dragged before or after one another,
// resized to twelfths of their row, removed, cloned, their text rewritten,
// a container laid out freely, links and buttons added — every gesture an
// operation of the page's **draft** (edit.js), kept in versions by the
// server (`/api/drafts`), forty steps of undo deep. A part of a real
// micro-frontend edited from a page forks that micro-frontend into a copy of
// the page's own. **micro-frontends**: each micro-frontend rendered alone
// from its fragment and fixture, a card per one, pins on the cards, and a
// form that makes a new one.
//
// The page is always shown as its draft says: the page's source is fetched,
// the operations applied, and the result put in the iframe (`srcdoc`, with
// a `<base>` so its links resolve beside the page). The pages come from
// `/mock/<branch slug>/<path>`, a micro-frontend alone from
// `/mock/<branch slug>/__preview/<Name>`; the branch's state from
// `/api/mockup?branch=`. A branch edited by a session is shown as its
// workspace holds it; `save the mock-up` writes the page as the draft shows
// it, the draft's own file and the forked fragments, then commits it on a
// `mockup/<name>` branch; `discard` drops it.
(() => {
  'use strict';

  const DEFAULT_WIDTH = 1280;
  const PRESETS = [['Mobile', 390], ['Tablet', 768], ['Laptop', 1280], ['Large', 1536]];
  const MIN_HEIGHT = 480;
  const CARD_W = 400;
  const CARD_GAP = 28;
  const CARD_MIN = 160;
  const CARD_MAX = 640;
  const POLL_MS = 3000;
  const SAVE_DELAY = 500;
  const SLOP = 5;
  const LAST = 'harness.mockup.page';
  const MF_ON = 'harness.mockup.mf';
  const WIDTHS = 'harness.mockup.widths';
  const COLOURS = ['#4ea3f0', '#f08a2c', '#4fbf72', '#b388f5', '#e2b240', '#ef6b8a', '#2bc0c8', '#c9a46b'];
  // What a pin may sit on, nearest first; what edit mode may take hold of.
  const PARTS = '[data-component],[id],[data-id],button,a,input,select,textarea,label,h1,h2,h3,h4,img,table,th,td,tr,li,form,section,article,aside,nav,header,footer,figure,p';
  const EDITABLE = '[data-component],[data-id],button,a,input,select,textarea,label,h1,h2,h3,h4,h5,h6,img,table,th,td,tr,ul,ol,li,form,section,article,aside,nav,header,footer,figure,p,div,span';
  const E = window.MockEdit;

  const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const plain = (s) => String(s || '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
  const slugOf = (branch) => String(branch || '').replace(/\//g, '__');
  const pageSlug = (path) => plain(String(path || '').replace(/^pages\//, '').replace(/\.html?$/, '')) || 'home';
  const newId = (prefix) => prefix + Date.now().toString(36) + Math.random().toString(36).slice(2, 6);

  function kept(key, fallback) {
    try { const v = localStorage.getItem(key); return v == null ? fallback : JSON.parse(v); } catch (e) { return fallback; }
  }
  function keep(key, value) {
    try { localStorage.setItem(key, JSON.stringify(value)); } catch (e) { /* a preference, not a record */ }
  }
  const remembered = (branch) => (kept(LAST, {}) || {})[branch] || '';
  function remember(branch, page) { const all = kept(LAST, {}) || {}; all[branch] = page; keep(LAST, all); }

  function mount(host, opts) {
    host.innerHTML = '<div class="dm mk">'
      + '<div class="dm-switch" hidden><div class="dm-branches" role="group" aria-label="branch"></div>'
      + '<div class="dm-features" hidden role="menu" aria-label="feature branches"></div></div>'
      + '<div class="dm-bar">'
      + '<div class="mk-views" role="group" aria-label="view"><button type="button" class="on" data-mk="view" data-view="page">frontend</button><button type="button" data-mk="view" data-view="components">micro-frontends</button></div>'
      + '<span class="dm-origin mk-origin">reading the mock-up…</span>'
      + '<label class="mk-page" hidden>page <select class="mk-pages" aria-label="page"></select></label>'
      + '<button type="button" class="mini mk-only-page" data-mk="new" title="A new page of the mock-up, drawn by an AI session from its objective">+ page</button>'
      + '<button type="button" class="mini mk-only-page" data-mk="mf" aria-pressed="false" title="Outline every micro-frontend in its own colour; pins and edits then take hold of micro-frontends">micro-frontends ◧</button>'
      + '<button type="button" class="mini mk-only-page" data-mk="edit" aria-pressed="false" title="Design the page: select, drag, resize, align, insert — every change a step of the page\'s draft">edit</button>'
      + '<button type="button" class="mini mk-only-page" data-mk="interact" aria-pressed="false" title="Click through to the page itself: its links, its buttons">interact</button>'
      + '<button type="button" class="mini" data-mk="full" aria-pressed="false" title="The mock-up over the whole window (Esc leaves)">⛶</button>'
      + '<button type="button" class="mini mk-only-components" data-mk="design" hidden title="A new micro-frontend, made by an AI session from its spec">+ micro-frontend</button>'
      + '<span class="spacer"></span>'
      + '<span class="mk-state"></span>'
      + '<input class="mk-name" maxlength="80" placeholder="name of the mock-up" hidden aria-label="name of the mock-up">'
      + '<button type="button" class="mini" data-mk="save" hidden>save the mock-up</button>'
      + '<button type="button" class="mini" data-mk="discard" hidden>discard</button>'
      + '</div>'
      + '<div class="mk-editbar" hidden>'
      + '<button type="button" class="mini" data-ed="undo" title="Undo the last step (ctrl+z)">↶ undo</button>'
      + '<button type="button" class="mini" data-ed="redo" title="Redo (ctrl+shift+z)">↷ redo</button>'
      + '<span class="mk-sep"></span>'
      + '<span class="fg-tools" role="group" aria-label="tools">'
      + '<button type="button" class="mini on" data-tool="move" title="Move · select (V)">↖</button>'
      + '<button type="button" class="mini" data-tool="section" title="Section: click to drop one, drag to size it (F)">▭</button>'
      + '<button type="button" class="mini" data-tool="text" title="Text (T)">T</button>'
      + '<button type="button" class="mini" data-tool="button" title="Button (B)">⊡</button>'
      + '<button type="button" class="mini" data-tool="link" title="Link to a page or a URL (L)">⇗</button>'
      + '<button type="button" class="mini" data-tool="image" title="Image placeholder (I)">▨</button>'
      + '<span class="fg-kit" title="A component of the design system\'s kit: type to search, pick one, then click where it goes on the page"><input class="fg-kit-search" type="search" placeholder="kit component…" aria-label="search a component of the kit" autocomplete="off"><div class="fg-kit-list" role="listbox" hidden></div></span>'
      + '</span>'
      + '<span class="mk-sep"></span>'
      + '<button type="button" class="mini" data-ed="apply-all" title="Apply every pending request of the page to the draft, in one go">simulate requests</button>'
      + '<span class="spacer"></span>'
      + '<label class="mk-vp" title="the width the page is shown at"><select class="mk-presets" aria-label="viewport">' + PRESETS.map(([n, w]) => `<option value="${w}">${n} · ${w}</option>`).join('') + '<option value="0">custom</option></select><input class="mk-width" type="number" min="320" max="3000" step="10" aria-label="width in pixels" hidden></label>'
      + '<span class="mk-draft-state"></span>'
      + '<button type="button" class="mini" data-ed="keep" title="Close this version into the history; the next change opens a new one">keep</button>'
      + '<button type="button" class="mini" data-ed="validate" title="Close this version as the next round of development">validate</button>'
      + '<button type="button" class="mini" data-ed="history" title="The versions of this page\'s draft">history</button>'
      + '</div>'
      + '<div class="mk-frozen" hidden></div>'
      + '<div class="mk-form" hidden></div>'
      + '<div class="dm-main"><aside class="mk-layers" hidden></aside><div class="dm-board mk-board"></div><aside class="mk-strip" hidden></aside><aside class="mk-panel" hidden></aside><aside class="mk-history" hidden></aside></div>'
      + '<div class="mk-toast" hidden></div>'
      + '</div>';
    const q = (s) => host.querySelector(s);
    const st = { branch: (opts && opts.branch) || '', page: (opts && opts.page) || '', view: (opts && opts.view) || 'page', versions: null, state: null, stamp: '', pinned: false,
      timer: null, gone: false, interact: false, mf: !!kept(MF_ON, false), board: null, frame: null, shield: null, doc: null, armed: null, proxies: new WeakMap(), fitted: false, saving: false,
      cards: [],
      // Edit mode: the page's source, its draft engine, the versions, the selection, a gesture under way.
      edit: !!(opts && opts.edit), source: '', engine: null, history: null, sel: '', hover: null, drag: null, resize: null, space: false, saveTimer: null, viewing: null, width: DEFAULT_WIDTH, lastLoadedPage: '', toastTimer: null,
      // The design system's kit, for the search field of the tools; the component picked to drop.
      kit: null, kitPick: '' };

    // ---- what the branch knows of its micro-frontends ----------------------------------
    const components = () => {
      const s = st.state || {};
      const units = (s.units && s.units.micro_uis) || [];
      const byName = new Map();
      (s.components || []).forEach((c) => byName.set(c.name, Object.assign({ mocked: true, built: false }, c)));
      units.forEach((u) => {
        const have = byName.get(u.name);
        if (have) { have.built = true; if (!have.description) have.description = u.description; if (!have.needs.length) have.needs = u.needs; if (!have.actions.length) have.actions = u.actions; }
        else byName.set(u.name, Object.assign({ mocked: false, built: true }, u));
      });
      // The forks the draft made: copies of the page's own.
      forks().forEach((f) => { if (!byName.has(f.as)) byName.set(f.as, { name: f.as, system: (byName.get(f.from) || {}).system || '', description: `${f.from}, as this page wants it`, fragment: `components/${f.as}.html`, needs: (byName.get(f.from) || { needs: [] }).needs, actions: (byName.get(f.from) || { actions: [] }).actions, props: {}, mocked: true, built: false, variantOf: f.from }); });
      const list = [...byName.values()].sort((a, b) => a.name.localeCompare(b.name));
      list.forEach((c, i) => { c.colour = COLOURS[i % COLOURS.length]; c.pages = (s.pages || []).filter((p) => (p.components || []).includes(c.name)).map((p) => p.title); });
      return list;
    };
    const componentOf = (name) => components().find((c) => c.name === name) || null;
    const specOf = (c) => `name: ${c.name}\nsystem: ${c.system || ''}\ndescription: ${c.description || ''}\nneeds: ${(c.needs || []).join(', ')}\nactions: ${(c.actions || []).join(', ')}\nprops:\n${Object.entries(c.props || {}).map(([k, v]) => `  ${k}: ${v}`).join('\n')}`.trim();
    const capabilities = () => (st.state && st.state.units && st.state.units.capabilities) || [];
    const dataCapabilities = () => (st.state && st.state.units && st.state.units.data_capabilities) || [];
    const forks = () => (st.engine ? st.engine.ops.filter((o) => o.kind === 'fork') : []);

    // ---- the parts of the page, reached through the iframe ------------------------------
    const proxy = (el) => {
      let p = st.proxies.get(el);
      if (p) return p;
      p = {
        el,
        getAttribute: (name) => (name === 'data-pin' ? E.targetOf(el) : el.getAttribute(name)),
        classList: { add: (c) => el.classList.add(c), remove: (c) => el.classList.remove(c) },
        getBoundingClientRect() { return screenRect(el); },
      };
      st.proxies.set(el, p);
      return p;
    };
    // An element's box on the screen, through the iframe's place and the board's zoom.
    function screenRect(el) {
      const fr = st.frame.getBoundingClientRect();
      const r = el.getBoundingClientRect();
      const k = st.board ? st.board.zoom() : 1;
      return { left: fr.left + r.left * k, top: fr.top + r.top * k, width: r.width * k, height: r.height * k, right: fr.left + (r.left + r.width) * k, bottom: fr.top + (r.top + r.height) * k };
    }
    const elementAt = (clientX, clientY, selector) => {
      if (!st.frame || !st.doc) return null;
      const fr = st.frame.getBoundingClientRect();
      if (clientX < fr.left || clientX > fr.right || clientY < fr.top || clientY > fr.bottom) return null;
      const k = st.board ? st.board.zoom() : 1;
      let el = st.doc.elementFromPoint((clientX - fr.left) / k, (clientY - fr.top) / k);
      el = el && el.closest(selector);
      if (!el || el === st.doc.body || el === st.doc.documentElement) return null;
      return el;
    };
    const resolve = (target) => {
      if (st.view !== 'page' || !target) return null;
      const el = E.elementOf(st.doc, target);
      return el ? proxy(el) : null;
    };
    const under = (clientX, clientY) => {
      if (st.view !== 'page') return null;
      const el = elementAt(clientX, clientY, st.mf ? '[data-component]' : PARTS);
      return el ? proxy(el) : null;
    };
    const describe = (target) => {
      if (!target) return st.view === 'page' ? 'the page' : 'the micro-frontends';
      if (target.startsWith('component:')) return (target.includes('>') ? 'inside micro-frontend ' : 'micro-frontend ') + target.slice(10).split('>')[0];
      if (target.startsWith('id:')) return '#' + target.slice(3).split('>')[0];
      if (target.startsWith('data:')) return target.slice(5).split('>')[0];
      return target.slice(5).split('>').map((s) => s.replace(/:nth-child\(\d+\)/, '')).slice(-3).join(' > ');
    };
    const context = (target) => {
      let text = '';
      if (st.view === 'page') {
        const page = currentPage();
        text = page ? `Page ${page.path} — ${page.title}: ${page.objective || '(no objective written)'}` : `Page ${st.page || '(none yet)'}`;
        if (page && page.screen) text += `\nMocks the screen ${page.screen}.`;
        if (page && page.components && page.components.length) text += `\nMicro-frontends injected: ${page.components.join(', ')}.`;
        if (st.engine && st.engine.ops.length) text += `\nThe page's draft holds ${st.engine.ops.length} operation(s) the human made in edit mode; the page below shows them.`;
      } else {
        text = 'The micro-frontends view: each micro-frontend of the branch rendered alone from its fragment and fixture.';
      }
      if (target && target.startsWith('component:')) {
        const c = componentOf(target.slice(10).split('>')[0]);
        if (c) text += `\nThe micro-frontend ${c.name} (${c.mocked ? 'mocked' : 'not mocked yet'}, ${c.built ? 'built under apps/' : 'not built yet'}):\n${specOf(c)}\nUsed by the pages: ${c.pages.join(', ') || 'none'}.`;
      }
      const el = E.elementOf(st.doc, target);
      if (el) {
        const html = el.outerHTML.replace(/\s+/g, ' ');
        text += `\nThe part the pin sits on (${describe(target)}):\n${html.length > 1500 ? html.slice(0, 1500) + '…' : html}`;
      }
      return text;
    };
    const currentPage = () => (st.state && st.state.pages || []).find((p) => p.path === st.page) || null;
    const boardOf = () => `mockup--${plain(st.branch) || 'integration'}--${st.view === 'components' ? 'micro-frontends' : st.page ? pageSlug(st.page) : 'home'}`;
    const pageUrl = () => `/mock/${encodeURIComponent(slugOf(st.branch))}/${st.page}`;
    const baseHref = () => `${location.origin}/mock/${encodeURIComponent(slugOf(st.branch))}/${st.page.replace(/[^/]*$/, '')}`;
    const previewSrc = (name) => `/mock/${encodeURIComponent(slugOf(st.branch))}/__preview/${encodeURIComponent(name)}?v=${encodeURIComponent(st.stamp)}`;

    // ---- the board: one per branch and page, one for the micro-frontends ------------
    function mountBoard() {
      if (st.board) { try { st.board.destroy(); } catch (e) { /* gone */ } }
      st.board = null; st.frame = null; st.doc = null; st.shield = null; st.fitted = false; st.proxies = new WeakMap(); st.cards = []; design.select([]);
      const onSession = opts && opts.onSession
        ? (note, ctx) => opts.onSession({ board: boardOf(), id: note.id, on: note.kind === 'link' ? 'page ' + (note.page || '?') : note.kind === 'design' ? 'micro-frontend ' + (note.component || '?') : describe(note.target), kind: note.kind, context: ctx, branch: st.branch, page: st.view === 'page' ? st.page : '' })
        : null;
      st.board = window.HarnessBoard.create(q('.mk-board'), {
        id: boardOf(),
        describe,
        resolve,
        context,
        onSession,
        under,
        kinds: st.view === 'page' ? ['note', 'request', 'link', 'design'] : ['note', 'request', 'design'],
        zones: st.view === 'page',
        pages: () => (st.state && st.state.pages) || [],
        capabilities,
        dataCapabilities,
        apply: st.view === 'page' ? (note) => applyRequests([note]) : null,
        backlogContext: (note) => {
          const page = currentPage();
          let text = page ? `Page: ${page.title} (\`${page.path}\`) on \`${st.branch}\`${page.objective ? ' — ' + page.objective : ''}` : `Branch: ${st.branch}`;
          if (note.target && note.target !== '') text += `\nPart: ${describe(note.target)}`;
          if (note.kind !== 'request' || note.target !== '') { const summary = draftSummary(); if (summary) text += '\n\n' + summary; }
          return text;
        },
        goto: (path) => { setView('page'); show(path); },
        onLoaded: () => { if (opts && opts.pin && !st.pinned && st.board && st.board.goTo(opts.pin)) st.pinned = true; design.refresh(); },
        bounds: () => (st.view === 'page'
          ? { x: 0, y: 0, w: st.width, h: Math.max(MIN_HEIGHT, st.frame ? parseFloat(st.frame.style.height) || MIN_HEIGHT : MIN_HEIGHT) }
          : { x: 0, y: 0, w: Math.max(DEFAULT_WIDTH, st.board ? parseFloat(st.board.content.style.width) || DEFAULT_WIDTH : DEFAULT_WIDTH), h: Math.max(MIN_HEIGHT, st.board ? parseFloat(st.board.content.style.height) || MIN_HEIGHT : MIN_HEIGHT) }),
      });
      if (st.view === 'components') { mountCards(); return; }
      const page = currentPage();
      if (!page) {
        st.board.content.innerHTML = `<div class="mk-empty" data-pin="page:none"><h2>${st.state && st.state.present ? 'No page chosen' : 'No page yet'}</h2>`
          + `<p>${st.state && st.state.pages && st.state.pages.length ? 'Pick a page above.' : 'The mock-up starts with its first page: <b>+ page</b> above, or drop a pin here and make it a <b>link</b> to a new page — an AI session draws it from the objective you write.'}</p></div>`;
        st.board.setSize(DEFAULT_WIDTH, MIN_HEIGHT);
        st.board.fit();
        return;
      }
      st.board.content.innerHTML = `<iframe class="mk-frame" title="${esc(page.title)}" style="width:${st.width}px;height:${MIN_HEIGHT}px"></iframe><div class="mk-shield"${st.interact ? ' hidden' : ''}></div><div class="mk-vp-handle" title="drag to change the width"></div>`;
      st.frame = q('.mk-frame');
      st.shield = q('.mk-shield');
      st.board.setSize(st.width, MIN_HEIGHT);
      st.frame.addEventListener('load', onFrameLoad);
      loadPage();
    }
    // The page's source and its draft: fetched, then shown as the draft says.
    async function loadPage() {
      const page = currentPage();
      if (!page) return;
      const board = boardOf();
      try {
        const [source, history] = await Promise.all([
          fetch(pageUrl() + '?v=' + encodeURIComponent(st.stamp)).then((r) => (r.ok ? r.text() : '')),
          fetch(`/api/drafts/${encodeURIComponent(board)}/open`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ base: page.hash || '' }) }).then((r) => (r.ok ? r.json() : null)),
        ]);
        if (st.gone || boardOf() !== board) return;
        st.source = source;
        st.history = history;
        const current = history && history.current;
        st.engine = E.engine({ source, ops: current ? current.ops : [] });
        const widths = kept(WIDTHS, {}) || {};
        st.width = (current && current.viewport) || widths[board] || DEFAULT_WIDTH;
        st.viewing = null;
        drawEditBar();
        render();
      } catch (e) {
        say('the page could not be read');
      }
    }
    // A page that loads no `mock.js` of its own is shown with the harness's
    // loader, so a kit component dropped on it is filled and the tokens
    // linked — on the page shown, never on the page saved.
    function withLoader(html) {
      if (/mock\.js/.test(html)) return html;
      const tag = `<script src="${location.origin}/mock/${encodeURIComponent(slugOf(st.branch))}/mock.js"></script>`;
      return /<\/body>/i.test(html) ? html.replace(/<\/body>/i, tag + '</body>') : html + tag;
    }
    // The page as the draft shows it, into the iframe.
    function render() {
      if (!st.frame || !st.engine) return;
      const html = st.viewing ? E.engine({ source: st.source, ops: st.viewing.ops }).html(baseHref()) : st.engine.html(baseHref());
      st.frame.style.width = st.width + 'px';
      st.frame.srcdoc = withLoader(html);
      st.board.setSize(st.width, parseFloat(st.frame.style.height) || MIN_HEIGHT);
      drawEditBar();
      drawFrozen();
    }
    function onFrameLoad() {
      if (!st.frame) return;
      let doc = null;
      try { doc = st.frame.contentDocument; } catch (e) { doc = null; }
      st.doc = doc;
      st.proxies = new WeakMap();
      if (!doc) return;
      // The page navigated itself (interact mode): follow it.
      const path = (() => { try { const u = st.frame.contentWindow.location; return u.protocol === 'about:' ? '' : decodeURIComponent(u.pathname); } catch (e) { return ''; } })();
      const prefix = `/mock/${slugOf(st.branch)}/`;
      if (path.startsWith(prefix) && path.slice(prefix.length) !== st.page) {
        const to = path.slice(prefix.length);
        if ((st.state.pages || []).some((p) => p.path === to)) { show(to); return; }
      }
      injectStyle(doc);
      outline();
      measure();
      if (!st.fitted) { st.fitted = true; st.board.fit(); }
      st.board.refresh();
      design.refresh();
    }
    function injectStyle(doc) {
      if (doc.getElementById('hb-style')) return;
      const style = doc.createElement('style');
      style.id = 'hb-style';
      style.textContent = '.hb-target{outline:2px dashed #e2b240 !important;outline-offset:2px}'
        + '.mf-on[data-component]{outline:2px solid var(--mf,#4ea3f0) !important;outline-offset:4px;position:relative}'
        + '.mf-on[data-component]::before{content:attr(data-component);position:absolute;left:-2px;top:-22px;background:var(--mf,#4ea3f0);color:#101418;font:600 11px/1 ui-monospace,monospace;padding:4px 7px;border-radius:4px 4px 0 0;letter-spacing:.03em;z-index:2147483000}'
        + '[data-layout="static"]{outline:1px dashed rgba(43,192,200,.5)}'
        + '.mk-added{outline:1px dotted #2bc0c8}';
      (doc.head || doc.documentElement).appendChild(style);
    }
    function outline() {
      if (!st.doc) return;
      const list = components();
      st.doc.querySelectorAll('[data-component]').forEach((el) => {
        const c = list.find((x) => x.name === el.dataset.component);
        el.classList.toggle('mf-on', st.mf);
        if (st.mf) el.style.setProperty('--mf', c ? c.colour : '#4ea3f0'); else el.style.removeProperty('--mf');
      });
    }
    function measure() {
      if (st.view === 'components') { measureCards(); return; }
      if (!st.doc || !st.frame) return;
      const h = Math.max(MIN_HEIGHT, st.doc.documentElement.scrollHeight, st.doc.body ? st.doc.body.scrollHeight : 0);
      if (Math.abs(h - (parseFloat(st.frame.style.height) || 0)) > 2) {
        st.frame.style.height = h + 'px';
        st.board.setSize(st.width, h);
        design.refresh();
      }
    }
    function reloadFrame() {
      if (st.view === 'components') { st.cards.forEach((c) => { c.frame.src = previewSrc(c.name); }); return; }
      if (!st.frame) { mountBoard(); return; }
      loadPage();
    }

    // ---- edit mode: the designer's hands (design.js), the draft's steps --------------
    const editing = () => !!(st.edit && st.view === 'page' && st.engine && !st.viewing);
    // The micro-frontend an element belongs to, when it is one the page must
    // fork before editing inside: a real or hand-written fragment not inlined.
    function forkNeeded(el) {
      const root = el && el.closest ? el.closest('[data-component]') : null;
      if (!root || root === el || root.dataset.inline) return null;
      return root;
    }
    // Forks a micro-frontend into a copy of the page's own, and answers the
    // target as it reads after the fork.
    function fork(root, target) {
      const from = root.dataset.component;
      const page = currentPage();
      const as = (from + (page ? page.title.replace(/[^A-Za-z0-9]+/g, '') : 'Copy')).replace(/^[a-z]/, (c) => c.toUpperCase());
      const copy = root.cloneNode(true);
      copy.querySelectorAll('script').forEach((x) => x.remove());
      const op = { kind: 'fork', by: 'human', target: 'component:' + from, as, from, html: copy.innerHTML };
      const r = st.engine.push(op);
      if (!r.ok) { say('the fork failed'); return target; }
      if (st.doc) E.applyOps(st.doc, [op]);
      toast(`fork <b>${esc(from)}</b> to <b>${esc(as)}</b> — click to rename`, () => renameFork(as));
      drawStrip();
      return target.replace(new RegExp('^component:' + from.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')), 'component:' + as);
    }
    function renameFork(as) {
      const panel = q('.mk-panel');
      panel.hidden = false;
      panel.innerHTML = `<div class="mk-panel-head"><b>rename the fork</b></div><input class="mk-fork-name" value="${esc(as)}" data-from="${esc(as)}" maxlength="80"><button type="button" class="mini on" data-ed="rename-fork" data-from="${esc(as)}">rename</button>`;
      panel.querySelector('input').focus();
    }
    function renameForkTo(from, to) {
      if (!/^[A-Z][A-Za-z0-9]*$/.test(to) || to === from) return;
      st.engine.ops.forEach((op) => {
        if (op.kind === 'fork' && op.as === from) op.as = to;
        ['target', 'before', 'after', 'into'].forEach((k) => { if (typeof op[k] === 'string') op[k] = op[k].replace('component:' + from, 'component:' + to); });
      });
      afterChange();
      q('.mk-panel').hidden = true;
    }
    function toast(html, onClick) {
      const t = q('.mk-toast');
      t.innerHTML = html;
      t.hidden = false;
      t.onclick = () => { t.hidden = true; if (onClick) onClick(); };
      clearTimeout(st.toastTimer);
      st.toastTimer = setTimeout(() => { t.hidden = true; }, 7000);
    }
    // One step of the draft: applied to the page as shown — without reloading
    // it, when every operation finds its part there — and kept.
    function step(ops) {
      const list = Array.isArray(ops) ? ops : [ops];
      const r = st.engine.push(list);
      if (!r.ok) { say(`not applied: ${r.failed} operation(s) found no target`); return false; }
      const failed = st.doc ? E.applyOps(st.doc, list) : [{ why: 'no page' }];
      if (failed.length) render();
      else { measure(); st.board.refresh(); design.refresh(); drawEditBar(); drawFrozen(); }
      persist();
      drawStrip();
      return true;
    }
    // After an undo, a redo or a version change: the page is shown again from scratch.
    function afterChange() {
      render();
      persist();
      drawStrip();
    }
    function persist() {
      clearTimeout(st.saveTimer);
      const board = boardOf();
      st.saveTimer = setTimeout(async () => {
        if (!st.engine || boardOf() !== board) return;
        try {
          const res = await fetch(`/api/drafts/${encodeURIComponent(board)}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ops: st.engine.ops, viewport: st.width, name: (st.history && st.history.current && st.history.current.name) || '' }) });
          if (!res.ok) throw new Error(await res.text());
          const current = await res.json();
          if (st.history) st.history.current = current;
          drawEditBar();
        } catch (e) { say(`draft not kept: ${esc(e.message || e)}`); }
      }, SAVE_DELAY);
    }
    // A target as an operation names it, forking first when it lies inside a
    // micro-frontend; `force` forks the micro-frontend itself.
    function opTarget(el, force) {
      const target = E.targetOf(el);
      if (force) { const root = el.closest('[data-component]'); return root && !root.dataset.inline ? fork(root, target) : target; }
      if (st.mf) return target;
      const root = forkNeeded(el);
      return root ? fork(root, target) : target;
    }
    const requestsOn = (target) => (st.board ? st.board.all() : []).filter((n) => (n.kind === 'request' || n.kind === 'asking' || n.kind === 'simulate') && n.target === target);
    function requestsHtml(target) {
      const list = requestsOn(target);
      return list.length ? '<ul>' + list.map((n) => `<li data-open-note="${esc(n.id)}"><i class="pin-dot request state-${esc(n.state || 'pending')}"></i>${esc((n.text || '').split('\n')[0] || '(empty)')}<small>${esc(n.state || 'pending')}</small></li>`).join('') + '</ul>' : '<div class="muted">none on this part</div>';
    }
    function newRequest(target) {
      if (st.board) st.board.newNote({ kind: 'request', target, text: '' });
    }
    function designSession(name) {
      const c = componentOf(name);
      if (!c || !st.board) return;
      const id = st.board.newNote({ kind: 'design', component: c.name, text: specOf(c), target: 'component:' + c.name });
      st.board.session(id);
    }
    const design = window.MockDesign.create({
      host,
      board: () => st.board,
      boardHost: () => q('.mk-board'),
      frame: () => st.frame,
      doc: () => st.doc,
      engine: () => st.engine,
      editing,
      mfOnly: () => st.mf,
      space: () => st.space,
      step,
      opTarget,
      proxy,
      describe,
      pages: () => (st.state && st.state.pages) || [],
      componentOf,
      requestsHtml,
      newRequest,
      mockComponent: designSession,
      designSession,
      onPanel: () => drawStrip(),
      kitPick: () => st.kitPick,
    });

    // ---- the kit's search field: the design system's components, dropped like a tool ------
    async function loadKit() {
      if (st.kit) return st.kit;
      try {
        const res = await fetch('/api/design');
        st.kit = res.ok ? ((await res.json()).catalog || []).filter((c) => c.state === 'generated') : [];
      } catch (e) { st.kit = []; }
      return st.kit;
    }
    async function searchKit(text) {
      const list = q('.fg-kit-list');
      const kit = await loadKit();
      const needle = String(text || '').trim().toLowerCase();
      const hits = kit.filter((c) => !needle || `${c.name} ${c.title} ${c.what} ${c.prompt}`.toLowerCase().includes(needle)).slice(0, 12);
      list.innerHTML = hits.length
        ? hits.map((c) => `<button type="button" class="fg-kit-item" data-kit-pick="${esc(c.name)}" role="option"><b>${esc(c.name)}</b><small>${esc(c.title)}${c.what ? ' · ' + esc(c.what) : ''}</small></button>`).join('')
        : `<div class="fg-kit-none">${kit.length ? 'no component matches' : 'no component generated yet — the design system\'s easel makes them'}</div>`;
      list.hidden = false;
    }
    function pickKit(name) {
      st.kitPick = name;
      q('.fg-kit-search').value = name;
      q('.fg-kit-list').hidden = true;
      design.setTool('kit');
      say(`<b>${esc(name)}</b> picked: click where it goes on the page (Esc cancels)`);
    }
    // A request the AI could not apply for want of a component: the four ways out.
    function offerSkipped(skipped) {
      const toastEl = q('.mk-toast');
      const missing = skipped.map((s) => ({ s, m: /^missing component:\s*([A-Za-z0-9]+)\s*[—–:-]?\s*(.*)$/.exec(String(s.skip || '')) })).filter((x) => x.m);
      skipped.filter((s) => !missing.some((x) => x.s === s)).forEach((s) => say(`request ${s.by}: ${esc(s.skip)}`));
      if (!missing.length) return;
      toastEl.innerHTML = missing.map(({ s, m }) => `<div class="mk-missing" data-missing="${esc(s.by)}" data-missing-name="${esc(m[1])}" data-missing-what="${esc(m[2] || '')}"><b>${esc(m[1])}</b> is not in the kit${m[2] ? ': ' + esc(m[2]) : ''}. <button type="button" class="mini on" data-miss="build">build it</button><button type="button" class="mini" data-miss="edit">edit a component</button><button type="button" class="mini" data-miss="placeholder">a placeholder</button><button type="button" class="mini" data-miss="without">do without</button></div>`).join('');
      toastEl.hidden = false;
      clearTimeout(st.toastTimer);
    }
    function onMissing(e) {
      const b = e.target.closest('[data-miss]');
      if (!b) return false;
      const box = b.closest('[data-missing]');
      const by = box.dataset.missing; const name = box.dataset.missingName; const what = box.dataset.missingWhat;
      if (b.dataset.miss === 'build') window.dispatchEvent(new CustomEvent('harness:open-pane', { detail: { kind: 'kit', tab: 'catalogue', custom: { name, prompt: what } } }));
      else if (b.dataset.miss === 'edit') window.dispatchEvent(new CustomEvent('harness:open-pane', { detail: { kind: 'kit', tab: 'canvas' } }));
      else if (b.dataset.miss === 'placeholder' && st.engine && st.doc) {
        const main = st.doc.querySelector('main') || st.doc.body.firstElementChild || st.doc.body;
        if (st.engine.push({ kind: 'add', by, what: 'image', label: name, as: newId('i'), into: E.targetOf(main) || 'path:' }).ok) { afterChange(); if (st.board) st.board.patch(by, { state: 'simulated' }); }
      } else if (b.dataset.miss === 'without' && st.board) st.board.patch(by, { state: 'simulated' });
      box.remove();
      if (!q('.mk-toast').children.length) q('.mk-toast').hidden = true;
      return true;
    }

    // Keys the pane keeps for itself: space pans, ctrl+z and ctrl+shift+z
    // undo and redo; the designer has the rest.
    const onKey = (e) => {
      if (e.target.closest('.mk-form') || e.target.classList.contains('mk-name') || e.target.closest('.mk-panel') || e.target.closest('.mk-layers') || e.target.closest('.mk-history') || e.target.closest('.mk-editbar')) {
        e.stopPropagation();
        if (e.key === 'Enter' && e.target.classList.contains('mk-name')) save();
        if (e.key === 'Escape' && e.target.closest('.mk-form')) drawForm(false);
        return;
      }
      if (!editing() || !host.isConnected) return;
      if (e.key === ' ' && e.type === 'keydown') { st.space = true; host.classList.add('mk-space'); }
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z' && !e.target.matches('input,textarea')) {
        e.preventDefault(); e.stopPropagation();
        if (e.shiftKey ? st.engine.redo() : st.engine.undo()) afterChange();
      }
    };
    const onKeyUp = (e) => { if (e.key === ' ') { st.space = false; host.classList.remove('mk-space'); } };
    host.addEventListener('focusin', (e) => { if (e.target.classList.contains('fg-kit-search')) searchKit(e.target.value); });
    host.addEventListener('input', (e) => { if (e.target.classList.contains('fg-kit-search')) { st.kitPick = ''; searchKit(e.target.value); } });
    host.addEventListener('keydown', (e) => {
      if (!e.target.classList.contains('fg-kit-search')) return;
      e.stopPropagation();
      if (e.key === 'Escape') { q('.fg-kit-list').hidden = true; st.kitPick = ''; e.target.value = ''; design.setTool('move'); e.target.blur(); }
      if (e.key === 'Enter') { const first = q('.fg-kit-list [data-kit-pick]'); if (first) pickKit(first.dataset.kitPick); }
    });
    host.addEventListener('pointerdown', (e) => { if (!e.target.closest('.fg-kit')) q('.fg-kit-list').hidden = true; }, true);
    host.addEventListener('click', (e) => {
      const pick = e.target.closest('[data-kit-pick]');
      if (pick) { pickKit(pick.dataset.kitPick); return; }
      onMissing(e);
    });

    // ---- requests applied to the draft, in one go ----------------------------------------
    async function applyRequests(notes) {
      if (!st.engine || !st.frame) return;
      const asked = notes.filter((n) => (n.text || '').trim()).map((n) => ({ id: n.id, text: n.text, target: n.target || '' }));
      if (!asked.length) { say('nothing to apply: write the intention first'); return; }
      const board = boardOf();
      const action = window.HarnessActions && window.HarnessActions.start({ key: `apply:${board}`, label: `apply ${asked.length} request(s) to the draft of ${st.page}`, button: q('[data-ed="apply-all"]') });
      say(`<span class="spin"></span> the AI applies ${asked.length} request(s)…`);
      try {
        const summary = components().map((c) => `- ${c.name} (${c.system || '?'}): ${c.description || ''}${c.needs.length ? ' · needs ' + c.needs.join(', ') : ''}${c.actions.length ? ' · triggers ' + c.actions.join(', ') : ''}`).join('\n');
        const res = await fetch(`/api/notes/${encodeURIComponent(board)}/apply`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ page: st.page, html: st.engine.materialized(), components: summary, requests: asked }) });
        const text = await res.text();
        if (!res.ok) throw new Error(text);
        const got = JSON.parse(text);
        const ops = (got.ops || []).filter((o) => !o.skip);
        const skipped = (got.ops || []).filter((o) => o.skip);
        // A request applied again: its former operations go, the new ones come.
        const ids = new Set(asked.map((a) => a.id));
        st.engine.setOps(st.engine.ops.filter((o) => !ids.has(o.by)));
        if (ops.length && !st.engine.push(ops).ok) throw new Error('the operations found no target on the page');
        afterChange();
        asked.forEach((a) => { if (!skipped.some((s) => s.by === a.id)) st.board.patch(a.id, { state: 'simulated' }); });
        offerSkipped(skipped);
        if (action) action.ok(`${ops.length} operation(s)${skipped.length ? `, ${skipped.length} left to the human` : ''}`);
        drawBar();
      } catch (e) {
        if (action) action.fail(e.message || e);
        say(`not applied: ${esc(String(e.message || e).split('\n')[0])}`);
      }
    }
    const applyAll = () => applyRequests((st.board ? st.board.all() : []).filter((n) => n.kind === 'request' && (n.state || 'pending') !== 'simulated'));

    // ---- the draft's versions ---------------------------------------------------------------
    async function draftCall(path, body) {
      const board = boardOf();
      try {
        const res = await fetch(`/api/drafts/${encodeURIComponent(board)}/${path}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body || {}) });
        if (!res.ok) throw new Error(await res.text());
        st.history = await res.json();
        if (boardOf() !== board) return;
        const current = st.history.current;
        if (current) { st.engine.setOps(current.ops); if (current.viewport) st.width = current.viewport; }
        st.viewing = null;
        render();
        drawHistory();
      } catch (e) { say(`draft: ${esc(e.message || e)}`); }
    }
    async function keepVersion() {
      clearTimeout(st.saveTimer);
      await fetch(`/api/drafts/${encodeURIComponent(boardOf())}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ops: st.engine.ops, viewport: st.width, name: (st.history && st.history.current && st.history.current.name) || '' }) }).catch(() => {});
      await draftCall('keep');
      await draftCall('open', { base: (currentPage() || {}).hash || '' });
    }
    async function validateVersion() {
      clearTimeout(st.saveTimer);
      await fetch(`/api/drafts/${encodeURIComponent(boardOf())}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ops: st.engine.ops, viewport: st.width, name: (st.history && st.history.current && st.history.current.name) || '' }) }).catch(() => {});
      const summary = draftSummary();
      await materialize();
      await draftCall('validate');
      // The page now embodies the draft: the branch is read again, and the
      // next version opens on the page as written, empty.
      clearTimeout(st.timer);
      st.stamp = '';
      await load();
      if (summary && st.board) st.board.newNote({ kind: 'request', target: '', text: summary, state: 'simulated' });
    }
    // The draft in words: what the human did to the page, for the pin that
    // sums a validated version up and the issue it becomes.
    function draftSummary() {
      if (!st.engine || !st.engine.ops.length) return '';
      const page = currentPage();
      const current = st.history && st.history.current;
      const ops = st.engine.ops;
      const name = (t) => (t ? describe(String(t).replace(/^col:(.+)#\d+$/, '$1')) : '');
      const lines = [];
      const human = ops.filter((o) => o.by === 'human');
      const byAi = ops.filter((o) => o.by !== 'human');
      const count = (kind) => human.filter((o) => o.kind === kind);
      if (count('move').length) lines.push(`moved: ${[...new Set(count('move').map((o) => name(o.target)))].slice(0, 8).join(', ')}`);
      if (count('resize').length) lines.push(`resized: ${[...new Set(count('resize').map((o) => name(o.target)))].slice(0, 8).join(', ')}`);
      if (count('remove').length) lines.push(`removed: ${[...new Set(count('remove').map((o) => name(o.target)))].slice(0, 8).join(', ')}`);
      if (count('clone').length) lines.push(`duplicated: ${[...new Set(count('clone').map((o) => name(o.target)))].slice(0, 8).join(', ')}`);
      if (count('text').length) lines.push(`rewritten: ${count('text').map((o) => `${name(o.target)} → “${String(o.text || '').slice(0, 40)}”`).slice(0, 6).join('; ')}`);
      if (count('add').length) lines.push(`added: ${count('add').map((o) => `${o.what}${o.label ? ' “' + String(o.label).slice(0, 30) + '”' : ''}${o.intent ? ' (' + String(o.intent).slice(0, 60) + ')' : ''}`).join(', ')}`);
      if (count('static').length) lines.push(`laid out freely: ${[...new Set(count('static').filter((o) => o.layout !== 'flow').map((o) => name(o.target)))].join(', ')}`);
      const forked = ops.filter((o) => o.kind === 'fork');
      if (forked.length) lines.push(`forked micro-frontends: ${forked.map((o) => `${o.from} → ${o.as}`).join(', ')}`);
      const requests = (st.board ? st.board.all() : []).filter((n) => n.kind === 'request' && (n.text || '').trim() && n.target !== '');
      if (byAi.length) lines.push(`${byAi.length} operation(s) applied from ${new Set(byAi.map((o) => o.by)).size} request(s)`);
      const head = `Draft v${current ? current.version : '?'} of the page ${page ? page.title : st.page} validated (${ops.length} operation${ops.length === 1 ? '' : 's'}, branch ${st.branch})`;
      const tail = requests.length ? '\n\nRequests on the page:\n' + requests.map((n) => `- ${(n.text || '').split('\n')[0]} [${n.state || 'pending'}]`).join('\n') : '';
      return `${head}\n${lines.map((l) => '- ' + l).join('\n')}${tail}\n\nThe page as drafted, its operations and its forks are in mock/drafts/${pageSlug(page ? page.path : st.page)}.json on the mockup/* branch saved from this mock-up.`;
    }
    function drawHistory() {
      const panel = q('.mk-history');
      if (panel.hidden || !st.history) return;
      const when = (at) => (at ? window.HarnessClock.stamp(at).slice(0, 16) : '');
      const row = (d, current) => `<li class="mk-version ${current ? 'current' : ''} ${d.state}" data-version="${d.version}">`
        + `<b>v${d.version}${d.name ? ' · ' + esc(d.name) : ''}</b><small>${esc(d.state)}${current ? ' · now' : ''} · ${d.ops.length} op${d.ops.length === 1 ? '' : 's'} · ${esc(when(d.closed_at || d.created_at))}</small>`
        + (current ? `<input class="mk-version-name" value="${esc(d.name)}" placeholder="a name for this version" maxlength="80">` : `<div class="mk-version-actions"><button type="button" class="mini" data-ed="view" data-version="${d.version}">${st.viewing && st.viewing.version === d.version ? 'back to now' : 'view'}</button><button type="button" class="mini" data-ed="restore" data-version="${d.version}">restore as latest</button><button type="button" class="mini" data-ed="${d.state === 'validated' ? 'unvalidate' : 'validate-kept'}" data-version="${d.version}">${d.state === 'validated' ? 'unvalidate' : 'validate'}</button></div>`)
        + '</li>';
      panel.innerHTML = '<div class="mk-panel-head"><b>versions</b><button type="button" class="mini" data-ed="history" aria-label="close">×</button></div><ul>'
        + (st.history.current ? row(st.history.current, true) : '<li class="muted">no open version</li>')
        + st.history.kept.map((d) => row(d, false)).join('')
        + '</ul><div class="muted">the last 3 kept versions stay beside the open one; validated ones always</div>';
    }
    function viewVersion(version) {
      if (st.viewing && st.viewing.version === version) { st.viewing = null; render(); drawHistory(); return; }
      const d = (st.history.kept || []).find((x) => x.version === version);
      if (!d) return;
      st.viewing = d;
      design.select([]);
      render();
      drawHistory();
    }
    function drawFrozen() {
      const bar = q('.mk-frozen');
      const page = currentPage();
      const current = st.history && st.history.current;
      const frozen = current && current.base && page && page.hash && current.base !== page.hash;
      bar.hidden = !frozen;
      if (!frozen) return;
      const failed = st.engine ? st.engine.failed.length : 0;
      bar.innerHTML = `<span>⚠ this draft was made on an earlier version of the page${failed ? ` — ${failed} of its operations find no part any more` : ' — its operations still find their parts'}.</span>`
        + '<button type="button" class="mini" data-ed="unfreeze">keep it on the page as it is</button>'
        + '<button type="button" class="mini" data-ed="reset">restart from the page as it is</button>';
    }
    async function resetDraft() {
      await draftCall('reset', { base: (currentPage() || {}).hash || '' });
      (st.board ? st.board.all() : []).filter((n) => n.kind === 'request' && n.state && n.state !== 'pending').forEach((n) => st.board.patch(n.id, { state: 'pending' }));
    }
    function drawEditBar() {
      const bar = q('.mk-editbar');
      bar.hidden = !(st.edit && st.view === 'page' && st.engine);
      q('[data-mk="edit"]').classList.toggle('on', st.edit);
      q('[data-mk="edit"]').setAttribute('aria-pressed', st.edit);
      host.classList.toggle('mk-editing', editing());
      if (editing() && !st.designOn) { st.designOn = true; design.attach(); } else if (!editing() && st.designOn) { st.designOn = false; design.detach(); }
      const slot = bar.hidden ? q('.dm-bar') : bar;
      if (q('.mk-name').parentElement !== slot) ['.mk-name', '[data-mk="save"]', '[data-mk="discard"]'].forEach((sel) => slot.appendChild(q(sel)));
      if (bar.hidden) return;
      bar.querySelector('[data-ed="undo"]').disabled = !st.engine.canUndo() || !!st.viewing;
      bar.querySelector('[data-ed="redo"]').disabled = !st.engine.canRedo() || !!st.viewing;
      const presets = bar.querySelector('.mk-presets');
      presets.value = PRESETS.some(([, w]) => w === st.width) ? String(st.width) : '0';
      bar.querySelector('.mk-width').value = st.width;
      const current = st.history && st.history.current;
      const pending = (st.board ? st.board.all() : []).filter((n) => n.kind === 'request' && (n.state || 'pending') !== 'simulated').length;
      bar.querySelector('[data-ed="apply-all"]').textContent = `simulate requests${pending ? ` (${pending})` : ''}`;
      bar.querySelector('.mk-width').hidden = presets.value !== '0';
      bar.querySelector('[data-ed="apply-all"]').disabled = !pending || !!st.viewing;
      bar.querySelector('.mk-draft-state').innerHTML = st.viewing
        ? `<span class="dm-src">viewing v${st.viewing.version} · read only</span>`
        : current ? `<span class="dm-src ${current.ops.length ? 'ok' : ''}">v${current.version} · ${st.engine.ops.length} op${st.engine.ops.length === 1 ? '' : 's'}${st.engine.failed.length ? ` · ${st.engine.failed.length} lost` : ''}</span>` : '<span class="dm-src bad">no version</span>';
    }
    function setEdit(on) {
      st.edit = on;
      drawEditBar();
      drawStrip();
    }
    function setWidth(w) {
      st.width = Math.max(320, Math.min(3000, Math.round(w)));
      const widths = kept(WIDTHS, {}) || {};
      widths[boardOf()] = st.width;
      keep(WIDTHS, widths);
      st.fitted = false;
      render();
      persist();
    }
    // The viewport handle: the page's right edge, dragged.
    const onViewportDown = (e) => {
      if (!e.target.classList.contains('mk-vp-handle') || e.button !== 0) return;
      e.stopPropagation(); e.preventDefault();
      st.vp = { startX: e.clientX, w0: st.width };
      q('.mk-board').setPointerCapture(e.pointerId);
    };
    const onViewportMove = (e) => {
      if (!st.vp) return;
      e.stopPropagation();
      const w = Math.max(320, Math.round((st.vp.w0 + (e.clientX - st.vp.startX) / st.board.zoom()) / 10) * 10);
      st.frame.style.width = w + 'px';
      q('.mk-width').value = w;
    };
    const onViewportUp = (e) => {
      if (!st.vp) return;
      e.stopPropagation();
      const w = Math.max(320, Math.round((st.vp.w0 + (e.clientX - st.vp.startX) / st.board.zoom()) / 10) * 10);
      st.vp = null;
      setWidth(w);
    };

    // ---- saving: the page as the draft shows it, the draft's file, the forks -------------
    async function materialize() {
      if (!st.engine || !currentPage()) return;
      const page = currentPage();
      const slug = pageSlug(page.path);
      const write = (path, text) => fetch('/api/mockup/write', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ branch: st.branch, path, text }) }).then(async (r) => { if (!r.ok) throw new Error(await r.text()); });
      const requests = (st.board ? st.board.all() : []).filter((n) => n.kind === 'request').map((n) => ({ id: n.id, target: n.target, text: n.text, state: n.state || 'pending' }));
      const forked = forks().map((f) => ({ from: f.from, as: f.as }));
      const current = st.history && st.history.current;
      await write(page.path, st.engine.materialized());
      await write(`drafts/${slug}.json`, JSON.stringify({ page: page.path, version: current ? current.version : 0, name: current ? current.name : '', viewport: st.width, ops: st.engine.ops.map((o) => (o.kind === 'fork' ? Object.assign({}, o, { html: undefined }) : o)), forks: forked, requests }, null, 2) + '\n');
      for (const f of forks()) await write(`components/${f.as}.html`, (f.html || '') + '\n');
      if (forked.length) {
        const manifest = await fetch(`/mock/${encodeURIComponent(slugOf(st.branch))}/pages.json?v=${Date.now()}`).then((r) => (r.ok ? r.json() : { version: 1, pages: [], components: [] }));
        manifest.components = manifest.components || [];
        forked.forEach((f) => {
          if (manifest.components.some((c) => c.name === f.as)) return;
          const origin = manifest.components.find((c) => c.name === f.from) || {};
          manifest.components.push({ name: f.as, system: origin.system || 'page', description: `${f.from} as the page ${page.title} wants it`, fragment: `components/${f.as}.html`, needs: origin.needs || [], actions: origin.actions || [], props: origin.props || {}, variantOf: f.from });
        });
        const entry = (manifest.pages || []).find((p) => p.path === page.path);
        if (entry) entry.components = [...new Set([...(entry.components || []).filter((n) => !forked.some((f) => f.from === n)), ...forked.map((f) => f.as)])];
        await write('pages.json', JSON.stringify(manifest, null, 2) + '\n');
      }
    }

    // ---- the micro-frontends view: one card per micro-frontend ------------------------
    function mountCards() {
      const list = components();
      const per = Math.max(1, Math.floor((DEFAULT_WIDTH + CARD_GAP) / (CARD_W + CARD_GAP)));
      if (!list.length) {
        st.board.content.innerHTML = '<div class="mk-empty" data-pin="components:none"><h2>No micro-frontend yet</h2><p>A micro-frontend is a Micro-UI as the mock shows it: a fragment on a fixture shaped like what its Capabilities answer. <b>+ micro-frontend</b> above makes the first one from its spec.</p></div>';
        st.board.setSize(DEFAULT_WIDTH, MIN_HEIGHT);
        st.board.fit();
        return;
      }
      st.board.content.innerHTML = list.map((c, i) => {
        const x = (i % per) * (CARD_W + CARD_GAP);
        const y = Math.floor(i / per) * (CARD_MIN + 70 + CARD_GAP);
        return `<div class="mk-card" data-pin="component:${esc(c.name)}" data-card="${esc(c.name)}" style="left:${x}px;top:${y}px;width:${CARD_W}px;--mf:${c.colour}">`
          + `<div class="mk-card-head"><b>${esc(c.name)}</b><span class="mk-card-sys">${esc(c.system || '')}</span>`
          + `<span class="mk-card-tags">${c.variantOf ? `<i title="forked from ${esc(c.variantOf)} by this page's draft">fork of ${esc(c.variantOf)}</i>` : ''}${c.mocked ? (c.source ? '<i class="ok" title="the real code, bundled on the mock server">real code</i>' : '<i class="ok">mocked</i>') : '<i class="bad">not mocked</i>'}${c.built ? '<i class="ok">built</i>' : '<i>to build</i>'}</span></div>`
          + `<div class="mk-card-desc">${esc(c.description || '')}</div>`
          + `<div class="mk-card-meta">${c.needs.length ? 'needs ' + c.needs.map(esc).join(', ') : 'needs nothing'}${c.actions.length ? ' · triggers ' + c.actions.map(esc).join(', ') : ''}${c.pages.length ? ' · on ' + c.pages.map(esc).join(', ') : ' · on no page'}${c.source ? ` · from ${esc(c.source)}` : ''}</div>`
          + (c.mocked && !c.variantOf
            ? `<div class="mk-card-body"><iframe class="mk-preview" title="${esc(c.name)}" src="${esc(previewSrc(c.name))}" style="height:${CARD_MIN}px"></iframe><div class="mk-shield"></div></div>`
            : c.variantOf ? '<div class="mk-card-body mk-card-none">a copy made on a page: shown there, saved with the mock-up</div>'
              : `<div class="mk-card-body mk-card-none">no fragment yet${c.built ? ' — the Micro-UI exists under apps/, its mock does not' : ''}<br><button type="button" class="mini on" data-mock-card="${esc(c.name)}">mock it with an AI session</button></div>`)
          + '</div>';
      }).join('');
      st.cards = [...st.board.content.querySelectorAll('.mk-card')].map((card) => ({ name: card.dataset.card, card, frame: card.querySelector('.mk-preview') }));
      st.cards.forEach((c) => { if (c.frame) c.frame.addEventListener('load', measureCards); });
      layoutCards();
      st.board.fit();
      st.fitted = true;
    }
    function measureCards() {
      let changed = false;
      st.cards.forEach((c) => {
        if (!c.frame) return;
        let doc = null;
        try { doc = c.frame.contentDocument; } catch (e) { doc = null; }
        if (!doc || !doc.body) return;
        const h = Math.min(CARD_MAX, Math.max(CARD_MIN, doc.documentElement.scrollHeight, doc.body.scrollHeight));
        if (Math.abs(h - (parseFloat(c.frame.style.height) || 0)) > 2) { c.frame.style.height = h + 'px'; changed = true; }
      });
      if (changed) layoutCards();
    }
    function layoutCards() {
      const per = Math.max(1, Math.floor((DEFAULT_WIDTH + CARD_GAP) / (CARD_W + CARD_GAP)));
      let y = 0; let rowH = 0; let maxX = 0;
      st.cards.forEach((c, i) => {
        if (i % per === 0 && i) { y += rowH + CARD_GAP; rowH = 0; }
        const x = (i % per) * (CARD_W + CARD_GAP);
        c.card.style.left = x + 'px'; c.card.style.top = y + 'px';
        rowH = Math.max(rowH, c.card.offsetHeight);
        maxX = Math.max(maxX, x + CARD_W);
      });
      st.board.setSize(Math.max(DEFAULT_WIDTH, maxX), Math.max(MIN_HEIGHT, y + rowH + 20));
      st.board.refresh();
    }

    // ---- the strip: the micro-frontends to drop on the page ------------------------------
    function drawStrip() {
      const strip = q('.mk-strip');
      strip.hidden = st.view !== 'page' || !st.state || (editing() && !q('.mk-panel').hidden);
      if (strip.hidden) return;
      const list = components();
      strip.innerHTML = '<div class="mk-strip-head">micro-frontends <small>drag one onto the page</small></div>'
        + (list.length ? list.map((c) => `<div class="mk-chip" data-chip="${esc(c.name)}" style="--mf:${c.colour}" title="${esc(c.description || c.name)}${c.mocked ? '' : ' — not mocked yet: its session makes the fragment first'}"><i></i><b>${esc(c.name)}</b><small>${esc(c.system || '')}${c.pages.length ? ' · ' + c.pages.length + ' page' + (c.pages.length > 1 ? 's' : '') : ''}</small></div>`).join('') : '<div class="hb-empty">none yet</div>')
        + '<button type="button" class="mini" data-mk="design">+ micro-frontend</button>';
    }
    const onStripDown = (e) => {
      const chip = e.target.closest('[data-chip]');
      if (!chip || e.button !== 0 || !st.board) return;
      const c = componentOf(chip.dataset.chip);
      if (!c) return;
      st.board.carryFrom(e, { kind: 'design', component: c.name, text: specOf(c) });
    };

    // ---- the form: a new micro-frontend ----------------------------------------------------
    function drawForm(open) {
      const form = q('.mk-form');
      form.hidden = !open;
      if (!open) return;
      const pick = (name, offered) => (offered.length
        ? offered.map((id) => `<label><input type="checkbox" name="${name}" value="${esc(id)}"> ${esc(id)}</label>`).join('')
        : '<span class="hb-empty">none declared on this branch</span>');
      form.innerHTML = '<div class="mk-form-row"><input name="name" maxlength="80" placeholder="Name, like RiskBadge" aria-label="name"><input name="system" maxlength="40" placeholder="system" aria-label="system"><input name="description" maxlength="300" placeholder="What it shows and lets the user do" aria-label="description"></div>'
        + `<div class="mk-form-row"><div class="mk-form-pick"><b>needs</b>${pick('needs', capabilities())}</div><div class="mk-form-pick"><b>actions</b>${pick('actions', dataCapabilities())}</div><textarea name="props" rows="2" placeholder="props, one per line: customerId: ID"></textarea></div>`
        + '<div class="mk-form-row"><span class="mk-form-state"></span><span class="spacer"></span><button type="button" class="mini" data-mk="form-cancel">cancel</button><button type="button" class="mini on" data-mk="form-make">Make it with an AI session</button></div>';
      form.querySelector('[name="name"]').focus();
    }
    function makeFromForm() {
      const form = q('.mk-form');
      const v = (n) => (form.querySelector(`[name="${n}"]`) || {}).value || '';
      const name = v('name').trim();
      if (!/^[A-Z][A-Za-z0-9]*$/.test(name)) { form.querySelector('.mk-form-state').textContent = 'name it like a Micro-UI: RiskBadge'; return; }
      const spec = {
        name, system: v('system').trim(), description: v('description').trim(),
        needs: [...form.querySelectorAll('[name="needs"]:checked')].map((i) => i.value),
        actions: [...form.querySelectorAll('[name="actions"]:checked')].map((i) => i.value),
        props: Object.fromEntries(v('props').split('\n').map((l) => l.split(':')).filter((p) => p[0] && p[0].trim()).map((p) => [p[0].trim(), (p[1] || '').trim()])),
      };
      drawForm(false);
      if (st.view !== 'components') { setView('components'); }
      const id = st.board.newNote({ kind: 'design', component: name, text: specOf(spec), target: '' });
      st.board.session(id);
    }

    // ---- reading -----------------------------------------------------------------------
    async function load() {
      clearTimeout(st.timer);
      if (st.gone) return;
      let reply = null;
      try {
        const [versions, state] = await Promise.all([
          fetch('/api/data-model/versions').then((r) => (r.ok ? r.json() : null)).catch(() => null),
          fetch('/api/mockup?' + new URLSearchParams({ branch: st.branch })).then((r) => (r.ok ? r.json() : null)),
        ]);
        st.versions = versions;
        reply = state;
      } catch (e) { reply = null; }
      if (st.gone) return;
      if (!reply) { say('The view did not answer.'); st.timer = setTimeout(load, 3000); return; }
      if (!reply.enabled) { say(esc(reply.error || 'Nothing reads the product.')); return; }
      st.branch = reply.branch;
      drawSwitch();
      if (reply.error) { say(`<span class="dm-src bad" title="${esc(reply.error)}">unreadable</span> ${esc(reply.error)}`); st.timer = setTimeout(load, 5000); return; }
      const before = st.state;
      st.state = reply;
      const pages = reply.pages || [];
      if (opts && opts.pageSlug && !st.page) {
        if (opts.pageSlug === 'micro-frontends') st.view = 'components';
        else { const found = pages.find((p) => pageSlug(p.path) === opts.pageSlug); if (found) st.page = found.path; }
      }
      if (!st.page || !pages.some((p) => p.path === st.page)) {
        const last = remembered(st.branch);
        st.page = pages.some((p) => p.path === last) ? last : (pages[0] ? pages[0].path : '');
      }
      drawBar();
      drawStrip();
      const listChanged = before && JSON.stringify(before.components) + JSON.stringify(before.units) !== JSON.stringify(reply.components) + JSON.stringify(reply.units);
      const pageChanged = !before || !st.board || st.board.id !== boardOf();
      if (pageChanged || (st.view === 'components' && listChanged)) { st.stamp = reply.stamp; mountBoard(); }
      else if (reply.stamp !== st.stamp && !st.drag && !st.resize) { st.stamp = reply.stamp; reloadFrame(); }
      else if (listChanged) outline();
      st.timer = setTimeout(load, POLL_MS);
    }
    function say(html) { q('.mk-origin').innerHTML = html; }
    function drawSwitch() {
      const v = st.versions;
      const sw = q('.dm-switch');
      sw.hidden = !(v && v.branches && v.branches.length);
      if (sw.hidden) return;
      const of = (kind) => v.branches.filter((b) => b.kind === kind);
      const current = v.branches.find((b) => b.branch === st.branch);
      const button = (b, tip) => `<button type="button" class="${current && current.branch === b.branch ? 'on' : ''}" data-branch="${esc(b.branch)}" title="${esc(tip)}">${esc(b.label)}</button>`;
      const features = of('feature');
      const featureOn = current && current.kind === 'feature';
      q('.dm-branches').innerHTML = of('release').map((b) => button(b, 'what a human released')).join('')
        + of('integration').map((b) => button(b, 'where the agents integrate')).join('')
        + `<button type="button" class="${featureOn ? 'on' : ''}" data-mk="features" ${features.length ? '' : 'disabled'} aria-haspopup="menu" title="a milestone's branch">${featureOn ? esc(current.label) : 'feature'} ▾</button>`;
      q('.dm-features').innerHTML = features.map((b) => `<button type="button" role="menuitem" data-branch="${esc(b.branch)}">${esc(b.label)}</button>`).join('');
    }
    function drawBar() {
      const s = st.state;
      const pages = s.pages || [];
      const page = currentPage();
      const n = components().length;
      say(`<span class="dm-src ${s.present ? 'ok' : ''}" title="mock/pages.json on ${esc(st.branch)}">${s.present ? `${pages.length} page${pages.length === 1 ? '' : 's'} · ${n} micro-frontend${n === 1 ? '' : 's'}` : 'no mock-up yet'}</span>`
        + (st.view === 'page' && page ? ` <span class="mk-objective" title="${esc(page.objective)}">${esc(page.title)}${page.screen ? ` · screen ${esc(page.screen)}` : ''}</span>` : ''));
      host.querySelectorAll('.mk-views button').forEach((b) => b.classList.toggle('on', b.dataset.view === st.view));
      host.querySelectorAll('.mk-only-page').forEach((b) => { b.hidden = st.view !== 'page'; });
      host.querySelectorAll('.mk-only-components').forEach((b) => { b.hidden = st.view !== 'components'; });
      const label = q('.mk-page');
      label.hidden = st.view !== 'page' || !pages.length;
      q('.mk-pages').innerHTML = pages.map((p) => `<option value="${esc(p.path)}"${p.path === st.page ? ' selected' : ''}>${esc(p.title)}</option>`).join('');
      q('[data-mk="mf"]').classList.toggle('on', st.mf);
      q('[data-mk="mf"]').setAttribute('aria-pressed', st.mf);
      const state = q('.mk-state');
      state.innerHTML = s.edited ? '<span class="dm-src bad">edited, not saved</span>' : s.saved ? `<span class="dm-src ok">saved as ${esc(s.saved)}</span>` : '';
      const dirty = s.edited || !!(st.engine && st.engine.ops.length);
      const inEditBar = st.edit && st.view === 'page' && !!st.engine;
      q('.mk-name').hidden = !(dirty || inEditBar);
      q('[data-mk="save"]').hidden = !(dirty || inEditBar);
      q('[data-mk="save"]').disabled = !dirty;
      q('[data-mk="discard"]').hidden = !(s.edited || s.saved);
      q('[data-mk="discard"]').textContent = st.armed === 'discard' ? 'discard — sure?' : 'discard';
      drawEditBar();
    }
    function leavePage() {
      // The open version closes with the page: the next change opens a new one.
      if (!st.engine || !st.history || !st.history.current) return;
      const board = boardOf();
      clearTimeout(st.saveTimer);
      const body = JSON.stringify({ ops: st.engine.ops, viewport: st.width, name: st.history.current.name || '' });
      fetch(`/api/drafts/${encodeURIComponent(board)}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body, keepalive: true }).catch(() => {});
      if (st.engine.ops.length) fetch(`/api/drafts/${encodeURIComponent(board)}/keep`, { method: 'POST', keepalive: true }).catch(() => {});
    }
    function show(path) {
      if (path === st.page && st.board && st.view === 'page') return;
      leavePage();
      st.page = path;
      remember(st.branch, path);
      st.stamp = st.state ? st.state.stamp : st.stamp;
      drawBar();
      mountBoard();
    }
    function setView(view) {
      if (view === st.view) return;
      leavePage();
      st.view = view;
      drawForm(false);
      q('.mk-history').hidden = true;
      if (st.state) { drawBar(); drawStrip(); mountBoard(); }
    }
    function showBranch(branch) {
      if (branch === st.branch) return;
      leavePage();
      st.branch = branch;
      st.page = '';
      st.state = null;
      q('.dm-features').hidden = true;
      load();
    }
    function newPage() {
      if (st.board && st.board.newLink) st.board.newLink();
    }
    async function save() {
      if (st.saving) return;
      const name = q('.mk-name').value.trim() || (currentPage() ? currentPage().title : st.branch);
      st.saving = true;
      const action = window.HarnessActions && window.HarnessActions.start({ key: `mockup:save:${st.branch}`, label: `save the mock-up of ${st.branch} as ${name}`, button: q('[data-mk="save"]') });
      try {
        if (st.engine && st.engine.ops.length) await materialize();
        const res = await fetch('/api/mockup/save', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ branch: st.branch, name }) });
        const text = await res.text();
        if (!res.ok) throw new Error(text);
        const saved = JSON.parse(text);
        if (action) action.ok(`pushed ${saved.branch}`);
        q('.mk-name').value = '';
      } catch (e) {
        if (action) action.fail(e.message || e);
        say(`not saved: ${esc(e.message || e)}`);
      } finally {
        st.saving = false;
        load();
      }
    }
    async function discard() {
      if (st.armed !== 'discard') { st.armed = 'discard'; drawBar(); setTimeout(() => { if (st.armed === 'discard') { st.armed = null; drawBar(); } }, 4000); return; }
      st.armed = null;
      const action = window.HarnessActions && window.HarnessActions.start({ key: `mockup:discard:${st.branch}`, label: `discard the edited mock-up of ${st.branch}`, button: q('[data-mk="discard"]') });
      try {
        const res = await fetch('/api/mockup/discard', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ branch: st.branch }) });
        if (!res.ok) throw new Error(await res.text());
        if (action) action.ok('the mock-up is as committed again');
      } catch (e) {
        if (action) action.fail(e.message || e);
      } finally {
        load();
      }
    }
    function setFull(on) {
      st.full = on;
      document.body.classList.toggle('full-pane', on);
      q('[data-mk="full"]').classList.toggle('on', on);
      q('[data-mk="full"]').setAttribute('aria-pressed', on);
      st.fitted = false;
      setTimeout(() => { if (st.board) st.board.fit(); design.refresh(); }, 50);
    }
    // Esc leaves the full window before anything else hears it.
    const onFullKey = (e) => { if (st.full && e.key === 'Escape' && !e.target.matches('input,textarea,select')) { e.stopPropagation(); e.preventDefault(); setFull(false); } };
    function setInteract(on) {
      st.interact = on;
      q('[data-mk="interact"]').setAttribute('aria-pressed', on);
      q('[data-mk="interact"]').classList.toggle('on', on);
      if (st.shield) st.shield.hidden = on;
    }
    function setOutlines(on) {
      st.mf = on;
      keep(MF_ON, on);
      drawBar();
      outline();
      design.select([]);
    }

    // ---- events --------------------------------------------------------------------------
    const onClick = (e) => {
      const branch = e.target.closest('[data-branch]');
      if (branch) { showBranch(branch.dataset.branch); return; }
      const ed = e.target.closest('[data-ed]');
      if (ed) { onEdit(ed, e); return; }
      const open = e.target.closest('[data-open-note]');
      if (open && st.board) { st.board.goTo(open.dataset.openNote); return; }
      const mk = e.target.closest('[data-mk]');
      if (!mk) return;
      if (mk.dataset.mk === 'features') { const m = q('.dm-features'); m.hidden = !m.hidden; }
      if (mk.dataset.mk === 'view') setView(mk.dataset.view);
      if (mk.dataset.mk === 'new') newPage();
      if (mk.dataset.mk === 'mf') setOutlines(!st.mf);
      if (mk.dataset.mk === 'edit') setEdit(!st.edit);
      if (mk.dataset.mk === 'interact') setInteract(!st.interact);
      if (mk.dataset.mk === 'full') setFull(!st.full);
      if (mk.dataset.mk === 'design') drawForm(q('.mk-form').hidden);
      if (mk.dataset.mk === 'form-cancel') drawForm(false);
      if (mk.dataset.mk === 'form-make') makeFromForm();
      if (mk.dataset.mk === 'save') save();
      if (mk.dataset.mk === 'discard') discard();
    };
    function onEdit(b, e) {
      const what = b.dataset.ed;
      const version = Number(b.dataset.version);
      if (what === 'undo' && st.engine.undo()) afterChange();
      if (what === 'redo' && st.engine.redo()) afterChange();
      if (what === 'apply-all') applyAll();
      if (what === 'keep') keepVersion();
      if (what === 'validate') validateVersion();
      if (what === 'history') { const h = q('.mk-history'); h.hidden = !h.hidden; drawHistory(); }
      if (what === 'view') viewVersion(version);
      if (what === 'restore') draftCall(`restore/${version}`);
      if (what === 'validate-kept') draftCall(`validate?version=${version}`);
      if (what === 'unvalidate') draftCall(`unvalidate?version=${version}`);
      if (what === 'unfreeze' && st.history && st.history.current) { st.history.current.base = (currentPage() || {}).hash || ''; drawFrozen(); persist(); }
      if (what === 'reset') resetDraft();
      if (what === 'rename-fork') renameForkTo(b.dataset.from, (q('.mk-fork-name') || {}).value || '');
      void e;
    }
    const onCardMock = (e) => {
      const b = e.target.closest('[data-mock-card]');
      if (!b || !st.board) return;
      const c = componentOf(b.dataset.mockCard);
      if (!c) return;
      const id = st.board.newNote({ kind: 'design', component: c.name, text: specOf(c), target: 'component:' + c.name });
      st.board.session(id);
    };
    const onChange = (e) => {
      if (e.target.classList.contains('mk-pages')) show(e.target.value);
      if (e.target.classList.contains('mk-presets')) { if (e.target.value !== '0') setWidth(Number(e.target.value)); else { q('.mk-width').hidden = false; q('.mk-width').focus(); } }
      if (e.target.classList.contains('mk-width')) setWidth(Number(e.target.value) || DEFAULT_WIDTH);
      if (e.target.classList.contains('mk-version-name') && st.history && st.history.current) { st.history.current.name = e.target.value.slice(0, 80); persist(); }
      if (e.target.classList.contains('mk-fork-name')) renameForkTo(e.target.dataset.from, e.target.value);
    };
    host.addEventListener('click', onClick);
    host.addEventListener('click', onCardMock);
    host.addEventListener('change', onChange);
    host.addEventListener('keydown', onKey);
    host.addEventListener('keyup', onKeyUp);
    q('.mk-strip').addEventListener('pointerdown', onStripDown);
    const boardHost = q('.mk-board');
    boardHost.addEventListener('pointerdown', onViewportDown, true);
    boardHost.addEventListener('pointermove', onViewportMove, true);
    boardHost.addEventListener('pointerup', onViewportUp, true);
    document.addEventListener('keydown', onKey);
    document.addEventListener('keyup', onKeyUp);
    window.addEventListener('keydown', onFullKey, true);
    const measured = setInterval(measure, 1000);

    load();

    return {
      get id() { return st.board ? st.board.id : ''; },
      // What a desk is told of the part under the pointer: a part of the page, or a card.
      contextAt(clientX, clientY, el) {
        let target = '';
        if (st.view === 'page') { const part = under(clientX, clientY); target = part ? part.getAttribute('data-pin') || '' : ''; }
        else { const card = el && el.closest ? el.closest('[data-pin]') : null; target = card ? card.getAttribute('data-pin') || '' : ''; }
        return { what: describe(target), text: context(target) };
      },
      reload() { st.state = null; return load(); },
      destroy() {
        leavePage();
        design.detach();
        st.gone = true;
        clearTimeout(st.timer);
        clearTimeout(st.saveTimer);
        clearInterval(measured);
        host.removeEventListener('click', onClick);
        host.removeEventListener('click', onCardMock);
        host.removeEventListener('change', onChange);
        host.removeEventListener('keydown', onKey);
        host.removeEventListener('keyup', onKeyUp);
        document.removeEventListener('keydown', onKey);
        document.removeEventListener('keyup', onKeyUp);
        window.removeEventListener('keydown', onFullKey, true);
        if (st.full) setFull(false);
        if (st.board) { try { st.board.destroy(); } catch (e) { /* gone */ } }
        host.innerHTML = '';
      },
    };
  }

  window.MockupPane = { mount };
})();
