// The plant's page: data, navigation, panes, the steward's terminal — and a
// bridge to the renderer, a Bevy scene compiled to WebAssembly that owns the
// canvas (crates/view-render).
//
//   A  the factory from outside       B  inside: six rooms
//   C  one room                        D  a pane over three quarters of the screen
//
// The renderer draws; this file decides. Every pointer event the scene sees
// comes back here as `{kind, hot}` and is acted on exactly as before: open a
// pane, enter a level, open GitHub, show a tooltip.
(() => {
  'use strict';

  const $ = (id) => document.getElementById(id);

  const S = {
    snap: null,
    level: 'A', room: null, pane: null,
    render: null,          // the wasm module, once it loaded
    renderReady: false,
    logTimer: null,
    term: null,
  };

  // ---- helpers -------------------------------------------------------------
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const usd = (v) => '$' + Number(v || 0).toFixed(2);
  const kfmt = (n) => (n >= 1e6 ? (n / 1e6).toFixed(1) + 'M' : n >= 1e3 ? (n / 1e3).toFixed(0) + 'k' : String(n || 0));
  // A run's consumed tokens: the total, then where they went. Its ledger
  // rows land as each stage ends, so a run still in its first stage has none.
  const fmtTokens = (t) => (t
    ? `<b>${kfmt(t.total)}</b> · in ${kfmt(t.input)} · out ${kfmt(t.output)} · cache ${kfmt(t.cache_read)} read / ${kfmt(t.cache_write)} written · ${t.stages} stage${t.stages > 1 ? 's' : ''} done`
    : '— (first stage still running)');
  const hhmm = (iso) => (iso ? iso.slice(11, 19) + 'Z' : '—');
  function fmtAge(secs) {
    if (secs == null) return '—';
    if (secs < 60) return secs + 's';
    if (secs < 3600) return Math.floor(secs / 60) + 'm';
    if (secs < 86400) return Math.floor(secs / 3600) + 'h ' + Math.floor((secs % 3600) / 60) + 'm';
    return Math.floor(secs / 86400) + 'd';
  }
  const roomByKey = (key) => (S.snap ? S.snap.rooms.find((r) => r.key === key) : null);

  // ---- the renderer --------------------------------------------------------
  // `window.harnessRender.onEvent` is what the scene calls back. Defined
  // before the module loads, so the first event finds it.
  window.harnessRender = {
    onEvent(json) {
      let ev;
      try { ev = JSON.parse(json); } catch (e) { return; }
      if (ev.kind === 'ready') { S.renderReady = true; pushPicture(); pushView(); return; }
      if (ev.kind === 'click') return act(ev.hot || {});
      if (ev.kind === 'hover') return showTip(ev.tip, ev.x, ev.y);
      if (ev.kind === 'leave') return hideTip();
      return undefined;
    },
  };
  function pushPicture() {
    if (S.render && S.renderReady && S.snap) S.render.setSnapshot(JSON.stringify(S.snap));
  }
  function pushView() {
    if (!S.render || !S.renderReady) return;
    const view = S.level === 'C' ? { level: 'C', room: S.room || 'construction' } : { level: S.level };
    S.render.setView(JSON.stringify(view));
  }
  async function loadRenderer() {
    const notice = $('render-notice');
    try {
      const mod = await import('/render/render.js');
      await mod.default();
      S.render = mod;
      mod.start('#scene');
      if (notice) notice.hidden = true;
    } catch (e) {
      if (notice) {
        notice.hidden = false;
        notice.innerHTML = `<b>The renderer is not built.</b><br>Run <code>scripts/build-render.sh</code> (needs wasm-pack and the wasm32 target), then reload.<br><span class="muted">${esc(e && e.message ? e.message : e)}</span>`;
      }
    }
  }

  // ---- tooltip ---------------------------------------------------------------
  function showTip(text, x, y) {
    const tip = $('tooltip');
    if (!text) return hideTip();
    tip.hidden = false;
    tip.textContent = text;
    const stage = $('stage').getBoundingClientRect();
    tip.style.left = Math.min(x + 16, stage.width - tip.offsetWidth - 8) + 'px';
    tip.style.top = Math.min(y + 16, stage.height - tip.offsetHeight - 8) + 'px';
    return undefined;
  }
  function hideTip() { $('tooltip').hidden = true; }

  // ---- data ----------------------------------------------------------------
  async function load() {
    try {
      const r = await fetch('/api/snapshot');
      S.snap = await r.json();
      onSnapshot();
    } catch (e) {
      $('watch-text').textContent = 'cannot reach the server';
    }
  }
  function subscribe() {
    const es = new EventSource('/api/events');
    es.addEventListener('snapshot', (e) => { S.snap = JSON.parse(e.data); onSnapshot(); });
    es.onerror = () => { $('watch-dot').className = 'dot warn'; $('watch-text').textContent = 'reconnecting…'; };
  }
  function onSnapshot() {
    status();
    crewBar();
    crumbs();
    pushPicture();
    // An issue is fetched once; the steward's terminal is a live connection.
    // Everything else is redrawn from the new picture.
    if (S.pane && S.pane.kind !== 'issue' && S.pane.kind !== 'steward') renderPane(S.pane, true);
    // `?open={"kind":"board"}` opens a pane straight from the URL — a deep
    // link to a station, an employee, the dashboards.
    const wanted = new URLSearchParams(location.search).get('open');
    if (wanted && !S.openedFromUrl) {
      S.openedFromUrl = true;
      try { openPane(JSON.parse(wanted)); } catch (e) { /* not a pane */ }
    }
  }
  // The agents at work, as faces down the left edge of the plant: a glance
  // says who is on what, a click opens their pane.
  const FACES = ['🧑‍🔧', '👷', '🧑‍💻', '🧑‍🔬', '🧑‍🚀', '🧑‍🏭'];
  function faceOf(id) {
    let h = 0;
    for (const c of id) h = (h * 31 + c.charCodeAt(0)) >>> 0;
    return FACES[h % FACES.length];
  }
  // The issue an agent works, from its name (`#64 · Dev loop`), or null.
  const issueOf = (e) => { const m = /^#(\d+)\b/.exec(e.name || ''); return m ? Number(m[1]) : null; };
  // An issue's title, from the board the snapshot carries, or ''.
  function issueTitle(n) {
    const b = S.snap.board || {};
    const all = [...(b.roadmap || []), ...(b.needs_human || []), ...(b.milestones || []).flatMap((m) => [m.issue, ...(m.tasks || [])])];
    const hit = all.find((i) => i && i.number === n);
    return hit ? hit.title : '';
  }
  function crewBar() {
    const bar = $('crew-bar');
    if (!bar) return;
    const at = S.snap.employees.filter((e) => e.active);
    bar.hidden = !at.length;
    // Several agents on one issue are one bubble, `#N - <title>`: a click
    // lists them first, then opens the one picked.
    const groups = [];
    for (const e of at) {
      const n = issueOf(e);
      const g = n != null && groups.find((x) => x.n === n);
      if (g) g.agents.push(e); else groups.push({ n, agents: [e] });
    }
    const html = groups.map(({ n, agents }) => {
      const e = agents[0];
      const ring = e.model ? ` ${esc(e.model)}` : '';
      if (agents.length === 1) {
        const open = esc(JSON.stringify({ kind: 'employee', id: e.id }));
        const tip = esc(`${e.name}${e.stage ? ' — ' + e.stage : ''}`);
        return `<button class="crew-bubble" data-crew='${open}' title="${tip}"><span class="face${ring}">${faceOf(e.id)}</span><span class="who"><b>${esc(e.name)}</b><small>${esc(e.stage || 'starting')}</small></span></button>`;
      }
      const title = `#${n} - ${issueTitle(n) || agents.map((a) => a.name.replace(/^#\d+ · /, '')).join(', ')}`;
      const open = esc(JSON.stringify({ kind: 'agents', number: n }));
      return `<button class="crew-bubble" data-crew='${open}' title="${esc(title)}"><span class="face${ring}">${faceOf(e.id)}<span class="count">${agents.length}</span></span><span class="who"><b>${esc(title)}</b><small>${agents.length} agents · ${esc(agents.map((a) => a.stage || 'starting').join(', '))}</small></span></button>`;
    }).join('');
    if (html !== S.crewHtml) { bar.innerHTML = html; S.crewHtml = html; }
  }
  $('crew-bar').addEventListener('click', (ev) => {
    const bubble = ev.target.closest('[data-crew]');
    if (bubble) openPane(JSON.parse(bubble.dataset.crew));
  });

  function status() {
    const f = S.snap.factory;
    $('watch-dot').className = 'dot ' + (f.watching ? 'on' : 'off');
    let s = f.watching ? 'watch polling' : 'watch off';
    if (f.last_tick_at) s += ` · last tick ${hhmm(f.last_tick_at)}`;
    if (f.in_flight) s += ` · running ${f.in_flight.workflow}${f.in_flight.subject ? ' (' + f.in_flight.subject + ')' : ''}`;
    else if (f.idle) s += ' · nobody at work';
    else s += ` · ${S.snap.employees.length} at work`;
    if (S.snap.demo) s += ' · DEMO';
    $('watch-text').textContent = s;
  }

  // ---- navigation ----------------------------------------------------------
  // Where you are is in the URL hash — `#B`, `#C/lines` — so a place can be
  // bookmarked or opened straight from a link.
  function go(level, room = null) {
    S.level = level; S.room = room;
    closePane();
    hideTip();
    crumbs();
    pushView();
    const hash = level === 'A' ? '' : level === 'B' ? '#B' : `#C/${room}`;
    if (location.hash !== hash) history.replaceState(null, '', location.pathname + location.search + hash);
  }
  function fromHash() {
    const m = /^#(B|C\/([a-z]+))$/.exec(location.hash);
    if (!m) { S.level = 'A'; S.room = null; return; }
    S.level = m[1] === 'B' ? 'B' : 'C';
    S.room = m[2] || null;
  }
  window.addEventListener('hashchange', () => { fromHash(); closePane(); crumbs(); pushView(); });
  function back() {
    if (S.pane) return closePane();
    if (S.level === 'C') return go('B');
    if (S.level === 'B') return go('A');
    return undefined;
  }
  function crumbs() {
    const parts = [{ label: '🏭 ' + (S.snap ? S.snap.project.name : 'factory'), go: 'A' }];
    if (S.level !== 'A') parts.push({ label: 'inside', go: 'B' });
    if (S.level === 'C') {
      const r = roomByKey(S.room);
      parts.push({ label: r ? `room ${r.id} · ${r.name}` : S.room, go: 'C', room: S.room });
    }
    $('crumbs').innerHTML = parts
      .map((p, i) => `${i ? '<span class="sep">›</span>' : ''}<button data-go="${p.go}" data-room="${p.room || ''}" class="${i === parts.length - 1 ? 'here' : ''}">${esc(p.label)}</button>`)
      .join('');
  }
  $('crumbs').addEventListener('click', (e) => {
    const b = e.target.closest('button');
    if (!b || b.classList.contains('here')) return;
    go(b.dataset.go, b.dataset.room || null);
  });
  window.addEventListener('keydown', (e) => {
    // Inside the steward's terminal every key belongs to the terminal.
    if (e.target.closest && e.target.closest('#term')) return;
    if (e.key === 'Escape' || e.key === 'Backspace') { if (e.target.tagName !== 'INPUT' && e.target.tagName !== 'CANVAS') back(); else if (e.key === 'Escape') back(); }
  });
  $('hud-steward').addEventListener('click', () => {
    if (S.pane && S.pane.kind === 'steward') closePane(); else openPane({ kind: 'steward' });
  });
  function act(h) {
    if (h.go) return go(h.go, h.room || null);
    if (h.url) return window.open(h.url, '_blank', 'noopener');
    if (h.pane) return openPane(h.pane);
    return undefined;
  }

  // ---- D · the pane ------------------------------------------------------------
  function openPane(p) {
    clearInterval(S.logTimer); S.logTimer = null;
    teardownTerminal();
    S.pane = Object.assign({}, p);
    document.body.classList.add('split');
    $('pane').hidden = false;
    $('pane-body').classList.toggle('terminal', p.kind === 'steward');
    hideTip();
    renderPane(S.pane, false);
  }
  function closePane() {
    if (!S.pane) return;
    S.pane = null;
    clearInterval(S.logTimer); S.logTimer = null;
    teardownTerminal();
    document.body.classList.remove('split');
    $('pane').hidden = true;
    $('pane-body').classList.remove('terminal');
  }
  $('pane-close').addEventListener('click', closePane);
  $('pane-body').addEventListener('click', (e) => {
    const issue = e.target.closest('[data-issue]');
    if (issue) return openPane({ kind: 'issue', number: Number(issue.dataset.issue) });
    const tab = e.target.closest('[data-file]');
    if (tab && S.pane && S.pane.kind === 'employee') { S.pane.file = tab.dataset.file; renderPane(S.pane, false); return undefined; }
    const machine = e.target.closest('[data-stage-idx]');
    if (machine && S.pane && S.pane.kind === 'employee') {
      // An older machine is pinned; the one the agent is at goes back to
      // following it live.
      const idx = Number(machine.dataset.stageIdx);
      const count = machine.parentElement ? machine.parentElement.querySelectorAll('[data-stage-idx]').length : 0;
      S.pane.file = null; S.pane.stageIdx = idx; S.pane.pinned = idx !== count - 1; S.pane.jump = true;
      renderPane(S.pane, false); return undefined;
    }
    const liveRun = e.target.closest('[data-live-run]');
    if (liveRun && S.pane) { S.pane.liveRun = liveRun.dataset.liveRun; S.pane.jump = true; renderPane(S.pane, false); return undefined; }
    const help = e.target.closest('[data-help]');
    if (help && S.pane) { S.pane.help = !S.pane.help; renderPane(S.pane, false); return undefined; }
    const paneTab = e.target.closest('[data-tab]');
    if (paneTab && S.pane) { S.pane.tab = paneTab.dataset.tab; renderPane(S.pane, false); return undefined; }
    const pane = e.target.closest('[data-pane]');
    if (pane) return openPane(JSON.parse(pane.dataset.pane));
    return undefined;
  });

  // ---- the steward's terminal --------------------------------------------------
  // xterm.js in the pane, a WebSocket to the desk. Closing the pane closes the
  // socket and nothing else: the program behind it keeps running, and the next
  // visit replays its screen.
  const CHIPS = [
    ['Is the plant running?', 'Is the plant running? Give me the status.'],
    ['Start the plant', 'Start the plant.'],
    ['Stop the plant', 'Stop the plant.'],
    ['What is blocking?', 'What is blocking the board right now, and what should I do?'],
    ['What did it cost today?', 'What did the plant spend today, and on what?'],
  ];
  function teardownTerminal() {
    if (!S.term) return;
    try { S.term.ro.disconnect(); } catch (e) { /* already gone */ }
    try { S.term.ws.close(); } catch (e) { /* already gone */ }
    try { S.term.term.dispose(); } catch (e) { /* already gone */ }
    S.term = null;
  }
  function stewardStatus(text) { const el = $('steward-status'); if (el) el.textContent = text; }
  async function mountTerminal() {
    const el = $('term');
    if (!el) return;
    let status = null;
    try { status = await (await fetch('/api/steward')).json(); } catch (e) { /* shown below */ }
    if (!status || !status.available) {
      $('pane-body').classList.remove('terminal');
      $('pane-body').innerHTML = '<div class="callout">No steward: the view runs with <code>--no-steward</code>, or the server is unreachable.</div>';
      return;
    }
    if (typeof Terminal === 'undefined' || typeof FitAddon === 'undefined') {
      el.textContent = 'xterm.js did not load';
      return;
    }
    const term = new Terminal({
      cursorBlink: true, fontSize: 13, lineHeight: 1.15, scrollback: 5000,
      fontFamily: 'ui-monospace, Menlo, "SF Mono", Consolas, monospace',
      theme: { background: '#070b10', foreground: '#e6edf3', cursor: '#5ad1e6', selectionBackground: 'rgba(90,209,230,0.3)', black: '#111821', brightBlack: '#4b5563' },
    });
    const fit = new FitAddon.FitAddon();
    term.loadAddon(fit);
    term.open(el);
    try { fit.fit(); } catch (e) { /* not laid out yet */ }
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${location.host}/api/steward/term?cols=${term.cols}&rows=${term.rows}`);
    ws.binaryType = 'arraybuffer';
    const enc = new TextEncoder();
    const send = (bytes) => { if (ws.readyState === 1) ws.send(bytes); };
    stewardStatus(`connecting · ${status.command}`);
    ws.onopen = () => { stewardStatus(`at the desk · ${status.command}`); term.focus(); };
    ws.onmessage = (ev) => { term.write(typeof ev.data === 'string' ? ev.data : new Uint8Array(ev.data)); };
    ws.onclose = () => stewardStatus('disconnected — reopen the pane to sit down again');
    ws.onerror = () => stewardStatus('the connection failed');
    term.onData((data) => send(enc.encode(data)));
    const ro = new ResizeObserver(() => {
      try { fit.fit(); send(JSON.stringify({ t: 'size', cols: term.cols, rows: term.rows })); } catch (e) { /* mid-layout */ }
    });
    ro.observe(el);
    S.term = { term, ws, ro };
    const restart = $('term-restart');
    if (restart) restart.onclick = () => {
      if (ws.readyState === 1 && window.confirm('Kill the steward\'s Claude Code and start a fresh one?')) {
        term.reset();
        send(JSON.stringify({ t: 'restart' }));
      }
    };
    // A chip types its sentence, then presses Enter a beat later — the way a
    // hand would, so the program sees a line and not a paste.
    $('pane-body').querySelectorAll('[data-type]').forEach((b) => {
      b.onclick = () => {
        send(enc.encode(b.dataset.type));
        setTimeout(() => { send(enc.encode('\r')); term.focus(); }, 180);
      };
    });
  }

  function renderPane(p, refresh) {
    const fn = PANES[p.kind] || PANES.placeholder;
    const [title, html, after, keep] = fn(p, refresh);
    $('pane-title').textContent = title;
    // A pane with a live log keeps its body on a refresh: its `after` updates
    // the parts that changed, and the log keeps its scroll.
    // An unchanged body is not rewritten either: a refresh that replaces the
    // buttons under the pointer swallows the click being made on them.
    if (!(refresh && (p.kind === 'employee' || keep)) && !(refresh && html === S.paneHtml)) {
      $('pane-body').innerHTML = html;
    }
    S.paneHtml = html;
    if (after) after(refresh);
  }

  const kv = (pairs) => '<dl class="kv">' + pairs.filter(([, v]) => v != null && v !== '').map(([k, v]) => `<dt>${esc(k)}</dt><dd>${v}</dd>`).join('') + '</dl>';
  const tag = (text, cls = '') => `<span class="tag ${cls}">${esc(text)}</span>`;
  // A pane's two tabs — what the element is, and who recently worked on it —
  // and its `?`, which says in a sentence what the element is for.
  const paneTabs = (p, purpose) => '<div class="tabs">' + [['details', 'Details'], ['recent', 'Recent work']].map(([k, label]) => `<button data-tab="${k}" class="${(p.tab || 'details') === k ? 'on' : ''}">${label}</button>`).join('')
    + (purpose ? `<button class="help-btn ${p.help ? 'on' : ''}" data-help title="What is this for?">?</button>` : '') + '</div>'
    + (purpose && p.help ? `<div class="callout purpose">${esc(purpose)}</div>` : '');
  // The runs of a line, newest first, each a link to its agent — opened on
  // `stage` when the list comes from a machine.
  function recentWork(line, runs, stage) {
    if (!runs.length) return '<div class="muted">nobody has worked here yet</div>';
    return '<ul class="issues">' + runs.map((r) => {
      const last = { id: `${line.id}/${r.run_id}`, name: r.name, workflow: line.id, run_id: r.run_id, tokens: r.tokens, active: r.active };
      const open = { kind: 'employee', id: last.id, last, stage: stage || null };
      const when = r.run_id.slice(9, 11) + ':' + r.run_id.slice(11, 13) + ' · ' + r.run_id.slice(4, 6) + '/' + r.run_id.slice(6, 8);
      return `<li><span class="t"><button class="link" data-pane='${esc(JSON.stringify(open))}'>${esc(r.name)}</button></span><span class="muted">${esc(when)} · ${esc(r.stages.join(' → ') || 'no stage finished')}${r.tokens ? ' · ' + kfmt(r.tokens.total) + ' tok' : ''}</span>${r.active ? tag('at work', 'ok') : ''}</li>`;
    }).join('') + '</ul>';
  }
  const ext = (url, text = 'GitHub ↗') => (url ? `<a class="ext" href="${esc(url)}" target="_blank" rel="noopener">${esc(text)}</a>` : '');
  function issueList(items) {
    if (!items.length) return '<div class="muted">nothing</div>';
    return '<ul class="issues">' + items.map((t) => `<li><span class="n">#${t.number}</span><span class="t"><button class="link" data-issue="${t.number}">${esc(t.title)}</button></span><span class="st ${t.status}">${t.status}</span>${t.url ? `<a class="ext" href="${esc(t.url)}" target="_blank" rel="noopener" title="GitHub">↗</a>` : ''}</li>`).join('') + '</ul>';
  }
  function bars(buckets, cls = '', fmt = (b) => `${usd(b.usd)} · ${b.count}`) {
    if (!buckets.length) return '<div class="muted">no rows</div>';
    const max = Math.max(...buckets.map((b) => b.usd), 0.0001);
    return '<div class="bars">' + buckets.map((b) => `<div class="k" title="${esc(b.key)}">${esc(b.key)}</div><div class="bar ${cls}"><span style="width:${Math.max(1, Math.round(100 * b.usd / max))}%"></span></div><div class="v">${fmt(b)}</div>`).join('') + '</div>';
  }
  const tiles = (items) => '<div class="tiles">' + items.map(([v, l]) => `<div class="tile"><div class="v">${v}</div><div class="l">${esc(l)}</div></div>`).join('') + '</div>';
  const KIND_TIP = {
    scanner: 'gate · a deterministic check',
    builder: 'LLM that writes',
    inspector: 'LLM that reads and judges',
    printer: 'gathers the context',
    arm: 'acts on the code or on GitHub',
  };

  const PANES = {
    board() {
      const b = S.snap.board;
      if (!b) return ['Board', '<div class="callout">No GitHub board yet: the view runs with <code>--no-board</code>, or <code>gh</code> has not answered.</div>'];
      let h = '';
      if (b.roadmap.length) h += '<h3>Roadmap</h3>' + issueList(b.roadmap);
      if (b.needs_human.length) h += '<h3>Waiting on you</h3>' + issueList(b.needs_human);
      for (const m of b.milestones) {
        const pct = m.total ? Math.round((100 * m.done) / m.total) : 0;
        h += `<h3>${m.issue.state === 'closed' ? '✓ ' : ''}<button class="link" data-issue="${m.issue.number}">#${m.issue.number} ${esc(m.issue.title)}</button> <span class="muted">· ${m.done}/${m.total}</span></h3>`;
        h += `<div class="progress"><span style="width:${pct}%"></span></div>`;
        h += `<div class="muted" style="margin:4px 0 8px;font:11px var(--mono)">${esc(m.branch)} ${m.branch_url ? '· ' + ext(m.branch_url, 'branch ↗') : ''}</div>`;
        for (const [st, label] of [['ready', 'Ready — next up'], ['todo', 'To do'], ['blocked', 'Blocked'], ['human', 'A human must act'], ['delivered', 'Delivered, waiting for the merge'], ['done', 'Done']]) {
          const items = m.tasks.filter((t) => t.status === st);
          if (items.length) h += `<div class="muted" style="font-size:12px;margin-top:8px">${label}</div>` + issueList(items);
        }
      }
      return ['Board · ' + S.snap.project.name, h];
    },

    employee(p, refresh) {
      const e = S.snap.employees.find((x) => x.id === p.id) || p.last;
      if (!e) return ['Employee', '<div class="callout">This employee has clocked out — the run is over.</div>'];
      p.last = e;
      const head = kv([
        ['line', esc(e.workflow)], ['run', esc(e.run_id)],
        ['tokens', fmtTokens(e.tokens)],
        ['stage', `${esc(e.stage || '—')} ${e.model ? tag(e.model, e.model) : ''}`],
        ['task', esc(e.task || '—')], ['milestone', esc(e.milestone || '—')], ['round', esc(e.round || '—')],
        ['since', e.since ? `${hhmm(e.since)} · last write ${fmtAge(e.age_secs)} ago` : null],
        ['status', e.active ? tag('at work', 'ok') : tag('clocked out', '')],
      ]);
      // The logs are read machine by machine: one button per stage the run
      // went through (its session in session.log), then the run's own files.
      const html = `<div id="emp-head">${head}</div><h3>Machines it went through</h3><div class="tabs" id="emp-machines"><span class="muted">loading…</span></div><pre class="log" id="live-log">loading…</pre>`;
      const after = (isRefresh) => {
        if (isRefresh) { const h = $('emp-head'); if (h) h.innerHTML = head; return; }
        const pull = async () => {
          try {
            const r = await fetch(`/api/runs/${e.workflow}/${e.run_id}/stages`);
            const stages = r.ok ? await r.json() : [];
            const pre = $('live-log');
            const bar = $('emp-machines');
            if (!pre || !bar) return;
            // A run that stopped at its gates opened no session: its run.log
            // is all there is.
            if (!stages.length && !p.file) p.file = 'run.log';
            // Follow the machine the agent is at — the last session — unless
            // a machine was picked: by a click, or by the Recent work entry
            // this pane was opened from.
            const last = stages.length - 1;
            if (p.stage && !p.pinned) {
              const wanted = stages.map((x) => x.stage).lastIndexOf(p.stage);
              if (wanted >= 0 && wanted !== last) { p.stageIdx = wanted; p.pinned = true; }
              p.stage = null;
            }
            if (!p.file && !(p.pinned && p.stageIdx != null && p.stageIdx <= last)) {
              if (p.stageIdx !== last) p.jump = true;
              p.stageIdx = last;
              p.pinned = false;
            }
            const seen = {};
            const labels = stages.map((x) => { seen[x.stage] = (seen[x.stage] || 0) + 1; return seen[x.stage] > 1 ? `${x.stage} (${seen[x.stage]})` : x.stage; });
            const barHtml = (stages.length ? '' : '<span class="muted">no machine reached — this run stopped before it opened a session; its run.log says why</span> ')
              + labels.map((label, i) => `<button data-stage-idx="${i}" class="${!p.file && i === p.stageIdx ? 'on' : ''}" title="${e.active && i === stages.length - 1 ? 'where the agent is now — followed live' : 'a machine it went through'}">${e.active && i === stages.length - 1 ? '● ' : ''}${esc(label)}</button>`).join('')
              + ['run.log', 'prompts.md'].map((f) => `<button data-file="${f}" class="muted ${f === p.file ? 'on' : ''}">${f}</button>`).join('');
            // Rewritten only when it changed: a button replaced between the
            // press and the release of a click swallows that click.
            if (barHtml !== p.barHtml) { bar.innerHTML = barHtml; p.barHtml = barHtml; }
            let text;
            if (p.file) {
              const f = await fetch(`/api/runs/${e.workflow}/${e.run_id}/${p.file}?bytes=14000`);
              text = f.ok ? (await f.text()) || '(empty)' : `(${f.status}) ${await f.text()}`;
            } else {
              text = (stages[p.stageIdx] && stages[p.stageIdx].text) || '(empty)';
            }
            const atBottom = pre.scrollHeight - pre.scrollTop - pre.clientHeight < 40;
            if (pre.textContent !== text) {
              pre.textContent = text;
              if (atBottom || p.jump) pre.scrollTop = pre.scrollHeight;
            }
            p.jump = false;
          } catch (err) { /* the next pull will say */ }
        };
        p.jump = true;
        p.barHtml = null;
        pull();
        clearInterval(S.logTimer);
        S.logTimer = setInterval(pull, 2000);
      };
      return [e.name, html, after];
    },

    // The agents at work on one issue: pick one, then its usual pane.
    agents(p) {
      const here = S.snap.employees.filter((e) => e.active && issueOf(e) === p.number);
      const title = `#${p.number} - ${issueTitle(p.number) || 'issue'}`;
      if (!here.length) return [title, '<div class="callout">Nobody is at work on this issue any more.</div>'];
      const items = here.map((e) => `<li><span class="t"><button class="link" data-pane='${esc(JSON.stringify({ kind: 'employee', id: e.id }))}'>${esc(e.name)}</button></span><span class="muted">${esc(e.stage || 'starting')} ${e.model ? tag(e.model, e.model) : ''} · ${fmtAge(e.age_secs)} ago</span></li>`).join('');
      return [title, `<p class="muted">${here.length} agents work this issue. Pick the one to follow:</p><ul class="issues">${items}</ul>`];
    },

    crew(p) {
      const here = S.snap.employees.filter((e) => e.workflow === p.line && (e.station || null) === (p.station || null));
      if (!here.length) return ['Crew', '<div class="callout">Nobody is at this station any more.</div>'];
      const items = here.map((e) => `<li><span class="t"><button class="link" data-pane='${esc(JSON.stringify({ kind: 'employee', id: e.id }))}'>${esc(e.name)}</button></span><span class="muted">${esc(e.stage || '')} ${e.model ? tag(e.model, e.model) : ''} · ${fmtAge(e.age_secs)} ago</span></li>`).join('');
      return [`${here.length} at work · ${esc(here[0].stage || p.station)}`, `<p class="muted">Several agents stand at this station. Pick the one to follow:</p><ul class="issues">${items}</ul>`];
    },

    station(p) {
      const line = S.snap.lines.find((l) => l.id === p.line);
      const st = line && line.stations.find((s) => s.id === p.id);
      if (!st) return ['Station', '<div class="callout">Unknown station.</div>'];
      if (p.tab === 'recent') {
        // A machine's work: the runs that finished its stage, or stand at it.
        const here = (line.recent_work || []).filter((r) => (st.stage && r.stages.includes(st.stage)) || S.snap.employees.some((e) => e.workflow === line.id && e.run_id === r.run_id && e.station === st.id) || !st.stage);
        return [st.label, paneTabs(p, st.purpose) + recentWork(line, here, st.stage)];
      }
      const bucket = st.stage ? S.snap.costs.by_stage.find((b) => b.key === st.stage) : null;
      const rows = st.stage ? S.snap.costs.last.filter((r) => r.stage === st.stage) : [];
      let h = kv([
        ['line', esc(line.title)], ['kind', `${esc(st.kind)} — ${esc(KIND_TIP[st.kind])}`],
        ['stage', st.stage ? esc(st.stage) : '<span class="muted">a gate of the stage beside it</span>'],
        ['model', st.model ? tag(st.model, st.model) : null],
        ['state', tag(st.state, st.state === 'active' ? 'acc' : st.state === 'done' ? 'ok' : '')],
      ]);
      if (bucket) h += '<h3>What this stage cost, all runs</h3>' + tiles([[usd(bucket.usd), 'total'], [bucket.count, 'sessions'], [usd(bucket.usd / Math.max(1, bucket.count)), 'per session']]);
      if (rows.length) {
        h += '<h3>Last sessions on it</h3><table class="rows"><tr><th>when</th><th>task</th><th>round</th><th class="num">cost</th><th class="num">turns</th><th>outcome</th></tr>' + rows.map((r) => `<tr><td>${esc(r.when)}</td><td>#${esc(r.task)}</td><td>${esc(r.round)}</td><td class="num">${r.cost_usd == null ? '—' : usd(r.cost_usd)}</td><td class="num">${r.turns ?? '—'}</td><td>${tag(r.outcome || '?', r.outcome === 'ok' ? 'ok' : r.outcome ? 'bad' : '')}</td></tr>`).join('') + '</table>';
      }
      if (st.kind === 'scanner') h += '<div class="callout">A gate judges and never writes: it reads the issue, the labels or the ledger, and either lets the product through, skips the stage, or halts the round.</div>';
      return [st.label, paneTabs(p, st.purpose) + h];
    },

    line(p) {
      const line = S.snap.lines.find((l) => l.id === p.id);
      if (!line) return ['Line', ''];
      if (p.tab === 'recent') {
        clearInterval(S.logTimer); S.logTimer = null; p.liveCount = 0;
        return [line.title, paneTabs(p, line.purpose) + recentWork(line, line.recent_work || [], null)];
      }
      // Who is at work on this line right now, each one's current machine
      // tailed live — the line's own window on what is happening.
      const live = S.snap.employees.filter((e) => e.workflow === line.id && e.active);
      if (!live.some((e) => e.id === p.liveRun)) p.liveRun = live.length ? live[0].id : null;
      const liveBar = live.map((e) => `<button data-live-run="${esc(e.id)}" class="${e.id === p.liveRun ? 'on' : ''}">${esc(e.name)}${e.stage ? ' · ' + esc(e.stage) : ''}</button>`).join('');
      const lr = line.last_run;
      let h = kv([['trigger', esc(line.trigger)], ['log folder', `<code>.llocal/logs/${esc(line.id)}/</code>`], ['runs', line.runs], ['status', line.active ? tag('at work', 'ok') : tag('idle')]]);
      if (lr) {
        h += '<h3>Latest run</h3>' + kv([['run', esc(lr.run_id)], ['started', hhmm(lr.started_at)], ['last line at', `${hhmm(lr.last_at)} · ${fmtAge(lr.age_secs)} ago`], ['task', esc(lr.task || '—')], ['round', esc(lr.round || '—')], ['last line', `<code>${esc(lr.last_line)}</code>`], ['warning', lr.warning ? `<span class="tag warn">${esc(lr.warning)}</span>` : null]]);
        const emp = S.snap.employees.find((e) => e.workflow === line.id);
        h += `<button class="link" data-pane='${esc(JSON.stringify({ kind: 'employee', id: `${line.id}/${lr.run_id}`, last: emp || null }))}'>open its logs →</button>`;
      }
      h += '<h3>Stations</h3><ul class="issues">' + line.stations.map((s) => `<li><span class="n">${esc(s.kind)}</span><span class="t"><button class="link" data-pane='${esc(JSON.stringify({ kind: 'station', line: line.id, id: s.id }))}'>${esc(s.label)}</button></span>${s.model ? tag(s.model, s.model) : ''}<span class="st ${s.state === 'done' ? 'done' : s.state === 'active' ? 'ready' : 'todo'}">${s.state}</span></li>`).join('') + '</ul>';
      const head = paneTabs(p, line.purpose);
      if (!live.length) { clearInterval(S.logTimer); S.logTimer = null; p.liveCount = 0; return [line.title, head + h]; }
      const html = `${head}<h3>Live log</h3><div class="tabs" id="line-live-bar">${liveBar}</div><div class="muted" id="line-live-where"></div><pre class="log" id="line-live-log">loading…</pre><div id="line-main">${h}</div>`;
      // Same agents on refresh: update around the log. A different count is a
      // different layout, redrawn whole.
      const keep = refresh && p.liveCount === live.length && !!$('line-live-log');
      p.liveCount = live.length;
      const after = (isRefresh) => {
        if (isRefresh && keep) {
          const main = $('line-main'); if (main && p.mainHtml !== h) { main.innerHTML = h; p.mainHtml = h; }
          const bar = $('line-live-bar'); if (bar && p.liveBarHtml !== liveBar) { bar.innerHTML = liveBar; p.liveBarHtml = liveBar; }
          return;
        }
        p.mainHtml = h; p.liveBarHtml = liveBar;
        const pull = async () => {
          const e = S.snap.employees.find((x) => x.id === p.liveRun);
          const pre = $('line-live-log');
          const where = $('line-live-where');
          if (!e || !pre) return;
          try {
            const r = await fetch(`/api/runs/${e.workflow}/${e.run_id}/stages`);
            const stages = r.ok ? await r.json() : [];
            const current = stages[stages.length - 1];
            let text;
            if (current) {
              text = current.text || '(empty)';
              if (where) where.textContent = `${e.name} — machine: ${current.stage}`;
            } else {
              const f = await fetch(`/api/runs/${e.workflow}/${e.run_id}/run.log?bytes=14000`);
              text = f.ok ? (await f.text()) || '(empty)' : '(no log yet)';
              if (where) where.textContent = `${e.name} — no machine reached yet, its run.log`;
            }
            const atBottom = pre.scrollHeight - pre.scrollTop - pre.clientHeight < 40;
            if (pre.textContent !== text) {
              pre.textContent = text;
              if (atBottom || p.jump) pre.scrollTop = pre.scrollHeight;
            }
            p.jump = false;
          } catch (err) { /* the next pull will say */ }
        };
        p.jump = true;
        pull();
        clearInterval(S.logTimer);
        S.logTimer = setInterval(pull, 2000);
      };
      return [line.title, html, after, keep];
    },

    chimney(p) {
      const ch = S.snap.factory.chimneys.find((c) => c.model === p.model);
      if (!ch) return ['Chimney', ''];
      const on = S.snap.employees.filter((e) => e.active && e.model === ch.model);
      const stages = S.snap.lines.flatMap((l) => l.stations.filter((s) => s.model === ch.model).map((s) => `${l.title} · ${s.stage}`));
      let h = kv([['model', tag(ch.model, ch.model)], ['state', ch.smoking ? tag(`smoking · ${ch.runs} open`, 'ok') : tag('cold')], ['spent', `${usd(ch.usd)} <span class="muted">— on the stages that open this model by default; the ledger has no model column</span>`]]);
      h += '<h3>At work on it</h3>' + (on.length ? '<ul class="issues">' + on.map((e) => `<li><span class="t"><button class="link" data-pane='${esc(JSON.stringify({ kind: 'employee', id: e.id }))}'>${esc(e.name)}</button></span><span class="muted">${esc(e.stage || '')}</span></li>`).join('') + '</ul>' : '<div class="muted">nobody</div>');
      h += '<h3>Stations on this model</h3>' + (stages.length ? '<ul class="issues">' + stages.map((s) => `<li><span class="t">${esc(s)}</span></li>`).join('') + '</ul>' : '<div class="muted">none by default</div>');
      return [`${ch.model} chimney`, h];
    },

    issue(p) {
      const html = `<div id="issue-body"><div class="muted">loading #${p.number}…</div></div>`;
      const after = async () => {
        const el = $('issue-body');
        try {
          const r = await fetch(`/api/issues/${p.number}`);
          if (!r.ok) { el.innerHTML = `<div class="callout">${esc(await r.text())}</div>`; return; }
          const i = await r.json();
          $('pane-title').textContent = `#${i.number} ${i.title}`;
          el.innerHTML = kv([['state', tag(i.state, i.state === 'open' ? 'ok' : '')], ['labels', i.labels.map((l) => tag(l.replace('harness:', ''), l.endsWith('ready') ? 'warn' : l.endsWith('human') || l.endsWith('needs-decision') ? 'bad' : '')).join('')], ['blocked by', i.blocked_by.length ? i.blocked_by.map((b) => `<button class="link" data-issue="${b.number}">#${b.number} ${esc(b.title)}</button> ${tag(b.state, b.state === 'closed' ? 'ok' : 'warn')}`).join('<br>') : null], ['on GitHub', ext(i.url)]])
            + '<div class="callout soon">Discuss this issue with Claude Code from here — coming. The office is read-only for now.</div>'
            + `<h3>Body</h3><pre class="md">${esc(i.body || '(empty)')}</pre>`;
        } catch (err) { el.innerHTML = '<div class="callout">cannot reach the server</div>'; }
      };
      return [`#${p.number}`, html, after];
    },

    features() {
      const b = S.snap.board;
      if (!b) return ['Features', '<div class="callout">No GitHub board: the feature list is the milestones, and they were not read.</div>'];
      let h = '<div class="callout">A feature is a milestone: delivered once its branch merged into the agents\' version. Its tasks are the feature\'s pieces.</div>';
      for (const m of b.milestones) {
        h += `<h3>${m.issue.state === 'closed' ? '✓ delivered · ' : m.done === m.total && m.total ? '◐ waiting for the merge · ' : '○ in progress · '}<button class="link" data-issue="${m.issue.number}">#${m.issue.number} ${esc(m.issue.title)}</button></h3>` + issueList(m.tasks);
      }
      return ['Features · ' + S.snap.project.name, h];
    },

    dashboards(p) {
      const c = S.snap.costs, q = S.snap.quota;
      let h = tiles([[usd(c.total_usd), 'spent, all runs'], [c.sessions, 'paid sessions'], [kfmt(c.tokens.output), 'output tokens'], [kfmt(c.tokens.cache_read), 'cache read'], [kfmt(c.tokens.cache_write), 'cache write'], [kfmt(c.tokens.input), 'uncached input']]);
      // The data layer runs on the strongest model: what it costs against the rest.
      const dl = (c.by_side || []).find((b) => b.key === 'data-layer');
      if (dl) h += tiles([[usd(dl.usd), 'data layer (strongest model)'], [c.total_usd > 0 ? Math.round(100 * dl.usd / c.total_usd) + '%' : '—', 'of all spending'], [dl.count, 'sessions']]);
      if ((c.by_side || []).length) h += '<h3>By side of the architecture</h3>' + bars(c.by_side);
      h += '<h3 id="dash-costs">By day</h3>' + bars(c.by_day.slice(-14));
      h += '<h3>By stage</h3>' + bars(c.by_stage);
      h += '<h3>By task</h3>' + bars(c.by_task);
      h += '<h3>By outcome</h3>' + bars(c.by_outcome.map((b) => ({ ...b })), '', (b) => `${usd(b.usd)} · ${b.count}`);
      h += '<h3 id="dash-quota">Rate-limit windows</h3>';
      if (q) {
        h += '<div class="gauges">' + q.windows.map((w) => { const pct = Math.round(w.utilization * 100); const reset = w.resets_at ? new Date(w.resets_at * 1000).toLocaleString() : 'unknown'; return `<div class="gauge ${pct > 90 ? 'bad' : pct > 70 ? 'warn' : ''}"><div>${esc(w.name)} · ${pct}% used</div><div class="bar"><span style="width:${pct}%"></span></div><div class="muted">resets ${esc(reset)}</div></div>`; }).join('') + '</div>' + `<div class="muted" style="margin-top:6px;font:11px var(--mono)">read ${new Date(q.at * 1000).toLocaleString()}</div>`;
      } else h += '<div class="muted">no reading yet</div>';
      h += '<h3 id="dash-errors">Last stops</h3>';
      h += S.snap.errors.length ? '<table class="rows"><tr><th>when</th><th>workflow</th><th>kind</th><th>reason</th></tr>' + S.snap.errors.map((e) => `<tr><td>${esc(e.when)}</td><td>${esc(e.workflow)}</td><td>${tag(e.kind, e.kind === 'QUOTA' ? 'warn' : e.kind === 'FAILED' ? 'bad' : '')}</td><td>${esc(e.reason.slice(0, 240))}${e.reason.length > 240 ? '…' : ''}</td></tr>`).join('') + '</table>' : '<div class="muted">none recorded</div>';
      h += '<h3 id="dash-journal">Watch journal</h3><pre class="log">' + esc(S.snap.recent.join('\n') || '(empty)') + '</pre>';
      h += '<h3>Last paid sessions</h3><table class="rows"><tr><th>when</th><th>task</th><th>stage</th><th class="num">cost</th><th class="num">turns</th><th class="num">min</th><th>outcome</th></tr>' + c.last.map((r) => `<tr><td>${esc(r.when)}</td><td>#${esc(r.task)}</td><td>${esc(r.stage)}</td><td class="num">${r.cost_usd == null ? '—' : usd(r.cost_usd)}</td><td class="num">${r.turns ?? '—'}</td><td class="num">${r.duration_ms == null ? '—' : (r.duration_ms / 60000).toFixed(1)}</td><td>${tag(r.outcome || '?', r.outcome === 'ok' ? 'ok' : r.outcome ? 'bad' : '')}</td></tr>`).join('') + '</table>';
      const after = (refresh) => { if (!refresh && p.focus) { const el = $('dash-' + p.focus); if (el) el.scrollIntoView({ block: 'start' }); } };
      return ['Control room', h, after];
    },

    versions() {
      const v = S.snap.versions;
      let h = kv([['repository', ext(v.repo_url, v.repo_url || '—')]]);
      h += '<h3>On sale</h3><ul class="issues">';
      h += `<li><span class="n">main</span><span class="t">${esc(v.main.label)}</span>${ext(v.main.url, 'open ↗')}</li>`;
      h += `<li><span class="n">${esc(v.integration.name)}</span><span class="t">${esc(v.integration.label)}</span>${ext(v.integration.url, 'open ↗')}</li>`;
      h += '</ul><h3>In development — one branch per open milestone</h3>';
      h += v.milestones.length ? '<ul class="issues">' + v.milestones.map((m) => `<li><span class="n">${esc(m.name.replace(/^milestone\/(\d+).*$/, '#$1'))}</span><span class="t">${esc(m.label)}<div class="muted" style="font:11px var(--mono)">${esc(m.name)}</div></span>${ext(m.url, 'open ↗')}</li>`).join('') + '</ul>' : '<div class="muted">none open</div>';
      return ['Distribution', h];
    },

    steward() {
      const html = `<div class="steward-bar"><span id="steward-status">connecting…</span><span class="spacer"></span><button class="mini" id="term-restart" title="kill this Claude Code and start a fresh one">restart</button></div>`
        + '<div class="chips">' + CHIPS.map(([label, say]) => `<button data-type="${esc(say)}">${esc(label)}</button>`).join('') + '</div>'
        + '<div id="term"></div>';
      return ['The steward · Claude Code in the harness checkout', html, () => { mountTerminal(); }];
    },

    placeholder(p) {
      return [p.title || 'Coming', `<div class="callout soon">${esc(p.text || 'Not built yet.')}</div>`];
    },
  };

  // ---- go --------------------------------------------------------------------
  fromHash();
  crumbs();
  loadRenderer();
  load().then(subscribe);
})();
