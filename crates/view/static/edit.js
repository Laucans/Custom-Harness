// The mock-up's edit mode, the engine half: a page's **draft** is a list of
// operations on the page's source — never another copy of its HTML — and
// this file applies them. It names parts of a page (targets), applies the
// operations to a document parsed from the page's source, serializes the
// result for the iframe and for the saved mock-up, and keeps the draft's
// undo. The gestures — selecting, dragging, resizing — are the pane's
// (mockup.js); the versions are the server's (`/api/drafts`).
//
//   const engine = MockEdit.engine({ source, base, slug, page });
//   engine.push(op); engine.undo(); engine.html()   // the page as the draft shows it
//   MockEdit.targetOf(el); MockEdit.elementOf(doc, target)
(() => {
  'use strict';

  const UNDO_DEPTH = 40;
  const COLS = 12;

  // ---- targets: how a part of a page is named --------------------------------------
  // `component:<Name>` for a micro-frontend, `id:<id>`, `data:<data-id>`, else
  // a path of `tag:nth-child(n)` steps from the body — resolved in order on
  // the page as it is at that point of the draft, so a path stays true as
  // long as the operations replay in order.
  // A kit component on the page, by its name — and its rank among those of
  // the same name when there are several (`kit:Menu@2`).
  function kitTarget(el) {
    const name = el.dataset.kit;
    const same = Array.prototype.filter.call(el.ownerDocument.querySelectorAll('[data-kit]'), (k) => k.dataset.kit === name);
    return 'kit:' + name + (same.length > 1 ? '@' + (same.indexOf(el) + 1) : '');
  }
  function targetOf(el) {
    if (!el || el.nodeType !== 1) return '';
    if (el.dataset && el.dataset.component) return 'component:' + el.dataset.component;
    if (el.dataset && el.dataset.kit) return kitTarget(el);
    if (el.id) return 'id:' + el.id;
    if (el.dataset && el.dataset.id) return 'data:' + el.dataset.id;
    const doc = el.ownerDocument;
    const steps = [];
    let at = el;
    while (at && at !== doc.body && steps.length < 12) {
      const parent = at.parentElement;
      if (!parent) break;
      const index = Array.prototype.indexOf.call(parent.children, at) + 1;
      steps.unshift(`${at.tagName.toLowerCase()}:nth-child(${index})`);
      if (parent.id || (parent.dataset && (parent.dataset.component || parent.dataset.kit || parent.dataset.id))) {
        // Anchored on the nearest stable ancestor: shorter, and steadier.
        return targetOf(parent) + '>' + steps.join('>');
      }
      at = parent;
    }
    return 'path:' + steps.join('>');
  }
  function elementOf(doc, target) {
    if (!doc || !target) return null;
    const cut = target.indexOf('>');
    const head = cut < 0 ? target : target.slice(0, cut);
    const rest = cut < 0 ? '' : target.slice(cut + 1);
    let root = null;
    try {
      if (head.startsWith('component:')) root = doc.querySelector(`[data-component="${CSS.escape(head.slice(10))}"]`);
      else if (head.startsWith('kit:')) {
        const [name, rank] = head.slice(4).split('@');
        const same = Array.prototype.filter.call(doc.querySelectorAll('[data-kit]'), (k) => k.dataset.kit === name);
        root = same[(Number(rank) || 1) - 1] || null;
      }
      else if (head.startsWith('id:')) root = doc.getElementById(head.slice(3));
      else if (head.startsWith('data:')) root = doc.querySelector(`[data-id="${CSS.escape(head.slice(5))}"]`);
      else if (head === 'path:') return doc.body;
      else if (head.startsWith('path:')) return doc.body ? doc.body.querySelector(':scope>' + head.slice(5)) : null;
      if (!root || !rest) return root;
      return root.querySelector(':scope>' + rest);
    } catch (e) { return null; }
  }
  // The micro-frontend a target lies in, if any — what edits inside fork.
  function componentOf(el) {
    const c = el && el.closest ? el.closest('[data-component]') : null;
    return c ? c.dataset.component : '';
  }

  // ---- the operations, applied to a document ------------------------------------------
  // Each one changes the document in place, or answers why it could not;
  // a failed operation is skipped and reported.
  const pct = (cols) => `${(Math.max(1, Math.min(COLS, cols)) / COLS * 100).toFixed(4)}%`;
  function place(doc, el, op) {
    if (op.before) { const t = elementOf(doc, op.before); if (!t || !t.parentNode) return 'no such target before'; t.parentNode.insertBefore(el, t); return ''; }
    if (op.after) { const t = elementOf(doc, op.after); if (!t || !t.parentNode) return 'no such target after'; t.parentNode.insertBefore(el, t.nextSibling); return ''; }
    if (op.into) { const t = elementOf(doc, op.into); if (!t) return 'no such container'; t.appendChild(el); return ''; }
    return 'nowhere to place it';
  }
  function placeFree(el, op) {
    el.style.position = 'absolute';
    el.style.left = `${op.x}%`; el.style.top = `${op.y}%`;
    if (op.w != null) el.style.width = `${op.w}%`;
    if (op.h != null) el.style.height = `${op.h}%`;
    el.style.margin = '0';
    el.style.boxSizing = 'border-box';
  }
  function moveColumn(table, from, to) {
    const rows = table.querySelectorAll('tr');
    rows.forEach((row) => {
      const cells = [...row.children];
      const cell = cells[from];
      if (!cell) return;
      const target = cells[to];
      if (!target) row.appendChild(cell);
      else row.insertBefore(cell, from < to ? target.nextSibling : target);
    });
  }
  const APPLY = {
    move(doc, op) {
      if (op.target.startsWith('col:')) {
        const [, tableTarget, index] = /^col:(.+)#(\d+)$/.exec(op.target) || [];
        const table = elementOf(doc, tableTarget || '');
        if (!table) return 'no such table';
        moveColumn(table, Number(index), Number(op.to));
        return '';
      }
      const el = elementOf(doc, op.target);
      if (!el) return 'no such part';
      if (op.as && !el.id && !(el.dataset && el.dataset.component)) el.dataset.id = op.as;
      const why = place(doc, el, op);
      if (why) return why;
      // A free item moved into a flow container flows again; a free container keeps its own footing.
      if (op.x != null) placeFree(el, op); else if (el.style.position === 'absolute' && el.parentElement && el.parentElement.dataset.layout !== 'static') { el.style.position = ''; el.style.left = ''; el.style.top = ''; el.style.width = ''; el.style.height = ''; el.style.margin = ''; }
      return '';
    },
    resize(doc, op) {
      const el = elementOf(doc, op.target);
      if (!el) return 'no such part';
      if (op.target.startsWith('col:')) return 'a column resizes by its header';
      el.style.width = op.cols ? pct(op.cols) : '';
      el.style.boxSizing = 'border-box';
      if (op.height) el.style.minHeight = `${op.height}px`;
      const display = (el.ownerDocument.defaultView ? el.ownerDocument.defaultView.getComputedStyle(el).display : '') || '';
      if (/^inline/.test(display) || ['A', 'BUTTON', 'SPAN', 'LABEL'].includes(el.tagName)) el.style.display = 'inline-block';
      return '';
    },
    remove(doc, op) {
      const el = elementOf(doc, op.target);
      if (!el) return 'no such part';
      el.remove();
      return '';
    },
    clone(doc, op) {
      const el = elementOf(doc, op.target);
      if (!el) return 'no such part';
      const copy = el.cloneNode(true);
      copy.querySelectorAll('[id]').forEach((x) => x.removeAttribute('id'));
      copy.removeAttribute('id');
      copy.querySelectorAll('[data-loaded]').forEach((x) => x.removeAttribute('data-loaded'));
      if (op.as) copy.dataset.id = op.as;
      el.parentNode.insertBefore(copy, el.nextSibling);
      return '';
    },
    text(doc, op) {
      const el = elementOf(doc, op.target);
      if (!el) return 'no such part';
      const textNode = [...el.childNodes].find((n) => n.nodeType === 3 && n.textContent.trim());
      if (textNode && el.children.length) textNode.textContent = op.text; else el.textContent = op.text;
      return '';
    },
    static(doc, op) {
      const el = elementOf(doc, op.target);
      if (!el) return 'no such container';
      if (op.layout === 'flow') {
        el.removeAttribute('data-layout');
        el.style.position = ''; el.style.height = '';
        [...el.children].forEach((c) => { c.style.position = ''; c.style.left = ''; c.style.top = ''; c.style.width = ''; c.style.height = ''; c.style.margin = ''; });
        return '';
      }
      el.dataset.layout = 'static';
      el.style.position = 'relative';
      if (op.height) el.style.height = `${op.height}px`;
      (op.items || []).forEach((item) => {
        const child = elementOf(doc, item.target);
        if (!child) return;
        child.style.position = 'absolute';
        child.style.left = `${item.x}%`; child.style.top = `${item.y}%`;
        child.style.width = `${item.w}%`; child.style.height = `${item.h}%`;
        child.style.margin = '0';
        child.style.boxSizing = 'border-box';
      });
      return '';
    },
    add(doc, op) {
      if (op.skip) return `skipped: ${op.skip}`;
      let el;
      if (op.what === 'link') {
        el = doc.createElement('a');
        el.textContent = op.label || 'link';
        el.setAttribute('href', op.page ? `../${op.page}` : (op.href || '#'));
      } else if (op.what === 'text') {
        el = doc.createElement(op.tag && /^(p|h1|h2|h3|h4|span)$/.test(op.tag) ? op.tag : 'p');
        el.textContent = op.label || 'Text';
      } else if (op.what === 'section') {
        el = doc.createElement('section');
        el.style.minHeight = `${op.height || 120}px`;
        el.style.border = '1px dashed rgba(43,192,200,.6)';
        el.style.borderRadius = '6px';
        if (op.label) el.setAttribute('aria-label', op.label);
      } else if (op.what === 'image') {
        el = doc.createElement('img');
        el.setAttribute('alt', op.label || 'image');
        el.setAttribute('src', 'data:image/svg+xml;utf8,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><rect width="200" height="120" fill="#dfe5ec"/><path d="M30 90 L75 50 L105 75 L125 60 L170 90 Z" fill="#aab4c0"/><circle cx="140" cy="38" r="12" fill="#aab4c0"/></svg>'));
        el.style.width = `${op.width || 200}px`;
        el.style.height = 'auto';
        el.style.display = 'block';
      } else if (op.what === 'component') {
        // A micro-frontend of the branch: the runtime fills it from components/<Name>.html.
        el = doc.createElement('div');
        el.dataset.component = op.name || '';
        if (op.props) el.dataset.props = JSON.stringify(op.props);
      } else if (op.what === 'kit') {
        // A component of the design system's kit: the loader fills it from kit/<Name>.html.
        el = doc.createElement('div');
        el.dataset.kit = op.name || '';
        el.dataset.props = JSON.stringify(op.props || {});
        el.setAttribute('aria-label', op.label || op.name || 'component');
      } else {
        el = doc.createElement('button');
        el.textContent = op.label || 'button';
        el.setAttribute('type', 'button');
        if (op.intent) el.dataset.intent = op.intent;
        if (op.action) el.dataset.action = op.action;
      }
      el.classList.add('mk-added');
      if (op.as) el.dataset.id = op.as;
      const why = place(doc, el, op);
      if (why) return why;
      if (op.x != null) placeFree(el, op);
      return '';
    },
    props(doc, op) {
      const el = elementOf(doc, op.target);
      if (!el) return 'no such part';
      el.dataset.props = JSON.stringify(op.props || {});
      return '';
    },
    fork(doc, op) {
      const el = elementOf(doc, op.target);
      if (!el) return 'no such micro-frontend';
      el.dataset.component = op.as;
      el.dataset.inline = '1';
      el.dataset.variantOf = op.from || op.target.replace(/^component:/, '');
      if (typeof op.html === 'string') el.innerHTML = op.html;
      return '';
    },
    viewport() { return ''; },
  };
  // Applies `ops` in order; answers the failures, `{ index, why }`.
  function applyOps(doc, ops) {
    const failed = [];
    ops.forEach((op, index) => {
      const fn = APPLY[op.kind];
      let why = fn ? fn(doc, op) : 'unknown operation';
      if (why) failed.push({ index, why, op });
    });
    return failed;
  }

  // ---- the page's source, parsed and serialized ----------------------------------------
  function parse(html) {
    return new DOMParser().parseFromString(html, 'text/html');
  }
  // The document for the iframe: a `<base>` so relative links resolve as
  // from the page's own folder, since `srcdoc` has no folder of its own.
  function serialize(doc, baseHref) {
    const copy = doc.cloneNode(true);
    if (baseHref) {
      const base = copy.createElement('base');
      base.setAttribute('href', baseHref);
      (copy.head || copy.documentElement).insertBefore(base, (copy.head || copy.documentElement).firstChild);
    }
    return '<!doctype html>\n' + copy.documentElement.outerHTML;
  }
  // The page as the saved mock-up carries it: no base, no load marks, the
  // micro-frontends' loaded content stripped (an inlined fork keeps its own).
  function materialize(doc) {
    const copy = doc.cloneNode(true);
    copy.querySelectorAll('[data-component]').forEach((el) => {
      el.removeAttribute('data-loaded');
      if (!el.dataset.inline) el.replaceChildren();
    });
    copy.querySelectorAll('#hb-style, .mk-sel, .mk-hover').forEach((el) => { if (el.id === 'hb-style') el.remove(); else el.classList.remove('mk-sel', 'mk-hover'); });
    return '<!doctype html>\n' + copy.documentElement.outerHTML;
  }

  // ---- the engine: the source, the operations, undo ---------------------------------
  function engine(setup) {
    const st = { source: setup.source, ops: [...(setup.ops || [])], redo: [], undoable: 0, failed: [], steps: [] };
    // A new step: kept for undo, the redo cleared, the depth bounded.
    function step(taken) {
      st.redo = [];
      st.steps.push(taken);
      if (st.steps.length > UNDO_DEPTH) st.steps.shift();
      st.undoable = Math.min(UNDO_DEPTH, st.undoable + 1);
    }
    function render() {
      const doc = parse(st.source);
      st.failed = applyOps(doc, st.ops);
      return doc;
    }
    return {
      get ops() { return st.ops; },
      get failed() { return st.failed; },
      setOps(ops) { st.ops = [...ops]; st.redo = []; st.undoable = 0; st.steps = []; },
      setSource(source) { st.source = source; },
      /** Adds an operation — or several as one undoable step — and answers whether all applied. */
      push(ops) {
        const list = Array.isArray(ops) ? ops : [ops];
        const probe = parse(st.source);
        const before = applyOps(probe, st.ops).length;
        const after = applyOps(probe, list).length;
        if (after > 0) return { ok: false, failed: after, before };
        st.ops.push(...list);
        step({ kind: 'push', n: list.length });
        return { ok: true };
      },
      /**
       * Replaces the operations signed by `by` (request ids) with `list`, as
       * one undoable step — a request applied, or applied again — and
       * answers whether all applied. `meta` rides along with the step.
       */
      swap(by, list, meta) {
        const ids = new Set(by);
        const kept = st.ops.filter((o) => !ids.has(o.by));
        const probe = parse(st.source);
        const before = applyOps(probe, kept).length;
        const after = applyOps(probe, list).length;
        if (after > 0) return { ok: false, failed: after, before };
        const was = st.ops.slice();
        st.ops = kept.concat(list);
        step({ kind: 'swap', before: was, after: st.ops.slice(), meta: meta || null });
        return { ok: true };
      },
      /** The step undo would take back, or null. */
      lastStep() { return st.steps && st.steps.length && st.undoable > 0 ? st.steps[st.steps.length - 1] : null; },
      canUndo() { return st.undoable > 0 && !!st.steps && st.steps.length > 0; },
      canRedo() { return st.redo.length > 0; },
      /** The step redo would bring back, or null. */
      nextRedo() { return st.redo.length ? st.redo[st.redo.length - 1] : null; },
      /** Takes the last step back; answers it (truthy), or false. */
      undo() {
        if (!this.canUndo()) return false;
        const taken = st.steps.pop();
        if (taken.kind === 'swap') st.ops = taken.before.slice();
        else taken.ops = st.ops.splice(st.ops.length - taken.n, taken.n);
        st.redo.push(taken);
        st.undoable -= 1;
        return taken;
      },
      /** Brings the last step undone back; answers it (truthy), or false. */
      redo() {
        const back = st.redo.pop();
        if (!back) return false;
        if (back.kind === 'swap') st.ops = back.after.slice();
        else st.ops.push(...back.ops);
        st.steps.push(back);
        st.undoable = Math.min(UNDO_DEPTH, st.undoable + 1);
        return back;
      },
      /** The page's document with the draft applied. */
      document: render,
      /** The page for the iframe. */
      html(baseHref) { return serialize(render(), baseHref); },
      /** The page for the saved mock-up. */
      materialized() { return materialize(render()); },
    };
  }

  // ---- what a static layout needs: the children's boxes as percentages -------------------
  function staticItems(container) {
    const r = container.getBoundingClientRect();
    if (!r.width || !r.height) return [];
    return [...container.children].map((child) => {
      const c = child.getBoundingClientRect();
      const snap = (v) => Math.round(v * 2) / 2;
      return { target: targetOf(child), x: snap((c.left - r.left) / r.width * 100), y: snap((c.top - r.top) / r.height * 100), w: snap(c.width / r.width * 100), h: snap(c.height / r.height * 100) };
    });
  }

  window.MockEdit = { targetOf, elementOf, componentOf, applyOps, parse, serialize, materialize, engine, staticItems, COLS, UNDO_DEPTH };
})();
