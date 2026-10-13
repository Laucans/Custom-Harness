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

  function mount(host, opts) {
    host.innerHTML = '<div class="dm">'
      + '<div class="dm-switch" hidden><div class="dm-branches" role="group" aria-label="branch"></div>'
      + '<div class="dm-features" hidden role="menu" aria-label="feature branches"></div>'
      + '<label class="dm-version">version <select aria-label="version"></select></label>'
      + '<span class="dm-compare-wrap" hidden><button type="button" class="dm-compare" data-dm="compare" aria-haspopup="menu" title="Hold this version against another branch or version">compare ▾</button>'
      + '<div class="dm-compares" hidden role="menu" aria-label="compare with"></div></span></div>'
      + '<div class="dm-bar"><span class="dm-origin">reading the data model…</span><span class="spacer"></span>'
      + '<input type="search" class="dm-search" placeholder="Find a table or a field" aria-label="find a table or a field">'
      + '<button type="button" class="mini" data-dm="all">unfold all</button></div>'
      + '<div class="dm-main"><div class="dm-board"></div><aside class="dm-inspector" aria-live="polite"></aside></div>'
      + '</div>';
    const q = (s) => host.querySelector(s);
    const inspector = q('.dm-inspector');
    const search = q('.dm-search');
    const st = { model: null, body: '', open: remembered(), sel: null, boxes: new Map(), bounds: null, timer: null, fitted: false, gone: false,
      // Which version is shown: a branch and a folder of the archive; empty, the server's default.
      versions: null, branch: (opts && opts.branch) || '', dir: (opts && opts.dir) || '', newest: !(opts && opts.dir),
      // A model read from elsewhere than the product (the creation's draft): no versions, no builds, no usage.
      source: (opts && opts.source) || '',
      // A build the page started: polled until it ends; a model's asks a second click.
      fresh: false, building: null, armed: null, buildTimer: null, said: '',
      // Who uses each table and field: the version it was read for, and when.
      usage: { key: '', at: 0, by: {}, error: null },
      // Compared with another branch or version: what to hold it against,
      // what differs (the server's reply), and the model as drawn — the
      // shown one with what only the other side has ghosted in.
      compare: null, diff: null, drawn: null };

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
    // What a pin's AI session is told of what the pin sits on: the field or
    // the table as this version shows it.
    const context = (target) => {
      const m = st.model;
      if (!m || !target) return '';
      const name = target.slice(target.indexOf(':') + 1);
      const users = usersOf(target).map((u) => `  - ${u.id} (${u.how}, ${u.via})`).join('\n');
      const change = st.compare && st.diff && changeOf(target);
      const differs = change ? `\nCompared with ${otherLabel()}: ${DIFF_WORDS[change.change]}${change.notes.length ? ' — ' + change.notes.join(' · ') : ''}` : '';
      const version = `Shown: ${m.origin}.${differs}\nUsed by:\n${users || '  no capability'}`;
      if (target.startsWith('table:')) {
        const t = m.tables.find((x) => x.name === name);
        if (!t) return version;
        const fields = t.fields.map((f) => `  - ${f.name} ${f.data_type}${f.purpose ? ' — ' + f.purpose : ''}`).join('\n');
        return `${version}\nTable ${t.name}: ${t.purpose || '(no purpose written)'}\nBusiness rules: ${t.business.join(' · ') || 'none'}\nTechnical rules: ${t.technical.join(' · ') || 'none'}\nFields:\n${fields}`;
      }
      const cut = name.lastIndexOf('.');
      const t = m.tables.find((x) => x.name === name.slice(0, cut));
      const f = t && t.fields.find((x) => x.name === name.slice(cut + 1));
      if (!f) return version;
      const links = m.links
        .filter((l) => (l.from_table === t.name && l.from_field === f.name) || (l.to_table === t.name && l.to_field === f.name))
        .map((l) => `  - ${l.from_table}.${l.from_field} → ${l.to_table}.${l.to_field} (${l.kind}${l.on_delete ? ', on delete ' + l.on_delete : ''})`)
        .join('\n');
      return `${version}\nField ${t.name}.${f.name}, type ${f.data_type}${f.declared_type ? ` (the model says ${f.declared_type})` : ''}, ${f.nullable === false ? 'required' : 'may be empty'}.\n`
        + `Purpose: ${f.purpose || '(none written)'}\nTechnical rules: ${f.technical.join(' · ') || 'none'}\nBusiness rules: ${f.business.join(' · ') || 'none'}\nLinks:\n${links || '  none'}\nIts table, ${t.name}: ${t.purpose || '(no purpose written)'}`;
    };
    // A pin belongs to one branch: each branch has its own board.
    // `milestone/3-…` → `data-model--milestone-3`, `main_agent` → `data-model--main-agent`.
    const boardOf = (branch) => {
      const m = /^milestone\/(\d+)/.exec(branch);
      const slug = m ? `milestone-${m[1]}` : branch.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
      return `data-model--${slug || 'live'}`.slice(0, 40);
    };
    const onSession = opts && opts.onSession
      ? (note, ctx) => opts.onSession({ board: board.board(), id: note.id, on: describe(note.target), kind: note.kind, context: ctx, branch: st.branch, dir: st.newest ? '' : st.dir })
      : null;
    // A pin asked for from elsewhere (the office's pin board) is opened once its board is read.
    const onLoaded = () => {
      if (opts && opts.pin && !st.pinned && board.goTo(opts.pin)) st.pinned = true;
    };
    const board = window.HarnessBoard.create(q('.dm-board'), { id: null, describe, resolve, context, onSession, onLoaded, bounds: () => st.bounds });

    // ---- reading -------------------------------------------------------------------
    async function load() {
      clearTimeout(st.timer);
      if (st.gone) return;
      let reply = null;
      try {
        const [versions, model, diff] = st.source ? [null, await fetch(st.source).then((r) => (r.ok ? r.json() : null)), null] : await Promise.all([
          fetch('/api/data-model/versions').then((r) => (r.ok ? r.json() : null)),
          fetch('/api/data-model?' + new URLSearchParams({ branch: st.branch, dir: st.newest ? '' : st.dir, fresh: st.fresh ? '1' : '' })).then((r) => (r.ok ? r.json() : null)),
          st.compare
            ? fetch('/api/data-model/compare?' + new URLSearchParams({ branch: st.branch, dir: st.newest ? '' : st.dir, with_branch: st.compare.branch, with_dir: st.compare.dir || '' })).then((r) => (r.ok ? r.json() : null))
            : Promise.resolve(null),
        ]);
        st.versions = versions;
        st.fresh = false;
        st.diff = st.compare ? diff : null;
        reply = model;
      } catch (e) { reply = null; }
      if (st.gone) return;
      if (!reply) { say('The view did not answer.'); st.timer = setTimeout(load, 3000); return; }
      if (!reply.enabled) { say('Nothing reads the product\'s data model: the view runs with <code>--no-board</code> and no <code>--data-dir</code>.'); return; }
      if (!reply.model) { say(esc(reply.error || 'Reading the data model…')); drawSwitch(); st.timer = setTimeout(load, 2000); return; }
      st.branch = reply.branch;
      st.dir = reply.dir;
      board.setBoard(boardOf(st.branch || ''));
      readUsage();
      drawSwitch();
      const body = JSON.stringify(reply.model) + '\n' + JSON.stringify(st.diff);
      if (body !== st.body) {
        st.body = body;
        st.model = reply.model;
        draw();
      }
      st.timer = setTimeout(load, REFRESH_MS);
    }
    // The archive's versions: one button per kept branch kind, the features
    // in a menu, the versions of the branch shown in a list.
    function drawSwitch() {
      const v = st.versions;
      const sw = q('.dm-switch');
      sw.hidden = !(v && v.branches.length);
      if (sw.hidden) return;
      const of = (kind) => v.branches.filter((b) => b.kind === kind);
      const current = v.branches.find((b) => b.branch === st.branch);
      const button = (b, label, tip) => `<button type="button" class="${current && current.branch === b.branch ? 'on' : ''}" data-branch="${esc(b.branch)}" title="${esc(tip)}">${esc(label)}</button>`;
      const features = of('feature');
      const featureOn = current && current.kind === 'feature';
      q('.dm-branches').innerHTML = of('release').map((b) => button(b, b.label, 'what a human released')).join('')
        + of('integration').map((b) => button(b, b.label, 'where the agents integrate')).join('')
        + `<button type="button" class="${featureOn ? 'on' : ''}" data-dm="features" ${features.length ? '' : 'disabled'} aria-haspopup="menu" title="a milestone's branch">${featureOn ? esc(current.label) : 'feature'} ▾</button>`;
      q('.dm-features').innerHTML = features.map((b) => `<button type="button" role="menuitem" data-branch="${esc(b.branch)}">${esc(b.label)}<small>${b.versions.length ? `${b.versions.length} version${b.versions.length > 1 ? 's' : ''}` : 'live, not archived'}</small></button>`).join('');
      const when = (at) => window.HarnessClock.stamp(at).slice(0, 16);
      q('.dm-version').hidden = !(current && current.versions.length);
      q('.dm-version select').innerHTML = (current ? current.versions : []).map((x, i) => `<option value="${esc(x.dir)}"${x.dir === st.dir ? ' selected' : ''}>${esc(when(x.at))} · ${esc(x.sha.slice(0, 7))}${i === 0 ? ' · newest' : ''}</option>`).join('');
      drawCompare(v);
    }
    // The compare menu: every other branch at its newest, and the older
    // versions of the branch shown — what changed since.
    function drawCompare(v) {
      const when = (at) => window.HarnessClock.stamp(at).slice(0, 16);
      q('.dm-compare-wrap').hidden = false;
      const on = st.compare;
      const button = q('.dm-compare');
      button.classList.toggle('on', !!on);
      button.textContent = on ? `vs ${otherLabel()} ▾` : 'compare ▾';
      const items = [];
      if (on) items.push('<button type="button" role="menuitem" data-compare-off>× stop comparing</button>');
      for (const b of v.branches) {
        const newest = b.versions[0];
        const shownHere = b.branch === st.branch && (st.newest || (newest && newest.dir === st.dir));
        if (!shownHere) items.push(`<button type="button" role="menuitem" data-compare-branch="${esc(b.branch)}" data-compare-dir="">${esc(b.label)}<small>${newest ? esc(when(newest.at)) : 'live'}</small></button>`);
        if (b.branch !== st.branch) continue;
        for (const x of b.versions) {
          if (x.dir === st.dir || (st.newest && x === newest)) continue;
          items.push(`<button type="button" role="menuitem" data-compare-branch="${esc(b.branch)}" data-compare-dir="${esc(x.dir)}">${esc(b.label)} · ${esc(when(x.at))}<small>${esc(x.sha.slice(0, 7))}</small></button>`);
        }
      }
      q('.dm-compares').innerHTML = items.length ? items.join('') : '<div class="hb-empty">Nothing else to compare with.</div>';
    }
    // The other side of a comparison, in words: `main`, `#3 campaigns · 2026-10-09 17:52`.
    function otherLabel() {
      const on = st.compare;
      if (!on) return '';
      const b = st.versions && st.versions.branches.find((x) => x.branch === on.branch);
      const ver = on.dir && b && b.versions.find((x) => x.dir === on.dir);
      return (b ? b.label : on.branch) + (ver ? ' · ' + window.HarnessClock.stamp(ver.at).slice(0, 16) : '');
    }
    function compareWith(branch, dir) {
      st.compare = branch ? { branch, dir: dir || '' } : null;
      st.diff = null;
      st.sel = null;
      q('.dm-compares').hidden = true;
      load();
    }
    // One action per file and branch: what its log button is found by.
    const buildKey = (what) => `build:${what}:${(st.building && st.building.what === what && st.building.branch) || st.branch || 'integration'}`;
    // Starts the build of a missing file for the branch shown, then follows it.
    async function build(what) {
      if (what === 'model' && st.armed !== 'model') {
        st.armed = 'model';
        setTimeout(() => { if (st.armed === 'model') { st.armed = null; if (st.model) drawBar(st.model); } }, 5000);
        if (st.model) drawBar(st.model);
        return;
      }
      st.armed = null;
      st.said = '';
      const file = what === 'schema' ? 'schema.sql' : 'model.json';
      const action = window.HarnessActions
        ? window.HarnessActions.start({ key: buildKey(what), label: `build ${file} for ${st.branch || 'the integration branch'}`, button: host.querySelector(`[data-build="${what}"]`) })
        : null;
      st.lastKey = buildKey(what);
      try {
        const res = await fetch('/api/data-model/build', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ what, branch: st.branch }) });
        const text = await res.text();
        if (!res.ok) throw new Error(text);
        const started = JSON.parse(text);
        st.building = { what, branch: started.branch, id: started.id, action };
        if (action) {
          action.note(`started: harness ${what === 'schema' ? 'data-snapshot' : 'data-model-draft'} --branch ${started.branch}`);
          action.log = () => fetch(`/api/data-model/builds/${encodeURIComponent(started.id)}/log`).then((r) => (r.ok ? r.text() : ''));
        }
      } catch (e) {
        st.said = 'not started: ' + (e.message || e);
        if (action) action.fail(e.message || e);
      }
      if (st.model) drawBar(st.model);
      follow();
    }
    async function follow() {
      clearTimeout(st.buildTimer);
      if (st.gone || !st.building) return;
      let all = [];
      try { all = await (await fetch('/api/data-model/builds')).json(); } catch (e) { /* asked again */ }
      const b = all.find((x) => x.what === st.building.what && x.branch === st.building.branch);
      if (b && b.state === 'running') { st.buildTimer = setTimeout(follow, 3000); return; }
      const last = b ? b.tail.split('\n').pop() : '';
      st.said = !b ? '' : b.state === 'done'
        ? (st.building.what === 'schema' ? `schema of ${b.branch} built` : `model drafted: ${last}`)
        : `build failed: ${last}`;
      const action = st.building.action;
      if (action && b) { if (b.state === 'done') action.ok(st.said); else action.fail(last || 'the build failed'); }
      st.building = null;
      st.fresh = true;
      st.body = '';
      load();
    }
    function show(branch, dir) {
      st.branch = branch;
      st.dir = dir || '';
      st.newest = !dir;
      st.sel = null;
      q('.dm-features').hidden = true;
      if (st.compare && st.compare.branch === branch && (st.compare.dir || '') === (dir || '')) st.compare = null;
      load();
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
    // What is drawn: the model shown — and, compared, what only the other
    // side has ghosted in, every part marked with what changed.
    function drawn() {
      const m = st.model;
      const d = st.diff && st.diff.comparison;
      if (!m || !d) { st.drawn = m; return m; }
      const marked = (x, change, notes, gone) => Object.assign({}, x, { diff: change, notes: notes || [], gone: !!gone });
      const changes = new Map(d.tables.map((c) => [c.name, c]));
      const tables = m.tables.map((t) => {
        const c = changes.get(t.name);
        if (!c) return t;
        const fields = new Map(c.fields.map((f) => [f.name, f]));
        const own = t.fields.map((f) => (fields.has(f.name) ? marked(f, fields.get(f.name).change, fields.get(f.name).notes) : f));
        const gone = c.fields.filter((f) => f.change === 'removed' && f.was).map((f) => marked(f.was, 'removed', [], true));
        return Object.assign(marked(t, c.change, c.notes), { fields: own.concat(gone) });
      });
      for (const c of d.tables) {
        if (c.change === 'removed' && c.was) tables.push(Object.assign(marked(c.was, 'removed', [], true), { fields: c.was.fields.map((f) => marked(f, 'removed', [], true)) }));
      }
      const ends = (l) => `${l.from_table}.${l.from_field}>${l.to_table}.${l.to_field}`;
      const linkChanges = new Map(d.links.map((c) => [ends(c.link), c]));
      const links = m.links.map((l) => { const c = linkChanges.get(ends(l)); return c ? marked(l, c.change, c.notes) : l; });
      for (const c of d.links) if (c.change === 'removed') links.push(marked(c.link, 'removed', [], true));
      st.drawn = Object.assign({}, m, { tables, links });
      return st.drawn;
    }
    function draw() {
      const m = drawn();
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
        const what = s.path.endsWith('schema.sql') ? 'schema' : 'model';
        // A missing file is built on a click: a snapshot for the schema, a
        // draft proposed as a pull request for the model.
        if (!s.found && !s.problem && st.versions && st.versions.branches.length) {
          const running = st.building && st.building.what === what;
          const armed = st.armed === what;
          const label = running ? `building ${s.path}…` : armed ? `${s.path}: click again — Opus drafts it, a PR to merge` : `${s.path} · build it`;
          const hint = what === 'schema'
            ? 'Dump this branch now (the Docker Postgres, its migrations) and archive it on harness/data-model'
            : 'Opus reads the schema and the code, writes data/model.json and opens a pull request on this branch';
          const log = window.HarnessActions ? window.HarnessActions.chip(buildKey(what)) : '';
          return `<button type="button" class="dm-src build${armed ? ' armed' : ''}" data-build="${what}" ${running ? 'aria-disabled="true"' : ''} title="${esc(hint)}">${esc(label)}${log}</button>`;
        }
        return `<span class="dm-src ${cls}" title="${esc(tip)}">${esc(s.path)}</span>`;
      }).join('');
      const div = m.compared
        ? (m.divergences.length ? `<button type="button" class="dm-div bad" data-dm="divergences">${m.divergences.length} divergence${m.divergences.length > 1 ? 's' : ''}</button>` : '<span class="dm-div ok">in agreement</span>')
        : '';
      const lastLog = st.lastKey && window.HarnessActions ? window.HarnessActions.chip(st.lastKey) : '';
      let cmp = '';
      if (st.compare) {
        const d = st.diff && st.diff.comparison;
        const count = d ? d.tables.length + d.links.length + d.views.length : 0;
        const words = !st.diff ? 'comparing…' : st.diff.error ? 'not compared' : count ? `${count} difference${count > 1 ? 's' : ''}` : 'the same';
        cmp = `<button type="button" class="dm-div ${!st.diff ? '' : st.diff.error || count ? 'bad' : 'ok'}" data-dm="compare-summary" title="What this version changes, compared with ${esc(otherLabel())}">vs ${esc(otherLabel())} · ${words}</button>`;
      }
      say(`<b>${esc(m.origin)}</b>${src}${div}${cmp}${st.said ? `<span class="dm-built">${esc(st.said)}${lastLog}</span>` : ''}`);
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
        return `<li class="dm-field seen-${f.seen}${fsel ? ' sel' : ''}${f.diff ? ' diff-' + f.diff : ''}" data-pin="field:${esc(t.name)}.${esc(f.name)}" data-field="${esc(f.name)}" title="${esc(f.notes && f.notes.length ? f.notes.join(' · ') : f.gone ? 'only in the other version' : f.purpose || f.name)}">`
          + `${keyOf(f)}<span class="dm-name">${esc(f.name)}${f.nullable === true ? '<i>?</i>' : ''}</span><span class="dm-type">${esc(f.data_type)}</span>${warn}</li>`;
      }).join('') + '</ul>' : '';
      const mark = t.diff ? `<span class="dm-diff ${t.diff}" title="${esc(DIFF_WORDS[t.diff])}">${DIFF_MARKS[t.diff]}</span>` : '';
      return `<div class="dm-table seen-${t.seen}${open ? ' open' : ''}${sel && !st.sel.field ? ' sel' : ''}${t.diff ? ' diff-' + t.diff : ''}" data-pin="table:${esc(t.name)}" data-table="${esc(t.name)}" style="left:${b.x}px;top:${b.y}px;width:${b.w}px">`
        + `<div class="dm-head" title="${esc(t.gone ? 'only in the other version' : t.purpose || 'click to show its fields')}"><span class="dm-caret">${open ? '▾' : '▸'}</span><b>${esc(t.name)}</b>${seenTag(t.seen)}${mark}<span class="dm-count">${t.fields.length}</span></div>`
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
        const cls = ['dm-link', l.kind, 'seen-' + l.seen, hl ? 'hl' : '', l.diff ? 'diff-' + l.diff : ''].join(' ');
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

    // ---- who uses what --------------------------------------------------------------
    const USAGE_MS = 60000;
    // Read again for another version, or once a minute: manifests move.
    async function readUsage() {
      if (st.source) return;
      const key = `${st.branch}|${st.newest ? '' : st.dir}`;
      if (key === st.usage.key && Date.now() - st.usage.at < USAGE_MS) return;
      st.usage.key = key;
      st.usage.at = Date.now();
      try {
        const res = await fetch('/api/data-model/used-by?' + new URLSearchParams({ branch: st.branch, dir: st.newest ? '' : st.dir }));
        const reply = res.ok ? await res.json() : { used_by: {}, error: await res.text() };
        if (st.gone || st.usage.key !== key) return;
        st.usage.by = reply.used_by || {};
        st.usage.error = reply.error || null;
      } catch (e) {
        st.usage.error = String(e.message || e);
      }
      if (st.sel) inspect();
    }
    const usersOf = (target) => st.usage.by[target] || [];
    function usedBy(target) {
      const users = usersOf(target);
      const body = users.length
        ? `<ul class="dm-used">${users.map((u) => `<li><span class="tag ${esc(u.how)}">${u.how === 'writes' ? 'writes' : 'reads'}</span><b>${esc(u.id)}</b>`
          + `<small class="muted">${esc(u.kind === 'data_capability' ? 'data capability' : 'capability')} · ${esc(u.via)}</small></li>`).join('')}</ul>`
        : `<p class="muted">${st.usage.error ? esc(st.usage.error) : st.usage.at ? 'No capability uses it on this branch.' : 'Reading the manifests…'}</p>`;
      return `<h4>Used by <span class="muted">· ${users.length}</span></h4>${body}`;
    }

    // ---- compared with another version ----------------------------------------------
    const DIFF_WORDS = { added: 'here only', removed: 'there only', changed: 'differs' };
    const DIFF_MARKS = { added: '+', removed: '−', changed: '~' };
    const diffTag = (change) => `<span class="tag diff-${change}">${DIFF_WORDS[change]}</span>`;
    // What a table or a field is, compared: the change found for it.
    function changeOf(target) {
      const d = st.diff && st.diff.comparison;
      if (!d) return null;
      const name = target.slice(target.indexOf(':') + 1);
      if (target.startsWith('table:')) return d.tables.find((t) => t.name === name) || null;
      const cut = name.lastIndexOf('.');
      const t = d.tables.find((x) => x.name === name.slice(0, cut));
      return (t && t.fields.find((f) => f.name === name.slice(cut + 1))) || null;
    }
    function compared(target) {
      if (!st.compare) return '';
      const head = `<h4>Compared with <span class="muted">${esc(otherLabel())}</span></h4>`;
      if (!st.diff) return head + '<p class="muted">Comparing…</p>';
      if (st.diff.error) return head + `<p class="bad">${esc(st.diff.error)}</p>`;
      const c = changeOf(target);
      if (!c) return head + '<p class="ok">The same there.</p>';
      const fields = c.fields && c.fields.length
        ? `<ul class="dm-flist">${c.fields.map((f) => `<li>${diffTag(f.change)}<button type="button" class="dm-go" data-goto="${esc(c.name)}" data-goto-field="${esc(f.name)}">${esc(f.name)}</button>${f.notes.length ? `<small class="muted">${esc(f.notes.join(' · '))}</small>` : ''}</li>`).join('')}</ul>`
        : '';
      return head + `<p>${diffTag(c.change)}</p>${c.notes.length ? list(c.notes) : ''}${fields}`;
    }
    function comparedOverview() {
      const head = `<h4>Compared with <span class="muted">${esc(otherLabel())}</span></h4>`;
      if (!st.diff) return head + '<p class="muted">Comparing…</p>';
      if (st.diff.error) return head + `<p class="bad">${esc(st.diff.error)}</p>`;
      const d = st.diff.comparison;
      if (!d.tables.length && !d.links.length && !d.views.length) return head + '<p class="ok">The same data model.</p>';
      let h = head + '<p class="muted"><b>here only</b>: this version has it, the other has not. <b>there only</b>: the other has it, drawn ghosted. <b>differs</b>: on both, not the same.</p>';
      h += `<ul class="dm-flist">${d.tables.map((t) => {
        const bits = [];
        if (t.fields.length) bits.push(`${t.fields.length} field${t.fields.length > 1 ? 's' : ''}`);
        bits.push(...t.notes);
        return `<li>${diffTag(t.change)}<button type="button" class="dm-go" data-goto="${esc(t.name)}">${esc(t.name)}</button>${bits.length ? `<small class="muted">${esc(bits.join(' · '))}</small>` : ''}</li>`;
      }).join('')}</ul>`;
      if (d.links.length) h += `<h4>Links</h4><ul class="dm-links">${d.links.map((l) => `<li>${diffTag(l.change)} ${goto(l.link.from_table, l.link.from_field)} → ${goto(l.link.to_table, l.link.to_field)}${l.notes.length ? ` <small class="muted">${esc(l.notes.join(' · '))}</small>` : ''}</li>`).join('')}</ul>`;
      if (d.views.length) h += `<h4>Views</h4>${list(d.views.map((v) => `${DIFF_WORDS[v.change]}: ${v.name}`))}`;
      return h;
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
      if (st.compare) h += comparedOverview();
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
        + compared('table:' + t.name)
        + usedBy('table:' + t.name)
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
        + compared(`field:${t.name}.${f.name}`)
        + usedBy(`field:${t.name}.${f.name}`)
        + notesOn(`field:${t.name}.${f.name}`);
    }
    function inspect() {
      const m = st.drawn || st.model;
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
      if (act.dataset.dm === 'features') q('.dm-features').hidden = !q('.dm-features').hidden;
      if (act.dataset.dm === 'compare') q('.dm-compares').hidden = !q('.dm-compares').hidden;
      if (act.dataset.dm === 'compare-summary') { st.sel = null; draw(); }
      return undefined;
    };
    const onBuild = (e) => {
      const b = e.target.closest('[data-build]');
      // A running build's button stays clickable for its log button only.
      if (b && b.getAttribute('aria-disabled') !== 'true' && !st.building) build(b.dataset.build);
      return undefined;
    };
    q('.dm-switch').addEventListener('click', (e) => {
      const b = e.target.closest('[data-branch]');
      if (b) show(b.dataset.branch, '');
      const c = e.target.closest('[data-compare-branch]');
      if (c) compareWith(c.dataset.compareBranch, c.dataset.compareDir);
      if (e.target.closest('[data-compare-off]')) compareWith(null);
    });
    q('.dm-version select').addEventListener('change', (e) => {
      const first = e.target.options[0];
      show(st.branch, first && e.target.value === first.value ? '' : e.target.value);
    });
    host.addEventListener('click', onClick);
    host.addEventListener('click', onBuild);
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
    // A build started before this pane opened is followed too.
    if (!st.source) fetch('/api/data-model/builds').then((r) => r.json()).then((all) => {
      const running = all.find((x) => x.state === 'running');
      if (running) {
        const file = running.what === 'schema' ? 'schema.sql' : 'model.json';
        const action = window.HarnessActions
          ? window.HarnessActions.start({ key: `build:${running.what}:${running.branch}`, label: `build ${file} for ${running.branch}` })
          : null;
        if (action) {
          action.note(`picked up: started ${running.started}`);
          action.log = () => fetch(`/api/data-model/builds/${encodeURIComponent(running.id)}/log`).then((r) => (r.ok ? r.text() : ''));
        }
        st.building = { what: running.what, branch: running.branch, id: running.id, action };
        if (st.model) drawBar(st.model);
        follow();
      }
    }).catch(() => {});
    return {
      reload: () => { st.body = ''; board.reload(); return load(); },
      // What a desk is told of the part under the pointer: a table, a field.
      contextAt(clientX, clientY, el) {
        const part = el && el.closest ? el.closest('[data-pin]') : null;
        const target = part ? part.getAttribute('data-pin') || '' : '';
        return { what: describe(target), text: context(target) };
      },
      destroy() {
        st.gone = true;
        clearTimeout(st.buildTimer);
        clearTimeout(st.timer);
        board.destroy();
        host.innerHTML = '';
      },
    };
  }

  window.DataModelPane = { mount };
})();
