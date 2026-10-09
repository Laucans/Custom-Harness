// The board: a canvas to pan and zoom, and pins a human drags onto its parts.
// Reusable — a pane mounts one, draws into `board.content` (HTML placed in
// board units) and `board.edges` (an SVG in the same units), and names every
// part a pin may sit on with `data-pin="<target>"`.
//
// A pin is a note (yellow), a brainstorm (green: an AI session behind it,
// opened from the pin, remembered between visits) or an asking (orange: a
// change to consider when the next roadmap is built). Turning a brainstorm
// into a note has the AI write its conclusion; into an asking, the request.
// On the mock-up two more kinds: a link (blue) leads to another page or
// creates one, a simulate (purple) is a request an AI session answers by
// editing the page. A pin may be a zone — a rectangle, a circle or a
// hand-drawn outline around several parts — drawn with the toolbar's tools.
// Pins are kept by the server one by one (`/api/notes/<board>/<id>`); a
// deleted one comes back with ctrl+z, the last ten of a board.
//
//   const board = HarnessBoard.create(host, { id, describe, resolve, context, onSession,
//     kinds, zones, under, pages, goto, newPage });
//   board.setSize(w, h); board.refresh(); board.fit(); board.destroy();
//
// `resolve(target)` and `under(clientX, clientY)` may answer a stand-in for
// a part the board cannot reach itself — one inside an iframe — as long as
// it has getBoundingClientRect(), getAttribute('data-pin') and a classList.
(() => {
  'use strict';

  const MIN_ZOOM = 0.15;
  const MAX_ZOOM = 3;
  // A press that moves less than this is a click, not a drag.
  const SLOP = 4;
  const SAVE_DELAY = 600;
  const KINDS = {
    note: ['note', 'Your words'],
    brainstorm: ['brainstorm', 'A conversation with an AI session about this part'],
    asking: ['asking', 'A change to consider when the next roadmap is built'],
    link: ['link', 'Leads to another page of the mock-up — or creates one'],
    simulate: ['simulate', 'A request: an AI session edits the page to show its result'],
  };
  const DEFAULT_KINDS = ['note', 'brainstorm', 'asking'];
  // A zone smaller than this, in screen pixels, is a click that drew nothing.
  const MIN_ZONE = 8;
  // A hand-drawn outline keeps a point every so many board units.
  const PATH_STEP = 3;
  const MAX_POINTS = 4000;

  const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
  const newId = () => 'n' + Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
  const typing = (el) => el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.isContentEditable);

  function create(host, opts) {
    const id = opts.id;
    const describe = opts.describe || ((target) => (target ? target : 'the canvas'));
    const kinds = (opts.kinds || DEFAULT_KINDS).filter((k) => KINDS[k]);
    const base = `/api/notes/${encodeURIComponent(id)}`;
    host.classList.add('hb');
    host.innerHTML = '<div class="hb-world"><svg class="hb-edges" xmlns="http://www.w3.org/2000/svg"></svg><div class="hb-content"></div>'
      + '<svg class="hb-zones" xmlns="http://www.w3.org/2000/svg"></svg></div>'
      + '<div class="hb-pins"></div>'
      + '<div class="hb-tools" role="toolbar" aria-label="board tools">'
      + '<button type="button" class="hb-pin-tool" title="Drag this pin onto a part or the canvas to write a note">📍<span>pin</span></button>'
      + (opts.zones
        ? '<button type="button" data-zone="rect" title="Draw a rectangle around several parts">▭</button>'
          + '<button type="button" data-zone="circle" title="Draw a circle around several parts">◯</button>'
          + '<button type="button" data-zone="path" title="Draw an outline by hand">✎</button>'
        : '')
      + '<span class="hb-sep"></span>'
      + '<button type="button" data-hb="out" title="Zoom out" aria-label="zoom out">−</button>'
      + '<span class="hb-zoom" aria-live="polite">100 %</span>'
      + '<button type="button" data-hb="in" title="Zoom in" aria-label="zoom in">+</button>'
      + '<button type="button" data-hb="fit" title="Fit everything in view">fit</button>'
      + '<span class="hb-sep"></span>'
      + '<button type="button" data-hb="list" title="Every note on this board">notes <span class="hb-count">0</span></button>'
      + '<button type="button" data-hb="undo" title="Bring back the pin deleted last (ctrl+z)">undo</button>'
      + '</div>'
      + '<div class="hb-list" hidden></div>'
      + '<div class="hb-note" hidden role="dialog" aria-label="note">'
      + '<div class="hb-note-head"><span class="hb-note-on"></span><button type="button" data-note="close" title="Close (Esc)" aria-label="close">×</button></div>'
      + '<div class="hb-kinds" role="radiogroup" aria-label="kind of pin">'
      + kinds.map((k) => `<button type="button" role="radio" class="hb-kind ${k}" data-kind="${k}" title="${esc(KINDS[k][1])}">${KINDS[k][0]}</button>`).join('')
      + '</div>'
      + '<div class="hb-link" hidden>'
      + '<label>leads to <select class="hb-link-pages" aria-label="page"></select></label>'
      + '<div class="hb-link-new" hidden><input class="hb-link-title" maxlength="80" placeholder="Title of the new page"></div>'
      + '</div>'
      + '<textarea maxlength="4000" placeholder="Write a note…"></textarea>'
      + '<div class="hb-link-go" hidden><button type="button" data-note="go">Open the page →</button><button type="button" data-note="create">Create it with an AI session</button></div>'
      + '<div class="hb-session" hidden><button type="button" data-note="session">Open the AI session</button><span class="hb-turns"></span></div>'
      + '<div class="hb-note-foot"><span class="hb-note-state"></span><button type="button" data-note="delete">delete</button></div>'
      + '</div>';
    const q = (s) => host.querySelector(s);
    const world = q('.hb-world');
    const content = q('.hb-content');
    const edges = q('.hb-edges');
    const zoneLayer = q('.hb-zones');
    const pinLayer = q('.hb-pins');
    const editor = q('.hb-note');
    const textarea = editor.querySelector('textarea');
    const list = q('.hb-list');
    const linkBox = editor.querySelector('.hb-link');
    const linkPages = editor.querySelector('.hb-link-pages');
    const linkTitle = editor.querySelector('.hb-link-title');

    const view = { x: 40, y: 40, k: 1 };
    const size = { w: 800, h: 600 };
    let notes = [];
    let openId = null;
    let saveTimer = null;
    let pending = null; // the pin whose text waits to be saved
    let busy = false; // an AI is writing the open pin
    let press = null; // a press on the canvas, until it is a pan or a click
    let carry = null; // a pin being dragged: new, or one already placed
    let tool = null; // the zone tool armed: rect, circle, path
    let drawing = null; // a zone being drawn
    let swallowClick = false;

    const resolve = opts.resolve || ((target) => (target ? content.querySelector(`[data-pin="${CSS.escape(target)}"]`) : null));

    // ---- the view: where the board sits, how big ---------------------------------
    function apply() {
      world.style.transform = `translate(${view.x}px, ${view.y}px) scale(${view.k})`;
      q('.hb-zoom').textContent = Math.round(view.k * 100) + ' %';
      drawPins();
    }
    function toBoard(clientX, clientY) {
      const r = host.getBoundingClientRect();
      return { x: (clientX - r.left - view.x) / view.k, y: (clientY - r.top - view.y) / view.k };
    }
    function toScreen(x, y) {
      return { x: x * view.k + view.x, y: y * view.k + view.y };
    }
    // A part's box, in board units.
    function boxOf(el) {
      const r = el.getBoundingClientRect();
      const a = toBoard(r.left, r.top);
      return { x: a.x, y: a.y, w: r.width / view.k, h: r.height / view.k };
    }
    function zoomAt(k, sx, sy) {
      const nk = clamp(k, MIN_ZOOM, MAX_ZOOM);
      const bx = (sx - view.x) / view.k;
      const by = (sy - view.y) / view.k;
      view.k = nk;
      view.x = sx - bx * nk;
      view.y = sy - by * nk;
      apply();
    }
    function fit() {
      const r = host.getBoundingClientRect();
      const b = (opts.bounds && opts.bounds()) || { x: 0, y: 0, w: size.w, h: size.h };
      if (!r.width || !r.height || !b.w || !b.h) return;
      const k = clamp(Math.min((r.width - 60) / b.w, (r.height - 90) / b.h, 1.1), MIN_ZOOM, MAX_ZOOM);
      view.k = k;
      view.x = (r.width - b.w * k) / 2 - b.x * k;
      view.y = (r.height - b.h * k) / 2 - b.y * k + 20;
      apply();
    }
    // Centres a part, zooming in when it would read too small.
    function focus(el, minZoom = 0.8) {
      if (!el) return;
      const r = host.getBoundingClientRect();
      const b = boxOf(el);
      view.k = Math.max(view.k, minZoom);
      view.x = r.width / 2 - (b.x + b.w / 2) * view.k;
      view.y = r.height / 2 - (b.y + Math.min(b.h, r.height / view.k / 2) / 2) * view.k;
      apply();
    }
    function setSize(w, h) {
      size.w = w; size.h = h;
      content.style.width = w + 'px'; content.style.height = h + 'px';
      edges.setAttribute('width', w); edges.setAttribute('height', h);
      edges.setAttribute('viewBox', `0 0 ${w} ${h}`);
      // A zero-sized SVG paints nothing, overflow or not: the zones' layer is the board's size.
      zoneLayer.setAttribute('width', w); zoneLayer.setAttribute('height', h);
      zoneLayer.setAttribute('viewBox', `0 0 ${w} ${h}`);
    }

    // ---- the zones -----------------------------------------------------------------
    // A zone's outline, as SVG in board units; the layer transforms with the world.
    function zoneMarkup(shape, cls, extra) {
      const p = shape.points;
      if (!p || p.length < 2) return '';
      if (shape.kind === 'rect') {
        const x = Math.min(p[0][0], p[1][0]); const y = Math.min(p[0][1], p[1][1]);
        const w = Math.abs(p[1][0] - p[0][0]); const h = Math.abs(p[1][1] - p[0][1]);
        return `<rect class="${cls}" ${extra} x="${x}" y="${y}" width="${w}" height="${h}" rx="6"></rect>`;
      }
      if (shape.kind === 'circle') {
        const r = Math.hypot(p[1][0] - p[0][0], p[1][1] - p[0][1]);
        return `<circle class="${cls}" ${extra} cx="${p[0][0]}" cy="${p[0][1]}" r="${r}"></circle>`;
      }
      return `<polygon class="${cls}" ${extra} points="${p.map((q2) => `${q2[0]},${q2[1]}`).join(' ')}"></polygon>`;
    }
    // Where a zone's pin stands: the top of the outline.
    function anchorOf(shape) {
      const p = shape.points;
      if (shape.kind === 'rect') return { x: Math.min(p[0][0], p[1][0]), y: Math.min(p[0][1], p[1][1]) };
      if (shape.kind === 'circle') return { x: p[0][0], y: p[0][1] - Math.hypot(p[1][0] - p[0][0], p[1][1] - p[0][1]) };
      let top = p[0];
      for (const q2 of p) if (q2[1] < top[1]) top = q2;
      return { x: top[0], y: top[1] };
    }
    function drawZones() {
      let h = '';
      notes.forEach((n) => {
        if (!n.shape) return;
        const cls = ['hb-zone', n.kind || 'note', openId === n.id ? 'open' : ''].join(' ');
        h += zoneMarkup(n.shape, cls, `data-zone-id="${esc(n.id)}"`);
      });
      if (drawing && drawing.shape.points.length > 1) h += zoneMarkup(drawing.shape, 'hb-zone drawing', '');
      zoneLayer.innerHTML = h;
    }
    function armTool(name) {
      tool = tool === name ? null : name;
      host.querySelectorAll('[data-zone]').forEach((b) => b.classList.toggle('on', b.dataset.zone === tool));
      host.classList.toggle('drawing', !!tool);
    }
    function startDraw(e) {
      const at = toBoard(e.clientX, e.clientY);
      drawing = { shape: { kind: tool, points: [[at.x, at.y], [at.x, at.y]] }, pointer: e.pointerId, startX: e.clientX, startY: e.clientY };
      host.setPointerCapture(e.pointerId);
      if (openId) closeNote();
      list.hidden = true;
    }
    function moveDraw(e) {
      const at = toBoard(e.clientX, e.clientY);
      const p = drawing.shape.points;
      if (drawing.shape.kind === 'path') {
        const last = p[p.length - 1];
        if (Math.hypot(at.x - last[0], at.y - last[1]) >= PATH_STEP && p.length < MAX_POINTS) p.push([at.x, at.y]);
      } else {
        p[1] = [at.x, at.y];
      }
      drawZones();
    }
    function endDraw(e) {
      const d = drawing;
      drawing = null;
      armTool(null);
      const moved = Math.hypot(e.clientX - d.startX, e.clientY - d.startY);
      const enough = d.shape.kind === 'path' ? d.shape.points.length >= 3 : moved >= MIN_ZONE;
      if (!enough) { drawZones(); return; }
      const a = anchorOf(d.shape);
      const n = { id: newId(), text: '', at: '', kind: 'note', turns: 0, fresh: true, target: '', dx: 0, dy: 0, x: a.x, y: a.y, shape: d.shape };
      notes.push(n);
      drawZones();
      openNote(n);
    }

    // ---- the pins ----------------------------------------------------------------
    // Where a note's pin stands, in board units: on its part when the part is
    // drawn — kept inside its box, so a field's pin lands on its folded
    // table — else where it was last seen.
    function placeOf(n) {
      const el = n.target ? resolve(n.target) : null;
      if (!el) return { x: n.x, y: n.y, orphan: !!n.target };
      const b = boxOf(el);
      const x = b.x + clamp(n.dx, 0, b.w);
      const y = b.y + clamp(n.dy, 0, b.h);
      n.x = x; n.y = y;
      return { x, y, orphan: false };
    }
    function drawPins() {
      const r = host.getBoundingClientRect();
      let h = '';
      notes.forEach((n, i) => {
        if (carry && carry.note === n) return;
        const p = n.shape ? Object.assign(anchorOf(n.shape), { orphan: false }) : placeOf(n);
        if (n.shape) { n.x = p.x; n.y = p.y; }
        const s = toScreen(p.x, p.y);
        if (s.x < -30 || s.y < -30 || s.x > r.width + 30 || s.y > r.height + 60) return;
        const kind = n.kind || 'note';
        const cls = ['hb-pin', kind, p.orphan ? 'orphan' : '', n.shape ? 'zone' : '', n.text || kind !== 'note' ? '' : 'empty', openId === n.id ? 'open' : ''].join(' ');
        const tip = `${kind}${n.shape ? ' (zone)' : ''}: ${n.text || (kind === 'link' && n.page ? '→ ' + n.page : 'empty')}${p.orphan ? ' — what it was on is gone' : ''}`;
        h += `<button type="button" class="${cls}" data-note-id="${esc(n.id)}" style="left:${s.x}px;top:${s.y}px" title="${esc(tip)}"><span>${i + 1}</span></button>`;
      });
      pinLayer.innerHTML = h;
      q('.hb-count').textContent = notes.length;
      drawZones();
      placeEditor();
    }
    function placeEditor() {
      if (!openId) return;
      const n = notes.find((x) => x.id === openId);
      if (!n) { closeNote(); return; }
      const r = host.getBoundingClientRect();
      const s = toScreen(n.x, n.y);
      const w = editor.offsetWidth || 280;
      const h = editor.offsetHeight || 190;
      let left = s.x + 18;
      if (left + w > r.width - 8) left = s.x - w - 18;
      editor.style.left = clamp(left, 8, Math.max(8, r.width - w - 8)) + 'px';
      editor.style.top = clamp(s.y - 30, 8, Math.max(8, r.height - h - 8)) + 'px';
    }
    // The editor shows the pin's kind, its text, and — for a brainstorm or
    // an asking — its AI session.
    function drawEditor(n) {
      const kind = n.kind || 'note';
      editor.dataset.kind = kind;
      editor.querySelectorAll('.hb-kind').forEach((b) => {
        b.classList.toggle('on', b.dataset.kind === kind);
        b.setAttribute('aria-checked', b.dataset.kind === kind);
        // Not `disabled`: a log button inside must stay clickable.
        if (busy) b.setAttribute('aria-disabled', 'true'); else b.removeAttribute('aria-disabled');
      });
      const session = editor.querySelector('.hb-session');
      session.hidden = kind === 'note' || kind === 'link' || !opts.onSession;
      editor.querySelector('.hb-turns').textContent = n.turns ? `${n.turns} turn${n.turns > 1 ? 's' : ''} kept` : 'no conversation yet';
      textarea.disabled = busy;
      textarea.placeholder = {
        note: 'Write a note — or the first prompt of a brainstorm, an asking or a simulate…',
        link: 'The objective of the page this leads to — what a new page is drawn from',
        simulate: 'The request: what happens, and the page shows the result',
      }[kind] || 'Where the discussion stands — rewritten when you leave the session';
      drawLink(n, kind);
    }
    // A link's editor: the page it leads to, among the pages the pane knows,
    // or a new one to create — named here, drawn by an AI session.
    function drawLink(n, kind) {
      linkBox.hidden = kind !== 'link' || !opts.pages;
      editor.querySelector('.hb-link-go').hidden = linkBox.hidden;
      if (linkBox.hidden) return;
      const pages = opts.pages() || [];
      const known = pages.some((p) => p.path === n.page);
      const fresh = n.page && !known;
      linkPages.innerHTML = '<option value="">choose a page…</option>'
        + pages.map((p) => `<option value="${esc(p.path)}"${p.path === n.page ? ' selected' : ''}>${esc(p.title)}</option>`).join('')
        + `<option value="__new"${fresh ? ' selected' : ''}>+ a new page…</option>`;
      editor.querySelector('.hb-link-new').hidden = !fresh && linkPages.value !== '__new';
      if (fresh && !linkTitle.value) linkTitle.value = n.page.replace(/^pages\//, '').replace(/\.html$/, '').replace(/-/g, ' ');
      editor.querySelector('[data-note="go"]').hidden = !known;
      editor.querySelector('[data-note="create"]').hidden = !fresh || !opts.onSession;
      editor.querySelector('[data-note="create"]').textContent = n.turns ? 'Continue the page\'s session' : 'Create it with an AI session';
    }
    // `pages/<slug>.html`, from a title — the same plain name the server makes.
    const pagePath = (title) => {
      const slug = title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 60);
      return `pages/${slug || 'page'}.html`;
    };
    function openNote(n) {
      flush();
      openId = n.id;
      editor.hidden = false;
      editor.querySelector('.hb-note-on').textContent = 'on ' + describe(n.target);
      textarea.value = n.text || '';
      setState(n.at ? 'kept ' + window.HarnessClock.stamp(n.at).slice(0, 16) : '');
      drawEditor(n);
      drawPins();
      setTimeout(() => textarea.focus(), 0);
    }
    function closeNote() {
      flush();
      const n = notes.find((x) => x.id === openId);
      openId = null;
      editor.hidden = true;
      // An empty note that never was saved is a pin dropped by mistake.
      if (n && n.fresh && !n.text.trim() && (n.kind || 'note') === 'note' && !n.shape) {
        notes = notes.filter((x) => x !== n);
      }
      // A zone drawn and left without a word is still a zone: kept.
      if (n && n.fresh && n.shape) save(n, true);
      drawPins();
    }
    function setState(text) { editor.querySelector('.hb-note-state').textContent = text; }
    async function put(n) {
      const body = Object.assign({}, n);
      delete body.fresh;
      const res = await fetch(`${base}/${encodeURIComponent(n.id)}`, { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
      if (!res.ok) throw new Error(await res.text());
      delete n.fresh;
    }
    // Saves a pin now, or a beat after the last keystroke.
    function save(n, now) {
      clearTimeout(saveTimer);
      pending = n;
      const send = async () => {
        if (pending !== n) return;
        pending = null;
        if (openId === n.id) setState('saving…');
        try {
          await put(n);
          if (openId === n.id) setState('saved');
        } catch (e) {
          if (openId === n.id) setState('not saved: ' + (e.message || e));
        }
      };
      if (now) send(); else saveTimer = setTimeout(send, SAVE_DELAY);
    }
    // What waits to be saved is saved now: a pin closed, a pane left.
    function flush() {
      if (!pending) return;
      clearTimeout(saveTimer);
      const n = pending;
      pending = null;
      put(n).catch(() => { /* the next edit saves it again */ });
    }
    async function remove(n) {
      notes = notes.filter((x) => x !== n);
      if (openId === n.id) { openId = null; editor.hidden = true; }
      if (pending === n) { pending = null; clearTimeout(saveTimer); }
      drawPins();
      if (n.fresh) return;
      try { await fetch(`${base}/${encodeURIComponent(n.id)}`, { method: 'DELETE' }); } catch (e) { /* shown by its absence on reload */ }
    }
    async function undo() {
      try {
        const res = await fetch(`${base}/undo`, { method: 'POST' });
        if (!res.ok) return;
        const n = await res.json();
        if (!notes.some((x) => x.id === n.id)) notes.push(n);
        drawPins();
      } catch (e) { /* nothing to bring back */ }
    }
    async function load() {
      try {
        const res = await fetch(base);
        notes = res.ok ? await res.json() : [];
      } catch (e) { notes = []; }
      drawPins();
    }
    function showList() {
      if (!list.hidden) { list.hidden = true; return; }
      list.innerHTML = notes.length
        ? notes.map((n, i) => `<button type="button" data-goto-note="${esc(n.id)}"><b class="${esc(n.kind || 'note')}">${i + 1}</b><span>${esc(n.text || (n.kind || 'note'))}</span><small>${esc(n.kind || 'note')} · ${esc(describe(n.target))}</small></button>`).join('')
        : '<div class="hb-empty">No note yet. Drag the pin onto a part of the board.</div>';
      list.hidden = false;
    }
    // A pin's kind changed. From a brainstorm, the AI writes what it becomes:
    // the conclusion for a note, the request for an asking.
    async function rekind(n, kind) {
      const from = n.kind || 'note';
      if (kind === from || busy) return;
      if (from === 'brainstorm' && kind !== 'brainstorm') {
        if (n.fresh || !n.turns) {
          setState('nothing said yet: open the AI session first');
          return;
        }
        flush();
        busy = true;
        drawEditor(n);
        setState(kind === 'note' ? 'the AI draws the conclusion…' : 'the AI formulates the request…');
        const action = window.HarnessActions && window.HarnessActions.start({
          key: `pin:${n.id}:${kind}`,
          label: kind === 'note' ? 'conclude the brainstorm into a note' : 'turn the brainstorm into an asking',
          button: editor.querySelector(`.hb-kind.${kind}`),
        });
        try {
          const res = await fetch(`${base}/${encodeURIComponent(n.id)}/into/${kind}`, { method: 'POST' });
          const text = await res.text();
          if (!res.ok) throw new Error(text);
          Object.assign(n, JSON.parse(text));
          if (openId === n.id) textarea.value = n.text;
          setState(kind === 'note' ? 'concluded' : 'formulated');
          if (action) action.ok(n.text.split('\n')[0]);
        } catch (e) {
          setState('not changed: ' + (e.message || e));
          if (action) action.fail(e.message || e);
        } finally {
          busy = false;
          drawEditor(n);
          drawPins();
        }
        return;
      }
      // A brainstorm, an asking or a simulate opens its session on what the note says.
      if (kind !== 'note' && kind !== 'link' && !(n.text || '').trim()) {
        setState(kind === 'simulate' ? 'write the request first: it is what the AI simulates' : 'write the first prompt first: the note is what the AI is asked');
        textarea.focus();
        return;
      }
      n.kind = kind;
      n.at = new Date().toISOString();
      drawEditor(n);
      drawPins();
      save(n, true);
    }

    // ---- carrying a pin: a new one off the toolbar, or one already placed ----------
    function startCarry(e, note) {
      e.preventDefault();
      e.stopPropagation();
      list.hidden = true;
      const ghost = document.createElement('div');
      ghost.className = 'hb-pin ghost ' + ((note && note.kind) || 'note');
      ghost.innerHTML = '<span>+</span>';
      pinLayer.appendChild(ghost);
      carry = { note, ghost, over: null, startX: e.clientX, startY: e.clientY, moved: !note };
      host.setPointerCapture(e.pointerId);
      moveCarry(e);
      host.classList.add('placing');
      if (note) drawPins();
    }
    function moveCarry(e) {
      const r = host.getBoundingClientRect();
      if (Math.hypot(e.clientX - carry.startX, e.clientY - carry.startY) > SLOP) carry.moved = true;
      carry.ghost.style.left = (e.clientX - r.left) + 'px';
      carry.ghost.style.top = (e.clientY - r.top) + 'px';
      carry.ghost.hidden = true;
      const under = document.elementFromPoint(e.clientX, e.clientY);
      carry.ghost.hidden = false;
      let part = under && content.contains(under) ? under.closest('[data-pin]') : null;
      // A part the board cannot see itself — inside an iframe — is asked of the pane.
      if (!part && opts.under && under && content.contains(under)) part = opts.under(e.clientX, e.clientY);
      if (carry.over !== part) {
        if (carry.over) carry.over.classList.remove('hb-target');
        if (part) part.classList.add('hb-target');
        carry.over = part;
      }
    }
    function dropCarry(e) {
      const c = carry;
      carry = null;
      host.classList.remove('placing');
      c.ghost.remove();
      if (c.over) c.over.classList.remove('hb-target');
      // A placed pin pressed and released in place is opened, not moved.
      if (c.note && !c.moved) { openNote(c.note); return; }
      // The pointer is captured: what lies under it is asked, not the event's target.
      const under = document.elementFromPoint(e.clientX, e.clientY);
      const r = host.getBoundingClientRect();
      const inside = e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
      const onTools = under && under.closest('.hb-tools, .hb-list, .hb-note');
      if (!inside || onTools) { drawPins(); return; }
      const at = toBoard(e.clientX, e.clientY);
      const target = c.over ? c.over.getAttribute('data-pin') : '';
      const b = c.over ? boxOf(c.over) : { x: 0, y: 0 };
      const n = c.note || { id: newId(), text: '', at: '', kind: 'note', turns: 0, fresh: true };
      // A zone moved is a zone translated: its outline follows.
      if (n.shape) {
        const ddx = at.x - n.x; const ddy = at.y - n.y;
        n.shape.points = n.shape.points.map(([px, py]) => [px + ddx, py + ddy]);
        Object.assign(n, { target: '', dx: 0, dy: 0, x: at.x, y: at.y });
      } else {
        Object.assign(n, { target, dx: at.x - b.x, dy: at.y - b.y, x: at.x, y: at.y });
      }
      if (!c.note) notes.push(n);
      else if (!n.fresh) save(n, true);
      openNote(n);
    }

    // ---- events ------------------------------------------------------------------
    const onPointerDown = (e) => {
      if (e.button !== 0) return;
      const pinTool = e.target.closest('.hb-pin-tool');
      if (pinTool) return startCarry(e, null);
      const pin = e.target.closest('.hb-pin[data-note-id]');
      if (pin) return startCarry(e, notes.find((n) => n.id === pin.dataset.noteId));
      if (e.target.closest('.hb-tools, .hb-note, .hb-list')) return undefined;
      const zone = e.target.closest('[data-zone-id]');
      if (zone && !tool) {
        const n = notes.find((x) => x.id === zone.dataset.zoneId);
        if (n) { openNote(n); return undefined; }
      }
      if (tool) { startDraw(e); return undefined; }
      if (openId) closeNote();
      list.hidden = true;
      press = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y, panning: false, pointer: e.pointerId };
      return undefined;
    };
    const onPointerMove = (e) => {
      if (carry) return moveCarry(e);
      if (drawing) { moveDraw(e); return undefined; }
      if (!press) return undefined;
      const dx = e.clientX - press.x;
      const dy = e.clientY - press.y;
      if (!press.panning && Math.hypot(dx, dy) > SLOP) {
        press.panning = true;
        host.setPointerCapture(press.pointer);
        host.classList.add('panning');
      }
      if (press.panning) {
        view.x = press.vx + dx;
        view.y = press.vy + dy;
        apply();
      }
      return undefined;
    };
    const onPointerUp = (e) => {
      if (carry) return dropCarry(e);
      if (drawing) { endDraw(e); swallowClick = true; return undefined; }
      if (press && press.panning) swallowClick = true;
      press = null;
      host.classList.remove('panning');
      return undefined;
    };
    // A drag ends with a click on what was under it; that click is not one.
    const onClickCapture = (e) => {
      if (swallowClick) { swallowClick = false; e.stopPropagation(); e.preventDefault(); }
    };
    const onWheel = (e) => {
      if (e.target.closest('.hb-note, .hb-list')) return;
      e.preventDefault();
      const r = host.getBoundingClientRect();
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1;
      const rate = e.ctrlKey ? 0.01 : 0.0015;
      zoomAt(view.k * Math.exp(-e.deltaY * unit * rate), e.clientX - r.left, e.clientY - r.top);
    };
    // Opens the open pin's AI session; a pin the server does not have yet
    // is saved first.
    async function openSession(n) {
      flush();
      if (n.fresh) {
        try { await put(n); } catch (err) { setState('not saved: ' + (err.message || err)); return; }
      }
      opts.onSession(n, opts.context ? opts.context(n.target) : '');
    }
    const onToolClick = (e) => {
      const zoneTool = e.target.closest('[data-zone]');
      if (zoneTool) { armTool(zoneTool.dataset.zone); return; }
      const b = e.target.closest('[data-hb]');
      if (b) {
        const r = host.getBoundingClientRect();
        if (b.dataset.hb === 'in') zoomAt(view.k * 1.25, r.width / 2, r.height / 2);
        if (b.dataset.hb === 'out') zoomAt(view.k / 1.25, r.width / 2, r.height / 2);
        if (b.dataset.hb === 'fit') fit();
        if (b.dataset.hb === 'list') showList();
        if (b.dataset.hb === 'undo') undo();
        return;
      }
      const go = e.target.closest('[data-goto-note]');
      if (go) {
        list.hidden = true;
        const n = notes.find((x) => x.id === go.dataset.gotoNote);
        if (!n) return;
        const r = host.getBoundingClientRect();
        placeOf(n);
        view.x = r.width / 2 - n.x * view.k;
        view.y = r.height / 2 - n.y * view.k;
        apply();
        openNote(n);
        return;
      }
      const n = notes.find((x) => x.id === openId);
      const kindButton = e.target.closest('.hb-kind');
      if (kindButton && n) { rekind(n, kindButton.dataset.kind); return; }
      const act = e.target.closest('[data-note]');
      if (!act) return;
      if (act.dataset.note === 'close') closeNote();
      if (act.dataset.note === 'delete' && n) remove(n);
      if (act.dataset.note === 'session' && n && opts.onSession) openSession(n);
      if (act.dataset.note === 'go' && n && n.page && opts.goto) { flush(); opts.goto(n.page); }
      if (act.dataset.note === 'create' && n && n.page && opts.onSession) openSession(n);
    };
    // The page a link leads to: chosen among the pane's, or a new one named here.
    const onLinkChange = () => {
      const n = notes.find((x) => x.id === openId);
      if (!n) return;
      if (linkPages.value === '__new') {
        editor.querySelector('.hb-link-new').hidden = false;
        linkTitle.focus();
        return;
      }
      n.page = linkPages.value;
      n.at = new Date().toISOString();
      save(n, true);
      drawLink(n, 'link');
      drawPins();
    };
    const onLinkTitle = () => {
      const n = notes.find((x) => x.id === openId);
      if (!n || !linkTitle.value.trim()) return;
      n.page = pagePath(linkTitle.value);
      n.at = new Date().toISOString();
      save(n, false);
      editor.querySelector('[data-note="create"]').hidden = !opts.onSession;
    };
    const onInput = () => {
      const n = notes.find((x) => x.id === openId);
      if (!n || busy) return;
      n.text = textarea.value;
      n.at = new Date().toISOString();
      setState('…');
      save(n, false);
      drawPins();
    };
    // Keys typed in a note belong to the note: the page's Escape and
    // Backspace would leave the pane.
    const onKey = (e) => {
      e.stopPropagation();
      if (e.key === 'Escape') { e.preventDefault(); closeNote(); }
    };
    // ctrl+z (cmd+z) anywhere but in a field brings back the pin deleted last.
    const onUndoKey = (e) => {
      if (!(e.ctrlKey || e.metaKey) || e.key.toLowerCase() !== 'z' || e.shiftKey) return;
      if (typing(e.target) || !host.isConnected) return;
      e.preventDefault();
      undo();
    };

    host.addEventListener('pointerdown', onPointerDown);
    host.addEventListener('pointermove', onPointerMove);
    host.addEventListener('pointerup', onPointerUp);
    host.addEventListener('pointercancel', onPointerUp);
    host.addEventListener('click', onClickCapture, true);
    host.addEventListener('click', onToolClick);
    host.addEventListener('wheel', onWheel, { passive: false });
    textarea.addEventListener('input', onInput);
    linkPages.addEventListener('change', onLinkChange);
    linkTitle.addEventListener('input', onLinkTitle);
    editor.addEventListener('keydown', onKey);
    document.addEventListener('keydown', onUndoKey);
    const resized = new ResizeObserver(() => drawPins());
    resized.observe(host);

    load();
    apply();

    return {
      id,
      content,
      edges,
      setSize,
      fit,
      focus,
      refresh: drawPins,
      reload: load,
      zoom: () => view.k,
      // A link to a page that does not exist yet, dropped near the top left
      // of what is shown and opened on its "new page" form.
      newLink() {
        if (!kinds.includes('link')) return;
        const r = host.getBoundingClientRect();
        const at = toBoard(r.left + 70, r.top + 90);
        const n = { id: newId(), text: '', at: '', kind: 'link', page: '', turns: 0, fresh: true, target: '', dx: 0, dy: 0, x: at.x, y: at.y };
        notes.push(n);
        openNote(n);
        linkPages.value = '__new';
        editor.querySelector('.hb-link-new').hidden = false;
        linkTitle.value = '';
        setTimeout(() => linkTitle.focus(), 0);
      },
      notesOn: (target) => notes.filter((n) => n.target === target),
      open: (noteId) => { const n = notes.find((x) => x.id === noteId); if (n) openNote(n); },
      destroy() {
        // A note being typed is not lost to a closed pane.
        flush();
        document.removeEventListener('keydown', onUndoKey);
        resized.disconnect();
        host.innerHTML = '';
        host.classList.remove('hb', 'placing', 'panning');
      },
    };
  }

  window.HarnessBoard = { create };
})();
