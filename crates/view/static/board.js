// The board: a canvas to pan and zoom, and pins a human drags onto its parts.
// Reusable — a pane mounts one, draws into `board.content` (HTML placed in
// board units) and `board.edges` (an SVG in the same units), and names every
// part a pin may sit on with `data-pin="<target>"`.
//
// A pin is a note (yellow) or a request (orange: an intention on the part,
// applied to the page's draft by one AI call or discussed in a session
// that is remembered between visits, kept for the next roadmap; the former
// asking, simulate and brainstorm kinds read as this one). On the
// mock-up two more kinds: a link (blue) leads to another page or creates
// one, a design (teal) is a micro-frontend to make or inject. A pin may be
// a zone — a rectangle, a circle or a
// hand-drawn outline around several parts — drawn with the toolbar's tools.
// Pins are kept by the server one by one (`/api/notes/<board>/<id>`); a
// deleted one comes back with undo (cmd+z on a Mac, ctrl+z elsewhere), the
// last ten of a board; redo (shift added) takes the undo back.
//
//   const board = HarnessBoard.create(host, { id, describe, resolve, context, onSession,
//     kinds, zones, under, pages, goto, newPage });
//   board.setSize(w, h); board.refresh(); board.fit(); board.destroy();
//
// `resolve(target)` and `under(clientX, clientY)` may answer a stand-in for
// a part the board cannot reach itself — one inside an iframe — as long as
// it has getBoundingClientRect(), getAttribute('data-pin') and a classList.
//   board.setBoard(id)   // another set of pins: the same canvas, another branch
//
// `onChange()` is told when a pin is kept or goes.
// `only(note)` keeps the pins shown to some of the board's — the design
// system's one-component view shows that component's pins alone.
(() => {
  'use strict';
  // The shortcut key: cmd on a Mac, ctrl elsewhere.
  const MAC = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
  const modded = (e) => (MAC ? e.metaKey : e.ctrlKey);
  const KEYS = { undo: MAC ? '⌘Z' : 'ctrl+z', redo: MAC ? '⌘⇧Z' : 'ctrl+shift+z' };

  const MIN_ZOOM = 0.15;
  const MAX_ZOOM = 3;
  // A press that moves less than this is a click, not a drag.
  const SLOP = 4;
  const SAVE_DELAY = 600;
  const KINDS = {
    note: ['note', 'Your words'],
    request: ['request', 'An intention for the next roadmap — a change, a behaviour; on the mock-up, drafted by an AI when you press Propose'],
    link: ['link', 'Leads to another page of the mock-up — or creates one'],
    design: ['design', 'A micro-frontend: made by an AI session from its spec, or injected where the pin sits'],
  };
  const DEFAULT_KINDS = ['note', 'request'];
  // What an old pin's kind reads as now: a brainstorm was a conversation
  // about a part, which a request's "discuss it" now covers.
  const RENAMED = { asking: 'request', simulate: 'request', brainstorm: 'request' };
  const kindOf = (n) => RENAMED[n.kind] || n.kind || 'note';
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
    // Every listener this one sets on what outlives it (its host, the
    // document, the window) goes with it: a host given to the next one
    // would answer twice otherwise.
    const listening = new AbortController();
    const listen = (target, type, fn, options) => target.addEventListener(type, fn, Object.assign(typeof options === 'boolean' ? { capture: options } : Object.assign({}, options), { signal: listening.signal }));
    // No id yet: no pin is read until `setBoard` names the board.
    let id = opts.id || null;
    const describe = opts.describe || ((target) => (target ? target : 'the canvas'));
    const kinds = (opts.kinds || DEFAULT_KINDS).filter((k) => KINDS[k]);
    const baseOf = (board) => (board ? `/api/notes/${encodeURIComponent(board)}` : null);
    let base = baseOf(id);
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
      + `<button type="button" data-hb="undo" title="Undo the last step (${KEYS.undo})">undo</button>`
      + `<button type="button" data-hb="redo" title="Redo the step undone last (${KEYS.redo})">redo</button>`
      + '</div>'
      + '<div class="hb-list" hidden></div>'
      + '<div class="hb-note" hidden role="dialog" aria-label="note">'
      + '<div class="hb-note-head"><span class="hb-note-on"></span><button type="button" data-note="close" title="Close (Esc)" aria-label="close">×</button></div>'
      + '<div class="hb-kinds" role="radiogroup" aria-label="kind of pin">'
      + kinds.map((k) => `<button type="button" role="radio" class="hb-kind ${k}" data-kind="${k}" title="${esc(KINDS[k][1])}">${KINDS[k][0]}</button>`).join('')
      + '</div>'
      + '<div class="hb-link" hidden>'
      + '<label>leads to <select class="hb-link-pages" aria-label="page"></select></label>'
      + '<div class="hb-link-new" hidden><input class="hb-link-title" maxlength="80" placeholder="Title of the new page"><input class="hb-link-source" maxlength="500" placeholder="Drawn from: a Figma frame link (optional)"></div>'
      + '</div>'
      + '<div class="hb-design" hidden>'
      + '<div class="hb-design-row"><input class="hb-design-name" maxlength="80" placeholder="Name, like RiskBadge" aria-label="name"><input class="hb-design-system" maxlength="40" placeholder="system" aria-label="system"></div>'
      + '<input class="hb-design-desc" maxlength="300" placeholder="What it shows and lets the user do" aria-label="description">'
      + '<div class="hb-design-pick"><b>needs</b><div class="hb-design-needs"></div></div>'
      + '<div class="hb-design-pick"><b>actions</b><div class="hb-design-actions"></div></div>'
      + '<textarea class="hb-design-props" rows="2" placeholder="props, one per line: customerId: ID"></textarea>'
      + '</div>'
      + '<textarea class="hb-text" maxlength="4000" placeholder="Write a note…"></textarea>'
      + '<div class="hb-link-go" hidden><button type="button" data-note="go">Open the page →</button><button type="button" data-note="create">Create it with an AI session</button></div>'
      + '<div class="hb-apply" hidden><button type="button" data-note="apply">Propose</button><span class="hb-apply-state"></span></div>'
      + '<div class="hb-session" hidden><button type="button" data-note="session">Open the AI session</button><span class="hb-turns"></span></div>'
      + '<div class="hb-extra" hidden></div>'
      + '<div class="hb-backlog" hidden><button type="button" data-note="backlog" title="Open a harness:roadmap issue from this pin: the planner takes it from there">→ backlog</button><a class="hb-issue" target="_blank" rel="noopener" hidden>on the backlog ↗</a></div>'
      + '<div class="hb-note-foot"><span class="hb-note-state"></span><button type="button" data-note="undo-delete" title="Take back what the AI did for it, then delete it" hidden>undo &amp; delete</button><button type="button" data-note="delete">delete</button></div>'
      + '</div>';
    const q = (s) => host.querySelector(s);
    const world = q('.hb-world');
    const content = q('.hb-content');
    const edges = q('.hb-edges');
    const zoneLayer = q('.hb-zones');
    const pinLayer = q('.hb-pins');
    const editor = q('.hb-note');
    const textarea = editor.querySelector('textarea.hb-text');
    const list = q('.hb-list');
    const linkBox = editor.querySelector('.hb-link');
    const linkPages = editor.querySelector('.hb-link-pages');
    const linkTitle = editor.querySelector('.hb-link-title');
    const linkSource = editor.querySelector('.hb-link-source');
    const designBox = editor.querySelector('.hb-design');

    const view = { x: 40, y: 40, k: 1 };
    const size = { w: 800, h: 600 };
    let notes = [];
    // The board's own gestures and the pane's steps, for undo: latest last.
    let trail = [];
    // What redo brings back, the latest undone first; a new step clears it.
    let ahead = [];
    // The pins shown: every pin of the board, or those the pane keeps.
    const shown = () => (opts.only ? notes.filter((n) => opts.only(n)) : notes);
    let openId = null;
    let saveTimer = null;
    let pending = null; // the pin whose text waits to be saved
    let busy = false; // an AI is writing the open pin
    // The pins an AI is applying right now: their outline turns, their text is read only.
    const applying = new Set();
    // What the last proposal of a pin came to, said in its window: { ok, text }.
    const news = new Map();
    // What the AI is at for a pin being applied, said in its window.
    const progress = new Map();
    // A request proposed: its words and kind are what the AI worked from,
    // fixed until its Undo.
    const proposed = (n) => !!opts.undo && kindOf(n) === 'request' && ['simulated', 'stale', 'validated'].includes(n.state || 'pending');
    // Where the window was dragged to, until it opens on another pin.
    let editorAt = null;
    // Follows an application: the pins turn until it settles.
    function track(ids, work) {
      ids.forEach((x) => applying.add(x));
      drawPins();
      const n = notes.find((x) => x.id === openId);
      if (n && applying.has(n.id)) drawEditor(n);
      if (opts.onApplying) opts.onApplying();
      return Promise.resolve(work).finally(() => {
        ids.forEach((x) => applying.delete(x));
        if (opts.onApplying) opts.onApplying();
        drawPins();
        const open = notes.find((x) => x.id === openId);
        if (open) drawEditor(open);
      });
    }
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
      shown().forEach((n) => {
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
      // A part the pane hides (a card filtered out) hides its pins with it.
      // A pane may hand a stand-in for a part (the mock-up's, inside its
      // frame): one without `closest` hides nothing.
      if (typeof el.closest === 'function' && el.closest('[hidden]')) return { x: n.x, y: n.y, orphan: false, hidden: true };
      const b = boxOf(el);
      const x = b.x + clamp(n.dx, 0, b.w);
      const y = b.y + clamp(n.dy, 0, b.h);
      n.x = x; n.y = y;
      return { x, y, orphan: false };
    }
    // A pin whose part is gone, on a board that puts such pins aside
    // (`detach`): kept off the canvas, listed as detached.
    const detached = (n) => !!opts.detach && !n.shape && !!n.target && !resolve(n.target);
    function drawPins() {
      const r = host.getBoundingClientRect();
      let h = '';
      let aside = 0;
      shown().forEach((n, i) => {
        if (carry && carry.note === n) return;
        const p = n.shape ? Object.assign(anchorOf(n.shape), { orphan: false }) : placeOf(n);
        if (p.hidden) return;
        if (p.orphan && opts.detach) { aside += 1; return; }
        if (n.shape) { n.x = p.x; n.y = p.y; }
        const s = toScreen(p.x, p.y);
        if (s.x < -30 || s.y < -30 || s.x > r.width + 30 || s.y > r.height + 60) return;
        const kind = kindOf(n);
        const cls = ['hb-pin', kind, p.orphan ? 'orphan' : '', n.shape ? 'zone' : '', n.text || kind !== 'note' ? '' : 'empty', openId === n.id ? 'open' : '', kind === 'request' ? 'state-' + (n.state || 'pending') : '', applying.has(n.id) ? 'applying' : ''].join(' ');
        const tip = `${applying.has(n.id) ? 'being applied — ' : ''}${kind}${n.shape ? ' (zone)' : ''}: ${n.text || (kind === 'link' && n.page ? '→ ' + n.page : 'empty')}${p.orphan ? ' — what it was on is gone' : ''}`;
        h += `<button type="button" class="${cls}" data-note-id="${esc(n.id)}" style="left:${s.x}px;top:${s.y}px" title="${esc(tip)}"><span>${i + 1}</span></button>`;
      });
      pinLayer.innerHTML = h;
      q('.hb-count').textContent = shown().length + (aside ? ` · ${aside} detached` : '');
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
      if (editorAt) {
        editor.style.left = clamp(editorAt.x, 8, Math.max(8, r.width - w - 8)) + 'px';
        editor.style.top = clamp(editorAt.y, 8, Math.max(8, r.height - 40)) + 'px';
        return;
      }
      let left = s.x + 18;
      if (left + w > r.width - 8) left = s.x - w - 18;
      editor.style.left = clamp(left, 8, Math.max(8, r.width - w - 8)) + 'px';
      editor.style.top = clamp(s.y - 30, 8, Math.max(8, r.height - h - 8)) + 'px';
    }
    // The editor shows the pin's kind, its text, and — for a brainstorm or
    // a request — its AI session.
    function drawEditor(n) {
      const kind = kindOf(n);
      n.kind = kind;
      editor.dataset.kind = kind;
      // A pin being applied is read, not written: its words are what the AI works from.
      const locked = busy || applying.has(n.id);
      const fixed = proposed(n);
      editor.classList.toggle('applying', applying.has(n.id));
      editor.querySelectorAll('.hb-kind').forEach((b) => {
        b.classList.toggle('on', b.dataset.kind === kind);
        b.setAttribute('aria-checked', b.dataset.kind === kind);
        // Not `disabled`: a log button inside must stay clickable.
        if (locked || fixed) b.setAttribute('aria-disabled', 'true'); else b.removeAttribute('aria-disabled');
      });
      const session = editor.querySelector('.hb-session');
      session.hidden = kind === 'note' || kind === 'link' || !opts.onSession;
      session.querySelector('button').textContent = kind === 'design' ? (n.turns ? 'Continue the AI session' : 'Make / inject it with an AI session') : kind === 'request' ? 'Discuss it (AI session)' : 'Open the AI session';
      // A request on the mock-up: applied to the page's draft in one go.
      const apply = editor.querySelector('.hb-apply');
      apply.hidden = kind !== 'request' || !opts.apply;
      if (!apply.hidden) {
        const state = n.state || 'pending';
        const button = apply.querySelector('button');
        button.textContent = fixed ? 'Undo' : 'Propose';
        button.dataset.note = fixed ? 'undo' : 'apply';
        button.title = fixed ? 'Take back what the AI did for this request: the mock-up and the rules as they were' : 'The AI drafts it on the mock-up and reads the rules it touches';
        button.disabled = locked;
        const said = news.get(n.id);
        const line = apply.querySelector('.hb-apply-state');
        line.classList.toggle('bad', !!said && !said.ok && !applying.has(n.id));
        line.innerHTML = applying.has(n.id) ? `<span class="spin"></span> ${esc(progress.get(n.id) || 'the AI drafts it on the mock-up…')}`
          : said ? esc(said.text)
            : esc({ pending: 'not proposed yet', simulated: 'drafted on the mock-up — validate it in the bar above the page', stale: 'drafted once; the page moved since', validated: 'validated: the mock-up keeps it' }[state] || '');
      }
      editor.querySelector('.hb-turns').textContent = n.turns ? `${n.turns} turn${n.turns > 1 ? 's' : ''} kept` : 'no conversation yet';
      // What the pane offers for this pin beyond the board's own actions.
      const extra = editor.querySelector('.hb-extra');
      const offered = opts.noteActions ? opts.noteActions(n) : [];
      extra.hidden = !offered.length;
      extra.innerHTML = offered.map((a, i) => `<button type="button" data-note-extra="${i}" title="${esc(a.title || '')}">${esc(a.label)}</button>`).join('');
      // A request, a link or a design goes on the product's backlog as an issue.
      const backlog = editor.querySelector('.hb-backlog');
      backlog.hidden = opts.backlog === false || !(kind === 'request' || kind === 'link' || kind === 'design');
      if (!backlog.hidden) {
        backlog.querySelector('button').hidden = !!n.issue;
        backlog.querySelector('button').disabled = locked;
        const a = backlog.querySelector('.hb-issue');
        a.hidden = !n.issue;
        if (n.issue) { a.href = n.issue; a.textContent = 'on the backlog ' + (/\/(\d+)$/.exec(n.issue) ? '#' + /\/(\d+)$/.exec(n.issue)[1] : '') + ' ↗'; }
      }
      textarea.disabled = busy;
      textarea.readOnly = applying.has(n.id) || fixed;
      editor.querySelector('[data-note="delete"]').disabled = locked;
      // Proposed, a delete takes the AI's work back first; validated, a
      // plain delete keeps what it changed.
      const undoDelete = editor.querySelector('[data-note="undo-delete"]');
      undoDelete.hidden = !fixed;
      undoDelete.disabled = locked;
      editor.querySelector('[data-note="delete"]').hidden = fixed && (n.state || 'pending') !== 'validated';
      textarea.placeholder = {
        note: 'Write a note…',
        link: 'The objective of the page this leads to — what a new page is drawn from',
        request: 'The intention: what should change, what this part should do — Propose drafts it on the mock-up',
      }[kind] || 'Where the discussion stands — rewritten when you leave the session';
      drawLink(n, kind);
      drawDesign(n, kind);
    }
    // A design's editor: the micro-frontend's spec as fields; the note's
    // text is the spec written out, what the session reads.
    const SPEC_KEYS = ['name', 'system', 'description', 'needs', 'actions', 'props'];
    function parseSpec(text) {
      const spec = { name: '', system: '', description: '', needs: [], actions: [], props: [] };
      let key = '';
      (text || '').split('\n').forEach((line) => {
        const m = /^([a-z]+):\s*(.*)$/.exec(line);
        if (m && SPEC_KEYS.includes(m[1])) {
          key = m[1];
          const v = m[2].trim();
          if (key === 'needs' || key === 'actions') spec[key] = v ? v.split(/[,\s]+/).filter(Boolean) : [];
          else if (key === 'props') spec.props = v ? [v] : [];
          else spec[key] = v;
        } else if (key === 'props' && line.trim()) {
          spec.props.push(line.trim());
        }
      });
      return spec;
    }
    function writeSpec(spec) {
      return `name: ${spec.name}\nsystem: ${spec.system}\ndescription: ${spec.description}\nneeds: ${spec.needs.join(', ')}\nactions: ${spec.actions.join(', ')}\nprops:\n${spec.props.map((p) => '  ' + p).join('\n')}`.trim();
    }
    function drawDesign(n, kind) {
      designBox.hidden = kind !== 'design';
      textarea.hidden = kind === 'design';
      if (designBox.hidden) return;
      const spec = parseSpec(n.text);
      if (!spec.name && n.component) spec.name = n.component;
      const pick = (sel, offered, chosen) => {
        const all = [...new Set([...(offered || []), ...chosen])].sort();
        editor.querySelector(sel).innerHTML = all.length
          ? all.map((id) => `<label><input type="checkbox" value="${esc(id)}"${chosen.includes(id) ? ' checked' : ''}> ${esc(id)}</label>`).join('')
          : '<span class="hb-empty">none declared on this branch</span>';
      };
      editor.querySelector('.hb-design-name').value = spec.name;
      editor.querySelector('.hb-design-system').value = spec.system;
      editor.querySelector('.hb-design-desc').value = spec.description;
      pick('.hb-design-needs', opts.capabilities && opts.capabilities(), spec.needs);
      pick('.hb-design-actions', opts.dataCapabilities && opts.dataCapabilities(), spec.actions);
      editor.querySelector('.hb-design-props').value = spec.props.join('\n');
    }
    // The design's fields changed: the spec is written again, the pin named.
    const onDesignInput = () => {
      const n = notes.find((x) => x.id === openId);
      if (!n || (n.kind || 'note') !== 'design' || busy) return;
      const checked = (sel) => [...editor.querySelectorAll(`${sel} input:checked`)].map((i) => i.value);
      const spec = {
        name: editor.querySelector('.hb-design-name').value.trim(),
        system: editor.querySelector('.hb-design-system').value.trim(),
        description: editor.querySelector('.hb-design-desc').value.trim(),
        needs: checked('.hb-design-needs'),
        actions: checked('.hb-design-actions'),
        props: editor.querySelector('.hb-design-props').value.split('\n').map((l) => l.trim()).filter(Boolean),
      };
      n.component = spec.name;
      n.text = writeSpec(spec);
      n.at = new Date().toISOString();
      setState(/^[A-Z][A-Za-z0-9]*$/.test(spec.name) ? '…' : 'name it like a Micro-UI: RiskBadge');
      if (/^[A-Z][A-Za-z0-9]*$/.test(spec.name)) save(n, false);
      drawPins();
    };
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
      linkSource.value = n.source || '';
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
      if (openId !== n.id) editorAt = null;
      openId = n.id;
      if (opts.onOpen) opts.onOpen(n);
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
      hideBadge();
      const n = notes.find((x) => x.id === openId);
      openId = null;
      if (opts.onClose) opts.onClose();
      editor.hidden = true;
      // An empty note that never was saved is a pin dropped by mistake.
      if (n && n.fresh && !n.text.trim() && (n.kind || 'note') === 'note' && !n.shape) {
        notes = notes.filter((x) => x !== n);
      }
      // A zone drawn and left without a word is still a zone: kept.
      if (n && n.fresh && n.shape) save(n, true);
      // A design left without a name is nothing yet.
      if (n && n.fresh && (n.kind || 'note') === 'design' && !n.component) notes = notes.filter((x) => x !== n);
      drawPins();
    }
    function setState(text) { editor.querySelector('.hb-note-state').textContent = text; }
    // The pin becomes an issue on the product's backlog; the pane may add
    // what it knows (`opts.backlogContext(n)`: the draft, the objective).
    async function toBacklog(n) {
      flush();
      setState('opening the issue…');
      try {
        if (n.fresh) await put(n);
        const context = opts.backlogContext ? opts.backlogContext(n) : '';
        const res = await fetch(`${base}/${encodeURIComponent(n.id)}/backlog`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ context }) });
        if (!res.ok) throw new Error(await res.text());
        const got = await res.json();
        n.issue = got.issue;
        n.at = got.at;
        setState('on the backlog');
        drawEditor(n);
        drawPins();
      } catch (e) { setState(String(e.message || e).split('\n')[0].slice(0, 120)); }
    }
    // What the pane adds to a request that changes a pin: its history's
    // branch, a gesture's token.
    const sent = (more) => Object.assign({ 'Content-Type': 'application/json' }, opts.headers ? opts.headers() : {}, more || {});
    async function put(n, again, more) {
      const body = Object.assign({}, n);
      delete body.fresh;
      const placed = !!n.fresh;
      const res = await fetch(`${base}/${encodeURIComponent(n.id)}`, { method: 'PUT', headers: sent(more), body: JSON.stringify(body) });
      if (!res.ok) throw new Error(await res.text());
      delete n.fresh;
      if (placed) trail.push({ kind: 'placed', id: n.id });
      if (placed && !again) ahead = [];
    }
    // Saves a pin now, or a beat after the last keystroke.
    function save(n, now, more) {
      clearTimeout(saveTimer);
      pending = n;
      const send = async () => {
        if (pending !== n) return;
        pending = null;
        if (openId === n.id) setState('saving…');
        try {
          await put(n, false, more);
          if (openId === n.id) setState('saved');
          changed();
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
      put(n).then(changed).catch(() => { /* the next edit saves it again */ });
    }
    async function remove(n, again) {
      notes = notes.filter((x) => x !== n);
      if (openId === n.id) { openId = null; editor.hidden = true; }
      if (pending === n) { pending = null; clearTimeout(saveTimer); }
      drawPins();
      changed();
      if (n.fresh) return;
      trail.push({ kind: 'deleted', id: n.id });
      if (!again) ahead = [];
      try { await fetch(`${base}/${encodeURIComponent(n.id)}`, { method: 'DELETE', headers: sent() }); } catch (e) { /* shown by its absence on reload */ }
    }
    // The pane told a pin was kept or went — the design system counts its requests.
    function changed() {
      if (opts.onChange) { try { opts.onChange(); } catch (e) { /* the pane's own trouble */ } }
    }
    // What undo takes back, the latest first: a pin placed (destroyed — no
    // later undo brings it back), a pin deleted (brought back), a step the
    // pane told of (its own to take back). Past it, the board's server undo.
    async function undo() {
      if (opts.history) { opts.history('undo'); return; }
      const last = trail.pop();
      if (last && last.kind === 'placed') {
        const n = notes.find((x) => x.id === last.id);
        if (n) {
          notes = notes.filter((x) => x !== n);
          if (openId === n.id) { openId = null; editor.hidden = true; }
          if (pending === n) { pending = null; clearTimeout(saveTimer); }
          drawPins();
          changed();
          const kept = Object.assign({}, n);
          delete kept.fresh;
          ahead.push({ kind: 'placed', note: kept });
          try { await fetch(`${base}/${encodeURIComponent(n.id)}?forever=1`, { method: 'DELETE' }); } catch (e) { /* shown by its absence on reload */ }
          return;
        }
      }
      if (last && last.kind === 'pane') { if (opts.onUndo && opts.onUndo()) ahead.push({ kind: 'pane' }); return; }
      if (!last && opts.onUndo && opts.onUndo()) { ahead.push({ kind: 'pane' }); return; }
      try {
        const res = await fetch(`${base}/undo`, { method: 'POST' });
        if (!res.ok) return;
        const n = await res.json();
        if (!notes.some((x) => x.id === n.id)) notes.push(n);
        ahead.push({ kind: 'deleted', id: n.id });
        drawPins();
        changed();
      } catch (e) { /* nothing to bring back */ }
    }
    // Takes the last undo back: the pin placed again, the pin brought back
    // deleted again, the pane's step handed back to the pane.
    async function redo() {
      if (opts.history) { opts.history('redo'); return; }
      const next = ahead.pop();
      if (!next) return;
      if (next.kind === 'pane') { if (opts.onRedo && opts.onRedo()) trail.push({ kind: 'pane' }); return; }
      if (next.kind === 'deleted') { const n = notes.find((x) => x.id === next.id); if (n) await remove(n, true); return; }
      const n = Object.assign({ fresh: true }, next.note);
      notes.push(n);
      drawPins();
      changed();
      try { await put(n, true); } catch (e) { /* shown by its absence on reload */ }
    }
    async function load() {
      if (!base) { notes = []; drawPins(); return; }
      const asked = base;
      try {
        const res = await fetch(base);
        const read = res.ok ? await res.json() : [];
        // Another board was named meanwhile: its own read draws.
        if (asked !== base) return;
        notes = read;
      } catch (e) { notes = []; }
      // The pin open shows its words as read — rewritten by a session, say —
      // unless they are being typed.
      const open = notes.find((x) => x.id === openId);
      if (open && document.activeElement !== textarea) { textarea.value = open.text || ''; drawEditor(open); }
      drawPins();
      if (opts.onLoaded) opts.onLoaded(id);
    }
    // Centres a pin and opens it — the list's click, and a pin opened from elsewhere.
    function goTo(noteId) {
      const n = notes.find((x) => x.id === noteId);
      if (!n) return false;
      const r = host.getBoundingClientRect();
      placeOf(n);
      view.x = r.width / 2 - n.x * view.k;
      view.y = r.height / 2 - n.y * view.k;
      apply();
      openNote(n);
      return true;
    }
    // Another board on the same canvas: what is typed is saved where it was
    // written, then the other board's pins are read.
    function setBoard(next) {
      if (!next || next === id) return;
      flush();
      openId = null;
      editor.hidden = true;
      list.hidden = true;
      id = next;
      base = baseOf(id);
      notes = [];
      trail = [];
      ahead = [];
      drawPins();
      load();
    }
    function showList() {
      if (!list.hidden) { list.hidden = true; return; }
      list.innerHTML = shown().length
        ? shown().map((n, i) => (detached(n)
          ? `<div class="hb-detached"><b class="${esc(n.kind || 'note')}">${i + 1}</b><span>${esc(n.text || (n.kind || 'note'))}</span><small>detached — what it was on is gone</small><span class="hb-detached-acts"><button type="button" class="mini" data-replace-note="${esc(n.id)}" title="Drag it onto the part it is about">re-place</button><button type="button" class="mini" data-delete-note="${esc(n.id)}" title="Delete it (undo brings it back)">delete</button></span></div>`
          : `<button type="button" data-goto-note="${esc(n.id)}"><b class="${esc(n.kind || 'note')}">${i + 1}</b><span>${esc(n.text || (n.kind || 'note'))}</span><small>${esc(n.kind || 'note')} · ${esc(describe(n.target))}</small></button>`)).join('')
        : '<div class="hb-empty">No note yet. Drag the pin onto a part of the board.</div>';
      list.hidden = false;
    }
    // A pin's kind changed. From a brainstorm, the AI writes what it becomes:
    // the conclusion for a note, the intention for a request.
    async function rekind(n, kind) {
      const from = n.kind || 'note';
      if (kind === from || busy || applying.has(n.id)) return;
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
          label: kind === 'note' ? 'conclude the brainstorm into a note' : 'turn the brainstorm into a request',
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
      // A brainstorm opens its session on what the note says; a request may wait for its words.
      if (kind === 'brainstorm' && !(n.text || '').trim()) {
        setState(kind === 'request' ? 'write the intention first: it is what the AI applies' : 'write the first prompt first: the note is what the AI is asked');
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
    function startCarry(e, note, preset) {
      e.preventDefault();
      e.stopPropagation();
      list.hidden = true;
      const ghost = document.createElement('div');
      ghost.className = 'hb-pin ghost ' + ((note && note.kind) || (preset && preset.kind) || 'note');
      ghost.innerHTML = '<span>+</span>';
      pinLayer.appendChild(ghost);
      carry = { note, ghost, over: null, startX: e.clientX, startY: e.clientY, moved: !note, preset: preset || null };
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
      const n = c.note || Object.assign({ id: newId(), text: '', at: c.preset ? new Date().toISOString() : '', kind: 'note', turns: 0, fresh: true }, c.preset || {});
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
      // A preset pin — a micro-frontend dropped from the designer — is kept at once.
      if (!c.note && c.preset && c.preset.kind) save(n, true);
      openNote(n);
    }

    // ---- events ------------------------------------------------------------------
    const onPointerDown = (e) => {
      if (e.button !== 0) return;
      const pinTool = e.target.closest('.hb-pin-tool');
      if (pinTool) return startCarry(e, null);
      // A detached pin is carried off the list onto the part it is about.
      const again = e.target.closest('[data-replace-note]');
      if (again) {
        const n = notes.find((x) => x.id === again.dataset.replaceNote);
        if (n) { list.hidden = true; return startCarry(e, n); }
      }
      const pin = e.target.closest('.hb-pin[data-note-id]');
      if (pin && applying.has(pin.dataset.noteId)) { e.preventDefault(); e.stopPropagation(); const n = notes.find((x) => x.id === pin.dataset.noteId); if (n) openNote(n); return undefined; }
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
        if (b.dataset.hb === 'redo') redo();
        return;
      }
      const go = e.target.closest('[data-goto-note]');
      if (go) {
        list.hidden = true;
        goTo(go.dataset.gotoNote);
        return;
      }
      const drop = e.target.closest('[data-delete-note]');
      if (drop) {
        const gone = notes.find((x) => x.id === drop.dataset.deleteNote);
        if (gone) remove(gone);
        list.hidden = true;
        return;
      }
      const n = notes.find((x) => x.id === openId);
      const kindButton = e.target.closest('.hb-kind');
      if (n && proposed(n) && (kindButton || e.target === textarea)) { lockBadge(e, n); return; }
      if (kindButton && n) { rekind(n, kindButton.dataset.kind); return; }
      const more = e.target.closest('[data-note-extra]');
      if (more && n && opts.noteActions) {
        const offered = opts.noteActions(n)[Number(more.dataset.noteExtra)];
        if (offered) offered.run(n);
        return;
      }
      const act = e.target.closest('[data-note]');
      if (!act) return;
      if (act.dataset.note === 'close') closeNote();
      if (act.dataset.note === 'delete' && n) remove(n);
      if ((act.dataset.note === 'undo' || act.dataset.note === 'undo-delete') && n && opts.undo && !applying.has(n.id)) {
        hideBadge();
        undoPin(n, act.dataset.note === 'undo-delete');
        return;
      }
      // A proposed request's words are fixed: discussing them is a change too.
      if (act.dataset.note === 'session' && n && proposed(n)) { lockBadge(e, n); return; }
      if (act.dataset.note === 'session' && n && opts.onSession) openSession(n);
      if (act.dataset.note === 'apply' && n && opts.apply && !applying.has(n.id)) {
        if (!(n.text || '').trim()) { setState('write the request first: it is what the AI applies'); textarea.focus(); return; }
        flush();
        // The window stays: it says what the AI is doing, then what it came to.
        news.delete(n.id);
        track([n.id], (n.fresh ? put(n) : Promise.resolve()).then(() => opts.apply(n))
          .then((text) => { news.set(n.id, { ok: true, text: text || 'drafted on the mock-up' }); })
          .finally(() => progress.delete(n.id))
          .catch((err) => { news.set(n.id, { ok: false, text: 'not drafted: ' + String((err && err.message) || err).split('\n')[0] }); if (opts.onApplyError) opts.onApplyError(err); }));
      }
      if (act.dataset.note === 'backlog' && n) toBacklog(n);
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
    // The reference a page is drawn from: kept only as a link.
    const onLinkSource = () => {
      const n = notes.find((x) => x.id === openId);
      if (!n) return;
      const value = linkSource.value.trim();
      if (value && !/^https?:\/\//.test(value)) { setState('a reference is a link (https://…)'); return; }
      n.source = value;
      n.at = new Date().toISOString();
      save(n, false);
    };
    // A click on the words or the kind of a proposed pin: a badge by the
    // click says they are fixed, with the Undo that frees them.
    let badge = null;
    function hideBadge() { if (badge) { badge.remove(); badge = null; } }
    function lockBadge(e, n) {
      hideBadge();
      const r = host.getBoundingClientRect();
      badge = document.createElement('div');
      badge.className = 'hb-lock-badge';
      badge.setAttribute('role', 'status');
      badge.innerHTML = 'Any change needs an Undo first <button type="button" data-note="undo">Undo</button>';
      // The host may be scrolled: its children sit in its scrolled box.
      badge.style.left = (clamp(e.clientX - r.left + 8, 8, Math.max(8, r.width - 260)) + host.scrollLeft) + 'px';
      badge.style.top = (clamp(e.clientY - r.top + 12, 8, Math.max(8, r.height - 40)) + host.scrollTop) + 'px';
      host.appendChild(badge);
      const id = n.id;
      setTimeout(() => { if (badge && openId === id) hideBadge(); }, 5000);
    }
    // A proposed pin taken back — then deleted, when asked so.
    function undoPin(n, andDelete) {
      news.delete(n.id);
      progress.set(n.id, 'taking back what the AI did…');
      return track([n.id], Promise.resolve(opts.undo(n))
        .then((text) => { news.set(n.id, { ok: true, text: text || 'taken back' }); if (andDelete) return remove(n); return undefined; })
        .catch((err) => { news.set(n.id, { ok: false, text: 'not taken back: ' + String((err && err.message) || err).split('\n')[0] }); })
        .finally(() => progress.delete(n.id)));
    }
    const onInput = () => {
      const n = notes.find((x) => x.id === openId);
      if (!n || busy || applying.has(n.id)) return;
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
    // Undo and redo anywhere but in a field: cmd+z / cmd+shift+z on a Mac, ctrl elsewhere.
    const onUndoKey = (e) => {
      // The pane's history takes the keys itself.
      if (opts.history) return;
      if (!modded(e) || e.key.toLowerCase() !== 'z') return;
      if (typing(e.target) || !host.isConnected) return;
      e.preventDefault();
      if (e.shiftKey) redo(); else undo();
    };

    listen(host, 'pointerdown', onPointerDown);
    listen(host, 'pointermove', onPointerMove);
    listen(host, 'pointerup', onPointerUp);
    listen(host, 'pointercancel', onPointerUp);
    listen(host, 'click', onClickCapture, true);
    listen(host, 'click', onToolClick);
    listen(host, 'wheel', onWheel, { passive: false });
    textarea.addEventListener('input', onInput);
    designBox.addEventListener('input', onDesignInput);
    designBox.addEventListener('change', onDesignInput);
    linkPages.addEventListener('change', onLinkChange);
    linkTitle.addEventListener('input', onLinkTitle);
    linkSource.addEventListener('input', onLinkSource);
    editor.addEventListener('keydown', onKey);
    // The window dragged by its head, to see what lies under it.
    const head = editor.querySelector('.hb-note-head');
    head.addEventListener('pointerdown', (e) => {
      if (e.button !== 0 || e.target.closest('button')) return;
      e.preventDefault();
      const r = host.getBoundingClientRect();
      const from = { x: e.clientX, y: e.clientY, left: editor.offsetLeft, top: editor.offsetTop };
      head.setPointerCapture(e.pointerId);
      const move = (ev) => {
        editorAt = { x: from.left + ev.clientX - from.x, y: from.top + ev.clientY - from.y };
        editor.style.left = clamp(editorAt.x, 8, Math.max(8, r.width - editor.offsetWidth - 8)) + 'px';
        editor.style.top = clamp(editorAt.y, 8, Math.max(8, r.height - 40)) + 'px';
      };
      const up = () => { head.removeEventListener('pointermove', move); head.removeEventListener('pointerup', up); head.removeEventListener('pointercancel', up); };
      head.addEventListener('pointermove', move);
      head.addEventListener('pointerup', up);
      head.addEventListener('pointercancel', up);
    });
    listen(document, 'keydown', onUndoKey);
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
      setBoard,
      board: () => id,
      zoom: () => view.k,
      // A pin carried from outside the board — a micro-frontend dragged off
      // the designer — with the fields it lands with.
      carryFrom: (e, preset) => startCarry(e, null, preset),
      // A new pin with preset fields, near the top left of what is shown.
      newNote(preset) {
        const r = host.getBoundingClientRect();
        const at = toBoard(r.left + 70, r.top + 90 + 40 * notes.filter((x) => x.fresh).length);
        const n = Object.assign({ id: newId(), text: '', at: new Date().toISOString(), kind: 'note', turns: 0, fresh: true, target: '', dx: 0, dy: 0, x: at.x, y: at.y }, preset || {});
        notes.push(n);
        if (n.kind === 'design' && n.component) save(n, true);
        openNote(n);
        return n.id;
      },
      session: (noteId) => { const n = notes.find((x) => x.id === noteId); if (n && opts.onSession) openSession(n); },
      // Every pin, and a pin's fields changed from outside — a request applied.
      all: () => notes.map((n) => Object.assign({}, n)),
      // What the AI is at for a pin being applied, said in its window.
      progress(noteId, text) {
        progress.set(noteId, text);
        const open = notes.find((x) => x.id === openId);
        if (open && open.id === noteId) drawEditor(open);
      },
      // `more`: what the pane adds to the request — a gesture's token.
      patch(noteId, fields, more) {
        const n = notes.find((x) => x.id === noteId);
        if (!n) return;
        Object.assign(n, fields, { at: new Date().toISOString() });
        if (openId === n.id) drawEditor(n);
        drawPins();
        if (!n.fresh) save(n, true, more);
      },
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
        linkSource.value = '';
        setTimeout(() => linkTitle.focus(), 0);
      },
      notesOn: (target) => notes.filter((n) => n.target === target),
      open: (noteId) => { const n = notes.find((x) => x.id === noteId); if (n) openNote(n); },
      // Pins being applied by the pane itself — a card's "apply" — turn the same way.
      track: (ids, work) => track(ids, work),
      isApplying: (noteId) => applying.has(noteId),
      // A pin whose work is done goes, as a delete does: ctrl+z brings it back.
      drop: (noteId) => { const n = notes.find((x) => x.id === noteId); if (n) remove(n); },
      // The pane made an undoable step of its own (the AI's application to a
      // draft): the board's undo hands it back to the pane in its turn.
      noteStep: () => { trail.push({ kind: 'pane' }); ahead = []; },
      goTo,
      destroy() {
        listening.abort();
        // A note being typed is not lost to a closed pane.
        flush();
        document.removeEventListener('keydown', onUndoKey);
        resized.disconnect();
        host.innerHTML = '';
        host.classList.remove('hb', 'placing', 'panning');
      },
    };
  }

  window.HarnessBoard = { create, modded, keys: KEYS };
})();
