// The mock-up on its easel: the product's frontend, mocked — the static pages
// under `mock/` of the branch shown — run in an iframe on a board, so a
// human pins notes on its components, draws zones around several, links a
// page to another (or creates one from an objective), and asks for a
// simulation an AI session answers by editing the page. One board per
// branch and page: pins stay where they were left when the page changes.
//
// The pages come from `/mock/<branch slug>/<path>`; the branch's state —
// its pages, whether a session edited it, a stamp that changes with its
// files — from `/api/mockup?branch=`. A branch edited by a session is
// shown as its workspace holds it, and the page reloads as files change;
// `save the mock-up` commits it on a `mockup/<name>` branch, `discard` drops it.
(() => {
  'use strict';

  // The width a page is laid out at, in board units.
  const WIDTH = 1280;
  const MIN_HEIGHT = 480;
  const POLL_MS = 3000;
  const LAST = 'harness.mockup.page';

  const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const plain = (s) => String(s || '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
  const slugOf = (branch) => String(branch || '').replace(/\//g, '__');
  const pageSlug = (path) => plain(String(path || '').replace(/^pages\//, '').replace(/\.html?$/, '')) || 'home';
  // The parts of a page a pin may sit on, nearest first.
  const PARTS = '[data-component],[id],button,a,input,select,textarea,label,h1,h2,h3,h4,img,table,tr,li,form,section,article,aside,nav,header,footer,figure,p';

  function remembered(branch) {
    try { return JSON.parse(localStorage.getItem(LAST) || '{}')[branch] || ''; } catch (e) { return ''; }
  }
  function remember(branch, page) {
    try {
      const all = JSON.parse(localStorage.getItem(LAST) || '{}');
      all[branch] = page;
      localStorage.setItem(LAST, JSON.stringify(all));
    } catch (e) { /* a preference, not a record */ }
  }

  function mount(host, opts) {
    host.innerHTML = '<div class="dm mk">'
      + '<div class="dm-switch" hidden><div class="dm-branches" role="group" aria-label="branch"></div>'
      + '<div class="dm-features" hidden role="menu" aria-label="feature branches"></div></div>'
      + '<div class="dm-bar"><span class="dm-origin mk-origin">reading the mock-up…</span>'
      + '<label class="mk-page" hidden>page <select class="mk-pages" aria-label="page"></select></label>'
      + '<button type="button" class="mini" data-mk="new" title="A new page of the mock-up, drawn by an AI session from its objective">+ page</button>'
      + '<button type="button" class="mini" data-mk="interact" aria-pressed="false" title="Click through to the page itself: its links, its buttons">interact</button>'
      + '<span class="spacer"></span>'
      + '<span class="mk-state"></span>'
      + '<input class="mk-name" maxlength="80" placeholder="name of the mock-up" hidden aria-label="name of the mock-up">'
      + '<button type="button" class="mini" data-mk="save" hidden>save the mock-up</button>'
      + '<button type="button" class="mini" data-mk="discard" hidden>discard</button>'
      + '</div>'
      + '<div class="dm-main"><div class="dm-board mk-board"></div></div>'
      + '</div>';
    const q = (s) => host.querySelector(s);
    const st = { branch: (opts && opts.branch) || '', page: (opts && opts.page) || '', versions: null, state: null, stamp: '',
      timer: null, gone: false, interact: false, board: null, frame: null, shield: null, doc: null, armed: null, proxies: new WeakMap(), fitted: false, saving: false };

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
      if (!target) return null;
      const el = elementOf(target);
      return el ? proxy(el) : null;
    };
    const under = (clientX, clientY) => {
      if (!st.frame || !st.doc) return null;
      const fr = st.frame.getBoundingClientRect();
      if (clientX < fr.left || clientX > fr.right || clientY < fr.top || clientY > fr.bottom) return null;
      const k = st.board ? st.board.zoom() : 1;
      let el = st.doc.elementFromPoint((clientX - fr.left) / k, (clientY - fr.top) / k);
      el = el && el.closest(PARTS);
      if (!el || el === st.doc.body || el === st.doc.documentElement) return null;
      return proxy(el);
    };
    const describe = (target) => {
      if (!target) return 'the page';
      if (target.startsWith('component:')) return 'component ' + target.slice(10);
      if (target.startsWith('id:')) return '#' + target.slice(3);
      return target.slice(5).split('>').map((s) => s.replace(/:nth-child\(\d+\)/, '')).slice(-3).join(' > ');
    };
    // What a pin's session is told: the page, its objective, the part's markup.
    const context = (target) => {
      const page = currentPage();
      let text = page ? `Page ${page.path} — ${page.title}: ${page.objective || '(no objective written)'}` : `Page ${st.page || '(none yet)'}`;
      if (page && page.screen) text += `\nMocks the screen ${page.screen}.`;
      if (page && page.components && page.components.length) text += `\nComponents: ${page.components.join(', ')}.`;
      const el = elementOf(target);
      if (el) {
        const html = el.outerHTML.replace(/\s+/g, ' ');
        text += `\nThe part the pin sits on (${describe(target)}):\n${html.length > 1500 ? html.slice(0, 1500) + '…' : html}`;
      }
      return text;
    };
    const currentPage = () => (st.state && st.state.pages || []).find((p) => p.path === st.page) || null;
    const boardOf = () => `mockup--${plain(st.branch) || 'integration'}--${st.page ? pageSlug(st.page) : 'home'}`;
    const frameSrc = () => `/mock/${encodeURIComponent(slugOf(st.branch))}/${st.page}?v=${encodeURIComponent(st.stamp)}`;

    // ---- the board, one per branch and page ------------------------------------------
    function mountBoard() {
      if (st.board) { try { st.board.destroy(); } catch (e) { /* gone */ } }
      st.board = null; st.frame = null; st.doc = null; st.shield = null; st.fitted = false; st.proxies = new WeakMap();
      const onSession = opts && opts.onSession
        ? (note, ctx) => opts.onSession({ board: boardOf(), id: note.id, on: note.kind === 'link' ? 'page ' + (note.page || '?') : describe(note.target), kind: note.kind, context: ctx, branch: st.branch, page: st.page })
        : null;
      st.board = window.HarnessBoard.create(q('.mk-board'), {
        id: boardOf(),
        describe,
        resolve,
        context,
        onSession,
        under,
        kinds: ['note', 'brainstorm', 'asking', 'link', 'simulate'],
        zones: true,
        pages: () => (st.state && st.state.pages) || [],
        goto: (path) => show(path),
        bounds: () => ({ x: 0, y: 0, w: WIDTH, h: Math.max(MIN_HEIGHT, st.frame ? parseFloat(st.frame.style.height) || MIN_HEIGHT : MIN_HEIGHT) }),
      });
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
      if (!doc.getElementById('hb-style')) {
        const style = doc.createElement('style');
        style.id = 'hb-style';
        style.textContent = '.hb-target{outline:2px dashed #e2b240 !important;outline-offset:2px}';
        (doc.head || doc.documentElement).appendChild(style);
      }
      measure();
      if (!st.fitted) { st.fitted = true; st.board.fit(); }
      st.board.refresh();
    }
    // The page is as tall as its content: the whole page is on the board.
    function measure() {
      if (!st.doc || !st.frame) return;
      const h = Math.max(MIN_HEIGHT, st.doc.documentElement.scrollHeight, st.doc.body ? st.doc.body.scrollHeight : 0);
      if (Math.abs(h - (parseFloat(st.frame.style.height) || 0)) > 2) {
        st.frame.style.height = h + 'px';
        st.board.setSize(WIDTH, h);
      }
    }
    function reloadFrame() {
      if (!st.frame) { mountBoard(); return; }
      st.frame.src = frameSrc();
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
      if (!st.page || !pages.some((p) => p.path === st.page)) {
        const last = remembered(st.branch);
        st.page = pages.some((p) => p.path === last) ? last : (pages[0] ? pages[0].path : '');
      }
      drawBar();
      const pageChanged = !before || !st.board || st.board.id !== boardOf();
      if (pageChanged) { st.stamp = reply.stamp; mountBoard(); }
      else if (reply.stamp !== st.stamp) { st.stamp = reply.stamp; reloadFrame(); }
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
      say(`<span class="dm-src ${s.present ? 'ok' : ''}" title="mock/pages.json on ${esc(st.branch)}">${s.present ? `${pages.length} page${pages.length === 1 ? '' : 's'}` : 'no mock-up yet'}</span>`
        + (page ? ` <span class="mk-objective" title="${esc(page.objective)}">${esc(page.title)}${page.screen ? ` · screen ${esc(page.screen)}` : ''}</span>` : ''));
      const label = q('.mk-page');
      label.hidden = !pages.length;
      q('.mk-pages').innerHTML = pages.map((p) => `<option value="${esc(p.path)}"${p.path === st.page ? ' selected' : ''}>${esc(p.title)}</option>`).join('');
      const state = q('.mk-state');
      state.innerHTML = s.edited ? '<span class="dm-src bad">edited, not saved</span>' : s.saved ? `<span class="dm-src ok">saved as ${esc(s.saved)}</span>` : '';
      q('.mk-name').hidden = !s.edited;
      q('[data-mk="save"]').hidden = !s.edited;
      q('[data-mk="discard"]').hidden = !(s.edited || s.saved);
      q('[data-mk="discard"]').textContent = st.armed === 'discard' ? 'discard — sure?' : 'discard';
    }
    function show(path) {
      if (path === st.page && st.board) return;
      st.page = path;
      remember(st.branch, path);
      st.stamp = st.state ? st.state.stamp : st.stamp;
      drawBar();
      mountBoard();
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

    // ---- events --------------------------------------------------------------------------
    const onClick = (e) => {
      const branch = e.target.closest('[data-branch]');
      if (branch) { showBranch(branch.dataset.branch); return; }
      const mk = e.target.closest('[data-mk]');
      if (!mk) return;
      if (mk.dataset.mk === 'features') { const m = q('.dm-features'); m.hidden = !m.hidden; }
      if (mk.dataset.mk === 'new') newPage();
      if (mk.dataset.mk === 'interact') setInteract(!st.interact);
      if (mk.dataset.mk === 'save') save();
      if (mk.dataset.mk === 'discard') discard();
    };
    const onChange = (e) => {
      if (e.target.classList.contains('mk-pages')) show(e.target.value);
    };
    const onKey = (e) => {
      // The name of the mock-up is typed here, not in the page.
      if (e.target.classList.contains('mk-name')) { e.stopPropagation(); if (e.key === 'Enter') save(); }
    };
    host.addEventListener('click', onClick);
    host.addEventListener('change', onChange);
    host.addEventListener('keydown', onKey);
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
        host.removeEventListener('change', onChange);
        host.removeEventListener('keydown', onKey);
        if (st.board) { try { st.board.destroy(); } catch (e) { /* gone */ } }
        host.innerHTML = '';
      },
    };
  }

  window.MockupPane = { mount };
})();
