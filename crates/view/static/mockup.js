// The mock-up on its easel: the product's frontend, mocked — the static pages
// under `mock/` of the branch shown — run in an iframe on a board, so a
// human pins notes on its parts, draws zones around several, links a page
// to another (or creates one from an objective), asks for a simulation an
// AI session answers by editing the page, and drops micro-frontends on it.
// One board per branch and page: pins stay where they were left.
//
// Two views. **frontend**: the page, with a switch that outlines every
// micro-frontend in its own colour (pins then land on micro-frontends, not
// their parts), and a strip of the branch's micro-frontends to drag onto
// the page. **micro-frontends**: each micro-frontend rendered alone from
// its fragment and fixture, a card per one, pins on the cards, and a form
// that makes a new one — a design pin whose AI session writes the fragment.
//
// The pages come from `/mock/<branch slug>/<path>`, a micro-frontend alone
// from `/mock/<branch slug>/__preview/<Name>`; the branch's state — pages,
// micro-frontends, the manifests' Micro-UIs and Capabilities, whether a
// session edited it, a stamp that changes with its files — from
// `/api/mockup?branch=`. A branch edited by a session is shown as its
// workspace holds it; `save the mock-up` commits it on a `mockup/<name>`
// branch, `discard` drops it.
(() => {
  'use strict';

  // The width a page is laid out at, in board units.
  const WIDTH = 1280;
  const MIN_HEIGHT = 480;
  // A micro-frontend's card, in board units.
  const CARD_W = 400;
  const CARD_GAP = 28;
  const CARD_MIN = 160;
  const CARD_MAX = 640;
  const POLL_MS = 3000;
  const LAST = 'harness.mockup.page';
  const MF_ON = 'harness.mockup.mf';
  // One colour per micro-frontend, by its rank on the branch.
  const COLOURS = ['#4ea3f0', '#f08a2c', '#4fbf72', '#b388f5', '#e2b240', '#ef6b8a', '#2bc0c8', '#c9a46b'];

  const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const plain = (s) => String(s || '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
  const slugOf = (branch) => String(branch || '').replace(/\//g, '__');
  const pageSlug = (path) => plain(String(path || '').replace(/^pages\//, '').replace(/\.html?$/, '')) || 'home';
  // The parts of a page a pin may sit on, nearest first.
  const PARTS = '[data-component],[id],button,a,input,select,textarea,label,h1,h2,h3,h4,img,table,tr,li,form,section,article,aside,nav,header,footer,figure,p';

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
      + '<button type="button" class="mini mk-only-page" data-mk="mf" aria-pressed="false" title="Outline every micro-frontend in its own colour; pins then land on micro-frontends">micro-frontends ◧</button>'
      + '<button type="button" class="mini mk-only-page" data-mk="interact" aria-pressed="false" title="Click through to the page itself: its links, its buttons">interact</button>'
      + '<button type="button" class="mini mk-only-components" data-mk="design" hidden title="A new micro-frontend, made by an AI session from its spec">+ micro-frontend</button>'
      + '<span class="spacer"></span>'
      + '<span class="mk-state"></span>'
      + '<input class="mk-name" maxlength="80" placeholder="name of the mock-up" hidden aria-label="name of the mock-up">'
      + '<button type="button" class="mini" data-mk="save" hidden>save the mock-up</button>'
      + '<button type="button" class="mini" data-mk="discard" hidden>discard</button>'
      + '</div>'
      + '<div class="mk-form" hidden></div>'
      + '<div class="dm-main"><div class="dm-board mk-board"></div><aside class="mk-strip" hidden></aside></div>'
      + '</div>';
    const q = (s) => host.querySelector(s);
    const st = { branch: (opts && opts.branch) || '', page: (opts && opts.page) || '', view: (opts && opts.view) || 'page', versions: null, state: null, stamp: '', pinned: false,
      timer: null, gone: false, interact: false, mf: !!kept(MF_ON, false), board: null, frame: null, shield: null, doc: null, armed: null, proxies: new WeakMap(), fitted: false, saving: false,
      // The micro-frontends view: one iframe per card, measured as they load.
      cards: [] };

    // ---- what the branch knows of its micro-frontends ----------------------------------
    // The registry's and the manifests' together, by name: mocked, built, or both.
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
      const list = [...byName.values()].sort((a, b) => a.name.localeCompare(b.name));
      list.forEach((c, i) => { c.colour = COLOURS[i % COLOURS.length]; c.pages = (s.pages || []).filter((p) => (p.components || []).includes(c.name)).map((p) => p.title); });
      return list;
    };
    const componentOf = (name) => components().find((c) => c.name === name) || null;
    const specOf = (c) => `name: ${c.name}\nsystem: ${c.system || ''}\ndescription: ${c.description || ''}\nneeds: ${(c.needs || []).join(', ')}\nactions: ${(c.actions || []).join(', ')}\nprops:\n${Object.entries(c.props || {}).map(([k, v]) => `  ${k}: ${v}`).join('\n')}`.trim();
    const capabilities = () => (st.state && st.state.units && st.state.units.capabilities) || [];
    const dataCapabilities = () => (st.state && st.state.units && st.state.units.data_capabilities) || [];

    // ---- the parts of the page, reached through the iframe ------------------------------
    const targetOf = (el) => {
      if (el.dataset && el.dataset.component) return 'component:' + el.dataset.component;
      if (el.id) return 'id:' + el.id;
      const steps = [];
      let at = el;
      while (at && at !== st.doc.body && steps.length < 8) {
        const parent = at.parentElement;
        if (!parent) break;
        const index = Array.prototype.indexOf.call(parent.children, at) + 1;
        steps.unshift(`${at.tagName.toLowerCase()}:nth-child(${index})`);
        at = parent;
      }
      return 'path:' + steps.join('>');
    };
    const elementOf = (target) => {
      if (!st.doc || !target) return null;
      try {
        if (target.startsWith('component:')) return st.doc.querySelector(`[data-component="${CSS.escape(target.slice(10))}"]`);
        if (target.startsWith('id:')) return st.doc.getElementById(target.slice(3));
        if (target.startsWith('path:')) return st.doc.body.querySelector(':scope>' + target.slice(5));
      } catch (e) { return null; }
      return null;
    };
    // A stand-in the board can hold: its box in the page's coordinates,
    // through the iframe's place and the board's zoom.
    const proxy = (el) => {
      let p = st.proxies.get(el);
      if (p) return p;
      p = {
        el,
        getAttribute: (name) => (name === 'data-pin' ? targetOf(el) : el.getAttribute(name)),
        classList: { add: (c) => el.classList.add(c), remove: (c) => el.classList.remove(c) },
        getBoundingClientRect() {
          const fr = st.frame.getBoundingClientRect();
          const r = el.getBoundingClientRect();
          const k = st.board ? st.board.zoom() : 1;
          return { left: fr.left + r.left * k, top: fr.top + r.top * k, width: r.width * k, height: r.height * k, right: fr.left + (r.left + r.width) * k, bottom: fr.top + (r.top + r.height) * k };
        },
      };
      st.proxies.set(el, p);
      return p;
    };
    const resolve = (target) => {
      if (st.view !== 'page') return null;
      if (!target) return null;
      const el = elementOf(target);
      return el ? proxy(el) : null;
    };
    const under = (clientX, clientY) => {
      if (st.view !== 'page' || !st.frame || !st.doc) return null;
      const fr = st.frame.getBoundingClientRect();
      if (clientX < fr.left || clientX > fr.right || clientY < fr.top || clientY > fr.bottom) return null;
      const k = st.board ? st.board.zoom() : 1;
      let el = st.doc.elementFromPoint((clientX - fr.left) / k, (clientY - fr.top) / k);
      // With the micro-frontends outlined, a pin lands on the micro-frontend itself.
      el = el && el.closest(st.mf ? '[data-component]' : PARTS);
      if (!el || el === st.doc.body || el === st.doc.documentElement) return null;
      return proxy(el);
    };
    const describe = (target) => {
      if (!target) return st.view === 'page' ? 'the page' : 'the micro-frontends';
      if (target.startsWith('component:')) return 'micro-frontend ' + target.slice(10);
      if (target.startsWith('id:')) return '#' + target.slice(3);
      return target.slice(5).split('>').map((s) => s.replace(/:nth-child\(\d+\)/, '')).slice(-3).join(' > ');
    };
    // What a pin's session is told: the page, its objective, the part's markup;
    // for a micro-frontend, its spec and where it is used.
    const context = (target) => {
      let text = '';
      if (st.view === 'page') {
        const page = currentPage();
        text = page ? `Page ${page.path} — ${page.title}: ${page.objective || '(no objective written)'}` : `Page ${st.page || '(none yet)'}`;
        if (page && page.screen) text += `\nMocks the screen ${page.screen}.`;
        if (page && page.components && page.components.length) text += `\nMicro-frontends injected: ${page.components.join(', ')}.`;
      } else {
        text = 'The micro-frontends view: each micro-frontend of the branch rendered alone from its fragment and fixture.';
      }
      if (target && target.startsWith('component:')) {
        const c = componentOf(target.slice(10));
        if (c) text += `\nThe micro-frontend ${c.name} (${c.mocked ? 'mocked' : 'not mocked yet'}, ${c.built ? 'built under apps/' : 'not built yet'}):\n${specOf(c)}\nUsed by the pages: ${c.pages.join(', ') || 'none'}.`;
      }
      const el = elementOf(target);
      if (el) {
        const html = el.outerHTML.replace(/\s+/g, ' ');
        text += `\nThe part the pin sits on (${describe(target)}):\n${html.length > 1500 ? html.slice(0, 1500) + '…' : html}`;
      }
      return text;
    };
    const currentPage = () => (st.state && st.state.pages || []).find((p) => p.path === st.page) || null;
    const boardOf = () => `mockup--${plain(st.branch) || 'integration'}--${st.view === 'components' ? 'micro-frontends' : st.page ? pageSlug(st.page) : 'home'}`;
    const frameSrc = () => `/mock/${encodeURIComponent(slugOf(st.branch))}/${st.page}?v=${encodeURIComponent(st.stamp)}`;
    const previewSrc = (name) => `/mock/${encodeURIComponent(slugOf(st.branch))}/__preview/${encodeURIComponent(name)}?v=${encodeURIComponent(st.stamp)}`;

    // ---- the board: one per branch and page, one for the micro-frontends ------------
    function mountBoard() {
      if (st.board) { try { st.board.destroy(); } catch (e) { /* gone */ } }
      st.board = null; st.frame = null; st.doc = null; st.shield = null; st.fitted = false; st.proxies = new WeakMap(); st.cards = [];
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
        kinds: st.view === 'page' ? ['note', 'brainstorm', 'asking', 'link', 'simulate', 'design'] : ['note', 'brainstorm', 'asking', 'design'],
        zones: st.view === 'page',
        pages: () => (st.state && st.state.pages) || [],
        capabilities,
        dataCapabilities,
        goto: (path) => { setView('page'); show(path); },
        // A pin asked for from elsewhere (the office's pin board) is opened once its board is read.
        onLoaded: () => { if (opts && opts.pin && !st.pinned && st.board && st.board.goTo(opts.pin)) st.pinned = true; },
        bounds: () => (st.view === 'page'
          ? { x: 0, y: 0, w: WIDTH, h: Math.max(MIN_HEIGHT, st.frame ? parseFloat(st.frame.style.height) || MIN_HEIGHT : MIN_HEIGHT) }
          : { x: 0, y: 0, w: Math.max(WIDTH, st.board ? parseFloat(st.board.content.style.width) || WIDTH : WIDTH), h: Math.max(MIN_HEIGHT, st.board ? parseFloat(st.board.content.style.height) || MIN_HEIGHT : MIN_HEIGHT) }),
      });
      if (st.view === 'components') { mountCards(); return; }
      const page = currentPage();
      if (!page) {
        st.board.content.innerHTML = `<div class="mk-empty" data-pin="page:none"><h2>${st.state && st.state.present ? 'No page chosen' : 'No page yet'}</h2>`
          + `<p>${st.state && st.state.pages && st.state.pages.length ? 'Pick a page above.' : 'The mock-up starts with its first page: <b>+ page</b> above, or drop a pin here and make it a <b>link</b> to a new page — an AI session draws it from the objective you write.'}</p></div>`;
        st.board.setSize(WIDTH, MIN_HEIGHT);
        st.board.fit();
        return;
      }
      st.board.content.innerHTML = `<iframe class="mk-frame" title="${esc(page.title)}" src="${esc(frameSrc())}" style="width:${WIDTH}px;height:${MIN_HEIGHT}px"></iframe><div class="mk-shield"${st.interact ? ' hidden' : ''}></div>`;
      st.frame = q('.mk-frame');
      st.shield = q('.mk-shield');
      st.board.setSize(WIDTH, MIN_HEIGHT);
      st.frame.addEventListener('load', onFrameLoad);
    }
    function onFrameLoad() {
      if (!st.frame) return;
      let doc = null;
      try { doc = st.frame.contentDocument; } catch (e) { doc = null; }
      st.doc = doc;
      if (!doc) return;
      // The page navigated itself (interact mode): follow it.
      const path = (() => { try { return decodeURIComponent(st.frame.contentWindow.location.pathname); } catch (e) { return ''; } })();
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
    }
    // What the view adds to a page's document: the drop target's outline,
    // and the micro-frontends' outlines and labels when switched on.
    function injectStyle(doc) {
      if (doc.getElementById('hb-style')) return;
      const style = doc.createElement('style');
      style.id = 'hb-style';
      style.textContent = '.hb-target{outline:2px dashed #e2b240 !important;outline-offset:2px}'
        + '.mf-on[data-component]{outline:2px solid var(--mf,#4ea3f0) !important;outline-offset:4px;position:relative}'
        + '.mf-on[data-component]::before{content:attr(data-component);position:absolute;left:-2px;top:-22px;background:var(--mf,#4ea3f0);color:#101418;font:600 11px/1 ui-monospace,monospace;padding:4px 7px;border-radius:4px 4px 0 0;letter-spacing:.03em;z-index:2147483000}';
      (doc.head || doc.documentElement).appendChild(style);
    }
    // Every micro-frontend of the page outlined in its colour, or not.
    function outline() {
      if (!st.doc) return;
      const list = components();
      st.doc.querySelectorAll('[data-component]').forEach((el) => {
        const c = list.find((x) => x.name === el.dataset.component);
        el.classList.toggle('mf-on', st.mf);
        if (st.mf) el.style.setProperty('--mf', c ? c.colour : '#4ea3f0'); else el.style.removeProperty('--mf');
      });
    }
    // The page is as tall as its content: the whole page is on the board.
    function measure() {
      if (st.view === 'components') { measureCards(); return; }
      if (!st.doc || !st.frame) return;
      const h = Math.max(MIN_HEIGHT, st.doc.documentElement.scrollHeight, st.doc.body ? st.doc.body.scrollHeight : 0);
      if (Math.abs(h - (parseFloat(st.frame.style.height) || 0)) > 2) {
        st.frame.style.height = h + 'px';
        st.board.setSize(WIDTH, h);
      }
    }
    function reloadFrame() {
      if (st.view === 'components') { st.cards.forEach((c) => { c.frame.src = previewSrc(c.name); }); return; }
      if (!st.frame) { mountBoard(); return; }
      st.frame.src = frameSrc();
    }

    // ---- the micro-frontends view: one card per micro-frontend ------------------------
    function mountCards() {
      const list = components();
      const per = Math.max(1, Math.floor((WIDTH + CARD_GAP) / (CARD_W + CARD_GAP)));
      if (!list.length) {
        st.board.content.innerHTML = '<div class="mk-empty" data-pin="components:none"><h2>No micro-frontend yet</h2><p>A micro-frontend is a Micro-UI as the mock shows it: a fragment on a fixture shaped like what its Capabilities answer. <b>+ micro-frontend</b> above makes the first one from its spec.</p></div>';
        st.board.setSize(WIDTH, MIN_HEIGHT);
        st.board.fit();
        return;
      }
      st.board.content.innerHTML = list.map((c, i) => {
        const x = (i % per) * (CARD_W + CARD_GAP);
        const y = Math.floor(i / per) * (CARD_MIN + 70 + CARD_GAP);
        return `<div class="mk-card" data-pin="component:${esc(c.name)}" data-card="${esc(c.name)}" style="left:${x}px;top:${y}px;width:${CARD_W}px;--mf:${c.colour}">`
          + `<div class="mk-card-head"><b>${esc(c.name)}</b><span class="mk-card-sys">${esc(c.system || '')}</span>`
          + `<span class="mk-card-tags">${c.mocked ? (c.source ? '<i class="ok" title="the real code, bundled on the mock server">real code</i>' : '<i class="ok">mocked</i>') : '<i class="bad">not mocked</i>'}${c.built ? '<i class="ok">built</i>' : '<i>to build</i>'}</span></div>`
          + `<div class="mk-card-desc">${esc(c.description || '')}</div>`
          + `<div class="mk-card-meta">${c.needs.length ? 'needs ' + c.needs.map(esc).join(', ') : 'needs nothing'}${c.actions.length ? ' · triggers ' + c.actions.map(esc).join(', ') : ''}${c.pages.length ? ' · on ' + c.pages.map(esc).join(', ') : ' · on no page'}${c.source ? ` · from ${esc(c.source)}` : ''}</div>`
          + (c.mocked
            ? `<div class="mk-card-body"><iframe class="mk-preview" title="${esc(c.name)}" src="${esc(previewSrc(c.name))}" style="height:${CARD_MIN}px"></iframe><div class="mk-shield"></div></div>`
            : `<div class="mk-card-body mk-card-none">no fragment yet${c.built ? ' — the Micro-UI exists under apps/, its mock does not' : ''}<br><button type="button" class="mini on" data-mock-card="${esc(c.name)}">mock it with an AI session</button></div>`)
          + '</div>';
      }).join('');
      st.cards = [...st.board.content.querySelectorAll('.mk-card')].map((card) => ({ name: card.dataset.card, card, frame: card.querySelector('.mk-preview') }));
      st.cards.forEach((c) => { if (c.frame) c.frame.addEventListener('load', measureCards); });
      layoutCards();
      st.board.fit();
      st.fitted = true;
    }
    // Each preview as tall as its fragment; the cards re-laid in rows.
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
      const per = Math.max(1, Math.floor((WIDTH + CARD_GAP) / (CARD_W + CARD_GAP)));
      let y = 0; let rowH = 0; let maxX = 0;
      st.cards.forEach((c, i) => {
        if (i % per === 0 && i) { y += rowH + CARD_GAP; rowH = 0; }
        const x = (i % per) * (CARD_W + CARD_GAP);
        c.card.style.left = x + 'px'; c.card.style.top = y + 'px';
        rowH = Math.max(rowH, c.card.offsetHeight);
        maxX = Math.max(maxX, x + CARD_W);
      });
      st.board.setSize(Math.max(WIDTH, maxX), Math.max(MIN_HEIGHT, y + rowH + 20));
      st.board.refresh();
    }

    // ---- the strip: the micro-frontends to drop on the page ------------------------------
    function drawStrip() {
      const strip = q('.mk-strip');
      strip.hidden = st.view !== 'page' || !st.state;
      if (strip.hidden) return;
      const list = components();
      strip.innerHTML = '<div class="mk-strip-head">micro-frontends <small>drag one onto the page</small></div>'
        + (list.length ? list.map((c) => `<div class="mk-chip" data-chip="${esc(c.name)}" style="--mf:${c.colour}" title="${esc(c.description || c.name)}${c.mocked ? '' : ' — not mocked yet: its session makes the fragment first'}"><i></i><b>${esc(c.name)}</b><small>${esc(c.system || '')}${c.pages.length ? ' · ' + c.pages.length + ' page' + (c.pages.length > 1 ? 's' : '') : ''}</small></div>`).join('') : '<div class="hb-empty">none yet</div>')
        + '<button type="button" class="mini" data-mk="design">+ micro-frontend</button>';
    }
    // A chip pressed: the board carries a design pin with the micro-frontend's spec.
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
      // Asked for a page by its board's slug (the office's pin board): that page.
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
      else if (reply.stamp !== st.stamp) { st.stamp = reply.stamp; reloadFrame(); }
      else if (listChanged) outline();
      st.timer = setTimeout(load, POLL_MS);
    }
    function say(html) { q('.mk-origin').innerHTML = html; }
    // The branches, as the data pane offers them: main, the integration
    // branch, the milestones' in a menu.
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
      q('.mk-name').hidden = !s.edited;
      q('[data-mk="save"]').hidden = !s.edited;
      q('[data-mk="discard"]').hidden = !(s.edited || s.saved);
      q('[data-mk="discard"]').textContent = st.armed === 'discard' ? 'discard — sure?' : 'discard';
    }
    function show(path) {
      if (path === st.page && st.board && st.view === 'page') return;
      st.page = path;
      remember(st.branch, path);
      st.stamp = st.state ? st.state.stamp : st.stamp;
      drawBar();
      mountBoard();
    }
    function setView(view) {
      if (view === st.view) return;
      st.view = view;
      drawForm(false);
      if (st.state) { drawBar(); drawStrip(); mountBoard(); }
    }
    function showBranch(branch) {
      if (branch === st.branch) return;
      st.branch = branch;
      st.page = '';
      st.state = null;
      q('.dm-features').hidden = true;
      load();
    }
    // A new page: a link pin on the canvas, opened on its "new page" form —
    // a title, the objective, and an AI session that draws it.
    function newPage() {
      if (st.board && st.board.newLink) st.board.newLink();
    }
    async function save() {
      if (st.saving) return;
      const name = q('.mk-name').value.trim() || (currentPage() ? currentPage().title : st.branch);
      st.saving = true;
      const action = window.HarnessActions && window.HarnessActions.start({ key: `mockup:save:${st.branch}`, label: `save the mock-up of ${st.branch} as ${name}`, button: q('[data-mk="save"]') });
      try {
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
    }

    // ---- events --------------------------------------------------------------------------
    const onClick = (e) => {
      const branch = e.target.closest('[data-branch]');
      if (branch) { showBranch(branch.dataset.branch); return; }
      const mk = e.target.closest('[data-mk]');
      if (!mk) return;
      if (mk.dataset.mk === 'features') { const m = q('.dm-features'); m.hidden = !m.hidden; }
      if (mk.dataset.mk === 'view') setView(mk.dataset.view);
      if (mk.dataset.mk === 'new') newPage();
      if (mk.dataset.mk === 'mf') setOutlines(!st.mf);
      if (mk.dataset.mk === 'interact') setInteract(!st.interact);
      if (mk.dataset.mk === 'design') drawForm(q('.mk-form').hidden);
      if (mk.dataset.mk === 'form-cancel') drawForm(false);
      if (mk.dataset.mk === 'form-make') makeFromForm();
      if (mk.dataset.mk === 'save') save();
      if (mk.dataset.mk === 'discard') discard();
    };
    // A card without a fragment: a design pin with the manifest's spec, and its session.
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
    };
    const onKey = (e) => {
      // What is typed in the bar or the form belongs there, not to the page.
      if (e.target.closest('.mk-form') || e.target.classList.contains('mk-name')) {
        e.stopPropagation();
        if (e.key === 'Enter' && e.target.classList.contains('mk-name')) save();
        if (e.key === 'Escape' && e.target.closest('.mk-form')) drawForm(false);
      }
    };
    host.addEventListener('click', onClick);
    host.addEventListener('click', onCardMock);
    host.addEventListener('change', onChange);
    host.addEventListener('keydown', onKey);
    q('.mk-strip').addEventListener('pointerdown', onStripDown);
    const measured = setInterval(measure, 1000);

    load();

    return {
      get id() { return st.board ? st.board.id : ''; },
      reload() { st.state = null; return load(); },
      destroy() {
        st.gone = true;
        clearTimeout(st.timer);
        clearInterval(measured);
        host.removeEventListener('click', onClick);
        host.removeEventListener('click', onCardMock);
        host.removeEventListener('change', onChange);
        host.removeEventListener('keydown', onKey);
        if (st.board) { try { st.board.destroy(); } catch (e) { /* gone */ } }
        host.innerHTML = '';
      },
    };
  }

  window.MockupPane = { mount };
})();
