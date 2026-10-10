// The mock-up's edit mode, the hands: a Figma-like designer over the page
// in the iframe. Everything a human does to the page goes through here —
// the pane (mockup.js) owns the page, its draft engine (edit.js), the
// versions, the requests, the forks and the micro-frontends, and lends
// them to this file through `ctx`; this file owns the gestures and what
// they draw: hover and selection outlines with eight handles, a size
// label, multi-selection by shift and by marquee, the blue insertion bar
// when a part is dragged through a container laid out in flow, free
// placement with red smart guides inside a container laid out static
// (Figma's auto-layout and plain frames, the one switched into the other
// by shift+A), the layers tree at the left, the properties at the right,
// the insert tools (section, text, button, link, image), the right-click
// menu and the keyboard. Each gesture ends as operations of the draft
// (`ctx.step`), never as a change to the page's HTML.
//
//   const design = MockDesign.create(ctx);
//   design.attach(); design.refresh(); design.select(targets); design.detach();
(() => {
  'use strict';

  const E = window.MockEdit;
  const SLOP = 4;
  const SNAP = 6;
  const NUDGE_DELAY = 450;
  const EDITABLE = '[data-component],[data-id],button,a,input,select,textarea,label,h1,h2,h3,h4,h5,h6,img,table,th,td,tr,ul,ol,li,form,section,article,aside,nav,header,footer,figure,p,div,span,main';
  const CONTAINERS = /^(DIV|SECTION|MAIN|ARTICLE|NAV|HEADER|FOOTER|FORM|UL|OL|ASIDE|FIGURE|LI)$/;
  const TOOLS = { move: 'V', section: 'F', text: 'T', button: 'B', link: 'L', image: 'I', kit: '' };
  const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const newId = (prefix) => prefix + Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
  const round1 = (v) => Math.round(v * 2) / 2;

  function create(ctx) {
    const st = { on: false, sels: [], hover: null, tool: 'move', drag: null, resize: null, marquee: null, menu: null, nudge: null, nudgeTimer: null, raf: 0, k: 1, open: new Map(), layerDrag: null, props: null };
    const q = (s) => ctx.host.querySelector(s);
    const doc = () => ctx.doc();
    const zoom = () => (ctx.board() ? ctx.board().zoom() : 1);
    const isStatic = (el) => !!(el && el.dataset && el.dataset.layout === 'static');
    const freeParent = (el) => (el && el.parentElement && isStatic(el.parentElement) ? el.parentElement : null);
    const isContainer = (el) => !!el && CONTAINERS.test(el.tagName);
    const grabSelector = () => (ctx.mfOnly() ? '[data-component]' : EDITABLE);

    // ---- where things are: the page's coordinates are the board's -------------------
    function toPage(clientX, clientY) {
      const fr = ctx.frame().getBoundingClientRect();
      const k = zoom();
      return { x: (clientX - fr.left) / k, y: (clientY - fr.top) / k };
    }
    const box = (el) => { const r = el.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height }; };
    function elementAt(clientX, clientY, selector, excluding) {
      const d = doc();
      if (!d || !ctx.frame()) return null;
      const p = toPage(clientX, clientY);
      const fr = ctx.frame().getBoundingClientRect();
      if (clientX < fr.left || clientX > fr.right || clientY < fr.top || clientY > fr.bottom) return null;
      let el = d.elementFromPoint(p.x, p.y);
      if (!el) return null;
      el = el.closest(selector);
      while (el && excluding && excluding.some((x) => x === el || x.contains(el))) el = el.parentElement && el.parentElement.closest(selector);
      if (!el || el === d.body || el === d.documentElement) return null;
      return el;
    }
    const selected = () => st.sels.map((t) => E.elementOf(doc(), t)).filter(Boolean);
    const one = () => (st.sels.length === 1 ? E.elementOf(doc(), st.sels[0]) : null);

    // ---- the overlay: what is drawn over the page ---------------------------------------
    function overlay() {
      let o = ctx.board() && ctx.board().content.querySelector('.fg-overlay');
      if (!o && ctx.board() && ctx.frame()) {
        o = document.createElement('div');
        o.className = 'fg-overlay';
        o.innerHTML = '<div class="fg-hover" hidden><span class="fg-tag"></span></div><div class="fg-guides"></div><div class="fg-sels"></div><div class="fg-bar" hidden></div><div class="fg-ghost" hidden></div><div class="fg-marquee" hidden></div>';
        ctx.board().content.appendChild(o);
        o.addEventListener('pointerdown', onHandleDown, true);
      }
      return o;
    }
    function sizeOverlay() {
      const o = overlay();
      const f = ctx.frame();
      if (!o || !f) return;
      o.style.width = f.style.width; o.style.height = f.style.height;
      o.hidden = !ctx.editing();
      o.style.setProperty('--inv', String(1 / zoom()));
    }
    function drawHover() {
      const o = overlay();
      if (!o) return;
      const h = o.querySelector('.fg-hover');
      const el = st.hover && !st.sels.includes(E.targetOf(st.hover)) ? st.hover : null;
      if (!el || !ctx.editing()) { h.hidden = true; return; }
      const b = box(el);
      h.hidden = false;
      Object.assign(h.style, { left: b.x + 'px', top: b.y + 'px', width: b.w + 'px', height: b.h + 'px' });
      h.querySelector('.fg-tag').textContent = labelOf(el);
    }
    function drawSelection() {
      const o = overlay();
      if (!o) return;
      const root = o.querySelector('.fg-sels');
      const els = selected();
      if (!els.length || !ctx.editing()) { root.innerHTML = ''; return; }
      const single = els.length === 1;
      root.innerHTML = els.map((el, i) => {
        const b = box(el);
        const free = !!freeParent(el);
        const cell = el.tagName === 'TD' || el.tagName === 'TH';
        const handles = single && !cell ? ['n', 's', 'e', 'w', 'ne', 'nw', 'se', 'sw'].filter((h) => free || h === 'e' || h === 'w' || h === 's' || h === 'se' || h === 'sw').map((h) => `<i class="fg-h fg-h-${h}" data-handle="${h}"></i>`).join('') : '';
        return `<div class="fg-sel ${single ? 'single' : ''}" style="left:${b.x}px;top:${b.y}px;width:${b.w}px;height:${b.h}px" data-index="${i}">`
          + (i === 0 ? `<span class="fg-tag fg-name">${esc(single ? labelOf(el) : els.length + ' parts')}</span>` : '')
          + handles
          + (single ? `<span class="fg-size">${Math.round(b.w)} × ${Math.round(b.h)}</span>` : '')
          + '</div>';
      }).join('');
      if (!single) {
        // The group's box, around them all.
        const u = union(els.map(box));
        root.insertAdjacentHTML('beforeend', `<div class="fg-group" style="left:${u.x}px;top:${u.y}px;width:${u.w}px;height:${u.h}px"><span class="fg-size">${Math.round(u.w)} × ${Math.round(u.h)}</span></div>`);
      }
    }
    const union = (boxes) => {
      const x = Math.min(...boxes.map((b) => b.x)); const y = Math.min(...boxes.map((b) => b.y));
      const r = Math.max(...boxes.map((b) => b.x + b.w)); const btm = Math.max(...boxes.map((b) => b.y + b.h));
      return { x, y, w: r - x, h: btm - y };
    };
    function drawGuides(lines) {
      const o = overlay();
      if (!o) return;
      o.querySelector('.fg-guides').innerHTML = (lines || []).map((l) => (l.v
        ? `<i class="fg-guide v" style="left:${l.at}px;top:${l.from}px;height:${l.to - l.from}px"></i>`
        : `<i class="fg-guide h" style="top:${l.at}px;left:${l.from}px;width:${l.to - l.from}px"></i>`)).join('');
    }
    function drawBar(over, where) {
      const o = overlay();
      if (!o) return;
      const bar = o.querySelector('.fg-bar');
      const ghost = o.querySelector('.fg-ghost');
      ghost.hidden = true;
      if (!over || !where) { bar.hidden = true; return; }
      const b = box(over);
      bar.hidden = false;
      bar.className = 'fg-bar';
      if (where === 'into') { Object.assign(bar.style, { left: b.x + 'px', top: b.y + 'px', width: b.w + 'px', height: b.h + 'px' }); bar.classList.add('into'); return; }
      const horizontal = flowsHorizontally(over);
      if (horizontal) Object.assign(bar.style, { left: (where === 'before' ? b.x : b.x + b.w) + 'px', top: b.y + 'px', width: '0px', height: b.h + 'px' });
      else Object.assign(bar.style, { left: b.x + 'px', top: (where === 'before' ? b.y : b.y + b.h) + 'px', width: b.w + 'px', height: '0px' });
      bar.classList.add(horizontal ? 'v' : 'h');
    }
    function drawGhost(b) {
      const o = overlay();
      if (!o) return;
      const g = o.querySelector('.fg-ghost');
      if (!b) { g.hidden = true; return; }
      g.hidden = false;
      Object.assign(g.style, { left: b.x + 'px', top: b.y + 'px', width: b.w + 'px', height: b.h + 'px' });
    }
    function drawMarquee(b) {
      const o = overlay();
      if (!o) return;
      const m = o.querySelector('.fg-marquee');
      if (!b) { m.hidden = true; return; }
      m.hidden = false;
      Object.assign(m.style, { left: b.x + 'px', top: b.y + 'px', width: b.w + 'px', height: b.h + 'px' });
    }
    // In a flow container, do the children sit side by side?
    function flowsHorizontally(el) {
      const d = doc();
      if (!d || !el.parentElement) return false;
      const w = d.defaultView;
      const parent = w.getComputedStyle(el.parentElement);
      if (/flex/.test(parent.display)) return !/column/.test(parent.flexDirection || '');
      if (parent.display === 'grid') return true;
      if (el.tagName === 'TD' || el.tagName === 'TH') return true;
      return /^inline/.test(w.getComputedStyle(el).display) || el.style.display === 'inline-block';
    }
    function labelOf(el) {
      if (!el) return '';
      if (el.dataset && el.dataset.component) return el.dataset.component + (el.dataset.variantOf ? ' (fork)' : '');
      if (el.id) return '#' + el.id;
      const tag = el.tagName.toLowerCase();
      const text = (el.children.length ? ([...el.childNodes].find((n) => n.nodeType === 3 && n.textContent.trim()) || { textContent: '' }).textContent : el.textContent).trim().replace(/\s+/g, ' ');
      return text ? `${tag} · ${text.length > 18 ? text.slice(0, 18) + '…' : text}` : tag + (el.dataset && el.dataset.layout === 'static' ? ' · free' : '');
    }
    function refresh() {
      sizeOverlay();
      drawHover();
      drawSelection();
      drawLayers();
      drawProps();
    }
    // The handles and tags keep their size on the screen whatever the zoom.
    function tick() {
      if (!st.on) return;
      const k = zoom();
      if (k !== st.k) { st.k = k; const o = overlay(); if (o) o.style.setProperty('--inv', String(1 / k)); }
      st.raf = requestAnimationFrame(tick);
    }

    // ---- selection -------------------------------------------------------------------------
    function select(targets, { add = false, toggle = false } = {}) {
      const list = Array.isArray(targets) ? targets.filter(Boolean) : targets ? [targets] : [];
      if (toggle) list.forEach((t) => { const i = st.sels.indexOf(t); if (i >= 0) st.sels.splice(i, 1); else st.sels.push(t); });
      else if (add) list.forEach((t) => { if (!st.sels.includes(t)) st.sels.push(t); });
      else st.sels = list;
      drawSelection();
      drawLayers();
      drawProps();
      ctx.onSelect && ctx.onSelect(st.sels.slice());
    }
    function setHover(el) {
      if (st.hover === el) return;
      st.hover = el;
      drawHover();
      highlightLayer(el);
    }

    // ---- pointer: select, drag, marquee, tools ---------------------------------------------
    function onDown(e) {
      if (!ctx.editing() || e.button !== 0 || ctx.space()) return;
      if (!e.target.classList.contains('mk-shield')) return;
      closeMenu();
      e.stopPropagation();
      if (st.tool !== 'move') { startInsert(e); return; }
      const hit = elementAt(e.clientX, e.clientY, grabSelector());
      if (!hit) {
        if (!e.shiftKey) select([]);
        const p = toPage(e.clientX, e.clientY);
        st.marquee = { x0: p.x, y0: p.y, add: e.shiftKey, pointer: e.pointerId };
        ctx.boardHost().setPointerCapture(e.pointerId);
        return;
      }
      const target = E.targetOf(hit);
      if (e.shiftKey) { select(target, { toggle: true }); return; }
      if (!st.sels.includes(target)) select(target);
      const els = selected();
      st.drag = { startX: e.clientX, startY: e.clientY, moved: false, els, pointer: e.pointerId, over: null, where: '', free: els.length > 0 && els.every((x) => freeParent(x) === freeParent(els[0]) && freeParent(x)), column: els.length === 1 && (hit.tagName === 'TD' || hit.tagName === 'TH'), starts: els.map((x) => box(x)) };
      ctx.boardHost().setPointerCapture(e.pointerId);
    }
    function onMove(e) {
      if (st.resize) { e.stopPropagation(); moveResize(e); return; }
      if (st.insert) { e.stopPropagation(); moveInsert(e); return; }
      if (st.marquee) { e.stopPropagation(); moveMarquee(e); return; }
      if (!st.drag) {
        if (ctx.editing() && e.target.classList.contains('mk-shield')) setHover(st.tool === 'move' ? elementAt(e.clientX, e.clientY, grabSelector()) : null);
        return;
      }
      e.stopPropagation();
      const d = st.drag;
      if (!d.moved && Math.hypot(e.clientX - d.startX, e.clientY - d.startY) < SLOP) return;
      if (!d.moved) { d.moved = true; ctx.host.classList.add('fg-moving'); setHover(null); }
      const k = zoom();
      const dx = (e.clientX - d.startX) / k; const dy = (e.clientY - d.startY) / k;
      if (d.free) {
        // Free placement: the parts follow the pointer, snapped to their siblings.
        const parent = freeParent(d.els[0]);
        const snap = guidesFor(parent, d.els, d.starts.map((b) => ({ x: b.x + dx, y: b.y + dy, w: b.w, h: b.h })));
        d.els.forEach((el, i) => {
          const b = d.starts[i];
          const pb = box(parent);
          el.style.left = (b.x + dx + snap.dx - pb.x) + 'px';
          el.style.top = (b.y + dy + snap.dy - pb.y) + 'px';
        });
        drawGuides(snap.lines);
        drawSelection();
        return;
      }
      // Flow: where the parts would land.
      let over = elementAt(e.clientX, e.clientY, d.column ? 'th,td' : grabSelector(), d.els);
      if (d.column && over && over.closest('table') !== d.els[0].closest('table')) over = null;
      let where = '';
      if (over) {
        if (d.column) where = (e.clientX - over.getBoundingClientRect().left * k - ctx.frame().getBoundingClientRect().left) / Math.max(1, over.getBoundingClientRect().width * k) < 0.5 ? 'before' : 'after';
        else where = placement(over, e.clientX, e.clientY);
      }
      // Over a free container, or one of its parts: a ghost where the part would sit.
      const freeHost = over && (isStatic(over) ? over : freeParent(over));
      if (freeHost && !d.els.includes(freeHost) && !d.els.some((x) => x.contains(freeHost))) { over = freeHost; where = 'into'; }
      if (over && isStatic(over) && where === 'into') {
        const p = toPage(e.clientX, e.clientY);
        drawBar(null, '');
        drawGhost({ x: p.x - d.starts[0].w / 2, y: p.y - d.starts[0].h / 2, w: d.starts[0].w, h: d.starts[0].h });
        d.over = over; d.where = 'free';
        return;
      }
      if (d.over !== over || d.where !== where) { d.over = over; d.where = where; drawBar(over, where); }
    }
    function placement(target, clientX, clientY) {
      const p = toPage(clientX, clientY);
      const b = box(target);
      if (isStatic(target)) return 'into';
      if (!target.children.length && isContainer(target) && !target.textContent.trim()) return 'into';
      // Inside a container with room near its middle: into it, at the end.
      if (isContainer(target) && !isStatic(target) && target.children.length) {
        const last = box(target.children[target.children.length - 1]);
        if (p.y > last.y + last.h && p.y > b.y + b.h * 0.6) return 'into';
      }
      return flowsHorizontally(target) ? (p.x < b.x + b.w / 2 ? 'before' : 'after') : (p.y < b.y + b.h / 2 ? 'before' : 'after');
    }
    function onUp(e) {
      if (st.resize) { e.stopPropagation(); endResize(e); return; }
      if (st.insert) { e.stopPropagation(); endInsert(e); return; }
      if (st.marquee) { e.stopPropagation(); endMarquee(e); return; }
      if (!st.drag) return;
      e.stopPropagation();
      const d = st.drag;
      st.drag = null;
      ctx.host.classList.remove('fg-moving');
      drawBar(null, ''); drawGhost(null); drawGuides([]);
      if (!d.moved) { if (!e.shiftKey) select(E.targetOf(d.els[0])); return; }
      if (d.free) { commitFree(freeParent(d.els[0]), d.els); return; }
      if (!d.over || !d.where) { drawSelection(); return; }
      if (d.where === 'free') { dropIntoFree(d.over, d.els, e); return; }
      if (d.column) {
        const table = d.els[0].closest('table');
        const row = d.els[0].parentElement;
        const from = Array.prototype.indexOf.call(row.children, d.els[0]);
        const overIndex = Array.prototype.indexOf.call(d.over.parentElement.children, d.over);
        let to = d.where === 'before' ? overIndex : overIndex + 1;
        if (from < to) to -= 1;
        if (to === from) return;
        ctx.step({ kind: 'move', by: 'human', target: `col:${ctx.opTarget(table)}#${from}`, to });
        return;
      }
      // Several parts land in their document order, one after the other.
      const ordered = d.els.slice().sort((a, b) => (a.compareDocumentPosition(b) & 4 ? -1 : 1));
      const dest = ctx.opTarget(d.over);
      const ops = []; const ids = [];
      let anchor = dest; let where = d.where;
      ordered.forEach((el) => {
        const id = el.id || (el.dataset && (el.dataset.component || el.dataset.id)) ? null : newId('m');
        const op = { kind: 'move', by: 'human', target: ctx.opTarget(el) };
        if (id) op.as = id;
        op[where] = anchor;
        ops.push(op);
        const mine = id ? 'data:' + id : E.targetOf(el);
        ids.push(mine);
        // The next one lands after this one.
        anchor = mine; where = 'after';
      });
      if (ctx.step(ops)) select(ids);
    }
    // A free container's children as items, with the boxes the gesture left them.
    function itemsOf(parent, overrides) {
      const pb = box(parent);
      return [...parent.children].map((child) => {
        const b = (overrides && overrides.get(child)) || box(child);
        return { target: E.targetOf(child), x: round1((b.x - pb.x) / pb.w * 100), y: round1((b.y - pb.y) / pb.h * 100), w: round1(b.w / pb.w * 100), h: round1(b.h / pb.h * 100) };
      });
    }
    function commitFree(parent, els, overrides) {
      const items = itemsOf(parent, overrides);
      const targets = els.map((el) => E.targetOf(el));
      if (ctx.step({ kind: 'static', by: 'human', target: ctx.opTarget(parent), layout: 'static', height: Math.round(box(parent).h), items })) select(targets);
      else refresh();
    }
    function dropIntoFree(parent, els, e) {
      const p = toPage(e.clientX, e.clientY);
      const pb = box(parent);
      const ops = []; const ids = [];
      els.forEach((el, i) => {
        const b = box(el);
        const id = el.id || (el.dataset && (el.dataset.component || el.dataset.id)) ? null : newId('m');
        const x = round1(Math.max(0, (p.x - b.w / 2 - pb.x + i * 12) / pb.w * 100));
        const y = round1(Math.max(0, (p.y - b.h / 2 - pb.y + i * 12) / pb.h * 100));
        const op = { kind: 'move', by: 'human', target: ctx.opTarget(el), into: ctx.opTarget(parent), x, y, w: round1(b.w / pb.w * 100), h: round1(b.h / pb.h * 100) };
        if (id) op.as = id;
        ops.push(op);
        ids.push(id ? 'data:' + id : E.targetOf(el));
      });
      if (ctx.step(ops)) select(ids);
    }

    // ---- smart guides: the siblings' edges and centres, snapped to ----------------------
    function guidesFor(parent, moving, boxes) {
      const pb = box(parent);
      const others = [...parent.children].filter((c) => !moving.includes(c)).map(box);
      const xs = [pb.x, pb.x + pb.w / 2, pb.x + pb.w]; const ys = [pb.y, pb.y + pb.h / 2, pb.y + pb.h];
      others.forEach((b) => { xs.push(b.x, b.x + b.w / 2, b.x + b.w); ys.push(b.y, b.y + b.h / 2, b.y + b.h); });
      const u = union(boxes);
      const mine = { xs: [u.x, u.x + u.w / 2, u.x + u.w], ys: [u.y, u.y + u.h / 2, u.y + u.h] };
      const tol = SNAP / zoom();
      let dx = 0; let dy = 0; let bestX = tol + 1; let bestY = tol + 1; let lineX = null; let lineY = null;
      mine.xs.forEach((mx) => xs.forEach((x) => { const d = Math.abs(x - mx); if (d < bestX) { bestX = d; dx = x - mx; lineX = x; } }));
      mine.ys.forEach((my) => ys.forEach((y) => { const d = Math.abs(y - my); if (d < bestY) { bestY = d; dy = y - my; lineY = y; } }));
      const lines = [];
      if (bestX <= tol) lines.push({ v: true, at: lineX, from: pb.y, to: pb.y + pb.h }); else dx = 0;
      if (bestY <= tol) lines.push({ v: false, at: lineY, from: pb.x, to: pb.x + pb.w }); else dy = 0;
      return { dx, dy, lines };
    }

    // ---- resizing by the handles -------------------------------------------------------------
    function onHandleDown(e) {
      const h = e.target.closest('[data-handle]');
      if (!h || e.button !== 0) return;
      const el = one();
      if (!el) return;
      e.stopPropagation(); e.preventDefault();
      const b = box(el);
      st.resize = { el, handle: h.dataset.handle, startX: e.clientX, startY: e.clientY, b0: b, free: !!freeParent(el), pointer: e.pointerId, row: el.parentElement ? box(el.parentElement).w : b.w };
      ctx.boardHost().setPointerCapture(e.pointerId);
    }
    function resizedBox(e) {
      const r = st.resize;
      const k = zoom();
      const dx = (e.clientX - r.startX) / k; const dy = (e.clientY - r.startY) / k;
      let { x, y, w, h } = r.b0;
      if (/e/.test(r.handle)) w = Math.max(16, r.b0.w + dx);
      if (/w/.test(r.handle)) { w = Math.max(16, r.b0.w - dx); x = r.b0.x + r.b0.w - w; }
      if (/s/.test(r.handle)) h = Math.max(10, r.b0.h + dy);
      if (/n/.test(r.handle)) { h = Math.max(10, r.b0.h - dy); y = r.b0.y + r.b0.h - h; }
      if (e.shiftKey && r.b0.h) { const ratio = r.b0.w / r.b0.h; if (/[ew]/.test(r.handle)) h = w / ratio; else w = h * ratio; }
      return { x, y, w, h };
    }
    function moveResize(e) {
      const r = st.resize;
      let b = resizedBox(e);
      if (r.free) {
        const parent = freeParent(r.el);
        const snap = guidesFor(parent, [r.el], [b]);
        // Snapping moves the edge being dragged, not the whole box.
        if (/e/.test(r.handle) && snap.dx) b.w += snap.dx; else if (/w/.test(r.handle) && snap.dx) { b.x += snap.dx; b.w -= snap.dx; }
        if (/s/.test(r.handle) && snap.dy) b.h += snap.dy; else if (/n/.test(r.handle) && snap.dy) { b.y += snap.dy; b.h -= snap.dy; }
        drawGuides(snap.lines);
        const pb = box(parent);
        Object.assign(r.el.style, { left: (b.x - pb.x) + 'px', top: (b.y - pb.y) + 'px', width: b.w + 'px', height: b.h + 'px' });
      } else {
        r.el.style.width = b.w + 'px'; r.el.style.boxSizing = 'border-box';
        if (/[ns]/.test(r.handle)) r.el.style.minHeight = b.h + 'px';
      }
      r.last = b;
      drawSelection();
    }
    function endResize(e) {
      const r = st.resize;
      st.resize = null;
      drawGuides([]);
      const b = r.last || resizedBox(e);
      if (r.free) { commitFree(freeParent(r.el), [r.el], new Map([[r.el, b]])); return; }
      const cols = Math.max(1, Math.min(E.COLS, Math.round(b.w / (r.row / E.COLS))));
      const op = { kind: 'resize', by: 'human', target: ctx.opTarget(r.el), cols };
      if (/[ns]/.test(r.handle)) op.height = Math.round(b.h);
      if (!ctx.step(op)) refresh();
    }

    // ---- the marquee ----------------------------------------------------------------------
    function marqueeBox(e) {
      const p = toPage(e.clientX, e.clientY);
      const m = st.marquee;
      return { x: Math.min(m.x0, p.x), y: Math.min(m.y0, p.y), w: Math.abs(p.x - m.x0), h: Math.abs(p.y - m.y0) };
    }
    function moveMarquee(e) { drawMarquee(marqueeBox(e)); }
    function endMarquee(e) {
      const m = st.marquee;
      st.marquee = null;
      const b = marqueeBox(e);
      drawMarquee(null);
      if (b.w < SLOP && b.h < SLOP) return;
      const d = doc();
      const inside = (r) => r.x >= b.x && r.y >= b.y && r.x + r.w <= b.x + b.w && r.y + r.h <= b.y + b.h && r.w > 0;
      let hits = [...d.body.querySelectorAll(grabSelector())].filter((el) => inside(box(el)));
      // The outermost of them: a part whose ancestor is taken is not.
      hits = hits.filter((el) => !hits.some((other) => other !== el && other.contains(el)));
      select(hits.map((el) => E.targetOf(el)), { add: m.add });
    }

    // ---- the insert tools -----------------------------------------------------------------
    function setTool(name) {
      st.tool = name in TOOLS ? name : 'move';
      ctx.host.querySelectorAll('[data-tool]').forEach((b) => b.classList.toggle('on', b.dataset.tool === st.tool));
      const search = ctx.host.querySelector('.fg-kit');
      if (search) search.classList.toggle('on', st.tool === 'kit');
      ctx.host.classList.toggle('fg-inserting', st.tool !== 'move');
      setHover(null);
    }
    function startInsert(e) {
      const p = toPage(e.clientX, e.clientY);
      const over = elementAt(e.clientX, e.clientY, EDITABLE);
      st.insert = { x0: p.x, y0: p.y, over, pointer: e.pointerId, moved: false };
      ctx.boardHost().setPointerCapture(e.pointerId);
    }
    function moveInsert(e) {
      const p = toPage(e.clientX, e.clientY);
      const i = st.insert;
      if (Math.hypot(p.x - i.x0, p.y - i.y0) > SLOP) i.moved = true;
      drawGhost(i.moved ? { x: Math.min(i.x0, p.x), y: Math.min(i.y0, p.y), w: Math.abs(p.x - i.x0), h: Math.abs(p.y - i.y0) } : null);
    }
    function endInsert(e) {
      const i = st.insert;
      st.insert = null;
      drawGhost(null);
      const p = toPage(e.clientX, e.clientY);
      const rect = i.moved ? { x: Math.min(i.x0, p.x), y: Math.min(i.y0, p.y), w: Math.abs(p.x - i.x0), h: Math.abs(p.y - i.y0) } : null;
      const what = st.tool;
      const id = newId(what[0]);
      const op = { kind: 'add', by: 'human', what, as: id, label: { text: 'Text', button: 'Button', link: 'Link', section: '', image: 'image' }[what] };
      if (what === 'kit') {
        // The component picked in the search field beside the tools.
        op.name = ctx.kitPick ? ctx.kitPick() : '';
        if (!op.name) { setTool('move'); return; }
        op.label = op.name;
        op.props = {};
      }
      if (what === 'image' && rect) op.width = Math.round(rect.w);
      if (what === 'section' && rect) op.height = Math.round(rect.h);
      // Where it lands: in a free container at the point; in flow, after the part under the pointer.
      const container = i.over && (isStatic(i.over) ? i.over : freeParent(i.over));
      if (container && isStatic(container)) {
        const pb = box(container);
        const b = rect || { x: p.x, y: p.y, w: what === 'section' ? 240 : what === 'image' ? 200 : 120, h: what === 'section' ? 120 : what === 'image' ? 120 : 32 };
        Object.assign(op, { into: ctx.opTarget(container), x: round1((b.x - pb.x) / pb.w * 100), y: round1((b.y - pb.y) / pb.h * 100), w: round1(b.w / pb.w * 100), h: round1(b.h / pb.h * 100) });
      } else if (i.over && isContainer(i.over) && (!i.over.children.length || placement(i.over, e.clientX, e.clientY) === 'into')) {
        op.into = ctx.opTarget(i.over);
      } else if (i.over) {
        op[placement(i.over, e.clientX, e.clientY) === 'before' ? 'before' : 'after'] = ctx.opTarget(i.over);
      } else {
        const d = doc();
        const main = d.querySelector('main') || d.body.firstElementChild || d.body;
        op.into = E.targetOf(main) || 'path:';
      }
      setTool('move');
      if (ctx.step(op)) { select('data:' + id); if (what === 'text') setTimeout(() => { const f = q('.fg-text'); if (f) { f.focus(); f.select(); } }, 50); }
    }

    // ---- keyboard ---------------------------------------------------------------------------
    function onKey(e) {
      if (!ctx.editing()) return false;
      const typing = e.target.matches('input,textarea,select,[contenteditable]');
      const meta = e.ctrlKey || e.metaKey;
      if (e.key === 'Escape') { if (st.menu) { closeMenu(); return true; } if (st.tool !== 'move') { setTool('move'); return true; } if (st.sels.length) { select([]); return true; } return false; }
      if (typing) return false;
      const tool = Object.keys(TOOLS).find((t) => TOOLS[t] && TOOLS[t].toLowerCase() === e.key.toLowerCase());
      if (tool && !meta && !e.altKey) { setTool(tool); return true; }
      if (meta && e.key.toLowerCase() === 'd') { duplicate(); return true; }
      if (meta && e.key.toLowerCase() === 'a' && !e.shiftKey) { selectAll(); return true; }
      if (e.shiftKey && !meta && e.key.toLowerCase() === 'a') { toggleFree(); return true; }
      if (e.key === 'Delete' || e.key === 'Backspace') { remove(); return true; }
      if (e.shiftKey && e.key === '!') { ctx.board().fit(); return true; }
      if (e.shiftKey && e.key === '@') { zoomToSelection(); return true; }
      if (/^Arrow/.test(e.key)) { nudge(e); return true; }
      return false;
    }
    function selectAll() {
      const d = doc();
      const root = one() && isContainer(one()) ? one() : (st.sels.length ? selected()[0].parentElement : d.querySelector('main') || d.body);
      select([...root.children].filter((c) => c.matches(grabSelector())).map((c) => E.targetOf(c)));
    }
    function zoomToSelection() {
      const els = selected();
      if (!els.length || !ctx.board()) return;
      ctx.board().focus(ctx.proxy(els[0]), 1);
    }
    // Arrows: a pixel, ten with shift. In flow, a step among the siblings.
    function nudge(e) {
      const els = selected();
      if (!els.length) return;
      e.preventDefault();
      const free = els.every((x) => freeParent(x) && freeParent(x) === freeParent(els[0]));
      const step = e.shiftKey ? 10 : 1;
      const dx = e.key === 'ArrowLeft' ? -step : e.key === 'ArrowRight' ? step : 0;
      const dy = e.key === 'ArrowUp' ? -step : e.key === 'ArrowDown' ? step : 0;
      if (free) {
        if (!st.nudge) st.nudge = { parent: freeParent(els[0]), els, boxes: new Map(els.map((el) => [el, box(el)])) };
        const pb = box(st.nudge.parent);
        els.forEach((el) => { const b = st.nudge.boxes.get(el); b.x += dx; b.y += dy; el.style.left = (b.x - pb.x) + 'px'; el.style.top = (b.y - pb.y) + 'px'; });
        drawSelection();
        clearTimeout(st.nudgeTimer);
        st.nudgeTimer = setTimeout(() => { const n = st.nudge; st.nudge = null; commitFree(n.parent, n.els, n.boxes); }, NUDGE_DELAY);
        return;
      }
      if (els.length !== 1) return;
      const el = els[0];
      const back = dx < 0 || dy < 0;
      const sibling = back ? el.previousElementSibling : el.nextElementSibling;
      if (!sibling) return;
      const id = el.id || (el.dataset && (el.dataset.component || el.dataset.id)) ? null : newId('m');
      const op = { kind: 'move', by: 'human', target: ctx.opTarget(el) };
      if (id) op.as = id;
      op[back ? 'before' : 'after'] = ctx.opTarget(sibling);
      if (ctx.step(op)) select(id ? 'data:' + id : E.targetOf(el));
    }
    function duplicate() {
      const els = selected();
      if (!els.length) return;
      const ops = []; const ids = [];
      els.forEach((el) => { const id = newId('c'); ops.push({ kind: 'clone', by: 'human', target: ctx.opTarget(el), as: id }); ids.push('data:' + id); });
      if (ctx.step(ops)) select(ids);
    }
    function remove() {
      const els = selected();
      if (!els.length) return;
      if (ctx.step(els.map((el) => ({ kind: 'remove', by: 'human', target: ctx.opTarget(el) })))) select([]);
    }
    // shift+A: the selected container — or the selection's parent — switches
    // between flow (auto-layout) and free placement.
    function toggleFree() {
      const el = one();
      const target = el && isContainer(el) && el.children.length ? el : (selected()[0] || {}).parentElement;
      if (!target || target === doc().body) return;
      if (isStatic(target)) { ctx.step({ kind: 'static', by: 'human', target: ctx.opTarget(target), layout: 'flow' }); return; }
      ctx.step({ kind: 'static', by: 'human', target: ctx.opTarget(target), layout: 'static', height: Math.round(box(target).h), items: itemsOf(target) });
    }

    // ---- alignment, in a free container --------------------------------------------------
    function align(how) {
      const els = selected();
      const parent = els.length ? freeParent(els[0]) : null;
      if (!parent || !els.every((x) => freeParent(x) === parent)) return;
      const boxes = els.map(box);
      const u = els.length > 1 ? union(boxes) : box(parent);
      const over = new Map();
      if (how === 'h-distribute' || how === 'v-distribute') {
        const horizontal = how === 'h-distribute';
        const sorted = els.map((el, i) => ({ el, b: boxes[i] })).sort((a, b) => (horizontal ? a.b.x - b.b.x : a.b.y - b.b.y));
        const total = sorted.reduce((s, x) => s + (horizontal ? x.b.w : x.b.h), 0);
        const gap = ((horizontal ? u.w : u.h) - total) / Math.max(1, sorted.length - 1);
        let at = horizontal ? u.x : u.y;
        sorted.forEach(({ el, b }) => { over.set(el, horizontal ? { ...b, x: at } : { ...b, y: at }); at += (horizontal ? b.w : b.h) + gap; });
      } else {
        els.forEach((el, i) => {
          const b = { ...boxes[i] };
          if (how === 'left') b.x = u.x; if (how === 'right') b.x = u.x + u.w - b.w; if (how === 'h-center') b.x = u.x + (u.w - b.w) / 2;
          if (how === 'top') b.y = u.y; if (how === 'bottom') b.y = u.y + u.h - b.h; if (how === 'v-center') b.y = u.y + (u.h - b.h) / 2;
          over.set(el, b);
        });
      }
      commitFree(parent, els, over);
    }

    // ---- the right-click menu ---------------------------------------------------------------
    function onContext(e) {
      if (!ctx.editing() || !e.target.classList.contains('mk-shield')) return;
      e.preventDefault(); e.stopPropagation();
      const hit = elementAt(e.clientX, e.clientY, grabSelector());
      if (hit && !st.sels.includes(E.targetOf(hit))) select(E.targetOf(hit));
      const el = one();
      const comp = el && el.closest('[data-component]');
      const items = [];
      if (st.sels.length) {
        items.push(['duplicate', 'Duplicate', '⌘D'], ['remove', 'Delete', '⌫']);
        if (el && !freeParent(el)) items.push(['forward', 'Move forward', '→'], ['backward', 'Move backward', '←']);
        items.push(['free', el && isContainer(el) && isStatic(el) ? 'Back to auto-layout' : el && isContainer(el) && el.children.length ? 'Free placement' : freeParent(el) ? 'Parent back to auto-layout' : 'Parent to free placement', '⇧A']);
        items.push(['request', 'Request on this…', '']);
        if (comp && !comp.dataset.inline && !ctx.mfOnly()) items.push(['fork', `Fork ${comp.dataset.component} for this page`, '']);
        if (el) items.push(['zoom', 'Zoom to selection', '⇧2']);
      } else {
        items.push(['select-all', 'Select all', '⌘A'], ['fit', 'Zoom to fit', '⇧1']);
      }
      closeMenu();
      const m = document.createElement('div');
      m.className = 'fg-menu';
      m.innerHTML = items.map(([k, label, key]) => `<button type="button" data-menu="${k}"><span>${esc(label)}</span><kbd>${esc(key)}</kbd></button>`).join('');
      m.style.left = e.clientX + 'px'; m.style.top = e.clientY + 'px';
      document.body.appendChild(m);
      st.menu = m;
      m.addEventListener('click', (ev) => { const b = ev.target.closest('[data-menu]'); if (b) { closeMenu(); menuAction(b.dataset.menu); } });
      setTimeout(() => document.addEventListener('pointerdown', onMenuAway, true), 0);
    }
    const onMenuAway = (ev) => { if (st.menu && !st.menu.contains(ev.target)) closeMenu(); };
    function closeMenu() { if (st.menu) { st.menu.remove(); st.menu = null; document.removeEventListener('pointerdown', onMenuAway, true); } }
    function menuAction(what) {
      const el = one();
      if (what === 'duplicate') duplicate();
      if (what === 'remove') remove();
      if (what === 'free') toggleFree();
      if (what === 'select-all') selectAll();
      if (what === 'fit') ctx.board().fit();
      if (what === 'zoom') zoomToSelection();
      if (what === 'request' && st.sels[0]) ctx.newRequest(st.sels[0]);
      if (what === 'fork' && el) { const t = ctx.opTarget(el); select(t); }
      if ((what === 'forward' || what === 'backward') && el) nudge({ key: what === 'forward' ? 'ArrowRight' : 'ArrowLeft', shiftKey: false, preventDefault() {} });
    }

    // ---- the layers: the page as a tree -----------------------------------------------------
    function layerRows() {
      const d = doc();
      if (!d || !d.body) return [];
      const rows = [];
      const walk = (el, depth) => {
        [...el.children].forEach((c) => {
          if (/^(SCRIPT|STYLE|LINK|META|TEMPLATE)$/.test(c.tagName) || c.id === 'hb-style') return;
          const part = c.matches(EDITABLE);
          const comp = c.dataset && c.dataset.component;
          const kids = comp && !c.dataset.inline ? [] : [...c.children].filter((k) => !/^(SCRIPT|STYLE)$/.test(k.tagName));
          const target = E.targetOf(c);
          if (part) {
            const open = st.open.has(target) ? st.open.get(target) : depth < 2;
            rows.push({ el: c, target, depth, label: labelOf(c), comp, inline: !!(c.dataset && c.dataset.inline), free: isStatic(c), hasKids: kids.length > 0, open });
            if (kids.length && open && depth < 10) walk(c, depth + 1);
          } else walk(c, depth);
        });
      };
      walk(d.body, 0);
      return rows;
    }
    function drawLayers() {
      const panel = q('.mk-layers');
      if (!panel) return;
      panel.hidden = !ctx.editing();
      if (panel.hidden) return;
      const rows = layerRows();
      panel.innerHTML = '<div class="mk-panel-head"><b>layers</b><small>' + rows.length + '</small></div><ul class="fg-tree">'
        + rows.map((r) => `<li class="fg-row ${st.sels.includes(r.target) ? 'sel' : ''} ${r.comp ? 'comp' : ''}" data-layer="${esc(r.target)}" style="--depth:${r.depth}">`
          + (r.hasKids ? `<i class="fg-twist ${r.open ? 'open' : ''}" data-twist="${esc(r.target)}"></i>` : '<i class="fg-twist none"></i>')
          + `<span class="fg-ico">${r.comp ? '⟐' : r.free ? '▦' : isContainer(r.el) ? '▭' : r.el.tagName === 'IMG' ? '▨' : /^(BUTTON|A)$/.test(r.el.tagName) ? '⊡' : 'T'}</span>`
          + `<span class="fg-label">${esc(r.label)}</span>`
          + (r.comp ? `<small>${r.inline ? 'fork' : 'micro-frontend'}</small>` : '') + '</li>').join('')
        + '</ul>';
    }
    function highlightLayer(el) {
      const panel = q('.mk-layers');
      if (!panel || panel.hidden) return;
      panel.querySelectorAll('.fg-row.hover').forEach((r) => r.classList.remove('hover'));
      if (!el) return;
      const row = panel.querySelector(`[data-layer="${CSS.escape(E.targetOf(el))}"]`);
      if (row) row.classList.add('hover');
    }
    function onLayerDown(e) {
      const twist = e.target.closest('[data-twist]');
      if (twist) { const t = twist.dataset.twist; const row = layerRows().find((r) => r.target === t); st.open.set(t, row ? !row.open : true); e.preventDefault(); drawLayers(); return; }
      const row = e.target.closest('[data-layer]');
      if (!row || e.button !== 0) return;
      const target = row.dataset.layer;
      if (e.shiftKey) { select(target, { toggle: true }); return; }
      if (!st.sels.includes(target)) select(target);
      st.layerDrag = { target, startY: e.clientY, moved: false, over: null, where: '' };
      row.setPointerCapture(e.pointerId);
    }
    function onLayerMove(e) {
      const d = st.layerDrag;
      if (d) {
        if (!d.moved && Math.abs(e.clientY - d.startY) < SLOP) return;
        d.moved = true;
        const panel = q('.mk-layers');
        const rows = [...panel.querySelectorAll('[data-layer]')];
        rows.forEach((r) => r.classList.remove('drop-before', 'drop-after', 'drop-into'));
        const over = rows.find((r) => { const b = r.getBoundingClientRect(); return e.clientY >= b.top && e.clientY <= b.bottom; });
        if (!over || over.dataset.layer === d.target) { d.over = null; return; }
        const b = over.getBoundingClientRect();
        const el = E.elementOf(doc(), over.dataset.layer);
        const ratio = (e.clientY - b.top) / b.height;
        const where = isContainer(el) && ratio > 0.3 && ratio < 0.7 ? 'into' : ratio < 0.5 ? 'before' : 'after';
        over.classList.add('drop-' + where);
        d.over = over.dataset.layer; d.where = where;
        return;
      }
      const row = e.target.closest('[data-layer]');
      setHover(row ? E.elementOf(doc(), row.dataset.layer) : null);
    }
    function onLayerUp() {
      const d = st.layerDrag;
      if (!d) return;
      st.layerDrag = null;
      const panel = q('.mk-layers');
      panel.querySelectorAll('.drop-before, .drop-after, .drop-into').forEach((r) => r.classList.remove('drop-before', 'drop-after', 'drop-into'));
      if (!d.moved || !d.over) return;
      const el = E.elementOf(doc(), d.target);
      const dest = E.elementOf(doc(), d.over);
      if (!el || !dest || el.contains(dest)) return;
      const id = el.id || (el.dataset && (el.dataset.component || el.dataset.id)) ? null : newId('m');
      const op = { kind: 'move', by: 'human', target: ctx.opTarget(el) };
      if (id) op.as = id;
      op[d.where] = ctx.opTarget(dest);
      if (ctx.step(op)) select(id ? 'data:' + id : E.targetOf(el));
    }

    // ---- the properties ------------------------------------------------------------------------
    function drawProps() {
      const panel = q('.mk-panel');
      if (!panel) return;
      const els = selected();
      panel.hidden = !ctx.editing() || !els.length;
      ctx.onPanel && ctx.onPanel(!panel.hidden);
      if (panel.hidden) return;
      const el = els.length === 1 ? els[0] : null;
      const free = els.every((x) => freeParent(x) && freeParent(x) === freeParent(els[0]));
      const alignBar = `<div class="fg-align" title="${free ? 'align, in the free container' : 'alignment needs parts of one free container'}">`
        + [['left', '⫷'], ['h-center', '⫿'], ['right', '⫸'], ['top', '⤒'], ['v-center', '⫼'], ['bottom', '⤓'], ['h-distribute', '⇹'], ['v-distribute', '⇳']].map(([k, g]) => `<button type="button" class="mini" data-align="${k}" ${free ? '' : 'disabled'} title="${k}">${g}</button>`).join('')
        + '</div>';
      if (!el) {
        panel.innerHTML = `<div class="mk-panel-head"><b>${els.length} parts</b></div>${alignBar}<div class="mk-panel-actions"><button type="button" class="mini" data-fg="duplicate">duplicate</button><button type="button" class="mini" data-fg="remove">remove</button></div>`;
        return;
      }
      const b = box(el);
      const parent = freeParent(el);
      const pb = parent ? box(parent) : null;
      const row = el.parentElement ? box(el.parentElement).w : b.w;
      const cols = row ? Math.max(1, Math.min(E.COLS, Math.round(b.w / (row / E.COLS)))) : E.COLS;
      const comp = el.closest('[data-component]');
      const own = comp === el;
      const text = !comp || own || comp.dataset.inline ? ([...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim()) || (!el.children.length && !/^(IMG|INPUT|SELECT|TEXTAREA)$/.test(el.tagName))) : false;
      const num = (k, v, on, title) => `<label class="fg-num" title="${title}"><span>${k}</span><input class="fg-field" data-field="${k}" type="number" step="1" value="${Math.round(v)}" ${on ? '' : 'disabled'}></label>`;
      const c = own ? ctx.componentOf(el.dataset.component) : null;
      panel.innerHTML = `<div class="mk-panel-head"><b>${esc(labelOf(el))}</b><small>${esc(el.tagName.toLowerCase())}${comp && !own ? ' · in ' + esc(comp.dataset.component) + (comp.dataset.inline ? ' (this page’s fork)' : ' — editing forks it') : ''}</small></div>`
        + alignBar
        + `<div class="fg-nums">${num('X', pb ? b.x - pb.x : b.x, !!pb, pb ? 'from the container’s left' : 'in flow: the layout decides')}${num('Y', pb ? b.y - pb.y : b.y, !!pb, pb ? 'from the container’s top' : 'in flow: the layout decides')}${num('W', b.w, true, pb ? 'width' : `width, kept as ${cols} of 12 columns`)}${num('H', b.h, !!pb || isContainer(el), pb ? 'height' : 'minimum height')}</div>`
        + (!pb ? `<div class="mk-panel-row"><span>columns</span><span class="mono">${cols} / 12</span></div>` : '')
        + (text ? `<label class="mk-panel-field">text<input class="fg-text" data-field="text" value="${esc(el.children.length ? ([...el.childNodes].find((n) => n.nodeType === 3 && n.textContent.trim()) || {}).textContent || '' : el.textContent)}" maxlength="500"></label>` : '')
        + (el.tagName === 'A' ? `<label class="mk-panel-field">goes to<select class="fg-page" data-field="page"><option value="">a URL…</option>${(ctx.pages() || []).map((p) => `<option value="${esc(p.path)}"${el.getAttribute('href') === '../' + p.path ? ' selected' : ''}>${esc(p.title)}</option>`).join('')}</select><input class="fg-href" data-field="href" value="${esc(el.getAttribute('href') || '')}" placeholder="https://…"></label>` : '')
        + (el.tagName === 'BUTTON' ? `<label class="mk-panel-field">intent<textarea class="fg-intent" data-field="intent" rows="2" placeholder="what this button is for — written for the next roadmap">${esc(el.dataset.intent || '')}</textarea></label>` : '')
        + (el.dataset.kit ? `<div class="fg-comp"><b>kit component</b> <span class="mono">${esc(el.dataset.kit)}</span><div class="muted">a visual primitive of the design system: its look is edited on the design system's easel, not here</div><label class="mk-panel-field">props (JSON)<textarea class="fg-props" data-field="props" rows="2">${esc(el.dataset.props || '{}')}</textarea></label><div class="mk-panel-actions"><button type="button" class="mini" data-fg="kit">open on the easel</button></div></div>` : '')
        + (c ? `<div class="fg-comp"><b>micro-frontend</b><div class="muted">${esc(c.description || '')}</div><div class="muted">${c.needs.length ? 'needs ' + c.needs.map(esc).join(', ') : 'needs nothing'}${c.actions.length ? ' · triggers ' + c.actions.map(esc).join(', ') : ''}</div>`
          + `<div class="mk-panel-actions">${c.mocked ? '' : '<button type="button" class="mini on" data-fg="mock">mock it with an AI session</button>'}${el.dataset.inline ? '' : '<button type="button" class="mini" data-fg="fork">fork for this page</button>'}<button type="button" class="mini" data-fg="design">discuss its design</button></div></div>` : '')
        + '<div class="mk-panel-actions">'
        + '<button type="button" class="mini" data-fg="duplicate" title="⌘D">duplicate</button>'
        + '<button type="button" class="mini" data-fg="remove" title="⌫">remove</button>'
        + (isContainer(el) && el.children.length ? `<button type="button" class="mini" data-fg="free" title="⇧A">${isStatic(el) ? 'back to auto-layout' : 'free placement'}</button>` : '')
        + '</div>'
        + `<div class="mk-panel-requests"><b>requests</b> <button type="button" class="mini" data-fg="request">+ request</button>${ctx.requestsHtml(st.sels[0])}</div>`;
    }
    function onPanelChange(e) {
      const el = one();
      if (!el) return;
      const f = e.target.dataset.field;
      if (!f) return;
      const v = e.target.value;
      if (f === 'text') { ctx.step({ kind: 'text', by: 'human', target: ctx.opTarget(el), text: v }); return; }
      if (f === 'props') { let props; try { props = JSON.parse(v || '{}'); } catch (err) { return; } ctx.step({ kind: 'props', by: 'human', target: ctx.opTarget(el), props }); return; }
      if (f === 'page' || f === 'href') { setHref(el, f === 'page' ? (v ? '../' + v : '') : v); return; }
      if (f === 'intent') { const id = el.dataset.id || newId('b'); ctx.step([{ kind: 'add', by: 'human', what: 'button', label: el.textContent, intent: v, as: id, after: ctx.opTarget(el) }, { kind: 'remove', by: 'human', target: ctx.opTarget(el) }]) && select('data:' + id); return; }
      if (/^[XYWH]$/.test(f)) {
        const n = Number(v);
        if (!Number.isFinite(n)) return;
        const parent = freeParent(el);
        if (parent) {
          const b = box(el); const pb = box(parent);
          if (f === 'X') b.x = pb.x + n; if (f === 'Y') b.y = pb.y + n; if (f === 'W') b.w = Math.max(4, n); if (f === 'H') b.h = Math.max(4, n);
          commitFree(parent, [el], new Map([[el, b]]));
          return;
        }
        const row = el.parentElement ? box(el.parentElement).w : box(el).w;
        const op = { kind: 'resize', by: 'human', target: ctx.opTarget(el), cols: f === 'W' ? Math.max(1, Math.min(E.COLS, Math.round(n / (row / E.COLS)))) : Math.max(1, Math.min(E.COLS, Math.round(box(el).w / (row / E.COLS)))) };
        if (f === 'H') op.height = Math.round(n);
        ctx.step(op);
      }
    }
    function setHref(el, href) {
      const id = el.dataset.id || newId('l');
      if (ctx.step([{ kind: 'add', by: 'human', what: 'link', label: el.textContent, href, as: id, after: ctx.opTarget(el) }, { kind: 'remove', by: 'human', target: ctx.opTarget(el) }])) select('data:' + id);
    }
    function onPanelClick(e) {
      if (e.target.closest('[data-fg="kit"]')) { window.dispatchEvent(new CustomEvent('harness:open-pane', { detail: { kind: 'kit', tab: 'canvas' } })); return; }
      const a = e.target.closest('[data-align]');
      if (a) { align(a.dataset.align); return; }
      const b = e.target.closest('[data-fg]');
      if (!b) return;
      const el = one();
      const what = b.dataset.fg;
      if (what === 'duplicate') duplicate();
      if (what === 'remove') remove();
      if (what === 'free') toggleFree();
      if (what === 'request' && st.sels[0]) ctx.newRequest(st.sels[0]);
      if (what === 'fork' && el) { const t = ctx.opTarget(el, true); select(t); }
      if (what === 'mock' && el) ctx.mockComponent(el.dataset.component);
      if (what === 'design' && el) ctx.designSession(el.dataset.component);
    }

    // ---- wiring -----------------------------------------------------------------------------
    const onKeyDown = (e) => { if (e.target.closest('.mk-panel,.mk-layers,.mk-history,.mk-editbar,.mk-form,.hb-note') && e.key !== 'Escape') return; if (onKey(e)) { e.preventDefault(); e.stopPropagation(); } };
    const onPanelKey = (e) => { if (e.key === 'Enter' && e.target.matches('input')) { e.target.blur(); } };
    function attach() {
      if (st.on) return;
      st.on = true;
      const h = ctx.boardHost();
      h.addEventListener('pointerdown', onDown, true);
      h.addEventListener('pointermove', onMove, true);
      h.addEventListener('pointerup', onUp, true);
      h.addEventListener('contextmenu', onContext, true);
      window.addEventListener('keydown', onKeyDown, true);
      const layers = q('.mk-layers'); const panel = q('.mk-panel');
      if (layers) { layers.addEventListener('pointerdown', onLayerDown); layers.addEventListener('pointermove', onLayerMove); layers.addEventListener('pointerup', onLayerUp); layers.addEventListener('pointerleave', () => { if (!st.layerDrag) setHover(null); }); }
      if (panel) { panel.addEventListener('change', onPanelChange); panel.addEventListener('click', onPanelClick); panel.addEventListener('keydown', onPanelKey); }
      ctx.host.querySelectorAll('[data-tool]').forEach((b) => { b.onclick = () => setTool(b.dataset.tool === st.tool && b.dataset.tool !== 'move' ? 'move' : b.dataset.tool); });
      st.k = zoom();
      tick();
      refresh();
    }
    function detach() {
      if (!st.on) return;
      st.on = false;
      cancelAnimationFrame(st.raf);
      closeMenu();
      const h = ctx.boardHost();
      h.removeEventListener('pointerdown', onDown, true);
      h.removeEventListener('pointermove', onMove, true);
      h.removeEventListener('pointerup', onUp, true);
      h.removeEventListener('contextmenu', onContext, true);
      window.removeEventListener('keydown', onKeyDown, true);
      st.sels = []; st.hover = null; st.drag = null; st.resize = null; st.marquee = null; st.insert = null;
      setTool('move');
      const o = ctx.board() && ctx.board().content.querySelector('.fg-overlay');
      if (o) o.hidden = true;
      const layers = q('.mk-layers'); const panel = q('.mk-panel');
      if (layers) layers.hidden = true;
      if (panel) panel.hidden = true;
      ctx.onPanel && ctx.onPanel(false);
    }

    return {
      setTool, attach, detach, refresh, select, selection: () => st.sels.slice(), setTool, tool: () => st.tool, duplicate, remove, toggleFree, align };
  }

  window.MockDesign = { create, TOOLS };
})();
