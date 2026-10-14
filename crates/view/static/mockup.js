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

  const { DEFAULT_WIDTH, clamp: clampWidth } = window.Viewport;
  const MIN_HEIGHT = 480;
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
      + '<span class="mk-readonly" hidden>read-only · demo</span>'
      + '<label class="mk-page" hidden>page <select class="mk-pages" aria-label="page"></select></label>'
      + '<button type="button" class="mini mk-only-page" data-mk="new" title="A new page of the mock-up, drawn by an AI session from its objective">+ page</button>'
      + '<button type="button" class="mini mk-only-page" data-mk="mf" aria-pressed="false" title="Outline every micro-frontend in its own colour; pins and edits then take hold of micro-frontends">micro-frontends ◧</button>'
      + '<button type="button" class="mini mk-only-page" data-mk="edit" aria-pressed="false" title="Design the page: select, drag, resize, align, insert — every change a step of the page\'s draft">edit</button>'
      + '<button type="button" class="mini mk-only-page" data-mk="interact" aria-pressed="false" title="Click through to the page itself: its links, its buttons">interact</button>'
      + '<button type="button" class="mini" data-mk="full" aria-pressed="false" title="The mock-up over the whole window (Esc leaves)">⛶</button>'
      + '<button type="button" class="mini mk-only-components" data-mk="design" hidden title="A new micro-frontend, from its spec: generated, or made with an AI session">+ micro-frontend</button>'
      + '<button type="button" class="mini on mk-only-components" data-mk="generate-all" hidden title="Generate every micro-frontend not mocked yet, several at once — the kit makes what they lack first">generate</button>'
      + '<span class="mk-gens mk-only-components" hidden></span>'
      + '<span class="spacer"></span>'
      + '<span class="mk-state"></span>'
      + '<input class="mk-name" maxlength="80" placeholder="name of the mock-up" hidden aria-label="name of the mock-up">'
      + '<button type="button" class="mini" data-mk="save" hidden>save the mock-up</button>'
      + '<button type="button" class="mini" data-mk="discard" hidden>discard</button>'
      + '<button type="button" class="mini" data-mk="restore" hidden title="Bring back the mock-up as it was before the last discard">undo discard</button>'
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
      + window.Viewport.markup()
      + '<span class="mk-draft-state"></span>'
      + '<button type="button" class="mini" data-ed="keep" title="Close this version into the history; the next change opens a new one">keep</button>'
      + '<button type="button" class="mini" data-ed="validate" title="Close this version as the next round of development">validate</button>'
      + '<button type="button" class="mini" data-ed="history" title="The versions of this page\'s draft">history</button>'
      + '</div>'
      + '<div class="mk-frozen" hidden></div>'
      + '<div class="mk-form" hidden></div>'
      + '<div class="mk-mfbar kt-detail-bar" hidden></div>'
      + '<div class="mk-mfsave kt-save" hidden></div>'
      + '<div class="dm-main"><div class="mk-gallery-host kt-gallery-host" hidden></div><aside class="mk-layers" hidden></aside><div class="dm-board mk-board"></div><aside class="mk-rules" hidden></aside><aside class="mk-strip" hidden></aside><aside class="mk-panel" hidden></aside><aside class="mk-history" hidden></aside></div>'
      + '<div class="mk-toast" hidden></div>'
      + '</div>';
    const q = (s) => host.querySelector(s);
    // The demo never writes or spends a session: every mutating call this
    // pane makes (apply, save, discard, drafts, generate…) goes through
    // `fetch`, so shadowing it here — for this closure only — is the one
    // place that catches all of them, including the drag-to-edit autosave
    // timer and the unload `keepalive` calls, without touching each call site.
    const realFetch = window.fetch.bind(window);
    function fetch(url, init) {
      if (st.readOnly && init && init.method && init.method !== 'GET') return Promise.resolve(new Response('', { status: 403, statusText: 'read-only in the demo' }));
      return realFetch(url, init);
    }
    const st = { branch: (opts && opts.branch) || '', page: (opts && opts.page) || '', view: (opts && opts.view) || 'page', versions: null, state: null, stamp: '', pinned: false, readOnly: !!(opts && opts.readOnly),
      timer: null, gone: false, interact: false, mf: !!kept(MF_ON, false),
      // The micro-frontends' list beside the page (off where a step hides it); the one pointed at, on the page and in the list.
      strip: !(opts && opts.strip === false), hot: '',
      // A rule picked on the creation's rules step: only its micro-frontends listed, its parts marked on the page.
      focus: null, board: null, frame: null, shield: null, doc: null, armed: null, proxies: new WeakMap(), fitted: false, saving: false,
      cards: [],
      // Edit mode: the page's source, its draft engine, the versions, the selection, a gesture under way.
      edit: !!(opts && opts.edit), source: '', engine: null, history: null, sel: '', hover: null, drag: null, resize: null, space: false, saveTimer: null, viewing: null, width: DEFAULT_WIDTH, lastLoadedPage: '', toastTimer: null,
      // The design system's kit, for the search field of the tools; the component picked to drop.
      kit: null, kitPick: '',
      // The micro-frontends view: its gallery, the one opened alone, its mode, the part picked, the units read.
      gens: [], mfDrafts: [], mfApplying: {}, mfSaveOpen: false, mfGallery: null, mfDetail: (opts && opts.mf) || null, mfMode: 'use', mfPicked: null, mfTop: 0, mfH: 0, mfFitted: false, caps: null, capsAt: 0 };
    if (st.readOnly) {
      const ro = q('.mk-readonly');
      if (ro) ro.hidden = false;
      // The toolbar redraws parts of itself (the micro-frontends gallery's
      // own "+ micro-frontend" card, "save" appearing once something is
      // edited…) — a one-time pass would miss whatever it builds later, so
      // the lock re-applies on every change under the pane instead. The
      // fetch shim above is the actual safety net either way; this is the
      // UI telling the human the same thing before they click.
      const LOCK = '[data-mk="new"], [data-mk="design"], [data-mk="generate-all"], [data-mk="save"], [data-mk="discard"], [data-mk="restore"], [data-mk="edit"], [data-ed="apply-all"], [data-ed="keep"], [data-ed="validate"]';
      const lock = () => host.querySelectorAll(LOCK).forEach((b) => { b.disabled = true; });
      lock();
      new MutationObserver(lock).observe(host, { childList: true, subtree: true, attributes: true, attributeFilter: ['disabled', 'hidden'] });
      // Belt and braces: this pane's own async bootstrap (fetching the
      // branch's state, drawing the bar, the micro-frontends strip) sets
      // some of these buttons' disabled/hidden state itself, after this
      // mount() call returns — a poll for the pane's short lifetime catches
      // whatever that first render overwrites that the observer missed.
      let ticks = 0;
      const poll = setInterval(() => { lock(); if ((ticks += 1) > 20) clearInterval(poll); }, 300);
    }

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
        // A part hidden in the page hides its pins: the board asks the frame's own element.
        closest: (selector) => el.closest(selector),
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
      layoutView();
      const components = st.view === 'components';
      st.board = window.HarnessBoard.create(q('.mk-board'), {
        id: boardOf(),
        describe,
        // A page's parts are reached through its frame; a micro-frontend alone is one part, the board's own.
        resolve: components ? undefined : resolve,
        context,
        onSession,
        under: components ? undefined : under,
        // The micro-frontend opened alone shows its own pins.
        only: components ? (n) => !n.target || n.target === 'component:' + st.mfDetail || n.target.startsWith('component:' + st.mfDetail + '>') : undefined,
        kinds: st.view === 'page' ? ['note', 'request', 'link', 'design'] : ['note', 'request', 'design'],
        zones: st.view === 'page',
        // A page redrawn leaves pins whose part is gone: put aside, not left floating.
        detach: st.view === 'page',
        pages: () => (st.state && st.state.pages) || [],
        capabilities,
        dataCapabilities,
        apply: st.view === 'page' ? (note) => applyRequests([note]) : (note) => applyMf(note.target ? note.target.slice(10).split('>')[0] : '', [note]),
        onChange: () => { if (st.view === 'components' && st.mfDetail) drawMfBar(); },
        backlogContext: (note) => {
          const page = currentPage();
          let text = page ? `Page: ${page.title} (\`${page.path}\`) on \`${st.branch}\`${page.objective ? ' — ' + page.objective : ''}` : `Branch: ${st.branch}`;
          if (note.target && note.target !== '') text += `\nPart: ${describe(note.target)}`;
          if (note.kind !== 'request' || note.target !== '') { const summary = draftSummary(); if (summary) text += '\n\n' + summary; }
          return text;
        },
        goto: (path) => { setView('page'); show(path); },
        onLoaded: () => {
          // A pin asked for on a micro-frontend opens it first.
          if (opts && opts.pin && !st.pinned && st.board && st.view === 'components' && !st.mfDetail) {
            const n = st.board.all().find((x) => x.id === opts.pin);
            const name = n && n.target && n.target.startsWith('component:') ? n.target.slice(10).split('>')[0] : '';
            if (name && componentOf(name)) openMf(name);
          }
          if (opts && opts.pin && !st.pinned && st.board && st.board.goTo(opts.pin)) st.pinned = true;
          design.refresh();
        },
        bounds: () => (st.view === 'page'
          ? { x: 0, y: 0, w: st.width, h: Math.max(MIN_HEIGHT, st.frame ? parseFloat(st.frame.style.height) || MIN_HEIGHT : MIN_HEIGHT) }
          : { x: 0, y: 0, w: MF_W, h: st.mfH || MIN_HEIGHT }),
      });
      if (st.view === 'components') { mountMf(); return; }
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
      st.hot = '';
      marksFocus();
      // In interact mode the pointer is in the frame itself: the part under it lights its entry.
      doc.addEventListener('pointermove', (ev) => { const el = ev.target && ev.target.closest ? ev.target.closest('[data-component]') : null; setHot(el ? el.dataset.component : ''); });
      doc.addEventListener('pointerleave', () => setHot(''));
      drawStrip();
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
        + '.mf-focus[data-component]{outline:2px dashed #4ea3f0 !important;outline-offset:3px}'
        + '.mf-hot[data-component]{outline:3px solid #e2b240 !important;outline-offset:4px;box-shadow:0 0 0 4px rgba(226,178,64,.18),0 0 24px rgba(226,178,64,.35) !important}'
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
      if (st.view === 'components') { if (st.mfDetail) sizeMf(); return; }
      if (!st.doc || !st.frame) return;
      const h = Math.max(MIN_HEIGHT, st.doc.documentElement.scrollHeight, st.doc.body ? st.doc.body.scrollHeight : 0);
      if (Math.abs(h - (parseFloat(st.frame.style.height) || 0)) > 2) {
        st.frame.style.height = h + 'px';
        st.board.setSize(st.width, h);
        design.refresh();
      }
    }
    function reloadFrame() {
      if (st.view === 'components') { mountMf(); return; }
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
    const applyAll = () => {
      const pins = (st.board ? st.board.all() : []).filter((n) => n.kind === 'request' && (n.state || 'pending') !== 'simulated' && (n.text || '').trim());
      if (st.board && st.board.track) st.board.track(pins.map((n) => n.id), applyRequests(pins)); else applyRequests(pins);
    };

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
      if (q('.mk-name').parentElement !== slot) ['.mk-name', '[data-mk="save"]', '[data-mk="discard"]', '[data-mk="restore"]'].forEach((sel) => slot.appendChild(q(sel)));
      if (bar.hidden) return;
      bar.querySelector('[data-ed="undo"]').disabled = !st.engine.canUndo() || !!st.viewing;
      bar.querySelector('[data-ed="redo"]').disabled = !st.engine.canRedo() || !!st.viewing;
      window.Viewport.sync(bar, st.width);
      const current = st.history && st.history.current;
      const pending = (st.board ? st.board.all() : []).filter((n) => n.kind === 'request' && (n.state || 'pending') !== 'simulated').length;
      bar.querySelector('[data-ed="apply-all"]').textContent = `simulate requests${pending ? ` (${pending})` : ''}`;
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
      st.width = clampWidth(w);
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

    // ---- the micro-frontends view: a gallery, and one micro-frontend alone with its rules ----------
    // The same rules as the design system's kit: a gallery that only scrolls
    // down (HarnessGallery), a micro-frontend opened alone on the board —
    // used as a visitor would, or inspected — and the gallery found where it
    // was left. Beside it, its rules: what each actuator triggers (a
    // DataCapability: its effect, what it sends, its invariants) and what
    // each part shows (a Capability: what it is handed, what it returns),
    // read from `/api/capabilities`. A part is tied to its unit by the
    // markup's own words — `data-writes="<DataCapability>"` on what writes,
    // `data-reads="<Capability>"` on what shows (contract N) — and a kit
    // component inside it opens on the design system's easel on a long press.
    const MF_W = 960;
    const hasDraft = (name) => (st.mfDrafts || []).includes(name);
    // A micro-frontend as the page shows it: its draft while it has one.
    const mfSrc = (name) => (hasDraft(name)
      ? `/mock/${encodeURIComponent(slugOf(st.branch))}/__preview/${encodeURIComponent(name)}?draft=1&v=${encodeURIComponent(st.stamp)}`
      : previewSrc(name));
    const LONG_PRESS = 550;
    const lower = (s) => String(s || '').toLowerCase();
    function mfFoot(c) {
      const tags = [
        c.variantOf ? `<i>fork of ${esc(c.variantOf)}</i>` : '',
        genOf(c.name) && genBusy(genOf(c.name)) ? `<i class="acc"><span class="spin"></span> ${esc(genLabel(genOf(c.name)))}</i>` : '',
        genOf(c.name) && genOf(c.name).step === 'done' && genOf(c.name).why ? `<i class="bad" title="${esc(genOf(c.name).why)}">generated, untagged parts</i>` : '',
        c.mocked ? (c.source ? '<i class="acc">real code</i>' : genOf(c.name) && genOf(c.name).step === 'done' ? '<i class="acc">generated</i>' : '<i>hand-written</i>') : '<i class="bad">not mocked</i>',
        hasDraft(c.name) ? '<i class="bad">draft</i>' : '',
        mfPending(c.name).length ? `<i class="acc">${mfPending(c.name).length} request${mfPending(c.name).length === 1 ? '' : 's'}</i>` : '',
        c.actions.length ? `<i>${c.actions.length} write${c.actions.length === 1 ? '' : 's'}</i>` : '',
        c.needs.length ? `<i>${c.needs.length} read${c.needs.length === 1 ? '' : 's'}</i>` : '',
        c.pages.length ? '' : '<i class="bad">on no page</i>',
      ].join('');
      return `<b class="kt-tile-title">${esc(c.name)}</b><span class="mono kt-tile-code">${esc(c.system || '')}</span><span class="kt-tile-tags">${tags}</span>`;
    }
    function ensureMfGallery() {
      if (st.mfGallery) return st.mfGallery;
      st.mfGallery = window.HarnessGallery.create(q('.mk-gallery-host'), {
        placeholder: 'Find a micro-frontend — its name, its system, what it reads or writes',
        chips: [
          ['all', 'all'],
          ['real', 'real code', (c) => c.mocked && !!c.source],
          ['hand', 'hand-written', (c) => c.mocked && !c.source && !c.variantOf],
          ['unmocked', 'not mocked', (c) => !c.mocked],
          ['writes', 'writes', (c) => c.actions.length > 0],
          ['unused', 'on no page', (c) => !c.pages.length],
        ],
        words: (c) => [c.name, c.system, c.description, ...c.needs, ...c.actions],
        key: (c) => c.name,
        pin: (c) => 'component:' + c.name,
        src: (c) => (c.mocked && !c.variantOf ? mfSrc(c.name) : ''),
        body: (c) => (c.variantOf
          ? 'a copy made on a page: shown there, saved with the mock-up'
          : genOf(c.name) && genBusy(genOf(c.name))
            ? `<span class="spin"></span> ${esc(genLabel(genOf(c.name)))}`
            : `no fragment yet${c.built ? ' — the Micro-UI exists under apps/, its mock does not' : ''}${genOf(c.name) && genOf(c.name).step === 'failed' ? `<span class="bad">${esc(genOf(c.name).why)}</span>` : ''}<span><button type="button" class="mini on" data-mf="generate" data-name="${esc(c.name)}">generate it</button> <button type="button" class="mini" data-mock-card="${esc(c.name)}">AI session</button></span>`),
        foot: mfFoot,
        empty: (total, find, chip) => (total
          ? `<h2>No micro-frontend matches</h2><p>Nothing here is named like <b>${esc(find)}</b>${chip ? ` among the <b>${esc(chip)}</b>` : ''}.</p>`
          : '<h2>No micro-frontend yet</h2><p>A micro-frontend is a Micro-UI as the mock shows it: a fragment on a fixture shaped like what its Capabilities answer. <b>+ micro-frontend</b> above asks an AI session for one.</p>'),
        onOpen: (c) => openMf(c.name),
      });
      return st.mfGallery;
    }
    // Which of the view's parts are shown: the page's board, the gallery, or one micro-frontend.
    function layoutView() {
      const components = st.view === 'components';
      const detail = components && !!st.mfDetail;
      q('.mk-gallery-host').hidden = !components || detail;
      q('.mk-board').hidden = components && !detail;
      q('.mk-rules').hidden = !detail;
      q('.mk-mfbar').hidden = !detail;
      if (!detail) q('.mk-mfsave').hidden = true;
      if (st.mfGallery) st.mfGallery.pause(!components || detail);
    }
    function mountMf() {
      if (st.mfDetail && !componentOf(st.mfDetail)) st.mfDetail = null;
      if (opts && opts.onPlace) opts.onPlace({ view: st.view, mf: st.mfDetail });
      layoutView();
      if (!st.mfDetail) { ensureMfGallery().draw(components()); return; }
      drawMf();
    }
    function openMf(name) {
      if (!componentOf(name)) return;
      if (!st.mfDetail) st.mfTop = ensureMfGallery().top();
      if (name !== st.mfDetail) { st.mfPicked = null; st.mfFitted = false; }
      st.mfDetail = name;
      st.mfMode = 'use';
      st.mfSaveOpen = false;
      loadCaps();
      mountMf();
      st.board.refresh();
    }
    function closeMf() {
      st.mfDetail = null;
      st.mfPicked = null;
      mountMf();
      ensureMfGallery().scrollTo(st.mfTop || 0);
    }
    function stepMf(by) {
      const list = ensureMfGallery().list();
      const at = list.findIndex((c) => c.name === st.mfDetail);
      if (list[at + by]) openMf(list[at + by].name);
    }
    // The units of the product, with their rules — read again now and then.
    async function loadCaps(force) {
      if (!force && st.caps && Date.now() - st.capsAt < 30000) return;
      st.capsAt = Date.now();
      try {
        const res = await fetch('/api/capabilities');
        const got = res.ok ? await res.json() : null;
        st.caps = new Map(((got && got.entries) || []).map((e) => [lower(e.id), e]));
      } catch (e) { st.caps = st.caps || new Map(); }
      if (!st.gone && st.mfDetail) drawRules();
    }
    const unitOf = (id) => (st.caps ? st.caps.get(lower(id)) : null);

    // ---- the generations: on the kit's model, the kit making what they lack first ------------------
    const genOf = (name) => (st.gens || []).find((j) => j.name === name) || null;
    const genBusy = (j) => ['queued', 'planning', 'kit', 'writing'].includes(j.step);
    const genLabel = (j) => ({ queued: 'waiting for a session', planning: 'planning its components', kit: `the kit makes ${(j.made || []).join(', ') || 'what it lacks'}`, writing: 'writing it' }[j.step] || j.step);
    async function generateMfs(list) {
      const items = list.map((c) => ({ name: c.name, system: c.system || '', description: c.description || '', needs: c.needs || [], actions: c.actions || [], props: c.props || {} }));
      if (!items.length) { toast('nothing to generate: every micro-frontend is mocked'); return; }
      try {
        const res = await fetch('/api/mockup/generate', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ branch: st.branch, items }) });
        const text = await res.text();
        if (!res.ok) throw new Error(text);
        toast(`${items.length === 1 ? esc(items[0].name) : items.length + ' micro-frontends'}: generating — the kit makes what they lack first`);
        await loadGens();
        if (st.view === 'components') mountMf();
      } catch (e) { toast(esc(String(e.message || e).split('\n')[0])); }
    }
    async function loadGens() {
      try {
        const res = await fetch('/api/mockup/generations?' + new URLSearchParams({ branch: st.branch }));
        const got = res.ok ? await res.json() : { jobs: [], drafts: [] };
        st.gens = got.jobs || [];
        st.mfDrafts = got.drafts || [];
      } catch (e) { st.gens = st.gens || []; }
      const busy = st.gens.filter(genBusy).length;
      const failed = st.gens.filter((j) => j.step === 'failed').length;
      const span = q('.mk-gens');
      if (span) span.innerHTML = busy ? `<span class="spin"></span> ${busy} generating` : failed ? `<span class="bad" title="${esc(st.gens.filter((j) => j.step === 'failed').map((j) => `${j.name}: ${j.why}`).join('\n'))}">${failed} failed</span>` : '';
      const all = q('[data-mk="generate-all"]');
      if (all) { const n = components().filter((c) => !c.mocked && !c.variantOf && !(genOf(c.name) && genBusy(genOf(c.name)))).length; all.textContent = n ? `generate ${n}` : 'generate'; all.disabled = !n; }
    }

    // ---- the stage: the micro-frontend alone, used or inspected --------------------------------
    const mfFrame = () => q('.mk-board .mk-stage iframe');
    const mfDoc = () => { try { const f = mfFrame(); return f && f.contentDocument; } catch (e) { return null; } };
    function drawMf() {
      const c = componentOf(st.mfDetail);
      const content = st.board.content;
      let stage = content.querySelector('.mk-stage');
      if (!stage || stage.dataset.card !== c.name) {
        content.innerHTML = `<div class="kt-stage mk-stage" data-pin="component:${esc(c.name)}" data-card="${esc(c.name)}" style="width:${MF_W}px">`
          + (c.mocked && !c.variantOf ? `<iframe class="mk-preview" title="${esc(c.name)}"></iframe>` : `<div class="kt-tile-body mk-stage-none">${c.variantOf ? 'a copy made on a page: shown there' : 'no fragment yet: mock it from the gallery'}</div>`)
          + '<div class="mk-marks"></div><div class="kt-hover" hidden></div><div class="kt-pick" hidden></div><div class="mk-shield"></div></div>';
        stage = content.querySelector('.mk-stage');
        armStage(stage);
        const frame = stage.querySelector('iframe');
        if (frame) frame.addEventListener('load', () => { sizeMf(); armLongPress(frame); drawRules(); });
        else sizeMf();
      }
      const frame = stage.querySelector('iframe');
      const src = mfSrc(c.name);
      if (frame && frame.dataset.loaded !== src) { frame.dataset.loaded = src; frame.src = src; }
      stage.querySelector('.mk-shield').hidden = st.mfMode !== 'inspect';
      stage.classList.toggle('inspecting', st.mfMode === 'inspect');
      drawMfBar();
      drawRules();
    }
    function sizeMf() {
      const frame = mfFrame();
      let h = 240;
      try {
        const doc = frame && frame.contentDocument;
        if (doc && doc.body) h = Math.min(4000, Math.max(320, doc.documentElement.scrollHeight, doc.body.scrollHeight));
      } catch (e) { /* the frame let go */ }
      if (h === st.mfH && st.mfFitted && (!frame || frame.style.height === h + 'px')) { drawMarks(); return; }
      if (frame) frame.style.height = h + 'px';
      st.mfH = h;
      st.board.setSize(MF_W, h);
      if (!st.mfFitted) { st.board.fit(); st.mfFitted = true; }
      st.board.refresh();
      drawMarks();
    }
    function drawMfBar() {
      const c = componentOf(st.mfDetail);
      if (!c) return;
      const list = ensureMfGallery().list();
      const at = list.findIndex((x) => x.name === c.name);
      const html = `<div class="kt-detail-row">
          <button type="button" class="mini" data-mf="back" title="Back to the gallery, where you left it">← all micro-frontends</button>
          <button type="button" class="mini" data-mf="step" data-step="-1" ${at <= 0 ? 'disabled' : ''}>‹</button>
          <span class="muted kt-detail-at">${at >= 0 ? at + 1 : '–'} / ${list.length}</span>
          <button type="button" class="mini" data-mf="step" data-step="1" ${at < 0 || at >= list.length - 1 ? 'disabled' : ''}>›</button>
          <b class="kt-detail-title">${esc(c.name)}</b><span class="mono muted">${esc(c.system || '')}</span><span class="kt-tile-tags">${mfFoot(c).split('<span class="kt-tile-tags">')[1] || ''}
          <span class="spacer"></span>
          ${mfActions(c)}
          <div class="mk-views" role="group" aria-label="mode"><button type="button" data-mf="mode" data-mode="use" class="${st.mfMode === 'use' ? 'on' : ''}" title="Click, type, open: the micro-frontend as a visitor uses it on its fixture">use it</button><button type="button" data-mf="mode" data-mode="inspect" class="${st.mfMode === 'inspect' ? 'on' : ''}" title="Pick a part: what it triggers or shows, and the rules behind it">inspect</button></div>
        </div>
        <div class="kt-detail-sub muted">${esc(c.description || '')}${c.pages.length ? ' · on ' + c.pages.map(esc).join(', ') : ''} · a long press on a kit component opens it on the design system's easel</div>`;
      if (st.lastMfBar !== html) { st.lastMfBar = html; q('.mk-mfbar').innerHTML = html; }
      drawMfSave(c);
    }

    // ---- requests on a micro-frontend: applied to its draft, saved in place or as a fork -------------
    // The requests waiting on it: written, not applied, not being applied.
    const mfPending = (name) => (st.board && st.view === 'components' ? st.board.all() : []).filter((n) => n.target && (n.target === 'component:' + name || n.target.startsWith('component:' + name + '>')) && n.kind === 'request' && (n.state || 'pending') !== 'simulated' && (n.text || '').trim() && !st.board.isApplying(n.id));
    function mfActions(c) {
      const pending = mfPending(c.name).length;
      const working = st.mfApplying[c.name] || 0;
      return `${working ? `<span class="kt-working"><span class="spin"></span> the AI rewrites it${working > 1 ? ` · ${working} in a row` : ''}</span>` : ''}`
        + `${pending ? `<button type="button" class="mini on" data-mf="apply" title="Apply the pending requests to its draft, in one AI call">apply ${pending} request${pending === 1 ? '' : 's'}</button>` : ''}`
        + `${hasDraft(c.name) && !working ? '<button type="button" class="mini on" data-mf="save">save…</button><button type="button" class="mini" data-mf="drop-draft">drop the draft</button>' : ''}`;
    }
    function drawMfSave(c) {
      const box = q('.mk-mfsave');
      box.hidden = !(st.mfSaveOpen && hasDraft(c.name));
      if (box.hidden) return;
      const kept = (box.querySelector('.kt-fork-name') || {}).value || '';
      const html = `<b>Save the draft</b><div class="kt-row"><button type="button" class="mini on" data-mf="save-replace" title="The draft takes its place: every page that injects it shows the new one">replace ${esc(c.name)}${c.pages.length ? ` (on ${c.pages.length} page${c.pages.length === 1 ? '' : 's'})` : ''}</button><input class="kt-fork-name" maxlength="60" placeholder="${esc(c.name)}2" aria-label="fork name"><button type="button" class="mini" data-mf="save-fork" title="A new micro-frontend from the draft; this one stays as it is">fork</button></div>${c.pages.length ? `<div class="muted">Replacing changes: ${c.pages.map(esc).join(', ')}</div>` : ''}`;
      if (box.dataset.html !== html) { box.dataset.html = html; box.innerHTML = html; box.querySelector('.kt-fork-name').value = kept; }
    }
    // Applies requests to a micro-frontend: its draft rewritten, the pins
    // applied go (ctrl+z on the board brings one back), the skipped stay.
    async function applyMf(name, notes) {
      const c = componentOf(name);
      if (!c || !st.board) return;
      const asked = notes.filter((n) => (n.text || '').trim()).map((n) => ({ id: n.id, text: n.text, target: n.part || '' }));
      if (!asked.length) { toast('nothing to apply: write the request first'); return; }
      const spec = { name: c.name, system: c.system || '', description: c.description || '', needs: c.needs || [], actions: c.actions || [], props: c.props || {} };
      st.mfApplying[name] = (st.mfApplying[name] || 0) + 1;
      drawMfBar();
      const work = (async () => {
        try {
          const res = await fetch(`/api/mockup/mf/${encodeURIComponent(name)}/apply`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ branch: st.branch, spec, requests: asked }) });
          const text = await res.text();
          if (!res.ok) throw new Error(text);
          const skipped = (JSON.parse(text).skipped) || [];
          asked.forEach((a) => { if (!skipped.some(([id]) => id === a.id)) st.board.drop(a.id); });
          skipped.forEach(([id, why]) => toast(`request ${esc(id)}: ${esc(why)}`));
        } catch (e) {
          toast(`${esc(name)}: not applied — ${esc(String(e.message || e).split('\n')[0])}`);
        } finally {
          st.mfApplying[name] = Math.max(0, (st.mfApplying[name] || 1) - 1);
          if (!st.gone) load();
        }
      })();
      st.board.track(asked.map((a) => a.id), work);
      return work;
    }
    async function mfCall(url, body, done) {
      try {
        const res = await fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(Object.assign({ branch: st.branch }, body || {})) });
        const text = await res.text();
        if (!res.ok) throw new Error(text);
        if (done) done(text ? JSON.parse(text) : {});
      } catch (e) { toast(esc(String(e.message || e).split('\n')[0])); }
      st.mfSaveOpen = false;
      load();
    }
    // A part of the frame named by its path of children from the micro-frontend's root.
    function mfPathOf(el) {
      const doc = mfDoc();
      const root = doc && doc.querySelector('[data-component]');
      if (!root || !el || !root.contains(el) || el === root) return '';
      const steps = [];
      for (let at = el; at && at !== root; at = at.parentElement) {
        const index = Array.prototype.indexOf.call(at.parentElement.children, at) + 1;
        steps.unshift(`${at.tagName.toLowerCase()}${at.classList && at.classList[0] ? '.' + at.classList[0] : ''}:nth-child(${index})`);
      }
      return steps.join('>');
    }
    function pinMfPart() {
      const picked = st.mfPicked;
      if (!picked || !picked.el || !st.board) return;
      const b = picked.el.getBoundingClientRect();
      const x = b.left + Math.min(b.width / 2, 24);
      const y = b.top + Math.min(b.height / 2, 12);
      st.board.newNote({ kind: 'request', target: 'component:' + st.mfDetail, part: mfPathOf(picked.el), dx: x, dy: y, x, y });
    }

    // ---- the rules: every actuator and every view of the micro-frontend --------------------------
    // The parts of the frame tied to a unit, by its markup's own words.
    function partsOf(attr, id) {
      const doc = mfDoc();
      if (!doc) return [];
      return [...doc.querySelectorAll(`[${attr}]`)].filter((el) => lower(el.getAttribute(attr)).split(/\s+/).includes(lower(id)));
    }
    function unitHtml(id, how) {
      const u = unitOf(id);
      const attr = how === 'writes' ? 'data-writes' : 'data-reads';
      const n = partsOf(attr, id).length;
      const on = st.mfPicked && st.mfPicked.unit && lower(st.mfPicked.unit) === lower(id) ? ' on' : '';
      const where = n
        ? `<span class="mk-rule-n" title="the parts of the view tied to it — hover to see them">${n} part${n === 1 ? '' : 's'}</span>`
        : `<span class="mk-rule-n bad" title="no part of the view carries ${attr}=&quot;${esc(id)}&quot; — contract N asks for it">not tagged</span>`;
      if (!u) return `<section class="mk-rule${on}" data-unit="${esc(id)}" data-how="${how}"><header><b class="mono">${esc(id)}</b>${where}</header><p class="muted">${st.caps ? 'no manifest found for it on any branch' : 'reading the units…'}</p></section>`;
      const k = u.contract || {};
      const input = Object.entries(k.input || {}).map(([f, t]) => `<code>${esc(f)}</code> <small>${esc(t)}</small>`).join(' · ');
      const rules = (u.rules || []).map((r) => `<li><b class="mono">${esc(r.id)}</b> ${esc(r.text)} <small class="muted">${esc(r.from)}</small></li>`).join('');
      const head = how === 'writes'
        ? `<span class="tag">${esc(k.effect || 'writes')}${k.target ? ' → ' + esc(k.target) : ''}</span>`
        : `<span class="tag">reads${k.implements ? ' · ' + esc(k.implements) : ''}</span>`;
      return `<section class="mk-rule${on}" data-unit="${esc(id)}" data-how="${how}">
          <header><b class="mono">${esc(u.id)}</b>${head}<span class="muted">${esc(u.status || '')}</span>${where}</header>
          ${u.role ? `<p>${esc(u.role)}</p>` : ''}
          ${input ? `<div class="mk-rule-k"><span class="muted">${how === 'writes' ? 'sends' : 'is handed'}</span> ${input}</div>` : ''}
          ${how === 'reads' && k.output ? `<div class="mk-rule-k"><span class="muted">returns</span> <code>${esc(k.output)}</code></div>` : ''}
          ${how === 'writes' && (k.mode || (k.permissions || []).length) ? `<div class="mk-rule-k"><span class="muted">mode</span> ${esc(k.mode || '—')}${(k.permissions || []).length ? ` · <span class="muted">asks</span> ${k.permissions.map(esc).join(', ')}` : ''}</div>` : ''}
          ${rules ? `<ul class="mk-rule-list">${rules}</ul>` : `<p class="muted">${how === 'writes' ? 'no invariant guards it' : 'no rule of its own'}</p>`}
        </section>`;
    }
    function drawRules() {
      const panel = q('.mk-rules');
      const c = st.mfDetail && componentOf(st.mfDetail);
      if (!c || panel.hidden) return;
      const doc = mfDoc();
      const kits = new Map();
      if (doc) doc.querySelectorAll('[data-kit]').forEach((el) => kits.set(el.dataset.kit, (kits.get(el.dataset.kit) || 0) + 1));
      const picked = st.mfPicked;
      const html = `${picked ? `<div class="mk-picked"><b>${esc(picked.label)}</b>${picked.kit ? ` <button type="button" class="mini" data-mf="open-kit" data-kit="${esc(picked.kit)}">open the kit component ${esc(picked.kit)} →</button>` : ''}${picked.unit ? '' : picked.kit ? '' : ' <span class="muted">— tied to no unit: it neither triggers nor shows one</span>'} <button type="button" class="mini" data-mf="pin-part" title="A request pin on this very part: the AI is told which element it is about">request on this part</button></div>` : ''}
        <h4>Actuators <small class="muted">what it triggers</small></h4>
        ${c.actions.length ? c.actions.map((a) => unitHtml(a, 'writes')).join('') : '<p class="muted">It writes nothing.</p>'}
        <h4>Views <small class="muted">what it shows</small></h4>
        ${c.needs.length ? c.needs.map((n) => unitHtml(n, 'reads')).join('') : '<p class="muted">It reads nothing.</p>'}
        ${Object.keys(c.props || {}).length ? `<h4>Props <small class="muted">what the screen hands it</small></h4><div class="mk-rule-k">${Object.entries(c.props).map(([k, v]) => `<code>${esc(k)}</code> <small>${esc(v)}</small>`).join(' · ')}</div>` : ''}
        <h4>Kit components <small class="muted">a long press on one opens it</small></h4>
        ${kits.size ? `<div class="mk-kits">${[...kits].map(([name, n]) => `<button type="button" class="kt-crumb" data-mf="open-kit" data-kit="${esc(name)}">${esc(name)}${n > 1 ? ' ×' + n : ''}</button>`).join('')}</div>` : '<p class="muted">None injected with <code>data-kit</code>.</p>'}`;
      if (st.lastRules !== html) { st.lastRules = html; panel.innerHTML = html; }
      drawMarks();
    }
    // The parts outlined: those of the unit the pointer is on in the panel, and the one picked.
    function drawMarks() {
      const marks = q('.mk-board .mk-marks');
      if (!marks) return;
      // The unit the pointer is on in the panel: kept across every redraw until the pointer leaves it.
      const { unit, how } = st.mfHover || {};
      const show = unit ? partsOf(how === 'writes' ? 'data-writes' : 'data-reads', unit) : [];
      marks.innerHTML = show.map((el) => { const b = el.getBoundingClientRect(); return `<i style="left:${b.left}px;top:${b.top}px;width:${b.width}px;height:${b.height}px"></i>`; }).join('');
      const pick = q('.mk-board .mk-stage .kt-pick');
      const el = st.mfPicked && st.mfPicked.el && st.mfPicked.el.isConnected ? st.mfPicked.el : null;
      if (pick) mfOutline(pick, el);
    }
    function mfOutline(box, el) {
      if (!el) { box.hidden = true; return; }
      const b = el.getBoundingClientRect();
      Object.assign(box.style, { left: b.left + 'px', top: b.top + 'px', width: b.width + 'px', height: b.height + 'px' });
      box.hidden = false;
    }
    // The part under a point of the screen, inside the micro-frontend's frame.
    function mfAt(clientX, clientY) {
      const frame = mfFrame();
      const doc = mfDoc();
      if (!frame || !doc) return null;
      const r = frame.getBoundingClientRect();
      const k = frame.offsetWidth ? r.width / frame.offsetWidth : 1;
      return doc.elementFromPoint((clientX - r.left) / k, (clientY - r.top) / k);
    }
    // What a part is tied to: the unit it triggers or shows, the kit component it is.
    function meaningOf(el) {
      if (!el || !el.closest) return null;
      const act = el.closest('[data-writes]');
      const reads = el.closest('[data-reads]');
      const kit = el.closest('[data-kit]');
      const tied = act || reads;
      const unit = act ? act.getAttribute('data-writes') : reads ? reads.getAttribute('data-reads') : '';
      const target = tied || kit || el;
      const label = act ? `triggers ${unit}` : reads ? `shows ${unit}` : kit ? `the kit component ${kit.dataset.kit}` : `<${el.tagName.toLowerCase()}>`;
      return { el: target, unit: unit.split(/\s+/)[0] || '', how: act ? 'writes' : reads ? 'reads' : '', kit: kit ? kit.dataset.kit : '', label };
    }
    function armStage(stage) {
      const shield = stage.querySelector('.mk-shield');
      let down = null;
      let timer = 0;
      shield.addEventListener('pointermove', (ev) => {
        if (st.mfMode !== 'inspect') return;
        const m = meaningOf(mfAt(ev.clientX, ev.clientY));
        mfOutline(stage.querySelector('.kt-hover'), m && m.el);
        if (down && Math.hypot(ev.clientX - down[0], ev.clientY - down[1]) > 6) clearTimeout(timer);
      });
      shield.addEventListener('pointerleave', () => { mfOutline(stage.querySelector('.kt-hover'), null); clearTimeout(timer); });
      shield.addEventListener('pointerdown', (ev) => {
        down = [ev.clientX, ev.clientY];
        const m = meaningOf(mfAt(ev.clientX, ev.clientY));
        clearTimeout(timer);
        if (m && m.kit) timer = setTimeout(() => { down = null; openKit(m.kit); }, LONG_PRESS);
      });
      shield.addEventListener('pointerup', () => clearTimeout(timer));
      shield.addEventListener('click', (ev) => {
        if (st.mfMode !== 'inspect' || !down || Math.hypot(ev.clientX - down[0], ev.clientY - down[1]) > 4) return;
        st.mfPicked = meaningOf(mfAt(ev.clientX, ev.clientY));
        drawRules();
        const row = st.mfPicked && st.mfPicked.unit && q(`.mk-rules .mk-rule[data-unit="${CSS.escape(st.mfPicked.unit)}"]`);
        if (row) row.scrollIntoView({ block: 'nearest' });
      });
    }
    // Used as a visitor would: a long press on a kit component still opens it.
    function armLongPress(frame) {
      let doc = null;
      try { doc = frame.contentDocument; } catch (e) { doc = null; }
      if (!doc) return;
      let timer = 0;
      let down = null;
      doc.addEventListener('pointerdown', (ev) => {
        const kit = ev.target && ev.target.closest && ev.target.closest('[data-kit]');
        clearTimeout(timer);
        if (!kit) return;
        down = [ev.clientX, ev.clientY];
        timer = setTimeout(() => openKit(kit.dataset.kit), LONG_PRESS);
      });
      doc.addEventListener('pointermove', (ev) => { if (down && Math.hypot(ev.clientX - down[0], ev.clientY - down[1]) > 6) clearTimeout(timer); });
      ['pointerup', 'pointercancel'].forEach((kind) => doc.addEventListener(kind, () => clearTimeout(timer)));
      ['click', 'keyup', 'transitionend'].forEach((kind) => doc.addEventListener(kind, () => setTimeout(() => { sizeMf(); drawRules(); }, 60)));
      // The real code draws itself once its data has come: the parts and the rules are counted again as it does.
      let again = 0;
      new MutationObserver(() => {
        clearTimeout(again);
        again = setTimeout(() => { sizeMf(); drawRules(); }, 120);
      }).observe(doc.documentElement, { subtree: true, childList: true, attributes: true, attributeFilter: ['data-writes', 'data-reads', 'data-kit', 'hidden'] });
    }
    // The kit component on the design system's easel, alone.
    function openKit(name) {
      window.dispatchEvent(new CustomEvent('harness:open-pane', { detail: { kind: 'kit', title: 'design system', tab: 'canvas', component: name } }));
    }

    // ---- the strip: the micro-frontends to drop on the page ------------------------------
    // The micro-frontends the page shows, by name, as the frame holds it (the
    // draft's drops included), else as the manifest lists them.
    function onPage() {
      if (st.doc) return [...new Set([...st.doc.querySelectorAll('[data-component]')].map((el) => el.dataset.component))];
      return ((currentPage() || {}).components || []).slice();
    }
    // Two sections: the micro-frontends on this page, then the rest — each
    // of the rest says where it is used.
    function drawStrip() {
      const strip = q('.mk-strip');
      strip.hidden = !st.strip || st.view !== 'page' || !st.state || (editing() && !q('.mk-panel').hidden);
      if (strip.hidden) return;
      const list = st.focus ? components().filter((c) => st.focus.includes(c.name)) : components();
      const here = new Set(onPage());
      const chip = (c) => {
        const used = c.pages.length ? 'used on: ' + c.pages.join(', ') : 'on no page';
        const title = [c.description || c.name, here.has(c.name) ? '' : used, c.mocked ? '' : 'not mocked yet: its session makes the fragment first'].filter(Boolean).join('\n');
        return `<div class="mk-chip${c.name === st.hot ? ' hot' : ''}" data-chip="${esc(c.name)}" style="--mf:${c.colour}" title="${esc(title)}"><i></i><b>${esc(c.name)}</b><small>${esc(c.system || '')}${c.pages.length ? ' · ' + c.pages.length + ' page' + (c.pages.length > 1 ? 's' : '') : ''}</small></div>`;
      };
      const section = (title, items, empty) => `<div class="mk-strip-sec">${title} <small>${items.length}</small></div>`
        + (items.length ? items.map(chip).join('') : `<div class="hb-empty">${empty}</div>`);
      strip.innerHTML = `<div class="mk-strip-head">micro-frontends <small>${st.focus ? "the rule's only" : 'drag one onto the page'}</small></div>`
        + section('on this page', list.filter((c) => here.has(c.name)), 'none on this page')
        + section('the rest', list.filter((c) => !here.has(c.name)), 'none')
        + '<button type="button" class="mini" data-mk="design">+ micro-frontend</button>';
    }
    // The picked rule's micro-frontends, marked on the page.
    function marksFocus() {
      if (st.doc) st.doc.querySelectorAll('[data-component]').forEach((el) => el.classList.toggle('mf-focus', !!st.focus && st.focus.includes(el.dataset.component)));
    }
    // One micro-frontend pointed at — from the list or on the page — lit in both.
    function setHot(name) {
      if (st.hot === name) return;
      st.hot = name;
      if (st.doc) st.doc.querySelectorAll('[data-component]').forEach((el) => el.classList.toggle('mf-hot', !!name && el.dataset.component === name));
      q('.mk-strip').querySelectorAll('[data-chip]').forEach((el) => el.classList.toggle('hot', !!name && el.dataset.chip === name));
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
        + '<div class="mk-form-row"><span class="mk-form-state"></span><span class="spacer"></span><button type="button" class="mini" data-mk="form-cancel">cancel</button><button type="button" class="mini" data-mk="form-make" title="An interactive session writes it with you">make it with an AI session</button><button type="button" class="mini on" data-mk="form-generate" title="Generated in one go, as the kit\'s components are: the kit makes what it lacks first">generate it</button></div>';
      form.querySelector('[name="name"]').focus();
    }
    function makeFromForm(generate) {
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
      if (generate) { generateMfs([spec]); return; }
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
      if (st.view === 'components') {
        const was = JSON.stringify(st.gens || []) + JSON.stringify(st.mfDrafts || []);
        await loadGens();
        if (was !== JSON.stringify(st.gens) + JSON.stringify(st.mfDrafts) && st.board && st.board.id === boardOf()) mountMf();
      }
      const listChanged = before && JSON.stringify(before.components) + JSON.stringify(before.units) !== JSON.stringify(reply.components) + JSON.stringify(reply.units);
      const pageChanged = !before || !st.board || st.board.id !== boardOf();
      if (pageChanged) { st.stamp = reply.stamp; mountBoard(); }
      else if (st.view === 'components' && (listChanged || reply.stamp !== st.stamp)) { st.stamp = reply.stamp; mountMf(); }
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
      host.querySelectorAll('.mk-views button[data-view]').forEach((b) => b.classList.toggle('on', b.dataset.view === st.view));
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
      q('[data-mk="restore"]').hidden = !(s.discarded && !s.edited);
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
      if (opts && opts.onPlace) opts.onPlace({ view: st.view, mf: st.mfDetail });
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
        toast('mock-up discarded — <b>undo</b>', restore);
      } catch (e) {
        if (action) action.fail(e.message || e);
      } finally {
        load();
      }
    }
    // The last discard taken back: the workspace as it was, uncommitted.
    async function restore() {
      const action = window.HarnessActions && window.HarnessActions.start({ key: `mockup:restore:${st.branch}`, label: `bring back the discarded mock-up of ${st.branch}`, button: q('[data-mk="restore"]') });
      try {
        const res = await fetch('/api/mockup/restore', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ branch: st.branch }) });
        if (!res.ok) throw new Error(await res.text());
        if (action) action.ok('the discarded mock-up is back');
      } catch (e) {
        if (action) action.fail(e.message || e); else toast(esc(String(e.message || e)));
      } finally {
        st.state = null;
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
      if (mk.dataset.mk === 'form-make') makeFromForm(false);
      if (mk.dataset.mk === 'form-generate') makeFromForm(true);
      if (mk.dataset.mk === 'generate-all') generateMfs(components().filter((c) => !c.mocked && !c.variantOf && !genOf(c.name)));
      if (mk.dataset.mk === 'save') save();
      if (mk.dataset.mk === 'discard') discard();
      if (mk.dataset.mk === 'restore') restore();
    };
    function onEdit(b, e) {
      const what = b.dataset.ed;
      const version = Number(b.dataset.version);
      // Nothing left to undo on the page: the last discard is what goes back.
      if (what === 'undo') { if (st.engine && st.engine.undo()) afterChange(); else if (st.state && st.state.discarded && !st.state.edited) restore(); }
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
      window.Viewport.change(e, setWidth);
      if (e.target.classList.contains('mk-version-name') && st.history && st.history.current) { st.history.current.name = e.target.value.slice(0, 80); persist(); }
      if (e.target.classList.contains('mk-fork-name')) renameForkTo(e.target.dataset.from, e.target.value);
    };
    host.addEventListener('click', onClick);
    host.addEventListener('click', onCardMock);
    host.addEventListener('click', (e) => {
      const b = e.target.closest('[data-mf]');
      if (!b || b.disabled) return;
      switch (b.dataset.mf) {
        case 'back': closeMf(); break;
        case 'step': stepMf(Number(b.dataset.step) || 0); break;
        case 'mode': st.mfMode = b.dataset.mode; if (st.mfMode === 'use') st.mfPicked = null; drawMf(); break;
        case 'open-kit': openKit(b.dataset.kit); break;
        case 'generate': { const c = componentOf(b.dataset.name); if (c) generateMfs([c]); break; }
        case 'apply': { const pins = mfPending(st.mfDetail); if (pins.length) applyMf(st.mfDetail, pins); break; }
        case 'save': st.mfSaveOpen = !st.mfSaveOpen; drawMfBar(); break;
        case 'save-replace': mfCall(`/api/mockup/mf/${encodeURIComponent(st.mfDetail)}/save`, { how: 'replace' }, () => toast(`${esc(st.mfDetail)} replaced: every page that injects it shows the new one`)); break;
        case 'save-fork': { const input = q('.mk-mfsave .kt-fork-name'); mfCall(`/api/mockup/mf/${encodeURIComponent(st.mfDetail)}/save`, { how: 'fork', name: input ? input.value.trim() : '' }, (got) => toast(`forked as <b>${esc(got.saved)}</b>; ${esc(st.mfDetail)} is as it was`)); break; }
        case 'drop-draft': mfCall(`/api/mockup/mf/${encodeURIComponent(st.mfDetail)}/draft/drop`, {}); break;
        case 'pin-part': pinMfPart(); break;
        default: break;
      }
    });
    // A unit pointed at in the panel outlines its parts in the frame.
    host.addEventListener('mouseover', (e) => {
      const row = e.target.closest && e.target.closest('.mk-rules .mk-rule');
      if (row) { st.mfHover = { unit: row.dataset.unit, how: row.dataset.how }; drawMarks(); }
    });
    host.addEventListener('mouseout', (e) => { if (e.target.closest && e.target.closest('.mk-rules') && !(e.relatedTarget && e.relatedTarget.closest && e.relatedTarget.closest('.mk-rules'))) { st.mfHover = null; drawMarks(); } });
    host.addEventListener('change', onChange);
    host.addEventListener('keydown', onKey);
    host.addEventListener('keyup', onKeyUp);
    q('.mk-strip').addEventListener('pointerdown', onStripDown);
    q('.mk-strip').addEventListener('mouseover', (e) => { const chip = e.target.closest && e.target.closest('[data-chip]'); setHot(chip ? chip.dataset.chip : ''); });
    q('.mk-strip').addEventListener('mouseleave', () => setHot(''));
    const boardHost = q('.mk-board');
    // Over the shield, the part under the pointer is found through the frame.
    boardHost.addEventListener('pointermove', (e) => {
      if (!st.strip || st.view !== 'page' || st.vp) return;
      const el = elementAt(e.clientX, e.clientY, '[data-component]');
      setHot(el ? el.dataset.component : '');
    });
    boardHost.addEventListener('pointerleave', () => setHot(''));
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
      // A rule picked elsewhere: its micro-frontends alone in the list and
      // marked on the page, the page shown when one is given; null lets go.
      focus(sel) {
        st.focus = sel ? [...(sel.names || [])] : null;
        if (sel && sel.page && st.view === 'page' && (st.state && st.state.pages || []).some((p) => p.path === sel.page)) show(sel.page);
        marksFocus();
        drawStrip();
      },
      // One micro-frontend's fragment generated again from its spec.
      async regenerate(name) { const c = componentOf(name); if (c) await generateMfs([c]); },
      // The draft's steps written into the workspace, so what the human
      // sees is what the server reads next — the creation page calls it
      // before it moves on or accepts.
      async flush() { if (st.engine && st.engine.ops.length && !st.readOnly) await materialize(); },
      destroy() {
        leavePage();
        design.detach();
        st.gone = true;
        clearTimeout(st.timer);
        clearTimeout(st.saveTimer);
        clearInterval(measured);
        if (st.mfGallery) st.mfGallery.destroy();
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
