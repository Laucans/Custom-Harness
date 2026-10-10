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
  function targetOf(el) {
    if (!el || el.nodeType !== 1) return '';
    if (el.dataset && el.dataset.component) return 'component:' + el.dataset.component;
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
      if (parent.id || (parent.dataset && (parent.dataset.component || parent.dataset.id))) {
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
      else if (head.startsWith('id:')) root = doc.getElementById(head.slice(3));
      else if (head.startsWith('data:')) root = doc.querySelector(`[data-id="${CSS.escape(head.slice(5))}"]`);
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
      return place(doc, el, op);
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
      } else {
        el = doc.createElement('button');
        el.textContent = op.label || 'button';
        el.setAttribute('type', 'button');
        if (op.intent) el.dataset.intent = op.intent;
        if (op.action) el.dataset.action = op.action;
      }
      el.classList.add('mk-added');
      if (op.as) el.dataset.id = op.as;
      return place(doc, el, op);
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
    const st = { source: setup.source, ops: [...(setup.ops || [])], redo: [], undoable: 0, failed: [] };
    function render() {
      const doc = parse(st.source);
      st.failed = applyOps(doc, st.ops);
      return doc;
    }
    return {
      get ops() { return st.ops; },
      get failed() { return st.failed; },
      setOps(ops) { st.ops = [...ops]; st.redo = []; st.undoable = 0; },
      setSource(source) { st.source = source; },
      /** Adds an operation — or several as one undoable step — and answers whether all applied. */
      push(ops) {
        const list = Array.isArray(ops) ? ops : [ops];
        const probe = parse(st.source);
        const before = applyOps(probe, st.ops).length;
        const after = applyOps(probe, list).length;
        if (after > 0) return { ok: false, failed: after, before };
        st.ops.push(...list);
        st.redo = [];
        st.undoable = Math.min(UNDO_DEPTH, st.undoable + 1);
        st.groups = (st.groups || []);
        st.groups.push(list.length);
        if (st.groups.length > UNDO_DEPTH) st.groups.shift();
        return { ok: true };
      },
      canUndo() { return st.undoable > 0 && st.groups && st.groups.length > 0; },
      canRedo() { return st.redo.length > 0; },
      undo() {
        if (!this.canUndo()) return false;
        const n = st.groups.pop();
        const taken = st.ops.splice(st.ops.length - n, n);
        st.redo.push(taken);
        st.undoable -= 1;
        return true;
      },
      redo() {
        const back = st.redo.pop();
        if (!back) return false;
        st.ops.push(...back);
        st.groups.push(back.length);
        st.undoable = Math.min(UNDO_DEPTH, st.undoable + 1);
        return true;
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
