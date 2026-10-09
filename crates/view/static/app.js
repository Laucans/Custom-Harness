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
      if (reloadIfRebuilt(S.snap = await r.json())) return;
      onSnapshot();
    } catch (e) {
      $('watch-text').textContent = 'cannot reach the server';
    }
  }
  // The server was rebuilt and restarted (watchexec): reload, so the page
  // runs the scripts and styles that go with it. The URL hash keeps the place.
  function reloadIfRebuilt(snap) {
    if (!snap.build) return false;
    if (S.build && snap.build !== S.build) { location.reload(); return true; }
    S.build = snap.build;
    return false;
  }
  function subscribe() {
    const es = new EventSource('/api/events');
    es.addEventListener('snapshot', (e) => { S.snap = JSON.parse(e.data); if (!reloadIfRebuilt(S.snap)) onSnapshot(); });
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
    // A gesture's own word wins until the plant shows its outcome.
    const flash = P.flash;
    if (flash && ((flash.until && Date.now() > flash.until) || (flash.doneWhen && flash.doneWhen(f)))) P.flash = null;
    if (P.flash) {
      $('watch-dot').className = 'dot ' + P.flash.tone;
      $('watch-text').textContent = P.flash.text;
      $('status').title = P.flash.detail || P.flash.text;
      if (!$('plant-menu').hidden) plantMenu();
      return;
    }
    $('status').title = 'Start or stop the plant';
    $('watch-dot').className = 'dot ' + (f.draining ? 'warn' : f.watching ? 'on' : 'off');
    let s = f.draining ? 'watch stopping (soft) — running tasks finish' : f.watching ? 'watch polling' : 'watch off';
    if (f.last_tick_at) s += ` · last tick ${hhmm(f.last_tick_at)}`;
    if (f.in_flight) s += ` · running ${f.in_flight.workflow}${f.in_flight.subject ? ' (' + f.in_flight.subject + ')' : ''}`;
    else if (f.idle) s += ' · nobody at work';
    else s += ` · ${S.snap.employees.length} at work`;
    if (S.snap.demo) s += ' · DEMO';
    $('watch-text').textContent = s;
    if (!$('plant-menu').hidden) plantMenu();
  }

  // ---- the plant's switch --------------------------------------------------
  // The status is a button: it opens the gestures that make sense now — start
  // an empty plant, or stop a running one softly (running tasks finish, none
  // starts) or hard (the watch and its lanes are killed, after a second click).
  const P = { status: null, armed: null, busy: false, note: '', bad: false, flash: null };
  // What the status line says while a gesture is under way, and when it lets go.
  const PENDING = {
    start: { text: 'starting the watch…', tone: 'warn pending' },
    soft: { text: 'asking a soft stop…', tone: 'warn pending' },
    hard: { text: 'killing the watch and its lanes…', tone: 'bad pending' },
  };
  function settled(gesture, message) {
    if (gesture === 'start') return { text: message + ' — waiting for its first tick', tone: 'on pending', doneWhen: (f) => f.watching && f.last_tick_at && Date.parse(f.last_tick_at) >= P.since - 2000, until: Date.now() + 90000 };
    if (gesture === 'soft') return { text: 'soft stop asked — waiting for the watch to drain', tone: 'warn pending', doneWhen: (f) => f.draining || !f.watching, until: Date.now() + 120000 };
    return { text: 'hard stop sent', tone: 'off', doneWhen: (f) => !f.watching, until: Date.now() + 15000 };
  }
  async function plantRead() {
    try { P.status = await (await fetch('/api/plant')).json(); }
    catch (e) { P.status = null; P.note = 'cannot reach the server'; P.bad = true; }
  }
  function plantOption(label, hint, gesture, cls = '') {
    return `<button type="button" role="option" data-gesture="${gesture}" class="${cls}"${P.busy ? ' disabled' : ''}>${esc(label)}<small>${esc(hint)}</small></button>`;
  }
  function plantMenu() {
    const st = P.status;
    const f = S.snap ? S.snap.factory : {};
    let html = '';
    if (!st) html = '<div class="note">reading the plant…</div>';
    else if (!st.available) html = '<div class="note">this view has no hand on the plant (demo)</div>';
    else if (!st.running) html = plantOption('▶ Start the plant', 'starts harness watch in this checkout', 'start');
    else {
      if (f.draining) html += '<div class="note">soft stop under way — running tasks finish, then the watch exits</div>';
      else html += plantOption('⏸ Soft stop', 'no new task; stops once the running ones finish', 'soft');
      html += P.armed
        ? plantOption('⏹ Click again to kill everything', 'running sessions are lost and paid again later', 'hard', 'hard armed')
        : plantOption('⏹ Hard stop', `kills the watch (pid ${st.running.pid}) and its lanes now`, 'hard', 'hard');
    }
    if (P.note) html += `<div class="note${P.bad ? ' bad' : ''}">${esc(P.note)}</div>`;
    $('plant-menu').innerHTML = html;
  }
  async function plantOpen() {
    P.note = ''; P.bad = false; P.armed = null;
    if (P.flash && P.flash.sticky) { P.flash = null; status(); }
    $('plant-menu').hidden = false;
    $('status').setAttribute('aria-expanded', 'true');
    plantMenu();
    await plantRead();
    plantMenu();
    const first = $('plant-menu').querySelector('button:not(:disabled)');
    if (first) first.focus();
  }
  function plantClose() {
    $('plant-menu').hidden = true;
    $('status').setAttribute('aria-expanded', 'false');
    if (P.armed) { clearTimeout(P.armed); P.armed = null; }
  }
  async function plantDo(gesture) {
    if (gesture === 'hard' && !P.armed) {
      P.armed = setTimeout(() => { P.armed = null; plantMenu(); }, 4000);
      plantMenu();
      const armed = $('plant-menu').querySelector('.armed');
      if (armed) armed.focus();
      return;
    }
    if (P.armed) { clearTimeout(P.armed); P.armed = null; }
    P.busy = true; P.since = Date.now();
    P.flash = PENDING[gesture];
    status(); plantMenu();
    try {
      const r = await fetch('/api/plant/' + gesture, { method: 'POST' });
      const text = await r.text();
      if (r.ok) {
        P.note = JSON.parse(text).message; P.bad = false;
        P.flash = settled(gesture, P.note);
      } else {
        P.note = text || r.statusText; P.bad = true;
        // A failure stays on the status line until the menu is opened again.
        P.flash = { text: `${gesture} failed — ${P.note.split('\n')[0]}`, detail: P.note, tone: 'bad', sticky: true };
      }
    } catch (e) {
      P.note = 'cannot reach the server'; P.bad = true;
      P.flash = { text: `${gesture} failed — cannot reach the server`, tone: 'bad', sticky: true };
    }
    P.busy = false;
    status();
    if (P.flash && P.flash.until) setTimeout(status, P.flash.until - Date.now() + 50);
    await plantRead();
    plantMenu();
  }
  $('status').addEventListener('click', () => { if ($('plant-menu').hidden) plantOpen(); else plantClose(); });
  $('plant-menu').addEventListener('click', (ev) => {
    const b = ev.target.closest('button[data-gesture]');
    if (b && !b.disabled) plantDo(b.dataset.gesture);
  });
  $('plant-menu').addEventListener('keydown', (ev) => {
    if (ev.key !== 'ArrowDown' && ev.key !== 'ArrowUp') return;
    ev.preventDefault();
    const items = [...$('plant-menu').querySelectorAll('button:not(:disabled)')];
    const at = items.indexOf(document.activeElement);
    const next = items[(at + (ev.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length];
    if (next) next.focus();
  });
  document.addEventListener('click', (ev) => {
    if (!$('plant-menu').hidden && !ev.target.closest('#status-wrap')) plantClose();
  });

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
    // An open switch menu takes Escape for itself, and the page stays put.
    if (e.key === 'Escape' && !$('plant-menu').hidden) { plantClose(); $('status').focus(); return; }
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
  $('pane-body').addEventListener('change', (e) => {
    const field = e.target.closest('[data-range]');
    if (!field) return;
    setRange({ ...(S.range || {}), [field.dataset.range]: toClock(field.value) });
  });
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
    if (e.target.closest('[data-limits-read]') && S.pane && S.pane.kind === 'dashboards') { readLimits(S.pane, true); return undefined; }
    const preset = e.target.closest('[data-range-preset]');
    if (preset) { presetRange(preset.dataset.rangePreset); return undefined; }
    if (e.target.closest('[data-range-clear]')) { setRange(null); return undefined; }
    const help = e.target.closest('[data-help]');
    if (help && S.pane) {
      const key = help.dataset.help;
      if (key) {
        const open = new Set(S.pane.helps || []);
        if (open.has(key)) open.delete(key); else open.add(key);
        S.pane.helps = [...open];
      } else S.pane.help = !S.pane.help;
      renderPane(S.pane, false);
      return undefined;
    }
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

  // ---- the control room's screens -------------------------------------------
  // `14:03:22`, with the day when it is not today — local time.
  function when(iso) {
    if (!iso) return '—';
    const d = new Date(iso);
    const t = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
    return d.toDateString() === new Date().toDateString() ? t : d.toLocaleDateString([], { month: 'short', day: 'numeric' }) + ' ' + t;
  }
  function lasted(from, to) {
    const ms = (to ? Date.parse(to) : Date.now()) - Date.parse(from);
    if (!(ms >= 0)) return '—';
    const m = Math.floor(ms / 60000);
    return m >= 60 ? `${Math.floor(m / 60)}h${String(m % 60).padStart(2, '0')}` : m >= 1 ? `${m} min` : `${Math.round(ms / 1000)} s`;
  }
  // ---- a chosen period --------------------------------------------------------
  // `S.range` holds two trace clocks (UTC, `2026-10-08T21:31:21Z`), either
  // null; `S.history` the figures the server recomputed for it.
  const toClock = (local) => (local ? new Date(local).toISOString().slice(0, 19) + 'Z' : null);
  const toLocal = (clock) => {
    if (!clock) return '';
    const d = new Date(clock), pad = (n) => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
  };
  const rangeKey = () => JSON.stringify(S.range || null);
  // The figures of the chosen period, once read; null when live, or not read yet.
  const period = () => (S.range && S.history && S.history.key === rangeKey() && S.history.data) || null;
  function rangeBar() {
    const r = S.range || {}, h = S.history;
    const preset = (key, label) => `<button class="mini" data-range-preset="${key}">${label}</button>`;
    let bar = `<div class="range-bar"><label>from <input type="datetime-local" data-range="from" value="${toLocal(r.from)}"></label>`
      + `<label>to <input type="datetime-local" data-range="to" value="${toLocal(r.to)}"></label>`
      + preset('1h', 'last hour') + preset('24h', 'last 24 h') + preset('today', 'today')
      + `<button class="mini ${S.range ? '' : 'on'}" data-range-clear>live</button></div>`;
    if (!S.range) return bar;
    const span = `${r.from ? when(r.from) : 'the start'} → ${r.to ? when(r.to) : 'now'}`;
    if (h && h.key === rangeKey() && h.error) bar += `<div class="callout soon">${esc(h.error)}</div>`;
    else if (!period()) bar += `<div class="callout soon">reading ${esc(span)}…</div>`;
    else bar += `<div class="callout soon">Showing ${esc(span)}, recomputed from the whole traces — these figures no longer follow the live plant. <b>live</b> goes back.</div>`;
    return bar;
  }
  async function readHistory() {
    const key = rangeKey();
    S.history = { key, busy: true };
    if (S.pane && S.pane.kind === 'dashboards') renderPane(S.pane, false);
    const q = new URLSearchParams();
    if (S.range.from) q.set('from', S.range.from);
    if (S.range.to) q.set('to', S.range.to);
    let done;
    try {
      const r = await fetch('/api/history?' + q);
      done = r.ok ? { key, data: await r.json() } : { key, error: await r.text() };
    } catch (e) { done = { key, error: 'cannot reach the server' }; }
    if (S.history && S.history.key === key) S.history = done;
    if (S.pane && S.pane.kind === 'dashboards') renderPane(S.pane, false);
  }
  function setRange(range) {
    S.range = range && (range.from || range.to) ? range : null;
    S.history = null;
    if (S.range) readHistory(); else if (S.pane && S.pane.kind === 'dashboards') renderPane(S.pane, false);
  }
  function presetRange(key) {
    const now = new Date();
    const start = key === '1h' ? new Date(now - 3600e3) : key === '24h' ? new Date(now - 86400e3) : new Date(now.getFullYear(), now.getMonth(), now.getDate());
    setRange({ from: start.toISOString().slice(0, 19) + 'Z', to: null });
  }

  async function readLimits(p, force) {
    p.limitsAsked = true; p.limitsBusy = true;
    if (S.pane === p) renderPane(p, false);
    try {
      const r = await fetch('/api/limits' + (force ? '?force=1' : ''));
      if (r.ok) p.limits = await r.json();
    } catch (e) { /* the screen keeps what it had */ }
    p.limitsBusy = false;
    if (S.pane === p) renderPane(p, false);
  }
  // A section's `?`: what it monitors, where the numbers come from. Open ones
  // are kept on the pane, so a refresh of the data does not close them.
  const helpBtn = (p, key) => `<button class="help-btn ${(p.helps || []).includes(key) ? 'on' : ''}" data-help="${esc(key)}" title="What is monitored here?" aria-expanded="${(p.helps || []).includes(key)}">?</button>`;
  const helpBox = (p, key) => ((p.helps || []).includes(key) && HELP[key] ? `<div class="callout purpose">${HELP[key]}</div>` : '');
  const section = (p, key, title) => `<h3 class="with-help">${esc(title)}${helpBtn(p, key)}</h3>` + helpBox(p, key);
  const HELP = {
    costs: 'What the plant has spent, from <code>.llocal/logs/agent-loop/costs.tsv</code>: one row per paid session a run opened (one stage of one task). The dollars are Claude\'s own estimate for each session, not an invoice. The bar above picks a period: <b>from</b> and <b>to</b> (either left empty), or a shortcut; the figures are then recomputed from the whole traces for that period, and <b>live</b> goes back.',
    'costs.totals': '<b>spent</b>: the sum of every session\'s estimate. <b>paid sessions</b>: rows in the ledger. The four token counts add up what every session reported: <b>output</b> written by the model, <b>cache read</b> and <b>cache write</b> of the prompt cache, and <b>uncached input</b>.',
    'costs.data': 'The tasks labelled <code>harness:data-layer</code> run every stage on the strongest model. Their spending, and its share of all spending, says what that choice costs.',
    'costs.side': 'Spending grouped by the side of the architecture the task carries as a label: <code>harness:read-side</code>, <code>harness:write-side</code>, <code>harness:data-layer</code>. A task with none is counted apart.',
    'costs.day': 'Spending per day (UTC), the last fourteen days the ledger holds.',
    'costs.stage': 'Spending per stage of the pipeline — technical refinement, code, create-test, review… — with how many sessions each took.',
    'costs.task': 'Spending per issue: what each task has cost so far, every round and stage together.',
    'costs.outcome': 'How the paid sessions ended: <b>ok</b>, <b>STOP</b> (the session stopped itself, or a question), <b>FAILED</b> (it rendered nothing usable, or was killed), <b>QUOTA</b> (the window was exhausted). Money spent on anything but ok bought nothing.',
    'costs.last': 'The newest sessions of the ledger: when, which task and stage, the estimate, the round-trips with the model, the minutes it took, and how it ended.',
    quota: 'How much of each rate limit is left, read when this screen opens — not remembered. <b>read again</b> reads them once more.',
    'quota.claude': 'Claude\'s subscription windows (<code>five_hour</code>, <code>seven_day</code>, …): the share already used and when each resets. Claude only says it inside a session, so opening this screen runs the smallest session there is — the cheapest model, no tool, about a tenth of a cent — and keeps its reading a minute, so clicks do not pay again. When that probe fails, the reading the runs kept at the end of their last session is shown instead, with its time. Above 70 % orange, above 90 % red.',
    'quota.github': 'The GitHub API buckets of the account <code>gh</code> is logged in with, from <code>gh api rate_limit</code> — a read GitHub does not count. <b>core</b> is the REST API (issues, labels, PRs), <b>graphql</b> the GraphQL API (sub-issues, blockers), <b>search</b> the search API; any other bucket shows once used. An exhausted bucket makes the watch\'s board reads fail until it resets (Stops, and failed board reads on Watch loop).',
    errors: 'Every time a workflow stopped, from <code>.llocal/logs/agent-loop/errors.tsv</code>, newest first. The bar above picks a period: <b>from</b> and <b>to</b> (either left empty), or a shortcut; the figures are then recomputed from the whole traces for that period, and <b>live</b> goes back.',
    'errors.list': '<b>STOP</b>: the run halted for a human — a question in its SPEC, a gate, or the breaker refusing to pay again for a prompt that already failed twice. <b>FAILED</b>: something broke — a command, a timeout, an unreadable board. <b>QUOTA</b>: the subscription window was exhausted. The reason is the run\'s own words.',
    journal: 'The polling loop, <code>harness watch</code>, read from <code>.llocal/logs/agent-loop/watch.log</code> (its last 256 kB). Every tick it reads the board, decides, and starts work on lanes or runs a workflow itself. The bar above picks a period: <b>from</b> and <b>to</b> (either left empty), or a shortcut; the figures are then recomputed from the whole traces for that period, and <b>live</b> goes back.',
    'journal.now': '<b>agents at work</b>: runs whose files moved in the last minutes — what the plant shows walking. <b>lanes open</b>: lanes the journal saw start and not yet finish. <b>ticks</b>: polls since the watch last started. <b>empty ticks</b>: polls that started nothing — the board had not moved, or every runnable task was already on a lane or parked. <b>failed board reads</b>: polls whose GitHub read failed (a rate limit, the network). When the start of the watch is older than the part of the journal read, the counts are a floor.',
    'journal.triggers': 'Everything the loop started, newest first: a <b>lane</b> (a dev loop on a task, a refinement, a split — a separate process, numbered by its lane) or a workflow it ran itself (planner, PR review, merges). <b>running</b>: not reported back yet. <b>ok</b>: exit 0. <b>failed</b>: a non-zero exit, or it could not start. <b>killed</b>: ended by a signal. <b>unknown</b>: the watch restarted before it reported. A task that stops (exit 1) is parked until its issue changes; the logs say so.',
    'journal.period': '<b>triggers</b>: everything that ran at some point of the period — started in it, or started before and still running. <b>failed or killed</b>: those of them that ended badly. <b>ticks</b>, <b>empty ticks</b>, <b>failed board reads</b>: the polls that began in the period. The whole journal is read, so a lane started before the period still shows how it ended.',
    'journal.logs': 'The last forty lines of the journal as written, clock in UTC: polls (<code>saw</code>, <code>quiet</code>), routes (<code>tick</code>), lanes taken and freed, warnings, and a soft stop (<code>draining</code>, <code>stopped</code>).',
  };
  const STATE_TAG = { running: ['running', 'acc'], ok: ['ok', 'ok'], failed: ['failed', 'bad'], killed: ['killed', 'bad'], unknown: ['unknown', 'warn'] };
  const DASH = {
    costs(p) {
      if (S.range && !period()) return '';
      const c = period() ? period().costs : S.snap.costs;
      let h = section(p, 'costs.totals', 'Totals') + tiles([[usd(c.total_usd), 'spent, all runs'], [c.sessions, 'paid sessions'], [kfmt(c.tokens.output), 'output tokens'], [kfmt(c.tokens.cache_read), 'cache read'], [kfmt(c.tokens.cache_write), 'cache write'], [kfmt(c.tokens.input), 'uncached input']]);
      // The data layer runs on the strongest model: what it costs against the rest.
      const dl = (c.by_side || []).find((b) => b.key === 'data-layer');
      if (dl) h += section(p, 'costs.data', 'Data layer') + tiles([[usd(dl.usd), 'data layer (strongest model)'], [c.total_usd > 0 ? Math.round(100 * dl.usd / c.total_usd) + '%' : '—', 'of all spending'], [dl.count, 'sessions']]);
      if ((c.by_side || []).length) h += section(p, 'costs.side', 'By side of the architecture') + bars(c.by_side);
      h += section(p, 'costs.day', 'By day') + bars(c.by_day.slice(-14));
      h += section(p, 'costs.stage', 'By stage') + bars(c.by_stage);
      h += section(p, 'costs.task', 'By task') + bars(c.by_task);
      h += section(p, 'costs.outcome', 'By outcome') + bars(c.by_outcome.map((b) => ({ ...b })), '', (b) => `${usd(b.usd)} · ${b.count}`);
      h += section(p, 'costs.last', 'Last paid sessions') + '<table class="rows"><tr><th>when</th><th>task</th><th>stage</th><th class="num">cost</th><th class="num">turns</th><th class="num">min</th><th>outcome</th></tr>' + c.last.map((r) => `<tr><td>${esc(r.when)}</td><td>#${esc(r.task)}</td><td>${esc(r.stage)}</td><td class="num">${r.cost_usd == null ? '—' : usd(r.cost_usd)}</td><td class="num">${r.turns ?? '—'}</td><td class="num">${r.duration_ms == null ? '—' : (r.duration_ms / 60000).toFixed(1)}</td><td>${tag(r.outcome || '?', r.outcome === 'ok' ? 'ok' : r.outcome ? 'bad' : '')}</td></tr>`).join('') + '</table>';
      return h;
    },
    quota(p) {
      const L = p.limits;
      const at = (secs) => when(new Date(secs * 1000).toISOString());
      const gauge = (name, pct, sub) => `<div class="gauge ${pct > 90 ? 'bad' : pct > 70 ? 'warn' : ''}"><div>${esc(name)} · ${pct}% used</div><div class="bar"><span style="width:${Math.min(pct, 100)}%"></span></div><div class="muted">${sub}</div></div>`;
      const resets = (secs) => (secs ? 'resets ' + esc(new Date(secs * 1000).toLocaleString()) : 'reset unknown');
      let h = `<div class="limits-bar"><span class="muted">${p.limitsBusy ? 'reading the limits now…' : L ? 'read at ' + esc(at(Math.max(L.claude.at, L.github.at))) : ''}</span><button class="mini" data-limits-read${p.limitsBusy ? ' disabled' : ''}>read again</button></div>`;
      // Claude: the probe's reading, or the one the runs kept when the probe failed.
      h += section(p, 'quota.claude', 'Claude');
      const kept = S.snap.quota;
      const claude = L && L.claude.value ? L.claude.value : null;
      const shown = claude || kept;
      if (L && L.claude.error) h += `<div class="callout soon">the probe failed: ${esc(L.claude.error)}${kept ? ' — showing the reading the runs last kept' : ''}</div>`;
      if (shown) {
        h += '<div class="gauges">' + shown.windows.map((w) => gauge(w.name, Math.round(w.utilization * 100), resets(w.resets_at))).join('') + '</div>';
        h += `<div class="muted limits-from">${claude ? 'read now by a minimal session' : 'kept by the last run that ended'}, ${esc(at(shown.at))}</div>`;
      } else h += `<div class="muted">${p.limitsBusy ? 'reading…' : 'no reading yet'}</div>`;
      // GitHub: free to read, read every time.
      h += section(p, 'quota.github', 'GitHub API');
      if (!L) h += `<div class="muted">${p.limitsBusy ? 'reading…' : 'not read'}</div>`;
      else if (L.github.error) h += `<div class="callout soon">${esc(L.github.error)}</div>`;
      else h += '<div class="gauges">' + L.github.value.map((w) => gauge(w.name, w.limit ? Math.round(100 * w.used / w.limit) : 0, `${w.used} / ${w.limit} requests · ${resets(w.resets_at)}`)).join('') + '</div>';
      return h;
    },
    errors(p) {
      if (S.range && !period()) return '';
      const rows = period() ? period().errors : S.snap.errors;
      return section(p, 'errors.list', period() ? 'Stops of the period' : 'Last stops') + (rows.length ? '<table class="rows"><tr><th>when</th><th>workflow</th><th>kind</th><th>reason</th></tr>' + rows.map((e) => `<tr><td>${esc(e.when)}</td><td>${esc(e.workflow)}</td><td>${tag(e.kind, e.kind === 'QUOTA' ? 'warn' : e.kind === 'FAILED' ? 'bad' : '')}</td><td>${esc(e.reason.slice(0, 240))}${e.reason.length > 240 ? '…' : ''}</td></tr>`).join('') + '</table>' : '<div class="muted">none recorded</div>');
    },
    journal(p) {
      if (S.range && !period()) return '';
      const P = period();
      const j = P ? P.journal : S.snap.journal, f = S.snap.factory;
      const pct = j.ticks ? Math.round(100 * j.empty_ticks / j.ticks) : 0;
      const counts = [
        [j.ticks, 'ticks' + (P || j.since ? '' : ' (at least)')],
        [`${j.empty_ticks} <small>${pct}%</small>`, 'empty ticks'],
        [j.failed_ticks, 'failed board reads'],
      ];
      let h;
      if (P) {
        h = section(p, 'journal.period', 'Over the period') + tiles([
          [j.triggers.length, 'triggers'],
          [j.triggers.filter((t) => t.state === 'failed' || t.state === 'killed').length, 'failed or killed'],
          ...counts,
        ]);
      } else {
        const atWork = S.snap.employees.filter((e) => e.active).length;
        const lanes = j.triggers.filter((t) => t.state === 'running' && t.lane != null).length;
        h = section(p, 'journal.now', 'Now') + tiles([[atWork, 'agents at work'], [lanes, 'lanes open'], ...counts]);
        h += `<div class="muted loop-line">${f.draining ? tag('stopping', 'warn') : f.watching ? tag('polling', 'ok') : tag('off')} `
          + (j.since ? `since ${esc(when(j.since))}` : 'its start is older than the journal read')
          + (j.last_empty_at ? ` · last empty tick ${esc(when(j.last_empty_at))}` : '') + '</div>';
      }
      h += section(p, 'journal.triggers', 'Triggers');
      h += j.triggers.length
        ? '<table class="rows triggers"><tr><th>when</th><th>what</th><th>on</th><th class="num">lane</th><th>state</th><th class="num">lasted</th><th>detail</th></tr>'
          + j.triggers.map((t) => {
            const on = t.issue != null
              ? `<button class="link" data-issue="${t.issue}">#${t.issue}</button> <span class="muted">${esc(issueTitle(t.issue).slice(0, 60))}</span>`
              : esc(t.subject || '');
            const [label, cls] = STATE_TAG[t.state] || [t.state, ''];
            return `<tr><td>${esc(when(t.at))}</td><td>${esc(t.what)}</td><td>${on}</td><td class="num">${t.lane ?? '—'}</td><td>${tag(label, cls)}</td><td class="num">${esc(lasted(t.at, t.ended_at))}</td><td class="muted">${esc((t.detail || '').slice(0, 160))}</td></tr>`;
          }).join('') + '</table>'
        : `<div class="muted">${P ? 'nothing triggered during the period' : 'nothing triggered since the start of the journal read'}</div>`;
      const logs = P ? P.logs : S.snap.recent;
      h += section(p, 'journal.logs', 'Logs') + (P && P.logs_cut ? '<div class="muted">the period holds more lines: its last ones are shown</div>' : '') + '<pre class="log">' + esc(logs.join('\n') || '(empty)') + '</pre>';
      return h;
    },
  };
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

    // The control room's four computers: one screen each.
    dashboards(p, refresh) {
      const screens = { costs: 'Spending', quota: 'Rate limits', errors: 'Stops', journal: 'Watch loop' };
      const focus = screens[p.focus] ? p.focus : 'costs';
      const nav = '<nav class="screens">' + Object.entries(screens).map(([k, name]) => `<button class="${k === focus ? 'here' : ''}" data-pane='${esc(JSON.stringify({ kind: 'dashboards', focus: k }))}'>${esc(name)}</button>`).join('') + helpBtn(p, focus) + '</nav>' + helpBox(p, focus);
      const timed = focus !== 'quota';
      const h = nav + (timed ? rangeBar() : '') + DASH[focus](p);
      // The rate limits are read when their screen opens, not remembered; a
      // chosen period is read when a screen that shows one opens on it.
      const after = () => {
        if (focus === 'quota' && !p.limitsAsked) readLimits(p, false);
        if (timed && S.range && (!S.history || S.history.key !== rangeKey()) ) readHistory();
      };
      // A date being typed is not wiped by a refresh of the figures.
      const editing = refresh && document.activeElement && document.activeElement.matches && document.activeElement.matches('#pane-body input');
      return ['Control room · ' + screens[focus], h, after, editing];
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
