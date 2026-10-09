// The product's data model, drawn as a UML class diagram on a board: one box
// per table (folded to its name, a click unfolds its fields), one curve per
// link, and an inspector beside it that tells a table or a field in full —
// type, purpose, links, the technical rules the database holds, the business
// rules the agents' model puts on it. Read from `/api/data-model`: the
// product's `data/schema.sql` (pg_dump) merged with its `data/model.json`.
(() => {
  'use strict';

  // Box geometry, in board units — the CSS draws the same sizes.
  const W = 260;
  const HEAD = 36;
  const ROW = 24;
  const PAD = 6;
  const GAP_X = 120;
  const GAP_Y = 34;
  const PER_COLUMN = 7;
  const MARGIN = 40;
  const REFRESH_MS = 20000;
  const KEPT = 'harness.data-model.open';

  const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const SEEN = {
    both: ['', ''],
    database: ['db only', 'in the database, not in the agents\' model'],
    model: ['model only', 'in the agents\' model, not in the database yet'],
  };

  function remembered() {
    try { return new Set(JSON.parse(localStorage.getItem(KEPT) || '[]')); } catch (e) { return new Set(); }
  }
  function remember(set) {
    try { localStorage.setItem(KEPT, JSON.stringify([...set])); } catch (e) { /* a private window keeps nothing */ }
  }

  function mount(host) {
    host.innerHTML = '<div class="dm">'
      + '<div class="dm-bar"><span class="dm-origin">reading the data model…</span><span class="spacer"></span>'
      + '<input type="search" class="dm-search" placeholder="Find a table or a field" aria-label="find a table or a field">'
      + '<button type="button" class="mini" data-dm="all">unfold all</button></div>'
      + '<div class="dm-main"><div class="dm-board"></div><aside class="dm-inspector" aria-live="polite"></aside></div>'
      + '</div>';
    const q = (s) => host.querySelector(s);
    const inspector = q('.dm-inspector');
    const search = q('.dm-search');
    const st = { model: null, body: '', open: remembered(), sel: null, boxes: new Map(), bounds: null, timer: null, fitted: false, gone: false };

    const describe = (target) => {
      if (!target) return 'the canvas';
      const [kind, name] = [target.slice(0, target.indexOf(':')), target.slice(target.indexOf(':') + 1)];
      return `${kind} ${name}`;
    };
    // A field folded away: its pin stands on its table.
    const resolve = (target) => {
      if (!target) return null;
      const el = board.content.querySelector(`[data-pin="${CSS.escape(target)}"]`);
      if (el || !target.startsWith('field:')) return el;
      const table = target.slice(6).split('.').slice(0, -1).join('.');
      return board.content.querySelector(`[data-pin="${CSS.escape('table:' + table)}"]`);
    };
    const board = window.HarnessBoard.create(q('.dm-board'), { id: 'data-model', describe, resolve, bounds: () => st.bounds });

    // ---- reading -------------------------------------------------------------------
    async function load() {
      clearTimeout(st.timer);
      if (st.gone) return;
      let reply = null;
      try {
        const res = await fetch('/api/data-model');
        reply = res.ok ? await res.json() : null;
      } catch (e) { reply = null; }
      if (st.gone) return;
      if (!reply) { say('The view did not answer.'); st.timer = setTimeout(load, 3000); return; }
      if (!reply.enabled) { say('Nothing reads the product\'s data model: the view runs with <code>--no-board</code> and no <code>--data-dir</code>.'); return; }
      if (!reply.model) { say('Reading the data model…'); st.timer = setTimeout(load, 2000); return; }
      const body = JSON.stringify(reply.model);
      if (body !== st.body) {
        st.body = body;
        st.model = reply.model;
        draw();
      }
      st.timer = setTimeout(load, REFRESH_MS);
    }
    function say(html) {
      q('.dm-origin').innerHTML = html;
    }

    // ---- layout: the tables others point at on the left, those that point on the right
    function layout(m) {
      const names = new Set(m.tables.map((t) => t.name));
      const parents = new Map(m.tables.map((t) => [t.name, new Set()]));
      for (const l of m.links) {
        if (l.kind === 'foreign_key' && l.from_table !== l.to_table && names.has(l.to_table) && names.has(l.from_table)) parents.get(l.from_table).add(l.to_table);
      }
      const level = new Map();
      const visiting = new Set();
      const depth = (n) => {
        if (level.has(n)) return level.get(n);
        if (visiting.has(n)) return 0; // a cycle: cut it here
        visiting.add(n);
        let d = 0;
        for (const p of parents.get(n)) d = Math.max(d, depth(p) + 1);
        visiting.delete(n);
        level.set(n, d);
        return d;
      };
      m.tables.forEach((t) => depth(t.name));
      // Tables nothing links stand apart, in a last column.
      const linked = new Set();
      for (const l of m.links) { if (names.has(l.from_table) && names.has(l.to_table)) { linked.add(l.from_table); linked.add(l.to_table); } }
      const byLevel = new Map();
      for (const t of m.tables) {
        const k = linked.has(t.name) ? level.get(t.name) : Infinity;
        if (!byLevel.has(k)) byLevel.set(k, []);
        byLevel.get(k).push(t);
      }
      const order = new Map();
      const boxes = new Map();
      let x = MARGIN;
      let bottom = 0;
      for (const k of [...byLevel.keys()].sort((a, b) => a - b)) {
        const tables = byLevel.get(k);
        // Each table near the tables it points at.
        const rank = (t) => {
          const ps = [...parents.get(t.name)].filter((p) => order.has(p));
          return ps.length ? ps.reduce((s, p) => s + order.get(p), 0) / ps.length : -1;
        };
        tables.sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
        for (let i = 0; i < tables.length; i += PER_COLUMN) {
          let y = MARGIN;
          tables.slice(i, i + PER_COLUMN).forEach((t, j) => {
            const h = heightOf(t);
            boxes.set(t.name, { x, y, w: W, h, t });
            order.set(t.name, i + j);
            y += h + GAP_Y;
          });
          bottom = Math.max(bottom, y);
          x += W + GAP_X;
        }
      }
      return { boxes, w: x - GAP_X + MARGIN, h: bottom + MARGIN };
    }
    const heightOf = (t) => HEAD + (st.open.has(t.name) && t.fields.length ? t.fields.length * ROW + 2 * PAD : 0);

    // ---- drawing -----------------------------------------------------------------
    function draw() {
      const m = st.model;
      if (!m) return;
      drawBar(m);
      if (!m.tables.length) {
        board.content.innerHTML = '';
        board.edges.innerHTML = '';
        inspector.innerHTML = overview(m);
        return;
      }
      const { boxes, w, h } = layout(m);
      st.boxes = boxes;
      st.bounds = { x: 0, y: 0, w, h };
      board.setSize(w, h);
      board.content.innerHTML = [...boxes.values()].map(tableBox).join('');
      drawEdges(m);
      markSearch();
      board.refresh();
      if (!st.fitted) { st.fitted = true; requestAnimationFrame(() => board.fit()); }
      inspect();
    }
    function drawBar(m) {
      const src = m.sources.map((s) => {
        const cls = s.problem ? 'bad' : s.found ? 'ok' : 'off';
        const tip = s.problem || (s.found ? 'read' : 'not there');
        return `<span class="dm-src ${cls}" title="${esc(tip)}">${esc(s.path)}</span>`;
      }).join('');
      const div = m.compared
        ? (m.divergences.length ? `<button type="button" class="dm-div bad" data-dm="divergences">${m.divergences.length} divergence${m.divergences.length > 1 ? 's' : ''}</button>` : '<span class="dm-div ok">in agreement</span>')
        : '';
      say(`<b>${esc(m.origin)}</b>${src}${div}`);
      q('[data-dm="all"]').textContent = m.tables.every((t) => st.open.has(t.name)) ? 'fold all' : 'unfold all';
    }
    function keyOf(f) {
      if (f.primary) return '<span class="dm-key pk" title="primary key">PK</span>';
      if (f.foreign) return '<span class="dm-key fk" title="points at another field">FK</span>';
      if (f.unique) return '<span class="dm-key uq" title="unique">UQ</span>';
      return '<span class="dm-key"></span>';
    }
    function seenTag(seen) {
      const [label, tip] = SEEN[seen] || SEEN.both;
      return label && st.model.compared ? `<span class="dm-seen ${seen}" title="${esc(tip)}">${label}</span>` : '';
    }
    function tableBox(b) {
      const t = b.t;
      const open = st.open.has(t.name) && t.fields.length;
      const sel = st.sel && st.sel.table === t.name;
      const rows = open ? '<ul class="dm-fields">' + t.fields.map((f) => {
        const fsel = sel && st.sel.field === f.name;
        const warn = f.declared_type ? `<span class="dm-warn" title="the model says ${esc(f.declared_type)}">≠</span>` : '';
        return `<li class="dm-field seen-${f.seen}${fsel ? ' sel' : ''}" data-pin="field:${esc(t.name)}.${esc(f.name)}" data-field="${esc(f.name)}" title="${esc(f.purpose || f.name)}">`
          + `${keyOf(f)}<span class="dm-name">${esc(f.name)}${f.nullable === true ? '<i>?</i>' : ''}</span><span class="dm-type">${esc(f.data_type)}</span>${warn}</li>`;
      }).join('') + '</ul>' : '';
      return `<div class="dm-table seen-${t.seen}${open ? ' open' : ''}${sel && !st.sel.field ? ' sel' : ''}" data-pin="table:${esc(t.name)}" data-table="${esc(t.name)}" style="left:${b.x}px;top:${b.y}px;width:${b.w}px">`
        + `<div class="dm-head" title="${esc(t.purpose || 'click to show its fields')}"><span class="dm-caret">${open ? '▾' : '▸'}</span><b>${esc(t.name)}</b>${seenTag(t.seen)}<span class="dm-count">${t.fields.length}</span></div>`
        + rows + '</div>';
    }
    // Where a link leaves or reaches a table: its field's row when unfolded,
    // the middle of its head when folded.
    function anchorY(b, field) {
      const i = b.t.fields.findIndex((f) => f.name === field);
      if (!st.open.has(b.t.name) || i < 0) return b.y + HEAD / 2;
      return b.y + HEAD + PAD + i * ROW + ROW / 2;
    }
    function drawEdges(m) {
      let paths = '';
      for (const l of m.links) {
        const a = st.boxes.get(l.from_table);
        const b = st.boxes.get(l.to_table);
        if (!a || !b) continue;
        const y1 = anchorY(a, l.from_field);
        const y2 = anchorY(b, l.to_field);
        let d;
        if (a === b) {
          const x = a.x + a.w;
          d = `M ${x} ${y1} C ${x + 60} ${y1}, ${x + 60} ${y2}, ${x} ${y2}`;
        } else {
          const right = a.x + a.w / 2 <= b.x + b.w / 2;
          const sameColumn = a.x === b.x;
          const x1 = right || sameColumn ? a.x + a.w : a.x;
          const x2 = sameColumn ? b.x + b.w : right ? b.x : b.x + b.w;
          const pull = sameColumn ? 70 : Math.max(50, Math.abs(x2 - x1) / 2);
          const c1 = right || sameColumn ? x1 + pull : x1 - pull;
          const c2 = sameColumn ? x2 + pull : right ? x2 - pull : x2 + pull;
          d = `M ${x1} ${y1} C ${c1} ${y1}, ${c2} ${y2}, ${x2} ${y2}`;
        }
        const hl = st.sel && ((l.from_table === st.sel.table && (!st.sel.field || l.from_field === st.sel.field))
          || (l.to_table === st.sel.table && (!st.sel.field || l.to_field === st.sel.field)));
        const cls = ['dm-link', l.kind, 'seen-' + l.seen, hl ? 'hl' : ''].join(' ');
        const tip = `${l.from_table}.${l.from_field} → ${l.to_table}.${l.to_field}${l.on_delete ? ' · on delete ' + l.on_delete : ''}${l.purpose ? ' — ' + l.purpose : ''}`;
        paths += `<path class="${cls}" d="${d}" marker-end="url(#dm-one)" marker-start="url(#dm-many)"><title>${esc(tip)}</title></path>`;
      }
      board.edges.innerHTML = '<defs>'
        + '<marker id="dm-one" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="9" markerHeight="9" orient="auto-start-reverse"><path d="M 0 1 L 9 5 L 0 9" fill="none" stroke="context-stroke" stroke-width="1.6"/></marker>'
        + '<marker id="dm-many" viewBox="0 0 10 10" refX="1" refY="5" markerWidth="8" markerHeight="8" orient="auto"><circle cx="5" cy="5" r="3" fill="context-stroke"/></marker>'
        + '</defs>' + paths;
    }
    function markSearch() {
      const needle = search.value.trim().toLowerCase();
      board.content.classList.toggle('searching', !!needle);
      for (const el of board.content.querySelectorAll('.dm-table')) {
        const t = st.boxes.get(el.dataset.table).t;
        const hit = !!needle && (t.name.toLowerCase().includes(needle) || t.fields.some((f) => f.name.toLowerCase().includes(needle)));
        el.classList.toggle('match', hit);
      }
    }

    // ---- the inspector -------------------------------------------------------------
    const list = (items, cls = '') => (items && items.length ? `<ul class="dm-rules ${cls}">${items.map((r) => `<li>${esc(r)}</li>`).join('')}</ul>` : '<p class="muted">None.</p>');
    const goto = (table, field) => `<button type="button" class="dm-go" data-goto="${esc(table)}" data-goto-field="${esc(field || '')}">${esc(table)}${field ? '.' + esc(field) : ''}</button>`;
    function notesOn(target) {
      const ns = board.notesOn(target);
      if (!ns.length) return '';
      return `<h4>Your notes</h4><ul class="dm-notes">${ns.map((n) => `<li><button type="button" data-open-note="${esc(n.id)}">${esc(n.text || 'empty note')}</button></li>`).join('')}</ul>`;
    }
    function overview(m) {
      const fields = m.tables.reduce((s, t) => s + t.fields.length, 0);
      let h = '<h3>The data model</h3>'
        + `<p class="muted">Read from <b>${esc(m.origin)}</b>. The structure comes from <code>data/schema.sql</code> (<code>pg_dump --schema-only</code>); purposes and business rules from <code>data/model.json</code>, the agents' model.</p>`
        + '<dl class="kv">'
        + m.sources.map((s) => `<dt>${esc(s.path)}</dt><dd>${s.problem ? `<span class="bad">${esc(s.problem)}</span>` : s.found ? 'read' : '<span class="muted">not there</span>'}</dd>`).join('')
        + `<dt>tables</dt><dd>${m.tables.length}</dd><dt>fields</dt><dd>${fields}</dd><dt>links</dt><dd>${m.links.length}</dd></dl>`;
      if (m.compared) {
        h += `<h4>Divergences <span class="muted">· ${m.divergences.length}</span></h4>`
          + (m.divergences.length ? list(m.divergences, 'bad') : '<p class="ok">The database and the model agree.</p>');
      }
      h += '<h4>How to read it</h4><ul class="dm-legend">'
        + '<li><span class="dm-key pk">PK</span> primary key · <span class="dm-key fk">FK</span> points at another field · <span class="dm-key uq">UQ</span> unique</li>'
        + '<li><i>?</i> after a name: it may be empty</li>'
        + '<li>A plain line is a foreign key; a dashed one a link the model declares outside any key; an amber one a key only the model intends.</li>'
        + '<li>Click a table to unfold its fields, a field to read it in full. Drag the 📍 pin onto anything to write a note.</li></ul>';
      return h;
    }
    function inspectTable(m, t) {
      const out = m.links.filter((l) => l.from_table === t.name);
      const into = m.links.filter((l) => l.to_table === t.name && l.from_table !== t.name);
      return `<h3>${esc(t.name)} ${seenTag(t.seen)}</h3>`
        + `<p>${t.purpose ? esc(t.purpose) : '<span class="muted">No purpose written yet.</span>'}</p>`
        + `<h4>Business rules</h4>${list(t.business)}`
        + (t.technical.length ? `<h4>Technical rules</h4>${list(t.technical)}` : '')
        + `<h4>Fields <span class="muted">· ${t.fields.length}</span></h4><ul class="dm-flist">`
        + t.fields.map((f) => `<li>${keyOf(f)}<button type="button" class="dm-go" data-goto="${esc(t.name)}" data-goto-field="${esc(f.name)}">${esc(f.name)}</button><span class="dm-type">${esc(f.data_type)}</span>${seenTag(f.seen)}</li>`).join('') + '</ul>'
        + (out.length ? `<h4>Points at</h4><ul class="dm-links">${out.map((l) => `<li>${esc(l.from_field)} → ${goto(l.to_table, l.to_field)}${linkWords(l)}</li>`).join('')}</ul>` : '')
        + (into.length ? `<h4>Pointed at by</h4><ul class="dm-links">${into.map((l) => `<li>${goto(l.from_table, l.from_field)} → ${esc(l.to_field)}${linkWords(l)}</li>`).join('')}</ul>` : '')
        + notesOn('table:' + t.name);
    }
    function linkWords(l) {
      const bits = [l.kind === 'semantic' ? 'declared link' : 'foreign key'];
      if (l.on_delete) bits.push('on delete ' + l.on_delete);
      if (l.on_update) bits.push('on update ' + l.on_update);
      if (l.seen === 'model' && l.kind === 'foreign_key') bits.push('not in the database yet');
      return ` <small class="muted">${esc(bits.join(' · '))}</small>${l.purpose ? `<div class="muted">${esc(l.purpose)}</div>` : ''}`;
    }
    function inspectField(m, t, f) {
      const out = m.links.filter((l) => l.from_table === t.name && l.from_field === f.name);
      const into = m.links.filter((l) => l.to_table === t.name && l.to_field === f.name);
      const badges = [f.primary && 'primary key', f.foreign && 'foreign key', f.unique && 'unique', f.nullable === true && 'may be empty', f.nullable === false && 'required']
        .filter(Boolean).map((b) => `<span class="tag">${b}</span>`).join(' ');
      const type = f.declared_type
        ? `${esc(f.data_type)} <span class="bad">· the model says ${esc(f.declared_type)}</span>`
        : esc(f.data_type || '—');
      return `<p class="dm-crumb">${goto(t.name)} ›</p><h3>${esc(f.name)} ${seenTag(f.seen)}</h3>`
        + `<p>${badges}</p>`
        + `<dl class="kv"><dt>type</dt><dd><code>${type}</code></dd>${f.default ? `<dt>default</dt><dd><code>${esc(f.default)}</code></dd>` : ''}</dl>`
        + `<h4>Purpose</h4><p>${f.purpose ? esc(f.purpose) : '<span class="muted">No purpose written yet.</span>'}</p>`
        + `<h4>Links</h4>${out.length || into.length ? '<ul class="dm-links">'
          + out.map((l) => `<li>→ ${goto(l.to_table, l.to_field)}${linkWords(l)}</li>`).join('')
          + into.map((l) => `<li>← ${goto(l.from_table, l.from_field)}${linkWords(l)}</li>`).join('') + '</ul>' : '<p class="muted">None.</p>'}`
        + `<h4>Technical rules</h4>${list(f.technical)}`
        + `<h4>Business rules</h4>${list(f.business)}`
        + notesOn(`field:${t.name}.${f.name}`);
    }
    function inspect() {
      const m = st.model;
      if (!m) return;
      const t = st.sel && m.tables.find((x) => x.name === st.sel.table);
      const f = t && st.sel.field && t.fields.find((x) => x.name === st.sel.field);
      inspector.innerHTML = f ? inspectField(m, t, f) : t ? inspectTable(m, t) : overview(m);
    }
    // Selects a table or a field, unfolding its table, and shows it.
    function select(table, field, center) {
      st.sel = table ? { table, field: field || null } : null;
      if (field && !st.open.has(table)) { st.open.add(table); remember(st.open); }
      draw();
      if (center && table) {
        const el = board.content.querySelector(`[data-pin="${CSS.escape(field ? `field:${table}.${field}` : `table:${table}`)}"]`);
        board.focus(el, 0.8);
      }
    }

    // ---- events ------------------------------------------------------------------
    board.content.addEventListener('click', (e) => {
      const row = e.target.closest('.dm-field');
      const box = e.target.closest('.dm-table');
      if (row && box) return select(box.dataset.table, row.dataset.field, false);
      if (box && e.target.closest('.dm-head')) {
        const name = box.dataset.table;
        const wasSelected = st.sel && st.sel.table === name && !st.sel.field;
        // A first click selects and unfolds; a click on the selected head folds it.
        if (st.open.has(name) && wasSelected) st.open.delete(name); else st.open.add(name);
        remember(st.open);
        return select(name, null, false);
      }
      if (!box) { st.sel = null; draw(); }
      return undefined;
    });
    const onClick = (e) => {
      const go = e.target.closest('[data-goto]');
      if (go) return select(go.dataset.goto, go.dataset.gotoField || null, true);
      const note = e.target.closest('[data-open-note]');
      if (note) return board.open(note.dataset.openNote);
      const act = e.target.closest('[data-dm]');
      if (!act || !st.model) return undefined;
      if (act.dataset.dm === 'all') {
        const all = st.model.tables.every((t) => st.open.has(t.name));
        st.open = all ? new Set() : new Set(st.model.tables.map((t) => t.name));
        remember(st.open);
        draw();
        board.fit();
      }
      if (act.dataset.dm === 'divergences') { st.sel = null; draw(); }
      return undefined;
    };
    host.addEventListener('click', onClick);
    search.addEventListener('input', markSearch);
    search.addEventListener('keydown', (e) => {
      e.stopPropagation(); // the page's Escape would leave the pane
      if (e.key === 'Escape') { search.value = ''; markSearch(); }
      if (e.key === 'Enter' && st.model) {
        const hit = board.content.querySelector('.dm-table.match');
        if (hit) select(hit.dataset.table, null, true);
      }
    });

    load();
    return {
      destroy() {
        st.gone = true;
        clearTimeout(st.timer);
        board.destroy();
        host.innerHTML = '';
      },
    };
  }

  window.DataModelPane = { mount };
})();
