// The plant's page: data, navigation, panes, the steward's and the doctor's terminals — and a
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
  // Every clock a human reads, in the harness machine's time zone (clock.js).
  const C = window.HarnessClock;

  const S = {
    snap: null,
    level: 'A', room: null, pane: null,
    render: null,          // the wasm module, once it loaded
    renderReady: false,
    logTimer: null,
    term: null,
    limits: null,          // the last /api/limits answer, shared by the panel and the quota screen
    limitsRead: null,      // the read under way, which a second asker waits on
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
  const hhmm = (iso) => C.time(iso);
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
  // The picture the scene draws: the snapshot, with the freshest Claude
  // reading — the kept one lags while a session runs.
  function picture() {
    const probed = S.limits && S.limits.claude.value;
    const kept = S.snap.quota;
    return probed && (!kept || probed.at > kept.at) ? { ...S.snap, quota: probed } : S.snap;
  }
  function pushPicture() {
    if (S.render && S.renderReady && S.snap) S.render.setSnapshot(JSON.stringify(picture()));
  }
  function pushView() {
    if (!S.render || !S.renderReady) return;
    const view = S.level === 'C' ? { level: 'C', room: S.room || 'construction' } : { level: S.level };
    S.render.setView(JSON.stringify(view));
    // Entering the control room reads the limits its panel shows.
    if (S.level === 'C' && S.room === 'control') readLimitsNow(false);
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
    C.zone(snap.time_zone);
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
    dock();
    crewBar();
    crumbs();
    pushPicture();
    // An issue is fetched once; a terminal is a live connection.
    // Everything else is redrawn from the new picture.
    if (S.pane && S.pane.kind !== 'issue' && !TERMINALS[S.pane.kind]) renderPane(S.pane, true);
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
    // Several agents at one station are one bubble: a click lists them
    // first, then opens the one picked — the same pane as the scene's.
    const groups = [];
    for (const e of at) {
      const station = e.station || null;
      const g = groups.find((x) => x.line === e.workflow && x.station === station);
      if (g) g.agents.push(e); else groups.push({ line: e.workflow, station, agents: [e] });
    }
    const html = groups.map(({ line, station, agents }) => {
      const e = agents[0];
      const ring = e.model ? ` ${esc(e.model)}` : '';
      if (agents.length === 1) {
        const open = esc(JSON.stringify({ kind: 'employee', id: e.id }));
        const tip = esc(`${e.name}${e.stage ? ' — ' + e.stage : ''}`);
        return `<div class="crew-item"><button class="crew-bubble" data-crew='${open}' title="${tip}"><span class="face${ring}">${faceOf(e.id)}</span><span class="who"><b>${esc(e.name)}</b><small>${esc(e.stage || 'starting')}</small></span></button>${worksOn(e, 'crew-link')}</div>`;
      }
      const lineTitle = (S.snap.lines.find((l) => l.id === line) || {}).title || line;
      const title = `${lineTitle} · ${e.stage || 'starting'}`;
      const open = esc(JSON.stringify({ kind: 'crew', line, station }));
      return `<div class="crew-item"><button class="crew-bubble" data-crew='${open}' title="${esc(title)}"><span class="face${ring}">${faceOf(e.id)}<span class="count">${agents.length}</span></span><span class="who"><b>${esc(title)}</b><small>${esc(agents.map((a) => a.name.replace(/ · .*$/, '')).join(', '))}</small></span></button></div>`;
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

  // ---- from a trigger to its run's logs ----------------------------------------------
  async function openTriggerLog(t, button) {
    const q = new URLSearchParams({ at: t.at });
    if (t.issue != null) q.set('issue', t.issue);
    let run = null;
    try {
      const r = await fetch(`/api/runs/${t.workflow}/locate?${q}`);
      if (r.ok) run = (await r.json()).run;
    } catch (e) { /* said below */ }
    if (!run) { button.title = 'no run of this trigger was found in the logs'; button.classList.add('missing'); return; }
    const failed = t.state === 'failed' || t.state === 'killed' || t.state === 'unknown';
    const id = `${t.workflow}/${run}`;
    const name = `${t.issue != null ? '#' + t.issue + ' · ' : ''}${t.what} · ${run}`;
    openPane({ kind: 'employee', id, last: { id, name, workflow: t.workflow, run_id: run, active: false }, file: 'run.log', jumpTo: failed ? 'error' : 'end' });
  }
  // A status known by its run id — a paid session, a stop — leads to that
  // run's logs: at the stop when it ended badly, on its stage's session
  // otherwise, and to the agent followed live while the run is at work. The
  // server says which line's logs hold the run.
  async function openRunLog(r, button) {
    let workflow = r.workflow || null;
    if (!workflow) {
      try {
        const q = r.stage ? '?' + new URLSearchParams({ stage: r.stage }) : '';
        const res = await fetch(`/api/run-of/${encodeURIComponent(r.run)}${q}`);
        if (res.ok) workflow = (await res.json()).workflow;
      } catch (e) { /* said below */ }
    }
    if (!workflow) { button.title = 'no log of this run was found'; button.classList.add('missing'); return; }
    const id = `${workflow}/${r.run}`;
    if (S.snap.employees.some((e) => e.id === id && e.active)) { openPane({ kind: 'employee', id }); return; }
    const last = { id, name: r.name || `${r.task ? '#' + String(r.task).replace(/^#/, '') + ' · ' : ''}${r.stage || workflow} · ${r.run}`, workflow, run_id: r.run, active: false };
    openPane(r.failed
      ? { kind: 'employee', id, last, file: 'run.log', jumpTo: 'error' }
      : { kind: 'employee', id, last, stage: r.stage || null });
  }
  // A process status that opens the logs it is the status of.
  const runTag = (label, cls, r) => (r.run
    ? `<button class="tag-link" data-run-log='${esc(JSON.stringify(r))}' title="${r.failed ? 'open its logs at the error' : 'open its logs'}">${tag(label, cls)}</button>`
    : tag(label, cls));
  // A paid session's outcome, leading to its run's logs.
  const sessionTag = (r, workflow) => runTag(r.outcome || '?', r.outcome === 'ok' ? 'ok' : r.outcome ? 'bad' : '', { run: r.run, workflow, stage: r.stage, task: r.task, failed: !!r.outcome && r.outcome !== 'ok' });
  // A running status: the agent at work, its session followed live.
  const liveTag = (id, label = 'running') => `<button class="tag-link" data-pane='${esc(JSON.stringify({ kind: 'employee', id }))}' title="open its session, followed live">${tag(label, 'acc')}</button>`;
  // What an agent works on — the issue refined, the pull request reviewed,
  // the branch a dev loop builds — one click away on GitHub.
  const worksOn = (e, cls = 'ext') => (e && e.works_on && e.works_on.url
    ? `<a class="${cls}" href="${esc(e.works_on.url)}" target="_blank" rel="noopener" title="open ${esc(e.works_on.label)} on GitHub">${esc(e.works_on.label)} ↗</a>`
    : '');

  // A line that says why a run stopped: its halt, a warning, a session that
  // broke, the breaker's refusal.
  const ERROR_LINE = /(^|\] )(warning: |STOP: |FAILED: |QUOTA: |! )|refusing to pay|timed out|printed nothing|ran past its/;

  // The colour of a log line. The journal heads a failure with `error: ` and
  // what is waited on with `warning: `; a run says its stop as `FAILED: `
  // (red) or `STOP: `/`QUOTA: ` (amber), and a session's stderr is `! `. A
  // journal written before `error: ` existed still has its failed lanes red.
  const LOG_ERROR = /(^|\] )(error: |FAILED: |! )|timed out|printed nothing|ran past its|-> cannot start|done with #\d+ \((exit status: (2|[4-9]|\d{2,})\b|signal)/;
  const LOG_WARN = /(^|\] )(warning: |STOP: |QUOTA: )|refusing to pay/;
  function logLine(line) {
    const level = LOG_ERROR.test(line) ? 'error' : LOG_WARN.test(line) ? 'warn' : '';
    const shown = esc(C.line(line));
    return level ? `<span class="log-${level}">${shown}</span>` : shown;
  }
  const logHtml = (text) => text.split('\n').map(logLine).join('\n');

  // ---- notifications ------------------------------------------------------------
  // Three signs at the bottom right — info, warning, error — grey when nothing
  // of their level is unread, in their colour when something is. A sign opens
  // the notifications pane on its level. Reading is acknowledging: leaving the
  // pane the standard way marks what it showed as read; "close, keep unread"
  // leaves everything as it was, and a notification clicked in the pane is
  // locked unread for that visit. A read notification is unread again when it
  // happens again (its `at` moves).
  const LEVELS = [['error', 'Errors'], ['warning', 'Warnings'], ['info', 'Info']];
  // `read`: what was acknowledged, by key, at which occurrence. `known`: every
  // notification seen lately, so one whose cause ended (a label removed, an
  // issue closed) stays listed — resolved — instead of vanishing unread.
  const KNOWN_FOR_MS = 24 * 3600e3;
  const R = { read: {}, known: {} };
  try { R.read = JSON.parse(localStorage.getItem('harness.read') || '{}'); } catch (e) { R.read = {}; }
  try { R.known = JSON.parse(localStorage.getItem('harness.known') || '{}'); } catch (e) { R.known = {}; }
  const store = (name, value) => { try { localStorage.setItem(name, JSON.stringify(value)); } catch (e) { /* kept for this visit only */ } };
  const isUnread = (n) => R.read[n.key] !== n.at;
  // Folds the latest notifications into what is known: present ones are
  // current, absent ones resolved; anything resolved for a day is forgotten.
  function remember() {
    const now = Date.now();
    const present = new Set();
    for (const n of (S.snap && S.snap.notifications) || []) {
      present.add(n.key);
      R.known[n.key] = { ...n, seen: now, resolved: false };
    }
    for (const [key, n] of Object.entries(R.known)) {
      if (present.has(key)) continue;
      if (!n.resolved) R.known[key] = { ...n, resolved: true, seen: now };
      else if (now - n.seen > KNOWN_FOR_MS) delete R.known[key];
    }
    store('harness.known', R.known);
  }
  const RANK = { error: 0, warning: 1, info: 2 };
  // A level's notifications: current ones first, then the resolved, newest first.
  const ofLevel = (level) => Object.values(R.known)
    .filter((n) => n.level === level)
    .sort((a, b) => (a.resolved - b.resolved) || (RANK[a.level] - RANK[b.level]) || ((b.at || '') > (a.at || '') ? 1 : -1));
  const SIGN = {
    info: '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="10" class="shape"/><rect x="10.8" y="5.5" width="2.4" height="8.5" rx="1.1" class="mark"/><circle cx="12" cy="17.6" r="1.4" class="mark"/></svg>',
    warning: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 2.2 23 21.3H1z" class="shape" stroke-linejoin="round"/><rect x="10.9" y="8.4" width="2.2" height="7.2" rx="1" class="mark"/><circle cx="12" cy="18.2" r="1.3" class="mark"/></svg>',
    error: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7.9 1.8h8.2l5.8 5.8v8.2l-5.8 5.8H7.9L2.1 15.8V7.6z" class="shape"/><rect x="10.8" y="5.8" width="2.4" height="8.4" rx="1.1" class="mark"/><circle cx="12" cy="17.7" r="1.4" class="mark"/></svg>',
  };
  function dock() {
    remember();
    const html = LEVELS.slice().reverse().map(([level, label]) => {
      const unread = ofLevel(level).filter(isUnread).length;
      return `<button class="sign ${level} ${unread ? 'lit' : ''}" data-notes="${level}" title="${label}${unread ? ` — ${unread} unread` : ''}" aria-label="${label}, ${unread} unread">${SIGN[level]}${unread ? `<span class="badge">${unread > 99 ? '99+' : unread}</span>` : ''}</button>`;
    }).join('');
    // Above the signs: the wrench, blue while a change waits for a restart.
    const waiting = S.waiting || 0;
    const wrench = `<button class="sign changes ${waiting ? 'lit' : ''}" data-changes title="${waiting ? `${waiting} change(s) wait for a restart` : 'No change waits for a restart'}" aria-label="changes waiting, ${waiting}">${wrenchIcon()}${waiting ? `<span class="badge">${waiting}</span>` : ''}</button>`;
    const all = wrench + html;
    if (all !== S.dockHtml) { $('dock').innerHTML = all; S.dockHtml = all; }
  }
  $('dock').addEventListener('click', (e) => {
    if (e.target.closest('[data-changes]')) { openPane({ kind: 'changes' }); return; }
    const sign = e.target.closest('[data-notes]');
    if (sign) openPane({ kind: 'notifications', level: sign.dataset.notes });
  });
  // Leaving the notifications pane: what it showed is read, but for what was
  // locked unread — unless it was left with "close, keep unread".
  function acknowledge(p) {
    if (!p || p.kind !== 'notifications' || p.keepAll) return;
    const locked = new Set(p.locked || []);
    for (const level of p.visited || []) {
      for (const n of ofLevel(level)) {
        // Kept unread means unread when the pane is left — even one that had
        // been read before.
        if (locked.has(n.key)) delete R.read[n.key];
        else R.read[n.key] = n.at;
      }
    }
    store('harness.read', R.read);
    dock();
  }
  function followNote(n) {
    if (!n.link) return;
    if (n.link.to === 'issue') { openPane({ kind: 'issue', number: n.link.number }); return; }
    if (n.link.to === 'data_model') { openPane({ kind: 'data_divergence', branch: n.link.branch, dir: n.link.dir, at: n.link.at }); return; }
    if (n.link.to === 'agent') {
      const id = `${n.link.workflow}/${n.link.run}`;
      const last = { id, name: `${n.link.workflow} · ${n.link.run}`, workflow: n.link.workflow, run_id: n.link.run, active: false };
      openPane({ kind: 'employee', id, last, file: 'run.log', jumpTo: 'error' });
      return;
    }
    if (n.link.at && n.link.screen !== 'quota') {
      // The screen, read around the moment it happened.
      const around = (ms) => new Date(Date.parse(n.link.at) + ms).toISOString().slice(0, 19) + 'Z';
      setRange({ from: around(-30 * 60e3), to: around(30 * 60e3) });
    }
    openPane({ kind: 'dashboards', focus: n.link.screen });
  }

  // ---- the plant's switch --------------------------------------------------
  // The status is a button: it opens the gestures that make sense now — start
  // an empty plant, or stop a running one softly (running tasks finish, none
  // starts) or hard (the watch and its lanes are killed, after a second click).
  const P = { status: null, armed: null, busy: false, note: '', bad: false, flash: null, lanes: 1 };
  // The agents a start runs at once: the last count asked, kept per viewer.
  try { P.lanes = Math.max(1, parseInt(localStorage.getItem('plant.lanes'), 10) || 1); } catch (e) { /* storage off: 1 */ }
  function lanesSet(n) {
    const max = (P.status && P.status.max_lanes) || 16;
    P.lanes = Math.min(max, Math.max(1, n));
    try { localStorage.setItem('plant.lanes', String(P.lanes)); } catch (e) { /* storage off */ }
  }
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
    else if (!st.running) {
      const max = st.max_lanes || 16;
      if (P.lanes > max) P.lanes = max;
      const off = P.busy ? ' disabled' : '';
      html = `<div class="lanes"><span>Agents at once<small>the most tasks the watch runs together</small></span>`
        + `<button type="button" class="step" data-lanes="-1" aria-label="one agent less"${off || (P.lanes <= 1 ? ' disabled' : '')}>−</button>`
        + `<output aria-live="polite">${P.lanes}</output>`
        + `<button type="button" class="step" data-lanes="1" aria-label="one agent more"${off || (P.lanes >= max ? ' disabled' : '')}>+</button></div>`
        + plantOption(`▶ Start the plant · ${P.lanes} agent${P.lanes > 1 ? 's' : ''}`, `starts harness watch --parallel ${P.lanes} in this checkout`, 'start');
    } else {
      if (st.running.lanes) html += `<div class="note">running with at most ${st.running.lanes} agent${st.running.lanes > 1 ? 's' : ''} at once</div>`;
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
    const first = $('plant-menu').querySelector('button[data-gesture]:not(:disabled)');
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
    const action = window.HarnessActions && window.HarnessActions.start({ key: 'plant', label: `${gesture} the plant`, button: $('status') });
    try {
      const r = await fetch('/api/plant/' + gesture + (gesture === 'start' ? '?lanes=' + P.lanes : ''), { method: 'POST' });
      const text = await r.text();
      if (r.ok) {
        P.note = JSON.parse(text).message; P.bad = false;
        P.flash = settled(gesture, P.note);
        if (action) action.ok(P.note);
      } else {
        P.note = text || r.statusText; P.bad = true;
        if (action) action.fail(P.note);
        // A failure stays on the status line until the menu is opened again.
        P.flash = { text: `${gesture} failed — ${P.note.split('\n')[0]}`, detail: P.note, tone: 'bad', sticky: true };
      }
    } catch (e) {
      P.note = 'cannot reach the server'; P.bad = true;
      if (action) action.fail(P.note);
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
    const step = ev.target.closest('button[data-lanes]');
    if (step && !step.disabled) {
      lanesSet(P.lanes + Number(step.dataset.lanes));
      plantMenu();
      const again = $('plant-menu').querySelector(`button[data-lanes="${step.dataset.lanes}"]:not(:disabled)`) || $('plant-menu').querySelector('button[data-gesture]');
      if (again) again.focus();
      return;
    }
    const b = ev.target.closest('button[data-gesture]');
    if (b && !b.disabled) plantDo(b.dataset.gesture);
  });
  $('plant-menu').addEventListener('keydown', (ev) => {
    // Left and right turn the agents counter, wherever the focus is.
    if ((ev.key === 'ArrowLeft' || ev.key === 'ArrowRight') && $('plant-menu').querySelector('.lanes') && !P.busy) {
      ev.preventDefault();
      lanesSet(P.lanes + (ev.key === 'ArrowRight' ? 1 : -1));
      const d = document.activeElement && document.activeElement.dataset;
      const focused = d && (d.gesture ? `[data-gesture="${d.gesture}"]` : d.lanes ? `[data-lanes="${d.lanes}"]` : null);
      plantMenu();
      // A step at its bound is disabled: the focus falls back to the start.
      const back = (focused && $('plant-menu').querySelector(`button${focused}:not(:disabled)`)) || $('plant-menu').querySelector('button[data-gesture]');
      if (back) back.focus();
      return;
    }
    if (ev.key !== 'ArrowDown' && ev.key !== 'ArrowUp') return;
    ev.preventDefault();
    const items = [...$('plant-menu').querySelectorAll('button:not(:disabled)')];
    const at = items.indexOf(document.activeElement);
    const next = items[(at + (ev.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length];
    if (next) next.focus();
  });
  document.addEventListener('click', (ev) => {
    // The path is taken at dispatch: a button the menu redrew under the click
    // is detached by now, yet the click still came from inside.
    if (!$('plant-menu').hidden && !ev.composedPath().includes($('status-wrap'))) plantClose();
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
    // Typing in a field is typing: Backspace erases, Escape leaves the field's pane.
    const typing = e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA' || e.target.isContentEditable;
    if (e.key === 'Escape' || e.key === 'Backspace') { if (!typing && e.target.tagName !== 'CANVAS') back(); else if (e.key === 'Escape') back(); }
  });
  for (const who of ['steward', 'doctor']) {
    $('hud-' + who).addEventListener('click', () => {
      if (S.pane && S.pane.kind === who) closePane(); else openPane({ kind: who });
    });
  }
  $('hud-janitor').addEventListener('click', () => {
    if (S.pane && S.pane.kind === 'janitor') closePane(); else openPane({ kind: 'janitor' });
  });
  function act(h) {
    if (h.go) return go(h.go, h.room || null);
    if (h.url) return window.open(h.url, '_blank', 'noopener');
    if (h.pane) return openPane(h.pane);
    return undefined;
  }

  // The panes drawn on a board: what mounts each.
  const CANVASES = {
    data_model: (el, p) => (window.DataModelPane ? window.DataModelPane.mount(el, {
      branch: p.branch,
      dir: p.dir,
      onSession: (s) => openPane({ kind: 'note_session', board: s.board, id: s.id, on: s.on, pin: s.kind, context: s.context, branch: s.branch, dir: s.dir }),
    }) : null),
  };

  // ---- D · the pane ------------------------------------------------------------
  // The panes walked through, so the arrow at the top left goes back to the
  // one this came from. Closing the pane forgets the walk.
  const PANE_HISTORY = 50;
  S.paneHistory = [];
  function openPane(p, goingBack = false) {
    // The office's screen was the feature list before it was renamed.
    if (p.kind === 'features') p = { ...p, kind: 'capabilities' };
    acknowledge(S.pane);
    if (S.pane && !goingBack) {
      S.paneHistory.push(S.pane);
      if (S.paneHistory.length > PANE_HISTORY) S.paneHistory.shift();
    }
    $('pane-back').hidden = !S.paneHistory.length;
    clearInterval(S.logTimer); S.logTimer = null;
    teardownTerminal();
    S.pane = Object.assign({}, p);
    document.body.classList.add('split');
    $('pane').hidden = false;
    $('pane-body').classList.toggle('terminal', !!TERMINALS[p.kind]);
    $('pane-body').classList.toggle('canvas', !!CANVASES[p.kind]);
    hideTip();
    renderPane(S.pane, false);
  }
  function closePane() {
    if (!S.pane) return;
    acknowledge(S.pane);
    S.pane = null;
    S.paneHistory = [];
    $('pane-back').hidden = true;
    clearInterval(S.logTimer); S.logTimer = null;
    teardownTerminal();
    document.body.classList.remove('split');
    $('pane').hidden = true;
    $('pane-body').classList.remove('terminal', 'canvas');
  }
  $('pane-close').addEventListener('click', closePane);
  $('pane-back').addEventListener('click', () => {
    const previous = S.paneHistory.pop();
    if (previous) openPane(previous, true);
  });
  $('pane-body').addEventListener('change', (e) => {
    const field = e.target.closest('[data-range]');
    if (!field) return;
    setRange({ ...(S.range || {}), [field.dataset.range]: toClock(field.value) });
  });
  $('pane-body').addEventListener('click', (e) => {
    const ask = e.target.closest('[data-diagnose]');
    if (ask) { askDoctor(ask, JSON.parse(ask.dataset.diagnose)); return undefined; }
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
    const turn = e.target.closest('[data-page]');
    if (turn && S.pane && !turn.disabled) {
      const pager = turn.closest('[data-pager]');
      S.pane.pages = { ...(S.pane.pages || {}), [pager.dataset.pager]: Number(turn.dataset.page) };
      renderPane(S.pane, false);
      return undefined;
    }
    const preset = e.target.closest('[data-range-preset]');
    if (preset) { presetRange(preset.dataset.rangePreset); return undefined; }
    if (e.target.closest('[data-range-clear]')) { setRange(null); return undefined; }
    if (S.pane && S.pane.kind === 'notifications') {
      const open = e.target.closest('[data-note-open]');
      if (open) { const n = ofLevel(S.pane.level).find((x) => x.key === open.dataset.noteOpen); if (n) followNote(n); return undefined; }
      const level = e.target.closest('[data-note-level]');
      if (level) { S.pane.level = level.dataset.noteLevel; S.pane.pages = {}; renderPane(S.pane, false); return undefined; }
      if (e.target.closest('[data-notes-unread]')) { S.pane.unreadOnly = !S.pane.unreadOnly; S.pane.pages = {}; renderPane(S.pane, false); return undefined; }
      if (e.target.closest('[data-notes-keep]')) { S.pane.keepAll = true; closePane(); return undefined; }
      const row = e.target.closest('[data-note-lock]');
      if (row) {
        const locked = new Set(S.pane.locked || []);
        if (locked.has(row.dataset.noteLock)) locked.delete(row.dataset.noteLock); else locked.add(row.dataset.noteLock);
        S.pane.locked = [...locked];
        renderPane(S.pane, false);
        return undefined;
      }
    }
    const toLog = e.target.closest('[data-trigger-log]');
    if (toLog) { openTriggerLog(JSON.parse(toLog.dataset.triggerLog), toLog); return undefined; }
    const toRun = e.target.closest('[data-run-log]');
    if (toRun) { openRunLog(JSON.parse(toRun.dataset.runLog), toRun); return undefined; }
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

  // ---- the doctor's diagnosis of one agent ------------------------------------------
  // An agent whose run stopped badly or warned gets a button in its pane: a
  // click pastes its logs into the doctor's terminal as a question. The button
  // pulses yellow while the doctor reads, turns green when a reply ended with
  // the run's mark, and then opens the doctor's pane — the back arrow returns.
  S.diagnoses = {};
  const DIAGNOSIS_LABEL = { running: '🦞 diagnosing…', done: '🦞 diagnostic done', lost: '🦞 ask the doctor again' };
  function doctorButton(e) {
    const d = S.diagnoses[e.id];
    const state = d ? d.state : 'idle';
    const title = { idle: 'ask the doctor what happened — its logs go with the question', running: 'the doctor is reading its logs — click to watch', done: 'open the diagnosis', lost: 'the doctor did not answer — ask again' }[state];
    return `<button class="mini doctor-btn ${state}" data-diagnose='${esc(JSON.stringify({ workflow: e.workflow, run: e.run_id, id: e.id }))}' title="${title}">${DIAGNOSIS_LABEL[state] || '🦞 ask the doctor'}</button>`;
  }
  // Whether the run went badly: its run.log says so, or the pane was opened
  // on an error. Decided once per pane, then the head is redrawn.
  async function sickness(p, e) {
    p.sick = false;
    if (p.jumpTo === 'error') p.sick = true;
    else {
      try {
        const r = await fetch(`/api/runs/${e.workflow}/${e.run_id}/run.log?bytes=20000`);
        if (r.ok) p.sick = ERROR_LINE.test(await r.text());
      } catch (err) { /* stays well */ }
    }
    if (p.sick) { await pullDiagnoses(); if (S.pane === p) renderPane(p, true); }
  }
  async function pullDiagnoses() {
    try {
      const r = await fetch('/api/doctor/diagnoses');
      if (r.ok) S.diagnoses = await r.json();
    } catch (err) { /* kept as known */ }
    const running = Object.values(S.diagnoses).some((d) => d.state === 'running');
    clearTimeout(S.diagnosisTimer);
    if (running) S.diagnosisTimer = setTimeout(pullDiagnoses, 2000);
    if (S.pane && S.pane.kind === 'employee') renderPane(S.pane, true);
  }
  async function askDoctor(button, r) {
    const d = S.diagnoses[r.id];
    if (d && (d.state === 'running' || d.state === 'done')) { openPane({ kind: 'doctor' }); return; }
    button.classList.remove('idle', 'lost'); button.classList.add('running'); button.textContent = DIAGNOSIS_LABEL.running;
    try {
      const res = await fetch(`/api/doctor/diagnose/${r.workflow}/${r.run}`, { method: 'POST' });
      if (!res.ok) { button.classList.remove('running'); button.classList.add('lost'); button.textContent = '🦞 ask the doctor'; button.title = await res.text(); return; }
      S.diagnoses[r.id] = await res.json();
    } catch (err) { button.classList.remove('running'); button.title = 'cannot reach the server'; return; }
    pullDiagnoses();
  }

  // ---- The data model's divergence: the diagnosis, and the issue -------------------
  // A little markdown: paragraphs, line breaks, **bold**, `code`.
  const md = (text) => String(text || '').split(/\n{2,}/).map((para) => '<p>' + esc(para)
    .replace(/\*\*(.+?)\*\*/g, '<b>$1</b>')
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\n/g, '<br>') + '</p>').join('');
  async function divergenceLoad(p) {
    let d = null;
    try {
      const res = await fetch('/api/data-model/divergence?' + new URLSearchParams({ branch: p.branch || '', dir: p.dir || '' }));
      d = res.ok ? await res.json() : null;
    } catch (e) { d = null; }
    const host = $('dv');
    if (!host || !S.pane || S.pane.kind !== 'data_divergence') return;
    if (!d) { host.innerHTML = '<div class="callout">No diagnosis recorded for this version: the events were swept, or the view reads no event store.</div>'; return; }
    const list = (items) => (items.length ? '<ul class="dm-rules">' + items.map((i) => `<li>${esc(i)}</li>`).join('') + '</ul>' : '<p class="muted">None.</p>');
    const proposed = d.actions.map((a) => '- ' + a).join('\n');
    host.innerHTML = `<div class="dv-head"><span class="tag bad">diverges</span><b>${esc(d.branch)}</b><code>${esc(d.sha.slice(0, 7))}</code><span class="muted">diagnosed ${esc(C.stamp(d.at))}</span></div>`
      + '<h4>State of affairs</h4>'
      + (d.diagnosis ? `<div class="dv-diagnosis">${md(d.diagnosis)}</div>` : `<div class="callout">Not diagnosed: ${esc(d.failure || 'the session gave no answer')}. The divergences below are the static comparison's.</div>`)
      + `<h4>The divergences <span class="muted">· ${d.divergences.length}</span></h4>${list(d.divergences)}`
      + `<h4>Proposed actions</h4>${list(d.actions)}`
      + '<h4>How to resolve it</h4>'
      + `<textarea id="dv-resolution" class="dv-resolution" rows="7" maxlength="8000" placeholder="What should be done: fix the migration, or describe the field in data/model.json…">${esc(proposed)}</textarea>`
      + '<div class="dv-actions"><button type="button" class="act primary" id="dv-open">Open the issue</button>'
      + '<button type="button" class="act" id="dv-see">See this version</button><span id="dv-said" class="muted"></span></div>'
      + '<p class="muted">The issue carries <code>harness:roadmap</code> and <code>harness:ready</code>: the planner, the split and the loop take it from there, and a write-side change still waits for your merge.</p>';
    $('dv-see').addEventListener('click', () => openPane({ kind: 'data_model', branch: d.branch, dir: d.dir }));
    $('dv-open').addEventListener('click', async () => {
      const said = $('dv-said');
      const button = $('dv-open');
      if (button.getAttribute('aria-disabled') === 'true') return;
      button.setAttribute('aria-disabled', 'true');
      said.textContent = 'opening…';
      const action = window.HarnessActions && window.HarnessActions.start({ key: 'issue:' + d.dir, label: `open the issue for ${d.branch}`, button });
      try {
        const res = await fetch('/api/data-model/issue', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ branch: d.branch, dir: d.dir, resolution: $('dv-resolution').value }) });
        const text = await res.text();
        if (!res.ok) throw new Error(text);
        const url = JSON.parse(text).url;
        said.innerHTML = /^https?:/.test(url) ? `opened <a href="${esc(url)}" target="_blank" rel="noopener">${esc(url)}</a>` : `opened ${esc(url)}`;
        if (action) action.ok(url);
      } catch (e) {
        said.textContent = 'not opened: ' + (e.message || e);
        if (action) action.fail(e.message || e);
        button.removeAttribute('aria-disabled');
      }
    });
  }

  // ---- The janitor: what the yard weighs, and a sweep ------------------------------
  // The yard is `.llocal/`: the clones, their build outputs and dependencies,
  // the runs' traces. The janitor weighs it every two hours on their own, or on a
  // click; a sweep removes what the rules say is no longer useful — no model
  // involved. Two pies: what the yard takes of what it may, and what fills it.
  const GIB = 1024 ** 3;
  const bytes = (n) => {
    const v = Number(n || 0);
    if (v >= GIB) return (v / GIB).toFixed(v >= 100 * GIB ? 0 : 1) + ' GB';
    if (v >= 1024 ** 2) return (v / 1024 ** 2).toFixed(0) + ' MB';
    if (v >= 1024) return (v / 1024).toFixed(0) + ' kB';
    return v + ' B';
  };
  // One hue per category, fixed: a slice keeps its colour whatever its rank.
  const SLICES = {
    code: ['Code', '#3987e5', 'the clones\' own files and .git — never swept'],
    build: ['Build outputs', '#d95926', 'target/, .next/… — swept 2 h after their workspace was last written'],
    dependencies: ['Dependencies', '#199e70', 'node_modules/, .venv/… — swept after a week idle; harness doctor reinstalls them'],
    traces: ['Run traces', '#c98500', 'logs/ — a run\'s folder is swept after 30 days'],
    init: ['init-repo clones', '#d55181', 'init/ — throwaway clones, swept a day after'],
    strays: ['Strays', '#9085e9', 'folders the harness never writes — swept only if you allow it'],
    stores: ['Stores', '#e66767', 'the event store, lanes\' journals, locks — never swept'],
  };
  const SC = { status: null, timer: null, armed: false, said: '' };
  async function janitorLoad(full) {
    try {
      SC.status = await (await fetch('/api/janitor')).json();
    } catch (e) { SC.status = null; }
    janitorHud();
    if (!S.pane || S.pane.kind !== 'janitor') return;
    janitorDraw(full);
    clearTimeout(SC.timer);
    if (SC.status && SC.status.doing && SC.status.doing !== 'idle') SC.timer = setTimeout(() => janitorLoad(false), 1500);
  }
  function janitorHud() {
    const d = SC.status && SC.status.diagnosis;
    const b = $('hud-janitor');
    b.classList.toggle('warn', !!d && d.over_threshold && !d.over_limit);
    b.classList.toggle('bad', !!d && d.over_limit);
    b.textContent = d ? `🧹 janitor · ${Math.round((100 * d.used) / Math.max(1, d.limit))} %` : '🧹 janitor';
  }
  // A ring of slices, with a 2px gap of the pane's own colour between them.
  function donut(parts, center, sub, marker) {
    const R = 54, W = 18, C = 2 * Math.PI * R;
    const total = parts.reduce((t, p) => t + p.value, 0) || 1;
    let at = 0;
    let svg = `<svg viewBox="0 0 140 140" class="donut" role="img" aria-label="${esc(center + ' ' + sub)}"><circle cx="70" cy="70" r="${R}" fill="none" stroke="var(--panel-2)" stroke-width="${W}"/>`;
    for (const p of parts) {
      const len = (p.value / total) * C;
      if (len <= 0) continue;
      const gap = parts.filter((q) => q.value > 0).length > 1 ? Math.min(2, len / 2) : 0;
      svg += `<circle cx="70" cy="70" r="${R}" fill="none" stroke="${p.color}" stroke-width="${W}" stroke-dasharray="${Math.max(0, len - gap)} ${C}" stroke-dashoffset="${-at}" transform="rotate(-90 70 70)"><title>${esc(p.title)}</title></circle>`;
      at += len;
    }
    if (marker != null) {
      const a = (marker * 2 * Math.PI) - Math.PI / 2;
      const x1 = 70 + Math.cos(a) * (R - W / 2 - 3), y1 = 70 + Math.sin(a) * (R - W / 2 - 3);
      const x2 = 70 + Math.cos(a) * (R + W / 2 + 3), y2 = 70 + Math.sin(a) * (R + W / 2 + 3);
      svg += `<line x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}" stroke="var(--text)" stroke-width="2.5" stroke-linecap="round"><title>auto-sweep threshold</title></line>`;
    }
    svg += `<text x="70" y="67" text-anchor="middle" class="big">${esc(center)}</text><text x="70" y="85" text-anchor="middle" class="small">${esc(sub)}</text></svg>`;
    return svg;
  }
  function janitorDraw(full) {
    const st = SC.status;
    const live = $('janitor-live');
    if (!live) return;
    if (!st || !st.available) {
      live.innerHTML = `<div class="callout">No janitor: the view runs with <code>--no-janitor</code>, or the server is unreachable.</div>`;
      $('janitor-settings').innerHTML = ''; $('janitor-lists').innerHTML = '';
      return;
    }
    const d = st.diagnosis;
    const busy = st.doing !== 'idle';
    const doing = { diagnosing: 'weighing the yard…', sweeping: 'sweeping…' }[st.doing] || '';
    const sweepLabel = SC.armed ? `🧹 Click again: sweep ${bytes(d ? d.freeable : 0)}` : '🧹 Run the sweep';
    let h = '<div class="janitor-actions">'
      + `<button class="act primary${SC.armed ? ' armed' : ''}" data-janitor="sweep" ${busy || (d && !d.freeable) ? 'disabled' : ''} title="remove what the rules below say is no longer useful">${sweepLabel}</button>`
      + `<button class="act" data-janitor="diagnose" ${busy ? 'disabled' : ''} title="weigh the yard again, remove nothing">🔍 Diagnose</button>`
      + `<span class="muted">${busy ? `<span class="spin"></span>${doing}` : d ? `weighed ${hhmm(d.at)} · next ${hhmm(st.next_at)}` : 'not weighed yet'}${SC.said ? ' · ' + esc(SC.said) : ''}</span></div>`;
    if (!d) { live.innerHTML = h + '<div class="muted">The first weighing runs when the view starts; it takes a few seconds.</div>'; }
    else {
      const pct = (100 * d.used) / Math.max(1, d.limit);
      const state = d.over_limit ? 'bad' : d.over_threshold ? 'warn' : 'ok';
      const usedColor = { ok: 'var(--ok)', warn: 'var(--warn)', bad: 'var(--bad)' }[state];
      const usedPie = donut(
        [{ value: Math.min(d.used, d.limit), color: usedColor, title: `used: ${bytes(d.used)}` }, { value: Math.max(0, d.limit - d.used), color: 'transparent', title: `free under the limit: ${bytes(d.limit - d.used)}` }],
        `${Math.round(pct)} %`, `${bytes(d.used)} of ${bytes(d.limit)}`, st.settings.auto_sweep ? Math.min(1, d.threshold / Math.max(1, d.limit)) : null);
      const slices = d.slices.filter((s) => s.bytes > 0);
      const fillPie = donut(slices.map((s) => ({ value: s.bytes, color: SLICES[s.category][1], title: `${SLICES[s.category][0]}: ${bytes(s.bytes)}${s.sweepable ? ` · ${bytes(s.sweepable)} sweepable` : ''}` })), bytes(d.used), 'in the yard');
      const legend = '<table class="rows legend"><tr><th></th><th>what</th><th class="num">size</th><th class="num">a sweep frees</th></tr>'
        + d.slices.map((s) => `<tr title="${esc(SLICES[s.category][2])}"><td><span class="swatch" style="background:${SLICES[s.category][1]}"></span></td><td>${esc(SLICES[s.category][0])}</td><td class="num">${bytes(s.bytes)}</td><td class="num">${s.sweepable ? bytes(s.sweepable) : '—'}</td></tr>`).join('')
        + `<tr class="total"><td></td><td>all told</td><td class="num">${bytes(d.used)}</td><td class="num">${bytes(d.freeable)}</td></tr></table>`;
      const verdict = d.over_limit
        ? `<span class="tag bad">✕ over the limit</span> ${bytes(d.used - d.limit)} too many`
        : d.over_threshold ? `<span class="tag warn">! past the threshold</span> ${st.settings.auto_sweep ? 'the next round sweeps on its own' : 'auto-sweep is off'}`
          : '<span class="tag ok">✓ under the limit</span>';
      h += `<div class="pies"><figure><figcaption>Used of the limit</figcaption>${usedPie}<div class="cap">${verdict}</div></figure>`
        + `<figure><figcaption>What fills it</figcaption>${fillPie}</figure></div>` + legend;
      live.innerHTML = h;
    }
    if (full || !$('janitor-settings').innerHTML) janitorSettings();
    janitorLists();
  }
  function janitorSettings() {
    const s = SC.status.settings;
    $('janitor-settings').innerHTML = '<h3>Settings</h3><form id="janitor-form" class="janitor-form">'
      + `<label>Limit <input name="limit" type="number" min="1" step="1" value="${Math.round(s.limit_bytes / GIB)}"> GB</label>`
      + `<label><input name="auto" type="checkbox" ${s.auto_sweep ? 'checked' : ''}> Auto-sweep: when the 2-hourly weighing passes the threshold below, sweep without asking</label>`
      + `<label>Auto-sweep threshold <input name="threshold" type="number" min="1" max="100" step="1" value="${s.threshold_percent}"> % of the limit</label>`
      + `<label><input name="strays" type="checkbox" ${s.sweep_strays ? 'checked' : ''}> Strays go too (folders the harness never writes)</label>`
      + '<div><button class="act" type="submit">Save</button> <span id="janitor-saved" class="muted"></span></div></form>';
  }
  function janitorLists() {
    const d = SC.status && SC.status.diagnosis;
    const r = SC.status && SC.status.report;
    let h = '';
    if (r) {
      h += `<h3>Last sweep · ${hhmm(r.at)}${r.automatic ? ' · on their own' : ''}</h3><div class="loop-line">${bytes(r.freed)} freed · ${r.removed.length} removed${r.failed.length ? ` · <span class="tag bad">${r.failed.length} refused</span>` : ''}</div>`;
      if (r.failed.length) h += '<table class="rows">' + r.failed.map((f) => `<tr><td>${esc(f.rel)}</td><td>${esc(f.why)}</td></tr>`).join('') + '</table>';
    }
    if (d) {
      const row = (x) => `<tr><td class="path">${esc(x.rel)}</td><td class="num">${bytes(x.bytes)}</td><td>${esc(x.why)}</td></tr>`;
      h += `<h3>A sweep would remove · ${bytes(d.freeable)}</h3>` + (d.sweep.length
        ? '<table class="rows"><tr><th>path under .llocal</th><th class="num">size</th><th>why</th></tr>' + d.sweep.slice(0, 40).map(row).join('') + '</table>' + (d.sweep.length > 40 ? `<div class="muted">and ${d.sweep.length - 40} more</div>` : '')
        : '<div class="muted">nothing: the yard is clean</div>');
      if (d.kept.length) h += `<details><summary>Kept although it weighs · ${d.kept.length}</summary><table class="rows">` + d.kept.slice(0, 40).map(row).join('') + '</table></details>';
    }
    $('janitor-lists').innerHTML = h;
  }
  async function janitorPost(path, body) {
    const r = await fetch('/api/janitor/' + path, { method: 'POST', body, headers: body ? { 'content-type': 'application/json' } : {} });
    if (!r.ok) throw new Error(await r.text());
    SC.status = await r.json();
  }
  $('pane-body').addEventListener('click', async (e) => {
    const b = e.target.closest('[data-janitor]');
    if (!b || !S.pane || S.pane.kind !== 'janitor') return;
    const what = b.dataset.janitor;
    if (what === 'sweep' && !SC.armed) { SC.armed = true; janitorDraw(false); setTimeout(() => { if (SC.armed) { SC.armed = false; if (S.pane && S.pane.kind === 'janitor') janitorDraw(false); } }, 4000); return; }
    SC.armed = false; SC.said = '';
    SC.status = Object.assign({}, SC.status, { doing: what === 'sweep' ? 'sweeping' : 'diagnosing' });
    janitorDraw(false);
    const action = window.HarnessActions && window.HarnessActions.start({ key: 'janitor:' + what, label: what === 'sweep' ? 'sweep the yard' : 'weigh the yard', button: b });
    try {
      await janitorPost(what);
      SC.said = what === 'sweep' ? `swept · ${bytes(SC.status.report ? SC.status.report.freed : 0)} freed` : 'weighed';
      if (action) action.ok(SC.said);
    } catch (err) { SC.said = String(err.message || err); if (action) action.fail(SC.said); }
    janitorHud();
    if (S.pane && S.pane.kind === 'janitor') janitorDraw(false);
  });
  $('pane-body').addEventListener('submit', async (e) => {
    if (e.target.id !== 'janitor-form') return;
    e.preventDefault();
    const f = e.target;
    const settings = {
      limit_bytes: Math.round(Number(f.limit.value) * GIB),
      auto_sweep: f.auto.checked,
      threshold_percent: Math.round(Number(f.threshold.value)),
      sweep_strays: f.strays.checked,
    };
    try {
      await janitorPost('settings', JSON.stringify(settings));
      $('janitor-saved').textContent = 'saved';
      janitorDraw(false);
      janitorHud();
    } catch (err) { $('janitor-saved').textContent = String(err.message || err); }
  });
  janitorLoad(false);
  setInterval(() => { if (!S.pane || S.pane.kind !== 'janitor') janitorLoad(false); }, 60e3);

  // ---- the terminals: the steward's, the doctor's -------------------------------
  // xterm.js in the pane, a WebSocket to a desk. Closing the pane closes the
  // socket and nothing else: the program behind it keeps running, and the next
  // visit replays its screen. Two desks, one per person; the doctor's opens
  // with the check-up already asked.
  const TERMINALS = {
    steward: {
      title: 'The steward · Claude Code in the harness checkout',
      flag: '--no-steward',
      chips: [
        ['Is the plant running?', 'Is the plant running? Give me the status.'],
        ['Start the plant', 'Start the plant.'],
        ['Stop the plant', 'Stop the plant.'],
        ['What is blocking?', 'What is blocking the board right now, and what should I do?'],
        ['What did it cost today?', 'What did the plant spend today, and on what?'],
      ],
    },
    // A pin's AI session: opened by the server with the pin's conversation
    // handed back, closed when the pane is left — its turns are then kept.
    note_session: {
      title: (p) => `${p.pin === 'asking' ? 'Asking' : 'Brainstorm'} · ${p.on}`,
      flag: '--no-steward',
      chips: [
        ['Where are we?', 'Summarize where we are: what is decided, what is open.'],
        ['What are the options?', 'List the options we have here, with their trade-offs.'],
        ['What would you recommend?', 'What would you recommend, and why?'],
      ],
      status: (p) => fetch(`/api/notes/${encodeURIComponent(p.board)}/${encodeURIComponent(p.id)}/session`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ context: p.context || '', branch: p.branch || '', dir: p.dir || '' }) })
        .then(async (r) => (r.ok ? r.json() : { available: false, why: await r.text() })),
      socket: (p) => `/api/notes/${encodeURIComponent(p.board)}/${encodeURIComponent(p.id)}/session/term`,
      close: (p) => fetch(`/api/notes/${encodeURIComponent(p.board)}/${encodeURIComponent(p.id)}/session/close`, { method: 'POST', keepalive: true }),
    },
    doctor: {
      title: 'The doctor · a check-up of the plant',
      flag: '--no-doctor',
      chips: [
        ['Check-up again', 'Give the plant a full check-up again — read, do not treat — and finish with the HEALTH line.'],
        ['Why did it stop?', 'Why did the harness last stop? Read errors.tsv and the run\'s logs, and say what would fix it.'],
        ['What would you repair?', 'Run `./target/release/harness doctor --dry-run` and explain what it would repair.'],
        ['Treat it', 'You have my go: run `./target/release/harness doctor` now, then confirm what it discarded.'],
        ['Disk under .llocal', 'Where does the disk go under .llocal? Name the stray folders and what is safe to delete by hand.'],
      ],
    },
  };
  // A pane drawn on a board (the data model) owns its canvas and its pins:
  // leaving the pane unmounts it.
  function teardownCanvas() {
    if (!S.canvas) return;
    try { S.canvas.destroy(); } catch (e) { /* already gone */ }
    S.canvas = null;
  }
  function teardownTerminal() {
    teardownCanvas();
    if (!S.term) return;
    // A pin's session ends with its pane: its turns are kept with the pin.
    if (S.term.close) { try { S.closing = S.term.close(); } catch (e) { /* the server keeps what it can */ } }
    try { S.term.ro.disconnect(); } catch (e) { /* already gone */ }
    try { S.term.ws.close(); } catch (e) { /* already gone */ }
    try { S.term.term.dispose(); } catch (e) { /* already gone */ }
    S.term = null;
  }
  function stewardStatus(text) { const el = $('term-status'); if (el) el.textContent = text; }
  async function mountTerminal(who, p) {
    const el = $('term');
    if (!el) return;
    const desk = TERMINALS[who];
    let status = null;
    if (desk.status) stewardStatus('opening the session — a long conversation is summarized first…');
    try { status = desk.status ? await desk.status(p) : await (await fetch('/api/' + who)).json(); } catch (e) { /* shown below */ }
    if (!S.pane || S.pane.kind !== who || !$('term')) return;
    if (!status || !status.available) {
      $('pane-body').classList.remove('terminal');
      $('pane-body').innerHTML = `<div class="callout">No ${who === 'note_session' ? 'session' : who}: ${status && status.why ? esc(status.why) : `the view runs with <code>${desk.flag}</code>, or the server is unreachable`}.</div>`;
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
    const path = desk.socket ? desk.socket(p) : `/api/${who}/term`;
    const ws = new WebSocket(`${proto}://${location.host}${path}?cols=${term.cols}&rows=${term.rows}`);
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
    S.term = { term, ws, ro, close: desk.close ? () => desk.close(p) : null };
    const restart = $('term-restart');
    if (restart) restart.onclick = () => {
      if (ws.readyState === 1 && window.confirm(`Kill the ${who}'s Claude Code and start a fresh one?`)) {
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
    applySorts();
    tickClocks();
  }

  // A desk's pane: the status bar, the chips, the terminal.
  function terminalPane(who, p) {
    const desk = TERMINALS[who];
    const html = `<div class="steward-bar"><span id="term-status">connecting…</span><span class="spacer"></span><button class="mini" id="term-restart" title="kill this Claude Code and start a fresh one">restart</button></div>`
      + '<div class="chips">' + desk.chips.map(([label, say]) => `<button data-type="${esc(say)}">${esc(label)}</button>`).join('') + '</div>'
      + '<div id="term"></div>';
    return [typeof desk.title === 'function' ? desk.title(p) : desk.title, html, (again) => { if (!again || !S.term) mountTerminal(who, p); }, true];
  }
  const kv = (pairs) => '<dl class="kv">' + pairs.filter(([, v]) => v != null && v !== '').map(([k, v]) => `<dt>${esc(k)}</dt><dd>${v}</dd>`).join('') + '</dl>';
  const tag = (text, cls = '') => `<span class="tag ${cls}">${esc(text)}</span>`;
  // A table shown a page at a time. The page is kept on the pane, so a
  // refresh of the data keeps the reader where they were.
  const PAGE_SIZE = 15;
  function pagedTable(p, key, cls, head, rows, row) {
    const pages = Math.max(1, Math.ceil(rows.length / PAGE_SIZE));
    const page = Math.min((p.pages || {})[key] || 0, pages - 1);
    // Sorted by a click on its header: every row is drawn, then ordered, then cut.
    const order = (p.sorts || {})[key];
    const drawn = order ? sortedRows(rows.map(row), order) : rows.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE).map(row);
    const shown = order ? drawn.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE) : drawn;
    let h = `<table class="rows ${cls}" data-paged="${esc(key)}"><tr>${head}</tr>` + shown.join('') + '</table>';
    if (pages > 1) {
      const go = (to, label, off) => `<button class="mini" data-page="${to}"${off ? ' disabled' : ''}>${label}</button>`;
      h += `<div class="pager" data-pager="${esc(key)}">${go(0, '«', page === 0)}${go(page - 1, '‹', page === 0)}`
        + `<span class="muted">${page * PAGE_SIZE + 1}–${page * PAGE_SIZE + shown.length} of ${rows.length} · page ${page + 1} / ${pages}</span>`
        + `${go(page + 1, '›', page === pages - 1)}${go(pages - 1, '»', page === pages - 1)}</div>`;
    }
    return h;
  }
  // ---- Every table sorts on a click of its header: ascending, then descending ----
  // A cell sorts by its `data-sort` when it has one (a date's raw value), else
  // by its text; numbers as numbers, empty cells last either way.
  const sortValue = (cell) => (cell ? (cell.dataset.sort ?? cell.textContent.trim()) : '');
  const asNumber = (text) => {
    const t = String(text).replace(/[$,%\s]/g, '').replace(/(tok|ms|min|s|k)$/i, '');
    return /^-?\d+(\.\d+)?$/.test(t) ? Number(t) : null;
  };
  function compareCells(a, b) {
    const empty = (v) => v === '' || v === '—';
    if (empty(a) || empty(b)) return empty(a) - empty(b);
    const x = asNumber(a);
    const y = asNumber(b);
    if (x !== null && y !== null) return x - y;
    return String(a).localeCompare(String(b), undefined, { numeric: true, sensitivity: 'base' });
  }
  function orderRows(rows, cellOf, order) {
    const keyed = rows.map((r, i) => ({ r, i, v: sortValue(cellOf(r)) }));
    keyed.sort((x, y) => {
      const empty = (v) => v === '' || v === '—';
      if (empty(x.v) !== empty(y.v)) return empty(x.v) - empty(y.v);
      return (order.desc ? -1 : 1) * compareCells(x.v, y.v) || x.i - y.i;
    });
    return keyed.map((k) => k.r);
  }
  // A paged table's rows, as HTML, read back to sort them.
  function sortedRows(html, order) {
    const t = document.createElement('template');
    t.innerHTML = '<table><tbody>' + html.join('') + '</tbody></table>';
    const trs = [...t.content.querySelectorAll('tbody > tr')];
    if (trs.length !== html.length) return html;
    const index = new Map(trs.map((tr, i) => [tr, i]));
    return orderRows(trs, (tr) => tr.cells[order.col], order).map((tr) => html[index.get(tr)]);
  }
  // Every table of the pane: its header marked, and — not paged — its rows
  // put in the order asked, again after each redraw.
  let sortObserver = null;
  function applySorts() {
    const body = $('pane-body');
    if (!S.pane || !body) return;
    const sorts = S.pane.sorts || {};
    if (sortObserver) sortObserver.disconnect();
    body.querySelectorAll('table.rows').forEach((t, i) => {
      const key = t.dataset.paged || 'table-' + i;
      t.dataset.sortKey = key;
      const head = t.rows[0];
      if (!head || !head.querySelector('th')) return;
      const order = sorts[key];
      [...head.cells].forEach((th, c) => {
        th.classList.toggle('sortable', th.textContent.trim() !== '');
        th.classList.toggle('asc', !!order && order.col === c && !order.desc);
        th.classList.toggle('desc', !!order && order.col === c && !!order.desc);
      });
      if (order && !t.dataset.paged) {
        const rows = [...t.rows].slice(1).filter((r) => r.querySelector('td'));
        const parent = rows.length && rows[0].parentNode;
        if (parent) orderRows(rows, (r) => r.cells[order.col], order).forEach((r) => parent.appendChild(r));
      }
    });
    if (!sortObserver) sortObserver = new MutationObserver(() => applySorts());
    sortObserver.observe(body, { childList: true, subtree: true });
  }
  $('pane-body').addEventListener('click', (e) => {
    const th = e.target.closest('table.rows th.sortable');
    if (!th || !S.pane) return;
    const table = th.closest('table');
    const key = table.dataset.sortKey;
    const col = th.cellIndex;
    S.pane.sorts = S.pane.sorts || {};
    const was = S.pane.sorts[key];
    S.pane.sorts[key] = { col, desc: !!was && was.col === col && !was.desc };
    if (table.dataset.paged) {
      S.pane.pages = { ...(S.pane.pages || {}), [key]: 0 };
      renderPane(S.pane, false);
    } else applySorts();
  });
  // A pane's two tabs — what the element is, and who recently worked on it —
  // and its `?`, which says in a sentence what the element is for.
  const paneTabs = (p, purpose, extra = []) => '<div class="tabs">' + [['details', 'Details'], ['recent', 'Recent work'], ...extra].map(([k, label]) => `<button data-tab="${k}" class="${(p.tab || 'details') === k ? 'on' : ''}">${label}</button>`).join('')
    + (purpose ? `<button class="help-btn ${p.help ? 'on' : ''}" data-help title="What is this for?">?</button>` : '') + '</div>'
    + (purpose && p.help ? `<div class="callout purpose">${esc(purpose)}</div>` : '');
  // The runs of a line, newest first, each a link to its agent — opened on
  // `stage` when the list comes from a machine.
  function recentWork(line, runs, stage) {
    if (!runs.length) return '<div class="muted">nobody has worked here yet</div>';
    return '<ul class="issues">' + runs.map((r) => {
      const last = { id: `${line.id}/${r.run_id}`, name: r.name, workflow: line.id, run_id: r.run_id, tokens: r.tokens, active: r.active };
      const open = { kind: 'employee', id: last.id, last, stage: stage || null };
      const when = runWhen(r.run_id);
      return `<li><span class="t"><button class="link" data-pane='${esc(JSON.stringify(open))}'>${esc(r.name)}</button></span><span class="muted">${esc(when)} · ${esc(r.stages.join(' → ') || 'no stage finished')}${r.tokens ? ' · ' + kfmt(r.tokens.total) + ' tok' : ''}</span>${r.active ? liveTag(last.id) : ''}</li>`;
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
  // `14:03:22`, with the day when it is not today — the harness machine's time.
  function when(iso) { return C.when(iso); }

  // A duration, to the second under the hour.
  function fmtDur(ms) {
    if (!(ms >= 0)) return '—';
    const sec = Math.floor(ms / 1000);
    if (sec < 60) return `${sec} s`;
    const m = Math.floor(sec / 60);
    return m >= 60 ? `${Math.floor(m / 60)}h${String(m % 60).padStart(2, '0')}` : `${m} min ${String(sec % 60).padStart(2, '0')} s`;
  }
  const lasted = (from, to) => fmtDur((to ? Date.parse(to) : Date.now()) - Date.parse(from));
  // How long something ran: its duration once over, a clock that ticks while
  // it runs. The ticking text is filled by `tickClocks`, so a pane's html
  // stays the same from one render to the next.
  function clockHtml(from, to, running) {
    if (!from) return '—';
    if (running) return `<span class="elapsed" data-since="${esc(from)}"></span>`;
    return to ? esc(lasted(from, to)) : '—';
  }
  function tickClocks() {
    for (const el of document.querySelectorAll('.elapsed[data-since]')) {
      const text = lasted(el.dataset.since, null);
      if (el.textContent !== text) el.textContent = text;
    }
  }
  setInterval(tickClocks, 1000);
  // A run's time: running for…, or took…. `c` is what its stages say.
  const runClock = (e, c) => {
    const from = (c && c.started_at) || e.since;
    if (!from) return '—';
    return e.active
      ? `running for ${clockHtml(from, null, true)}`
      : `took ${clockHtml(from, (c && c.last_at) || e.last_at, false)}`;
  };
  // ---- a chosen period --------------------------------------------------------
  // `S.range` holds two trace clocks (UTC, `2026-10-08T21:31:21Z`), either
  // null; `S.history` the figures the server recomputed for it.
  const toClock = (local) => C.fromInput(local);
  const toLocal = (clock) => C.toInput(clock);

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
    // Other rows: back to their first page.
    if (S.pane) S.pane.pages = {};
    if (S.range) readHistory(); else if (S.pane && S.pane.kind === 'dashboards') renderPane(S.pane, false);
  }
  function presetRange(key) {
    const ago = (ms) => new Date(Date.now() - ms).toISOString().slice(0, 19) + 'Z';
    // "today" starts at midnight on the harness machine, not in the browser.
    setRange({ from: key === '1h' ? ago(3600e3) : key === '24h' ? ago(86400e3) : C.midnight(), to: null });
  }

  // One read of the limits; the server caches the probe, so asking is cheap.
  function readLimitsNow(force) {
    if (S.limitsRead) return S.limitsRead;
    S.limitsRead = (async () => {
      try {
        const r = await fetch('/api/limits' + (force ? '?force=1' : ''));
        if (r.ok) S.limits = await r.json();
      } catch (e) { /* the panel and the screen keep what they had */ }
      S.limitsRead = null;
      pushPicture();
    })();
    return S.limitsRead;
  }
  async function readLimits(p, force) {
    p.limitsAsked = true; p.limitsBusy = true;
    if (S.pane === p) renderPane(p, false);
    await readLimitsNow(force);
    p.limits = S.limits;
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
    'costs.day': 'Spending per day — a day on the harness machine, midnight to midnight — the last fourteen days the ledger holds.',
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
    'journal.logs': 'The last forty lines of the journal as written, clock in the harness machine\'s time zone: polls (<code>saw</code>, <code>quiet</code>), routes (<code>tick</code>), lanes taken and freed, warnings, and a soft stop (<code>draining</code>, <code>stopped</code>).',
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
      h += section(p, 'costs.last', 'Last paid sessions') + pagedTable(p, 'sessions', '', '<th>when</th><th>task</th><th>stage</th><th class="num">cost</th><th class="num">turns</th><th class="num">min</th><th>outcome</th>', c.last, (r) => `<tr><td data-sort="${esc(r.when)}">${esc(when(r.when))}</td><td>#${esc(r.task)}</td><td>${esc(r.stage)}</td><td class="num">${r.cost_usd == null ? '—' : usd(r.cost_usd)}</td><td class="num">${r.turns ?? '—'}</td><td class="num">${r.duration_ms == null ? '—' : (r.duration_ms / 60000).toFixed(1)}</td><td>${sessionTag(r)}</td></tr>`);
      return h;
    },
    quota(p) {
      const L = p.limits;
      // Not asked yet is reading too: the screen asks the moment it opens.
      const busy = p.limitsBusy || !p.limitsAsked;
      const at = (secs) => when(secs * 1000);
      const gauge = (name, pct, sub) => `<div class="gauge ${pct > 90 ? 'bad' : pct > 70 ? 'warn' : ''}"><div>${esc(name)} · ${pct}% used</div><div class="bar"><span style="width:${Math.min(pct, 100)}%"></span></div><div class="muted">${sub}</div></div>`;
      const resets = (secs) => (secs ? 'resets ' + esc(C.when(secs * 1000)) : 'reset unknown');
      let h = `<div class="limits-bar"><span class="muted">${busy ? 'reading the limits now…' : L ? 'read at ' + esc(at(Math.max(L.claude.at, L.github.at))) : ''}</span><button class="mini" data-limits-read${busy ? ' disabled' : ''}>read again</button></div>`;
      // Claude: the probe's reading, or the one the runs kept when the probe failed.
      h += section(p, 'quota.claude', 'Claude');
      const kept = S.snap.quota;
      const claude = L && L.claude.value ? L.claude.value : null;
      // Tthey kept reading is old by nature: it stands in only when the probe
      // failed, never while it runs.
      const shown = claude || (L && L.claude.error && !busy ? kept : null);
      if (L && L.claude.error && !busy) h += `<div class="callout soon">the probe failed: ${esc(L.claude.error)}${kept ? ' — showing the reading the runs last kept' : ''}</div>`;
      if (shown && !busy) {
        h += '<div class="gauges">' + shown.windows.map((w) => gauge(w.name, Math.round(w.utilization * 100), resets(w.resets_at))).join('') + '</div>';
        h += `<div class="muted limits-from">${claude ? 'read now by a minimal session' : 'kept by the last run that ended'}, ${esc(at(shown.at))}</div>`;
      } else h += `<div class="muted">${busy ? 'reading…' : 'no reading yet'}</div>`;
      // GitHub: free to read, read every time.
      h += section(p, 'quota.github', 'GitHub API');
      if (!L || busy) h += `<div class="muted">${busy ? 'reading…' : 'not read'}</div>`;
      else if (L.github.error) h += `<div class="callout soon">${esc(L.github.error)}</div>`;
      else h += '<div class="gauges">' + L.github.value.map((w) => gauge(w.name, w.limit ? Math.round(100 * w.used / w.limit) : 0, `${w.used} / ${w.limit} requests · ${resets(w.resets_at)}`)).join('') + '</div>';
      return h;
    },
    errors(p) {
      if (S.range && !period()) return '';
      // A stop of the watch itself has no run folder: its logs are the
      // journal, on the Watch loop screen.
      const stopTag = (e) => {
        const cls = e.kind === 'QUOTA' ? 'warn' : e.kind === 'FAILED' ? 'bad' : '';
        return e.workflow === 'watch'
          ? `<button class="tag-link" data-pane='${esc(JSON.stringify({ kind: 'dashboards', focus: 'journal' }))}' title="open the watch's journal">${tag(e.kind, cls)}</button>`
          : runTag(e.kind, cls, { run: e.run, failed: true, name: `${e.workflow} · ${e.kind} · ${e.run}` });
      };
      const rows = period() ? period().errors : S.snap.errors;
      return section(p, 'errors.list', period() ? 'Stops of the period' : 'Last stops') + (rows.length ? pagedTable(p, 'stops', '', '<th>when</th><th>workflow</th><th>kind</th><th>reason</th>', rows, (e) => `<tr><td data-sort="${esc(e.when)}">${esc(when(e.when))}</td><td>${esc(e.workflow)}</td><td>${stopTag(e)}</td><td>${esc(e.reason.slice(0, 240))}${e.reason.length > 240 ? '…' : ''}</td></tr>`) : '<div class="muted">none recorded</div>');
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
        ? pagedTable(p, 'triggers', 'triggers', '<th>when</th><th>what</th><th>on</th><th class="num">lane</th><th>state</th><th class="num">lasted</th><th>detail</th>', j.triggers, (t) => {
            const on = t.issue != null
              ? `<button class="link" data-issue="${t.issue}">#${t.issue}</button> <span class="muted">${esc(issueTitle(t.issue).slice(0, 60))}</span>`
              : esc(t.subject || '');
            const [label, cls] = STATE_TAG[t.state] || [t.state, ''];
            // The state leads to the run's logs: on its error when it failed,
            // at its end otherwise.
            const state = t.workflow
              ? `<button class="tag-link" data-trigger-log='${esc(JSON.stringify({ workflow: t.workflow, issue: t.issue, at: t.at, state: t.state, what: t.what }))}' title="${t.state === 'failed' || t.state === 'killed' || t.state === 'unknown' ? 'open its logs at the error' : 'open its logs at the end'}">${tag(label, cls)}</button>`
              : tag(label, cls);
            return `<tr><td data-sort="${esc(t.at)}">${esc(when(t.at))}</td><td>${esc(t.what)}</td><td>${on}</td><td class="num">${t.lane ?? '—'}</td><td>${state}</td><td class="num">${t.ended_at ? esc(lasted(t.at, t.ended_at)) : t.state === 'running' ? clockHtml(t.at, null, true) : '—'}</td><td class="muted">${esc((t.detail || '').slice(0, 160))}</td></tr>`;
          })
        : `<div class="muted">${P ? 'nothing triggered during the period' : 'nothing triggered since the start of the journal read'}</div>`;
      const logs = P ? P.logs : S.snap.recent;
      h += section(p, 'journal.logs', 'Logs') + (P && P.logs_cut ? '<div class="muted">the period holds more lines: its last ones are shown</div>' : '') + '<pre class="log">' + (logs.length ? logHtml(logs.join('\n')) : '(empty)') + '</pre>';
      return h;
    },
  };
  // ---- the gates ------------------------------------------------------------
  // A station's state as a tag: what the latest run left there.
  const STATION_TAG = { done: ['✓ passed', 'ok'], skipped: ['! skipped', 'warn'], failed: ['✕ halted', 'bad'], active: ['● here', 'acc'], idle: ['idle', ''] };
  const stateTag = (state) => { const [label, cls] = STATION_TAG[state] || [state, '']; return tag(label, cls); };
  const VERDICT_TAG = { pass: ['✓ pass', 'ok'], skip: ['! skip', 'warn'], halt: ['✕ halt', 'bad'] };
  const verdictTag = (v) => { const [label, cls] = VERDICT_TAG[v] || [v || '—', '']; return tag(label, cls); };
  const STATE_CLASS = { done: 'done', skipped: 'skipped', failed: 'human', active: 'ready', idle: 'todo' };
  // `14:03 · 08/10` from a run id, for the lists.
  // A run id is its start in UTC (`20261009-192716`): `15:27 · 10/09` on the harness machine.
  function runWhen(runId) {
    const at = C.ofRunId(runId);
    return at ? C.time(at).slice(0, 5) + ' · ' + C.day(at).slice(5).replace('-', '/') : runId;
  }
  // A gate's own journal: one line per check of its last pass.
  const gateLog = (g) => (g.checks || []).map((c) => `[${c.at}] gate "${g.name}" · ${c.name}: ${c.verdict}${c.reason ? ' — ' + c.reason : ''}`).join('\n');
  // The run a gate last spoke in, as a link to its logs.
  const runLink = (lineId, runId, stage) => {
    if (!runId) return null;
    const open = { kind: 'employee', id: `${lineId}/${runId}`, last: { id: `${lineId}/${runId}`, name: runId, workflow: lineId, run_id: runId }, file: 'run.log', stage: stage || null };
    return `<button class="link" data-pane='${esc(JSON.stringify(open))}'>${esc(runId)} →</button>`;
  };
  // The runs a gate spoke in, newest first, each with its verdict.
  function gateRuns(line, names) {
    const runs = (line.recent_work || []).filter((r) => (r.gates || []).some((g) => names.includes(g.gate)));
    if (!runs.length) return '<div class="muted">no run has reached this gate yet — a run older than the event store leaves no verdict</div>';
    return '<ul class="issues">' + runs.map((r) => {
      const open = { kind: 'employee', id: `${line.id}/${r.run_id}`, last: { id: `${line.id}/${r.run_id}`, name: r.name, workflow: line.id, run_id: r.run_id, tokens: r.tokens, active: r.active }, file: 'run.log' };
      const said = (r.gates || []).filter((g) => names.includes(g.gate)).map((g) => `${names.length > 1 ? esc(g.gate) + ' ' : ''}${verdictTag(g.verdict)}${g.reason ? `<span class="muted"> ${esc(g.reason)}</span>` : ''}`).join('<br>');
      return `<li><span class="t"><button class="link" data-pane='${esc(JSON.stringify(open))}'>${esc(r.name)}</button><div class="muted">${esc(runWhen(r.run_id))}</div></span><span class="said">${said}</span>${r.active ? liveTag(open.id) : ''}</li>`;
    }).join('') + '</ul>';
  }
  // One gate in full: what it checks, what it last said, its checks, its log.
  function gateDetail(line, st, g) {
    let h = `<div class="callout purpose">${esc(g.purpose)}</div>`;
    h += kv([
      ['state', stateTag(g.state)],
      ['last verdict', g.verdict ? verdictTag(g.verdict) : '<span class="muted">never reached</span>'],
      ['reason', g.reason ? esc(g.reason) : null],
      ['when', g.at ? hhmm(g.at) + ' · ' + C.day(g.at) : null],
      ['run', runLink(line.id, g.run_id)],
    ]);
    if (g.checks && g.checks.length) {
      h += '<h3>Checks of its last pass</h3><table class="rows checks"><tr><th>check</th><th>verifies</th><th>verdict</th></tr>'
        + g.checks.map((c) => `<tr><td><code>${esc(c.name)}</code></td><td>${esc(c.purpose)}${c.reason ? `<div class="muted">${esc(c.reason)}</div>` : ''}</td><td>${verdictTag(c.verdict)}</td></tr>`).join('') + '</table>';
      h += '<h3>Its log</h3><pre class="log">' + logHtml(gateLog(g)) + '</pre>';
    } else {
      h += '<div class="muted">No verdict recorded: no run has reached this gate since the event store exists. Its checks are named in the log the first time it runs.</div>';
    }
    return h;
  }
  // A scanner's pane: its gates, each one a click away.
  function gatesPane(p, line, st) {
    const names = st.gates.map((g) => g.name);
    if (p.tab === 'recent') return [st.label, paneTabs(p, st.purpose) + gateRuns(line, names)];
    let h = kv([['line', esc(line.title)], ['kind', `${esc(st.kind)} — ${esc(KIND_TIP[st.kind])}`], ['state', stateTag(st.state)], ['gates', st.gates.length]]);
    h += `<h3>${st.gates.length > 1 ? 'Gates, in the order the product walks through them' : 'Gate'}</h3><ul class="issues gates">` + st.gates.map((g) => {
      const open = { kind: 'gate', line: line.id, station: st.id, id: g.id };
      return `<li><span class="t"><button class="link" data-pane='${esc(JSON.stringify(open))}'>${esc(g.name)}</button><div class="muted">${esc(g.purpose)}</div>${g.reason ? `<div class="muted why">${esc(g.reason)}</div>` : ''}</span>${stateTag(g.state)}</li>`;
    }).join('') + '</ul>';
    if (st.gates.length === 1) h += '<h3>In detail</h3>' + gateDetail(line, st, st.gates[0]);
    h += '<div class="callout">A gate judges and never writes: each of its checks reads the issue, the labels, the ledger or the checkout, and lets the product through, skips the stage, or halts the round. Every verdict is told to the run\'s log and kept in the event store.</div>';
    return [st.label, paneTabs(p, st.purpose) + h];
  }
  // The process graph of one run: a circle per gate, a square per stage,
  // filled by what the run made of it — green ✓, amber !, red ✕.
  function processGraph(lineId, stations) {
    if (!stations || !stations.length) return '<div class="muted">no graph: this run left no stations</div>';
    const step = 84, w = 16 + stations.length * step, h = 72;
    const marks = { done: '✓', skipped: '!', failed: '✕', active: '', idle: '' };
    let svg = `<svg viewBox="0 0 ${w} ${h}" width="${w}" height="${h}" class="process">`;
    stations.forEach((s, i) => {
      const cx = 8 + step / 2 + i * step, cy = 26;
      if (i < stations.length - 1) svg += `<line class="edge ${esc(s.state)}" x1="${cx + 13}" y1="${cy}" x2="${cx + step - 13}" y2="${cy}"/>`;
      const gate = s.kind === 'scanner';
      const open = gate && s.gates && s.gates.length ? { kind: 'station', line: lineId, id: s.id } : { kind: 'station', line: lineId, id: s.id };
      const shape = gate ? `<circle cx="${cx}" cy="${cy}" r="12"/>` : `<rect x="${cx - 12}" y="${cy - 12}" width="24" height="24" rx="4"/>`;
      const label = s.label.length > 14 ? s.label.slice(0, 13) + '…' : s.label;
      svg += `<g class="node ${esc(s.state)} ${gate ? 'gate' : 'stage'}" data-pane='${esc(JSON.stringify(open))}'><title>${esc(s.label)} · ${esc(s.state)}</title>${shape}<text class="mark" x="${cx}" y="${cy + 4.5}" text-anchor="middle">${marks[s.state] || ''}</text><text class="name" x="${cx}" y="${cy + 30}" text-anchor="middle">${esc(label)}</text></g>`;
    });
    return svg + '</svg>';
  }

  const KIND_TIP = {
    scanner: 'gate · a deterministic check',
    builder: 'LLM that writes',
    inspector: 'LLM that reads and judges',
    printer: 'gathers the context',
    arm: 'acts on the code or on GitHub',
  };

  // ---- a robot's prompts -----------------------------------------------------
  // What a paid station sends, shown on its pane and edited there. A save is a
  // draft: nothing runs it until the factory starts again — the harness then
  // applies every draft itself. The wrench in the dock says something waits;
  // a dry-run replays a past run with any version, in a session that cannot act.
  const PR = { byStation: {}, loading: {}, pending: { changes: [], restart: null }, trials: {}, reverted: {}, picks: {} };
  const isRobot = (st) => st.kind === 'builder' || st.kind === 'inspector';
  const PLACEHOLDER = /\{([a-z_][a-z0-9_]*)\}/g;
  const placeholdersOf = (text) => new Set([...String(text).matchAll(PLACEHOLDER)].map((m) => m[1]));
  const savedWhen = (secs) => (secs ? C.stamp(secs * 1000) : '—');
  const firstLine = (text) => { const l = String(text).split('\n').find((x) => x.trim()) || ''; return l.length > 70 ? l.slice(0, 70) + '…' : l; };
  // A declaration, not a constant: the dock draws it before this part of the
  // script has run.
  function wrenchIcon() {
    return '<svg viewBox="0 0 24 24" aria-hidden="true"><path class="shape" d="M21.4 6.2a5.6 5.6 0 0 1-7.3 6.7L6.6 20.4a2.1 2.1 0 0 1-3-3l7.5-7.5a5.6 5.6 0 0 1 6.7-7.3l-3.3 3.3.5 2.6 2.6.5z"/></svg>';
  }

  function loadPrompts(lineId, stId, then) {
    const at = lineId + '/' + stId;
    if (PR.loading[at]) return;
    PR.loading[at] = true;
    fetch(`/api/prompts/${encodeURIComponent(lineId)}/${encodeURIComponent(stId)}`)
      .then(async (r) => { PR.byStation[at] = r.ok ? await r.json() : { error: await r.text() }; })
      .catch(() => { PR.byStation[at] = { error: 'cannot reach the server' }; })
      .finally(() => { PR.loading[at] = false; if (then) then(); });
  }
  // What waits for the next start, and the restart under way: the wrench.
  async function loadPending() {
    try {
      const r = await fetch('/api/pending');
      if (r.ok) PR.pending = await r.json();
    } catch (e) { /* the next poll will say */ }
    S.waiting = PR.pending.changes.length;
    dock();
    if (S.pane && S.pane.kind === 'changes') renderPane(S.pane, true);
  }
  setInterval(loadPending, 5000);
  loadPending();

  function promptDrift(v, text) {
    const wanted = placeholdersOf(v.default);
    const named = placeholdersOf(text);
    const missing = [...wanted].filter((x) => !named.has(x));
    const unknown = [...named].filter((x) => !wanted.has(x));
    const code = (xs) => xs.map((x) => `<code>{${esc(x)}}</code>`).join(' ');
    return (missing.length ? `<div class="callout soon">dropped ${code(missing)} — the stage will no longer pass ${missing.length > 1 ? 'these values' : 'this value'} to the session</div>` : '')
      + (unknown.length ? `<div class="callout soon">${code(unknown)} — nothing fills ${unknown.length > 1 ? 'these' : 'this'}: the session reads ${unknown.length > 1 ? 'them' : 'it'} as written</div>` : '');
  }
  // The past runs a dry-run can replay: the ones that went through this
  // station, else every finished run of the line.
  function replayable(line, st) {
    const done = (line.recent_work || []).filter((r) => !r.active);
    const here = st.stage ? done.filter((r) => r.stages.includes(st.stage)) : [];
    return here.length ? here : done;
  }
  // Every version a dry-run can try, by value: `editor`, `running`, `draft`,
  // `default`, `history:i`, `reverted:i`.
  function versionText(v, card, which) {
    if (which === 'editor') return card.querySelector('.prompt-text').value;
    if (which === 'running') return v.running;
    if (which === 'draft') return v.draft ? v.draft.text : v.running;
    if (which === 'default') return v.default;
    const [list, i] = which.split(':');
    const kept = (list === 'history' ? v.history : list === 'shelved' ? (v.shelved || []) : v.reverted)[Number(i)];
    return kept ? kept.text : null;
  }
  const LIST_NAME = { history: 'edit', reverted: 'reverted', shelved: 'deactivated' };
  const versionName = (which) => (which.includes(':') ? `${LIST_NAME[which.split(':')[0]] || which} ${Number(which.split(':')[1]) + 1}` : which);
  function trialList(key) {
    const list = PR.trials[key];
    if (!list) return '<div class="muted">loading…</div>';
    if (!list.length) return '<div class="muted">No dry-run yet.</div>';
    return '<ul class="prompt-history trials">' + list.map((t) => {
      const head = `${tag(t.state, t.state === 'done' ? 'ok' : t.state === 'failed' ? 'bad' : 'wrench')} <b>${esc(versionName(t.version || '?'))}</b> on ${esc(t.run || '?')} <span class="muted">· ${esc(savedWhen(t.started_at))}</span>`;
      const numbers = t.state === 'done' ? ` <span class="muted">· ${t.duration_ms == null ? '—' : esc(fmtDur(t.duration_ms))} · ${t.cost_usd == null ? '—' : usd(t.cost_usd)} · ${t.turns ?? '—'} turns</span>` : '';
      const body = t.state === 'done' ? `<pre class="md">${esc(t.output || '(empty)')}</pre>` : t.state === 'failed' ? `<div class="callout">${esc(t.reason || 'failed')}</div>` : '<div class="muted">the session is answering…</div>';
      return `<li><details${t.state !== 'running' && t === list[0] ? ' open' : ''}><summary>${head}${numbers}</summary>${body}</details></li>`;
    }).join('') + '</ul>';
  }
  async function loadTrials(key) {
    try {
      const r = await fetch('/api/prompts/trials?key=' + encodeURIComponent(key));
      if (r.ok) PR.trials[key] = await r.json();
    } catch (e) { /* the next poll will say */ }
    const el = document.querySelector(`[data-prompt-key="${CSS.escape(key)}"] .trial-list`);
    if (el) el.innerHTML = trialList(key);
    // While one answers, look again.
    if ((PR.trials[key] || []).some((t) => t.state === 'running')) setTimeout(() => { if (document.querySelector(`[data-prompt-key="${CSS.escape(key)}"]`)) loadTrials(key); }, 4000);
  }
  // One entry of a prompt's history. What runs now is read-only; any other
  // text opens on an editor whose save makes it the draft for the next start.
  // `waiting` marks the draft itself: saving it replaces it, while saving any
  // other entry keeps the waiting draft in the history, deactivated.
  function histEntry(e, runs) {
    const body = e.running
      ? `<pre class="md">${esc(e.text)}</pre>`
      : `<textarea class="hist-text" spellcheck="false">${esc(e.text)}</textarea><div class="prompt-bar"><button class="mini" data-hist-save>save for the next start</button></div>`;
    const tryBtn = `<button class="mini" data-prompt-try="${e.version}" title="dry-run this version on the run picked above"${runs.length ? '' : ' disabled'}>dry-run</button>`;
    return `<li class="hist ${e.kind}"${e.kind === 'waiting' ? ' data-waiting' : ''}><details><summary>${esc(e.when)}${e.tags} <span class="muted">${esc(firstLine(e.text))}</span></summary>${body}</details>${tryBtn}${e.actions}</li>`;
  }
  function promptCard(v, line, st) {
    const next = v.draft ? v.draft.text : v.running;
    const reverted = !!PR.reverted[v.key];
    const runs = replayable(line, st);
    const shelved = v.shelved || [];
    const pick = PR.picks[v.key] || {};
    const option = (value, label) => `<option value="${esc(value)}"${pick.version === value ? ' selected' : ''}>${esc(label)}</option>`;
    const versions = option('editor', "the editor's text") + option('running', 'what runs now') + (v.draft ? option('draft', 'the draft waiting') : '') + option('default', 'the default')
      + v.history.map((h, i) => option('history:' + i, `edit ${i + 1} · ${savedWhen(h.saved_at)}`)).join('')
      + shelved.map((h, i) => option('shelved:' + i, `deactivated ${i + 1} · ${savedWhen(h.at)}`)).join('')
      + v.reverted.map((h, i) => option('reverted:' + i, `reverted ${i + 1} · ${savedWhen(h.saved_at)}`)).join('');
    const runOptions = runs.map((r) => `<option value="${esc(r.run_id)}"${pick.run === r.run_id ? ' selected' : ''}>${esc(r.name)} · ${esc(r.run_id)}</option>`).join('');
    const ranTags = (h) => (h.running ? ' ' + tag('runs now', 'ok') : '') + (h.is_default ? ' ' + tag('default') : '');
    const revertBtn = (at, off) => `<button class="mini" data-prompt-restore="${at}"${off ? ' disabled' : ''}>revert to it</button>`;
    const kept = (list, i, h) => ({ kind: 'kept', version: `${list}:${i}`, at: h.saved_at, when: savedWhen(h.saved_at), text: h.text, running: h.running, tags: ranTags(h), actions: revertBtn(`${list}:${i}`, h.running) });
    let entries;
    if (reverted) {
      entries = v.reverted.map((h, i) => kept('reverted', i, h));
    } else {
      // Newest first: the draft waiting, then the deactivated drafts and the
      // edits that ran, by date; the default closes the list.
      const dated = [
        ...shelved.map((h, i) => ({ kind: 'shelved', version: `shelved:${i}`, at: h.at, when: savedWhen(h.at), text: h.text, running: false, tags: ' ' + tag('deactivated', 'dim') + (h.to_default ? ' ' + tag('default') : ''), actions: `<button class="mini" data-prompt-reactivate="${i}">reactivate</button><button class="mini" data-prompt-delete="${i}">delete</button>` })),
        ...v.history.map((h, i) => kept('history', i, h)),
      ].sort((a, b) => b.at - a.at);
      entries = [
        ...(v.draft ? [{ kind: 'waiting', version: 'draft', at: v.draft.at, when: savedWhen(v.draft.at), text: v.draft.text, running: false, tags: ' ' + tag('waiting for a restart', 'wrench') + (v.draft.to_default ? ' ' + tag('default') : ''), actions: '<button class="mini" data-prompt-deactivate>deactivate</button>' }] : []),
        ...dated,
        { kind: 'default', version: 'default', when: 'default', text: v.default, running: !v.edited, tags: ' <span class="muted">— the workflow\'s own text</span>' + (!v.edited ? ' ' + tag('runs now', 'ok') : ''), actions: revertBtn('default', !v.edited) },
      ];
    }
    const list = entries.length ? entries.map((e) => histEntry(e, runs)).join('') : '<li class="muted">Nothing reverted yet.</li>';
    return `<section class="prompt-card" data-prompt-key="${esc(v.key)}">`
      + `<h3>${esc(v.title)} ${tag(v.edited ? 'edited' : 'default', v.edited ? 'warn' : '')}${v.draft ? ' ' + tag('waiting for a restart', 'wrench') : ''}</h3>`
      + (v.stations.length > 1 ? `<div class="muted">shared by ${v.stations.map(esc).join(', ')} — a change reaches every one of them</div>` : '')
      + (v.placeholders.length ? `<div class="muted">filled in by the stage: ${v.placeholders.map((x) => `<code>{${esc(x)}}</code>`).join(' ')}</div>` : '')
      + `<textarea class="prompt-text" spellcheck="false">${esc(next)}</textarea>`
      + `<div class="prompt-warn">${promptDrift(v, next)}</div>`
      + '<div class="prompt-bar"><button class="mini" data-prompt-save>save for the next start</button><button class="mini" data-prompt-discard>discard changes</button><span class="muted prompt-status"></span></div>'
      + '<h4>Dry-run — replays a past run with a version, in a session that cannot act</h4>'
      + (runs.length
        ? `<div class="prompt-bar"><select class="trial-version">${versions}</select><select class="trial-run">${runOptions}</select><button class="mini" data-prompt-try="">dry-run</button></div>`
        : '<div class="muted">No finished run of this line to replay yet.</div>')
      + `<div class="trial-list">${trialList(v.key)}</div>`
      + `<h4 class="kept-head">History <label class="switch" title="show the last 3 texts a revert replaced"><input type="checkbox" data-prompt-reverted${reverted ? ' checked' : ''}><span class="track"></span> recently reverted</label></h4>`
      + `<ul class="prompt-history">${list}</ul>`
      + '</section>';
  }
  // The prompt tab: one editor per prompt the station sends. The body is kept
  // on a refresh, so a snapshot never wipes what is being typed.
  function promptPane(p, line, st, head) {
    const html = head + '<div class="callout">The harness wraps this text in its own blocks — EXECUTION CONTEXT, SCOPE, the repository configuration — which are not edited here. A save is a draft: it runs once the factory starts again (the wrench, bottom right).</div><div id="prompt-editor"><div class="muted">loading…</div></div>';
    const after = (refresh) => {
      if (refresh) return;
      loadPrompts(line.id, st.id, () => fillEditor(line, st));
    };
    return [st.label, html, after, true];
  }
  function fillEditor(line, st) {
    const el = $('prompt-editor');
    const list = PR.byStation[line.id + '/' + st.id];
    if (!el || !list) return;
    el.innerHTML = list.error ? `<div class="callout">${esc(list.error)}</div>` : list.length > 1 ? promptFolds(list, line, st) : list.map((v) => promptCard(v, line, st)).join('');
    if (!list.error) list.forEach((v) => loadTrials(v.key));
  }
  // Several prompts: a list to scan first, each one unfolding into its
  // editor. What is open stays open across a redraw of the tab.
  PR.open = {};
  function promptFolds(list, line, st) {
    return `<div class="muted">${list.length} prompts — click one to open its editor.</div>` + list.map((v) => `<details class="prompt-fold" data-fold-key="${esc(v.key)}"${PR.open[v.key] ? ' open' : ''}>`
      + `<summary><span class="t">${esc(v.title)}</span>${tag(v.edited ? 'edited' : 'default', v.edited ? 'warn' : '')}${v.draft ? tag('waiting for a restart', 'wrench') : ''}`
      + `<span class="muted first">${esc(firstLine(v.draft ? v.draft.text : v.running))}</span></summary>`
      + promptCard(v, line, st) + '</details>').join('');
  }
  document.addEventListener('toggle', (e) => {
    const fold = e.target.closest && e.target.closest('[data-fold-key]');
    if (fold && fold === e.target) PR.open[fold.dataset.foldKey] = fold.open;
  }, true);
  function shownPrompt(key) {
    for (const list of Object.values(PR.byStation)) {
      if (Array.isArray(list)) { const v = list.find((x) => x.key === key); if (v) return v; }
    }
    return null;
  }
  const paneStation = () => {
    if (!S.pane || S.pane.kind !== 'station') return null;
    const line = S.snap.lines.find((l) => l.id === S.pane.line);
    const st = line && line.stations.find((s) => s.id === S.pane.id);
    return st ? [line, st] : null;
  };
  function redrawCard(v, status) {
    const here = paneStation();
    const card = document.querySelector(`[data-prompt-key="${CSS.escape(v.key)}"]`);
    if (!here || !card) return;
    card.outerHTML = promptCard(v, here[0], here[1]);
    const fresh = document.querySelector(`[data-prompt-key="${CSS.escape(v.key)}"] .prompt-status`);
    if (fresh && status) fresh.textContent = status;
  }
  async function sendPrompt(path, body, card, said) {
    const status = card.querySelector('.prompt-status');
    status.textContent = 'saving…';
    try {
      const r = await fetch('/api/prompts/' + path, { method: 'POST', body: JSON.stringify(body), headers: { 'content-type': 'application/json' } });
      if (!r.ok) { status.textContent = await r.text(); return; }
      const v = await r.json();
      // Every station that shows this prompt sees the change.
      for (const list of Object.values(PR.byStation)) {
        if (Array.isArray(list)) { const i = list.findIndex((x) => x.key === v.key); if (i >= 0) list[i] = v; }
      }
      redrawCard(v, said);
      loadPending();
    } catch (err) { status.textContent = 'cannot reach the server'; }
  }
  async function tryPrompt(card, which) {
    const key = card.dataset.promptKey;
    const v = shownPrompt(key);
    const here = paneStation();
    const status = card.querySelector('.prompt-status');
    const runEl = card.querySelector('.trial-run');
    if (!v || !here || !runEl) return;
    const version = which || card.querySelector('.trial-version').value;
    const text = versionText(v, card, version);
    if (!text) { status.textContent = 'nothing kept there'; return; }
    PR.picks[key] = { version, run: runEl.value };
    status.textContent = 'starting the dry-run…';
    try {
      const r = await fetch('/api/prompts/trial', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ key, version, text, workflow: here[0].id, run: runEl.value }) });
      status.textContent = r.ok ? 'dry-run started — its answer lands below' : await r.text();
    } catch (err) { status.textContent = 'cannot reach the server'; }
    loadTrials(key);
  }
  $('pane-body').addEventListener('click', (e) => {
    const card = e.target.closest('[data-prompt-key]');
    if (!card) return;
    const key = card.dataset.promptKey;
    if (e.target.closest('[data-prompt-save]')) { sendPrompt('edit', { key, text: card.querySelector('.prompt-text').value, shelve_waiting: false }, card, 'saved — it runs once the factory starts again'); return; }
    const histSave = e.target.closest('[data-hist-save]');
    if (histSave) {
      const entry = histSave.closest('li.hist');
      sendPrompt('edit', { key, text: entry.querySelector('.hist-text').value, shelve_waiting: !entry.hasAttribute('data-waiting') }, card, 'saved — it runs once the factory starts again');
      return;
    }
    if (e.target.closest('[data-prompt-deactivate]')) { sendPrompt('deactivate', { key }, card, 'deactivated — the factory keeps what runs; it stays in the history'); return; }
    const again = e.target.closest('[data-prompt-reactivate]');
    if (again) { sendPrompt('reactivate', { key, index: Number(again.dataset.promptReactivate) }, card, 'reactivated — it runs once the factory starts again'); return; }
    const drop = e.target.closest('[data-prompt-delete]');
    if (drop) { sendPrompt('discard', { key, index: Number(drop.dataset.promptDelete) }, card, 'deleted from the history'); return; }
    if (e.target.closest('[data-prompt-discard]')) {
      const v = shownPrompt(key);
      if (v) { const next = v.draft ? v.draft.text : v.running; card.querySelector('.prompt-text').value = next; card.querySelector('.prompt-warn').innerHTML = promptDrift(v, next); card.querySelector('.prompt-status').textContent = ''; }
      return;
    }
    const attempt = e.target.closest('[data-prompt-try]');
    if (attempt) { tryPrompt(card, attempt.dataset.promptTry); return; }
    const back = e.target.closest('[data-prompt-restore]');
    if (back) {
      const at = back.dataset.promptRestore;
      const [from, index] = at === 'default' ? ['default', 0] : at.split(':');
      sendPrompt('restore', { key, from, index: Number(index) }, card, 'revert saved — it runs once the factory starts again');
    }
  });
  $('pane-body').addEventListener('change', (e) => {
    const card = e.target.closest('[data-prompt-key]');
    if (!card) return;
    if (e.target.matches('[data-prompt-reverted]')) {
      PR.reverted[card.dataset.promptKey] = e.target.checked;
      const v = shownPrompt(card.dataset.promptKey);
      if (v) redrawCard(v);
    } else if (e.target.matches('.trial-version, .trial-run')) {
      PR.picks[card.dataset.promptKey] = { version: card.querySelector('.trial-version').value, run: card.querySelector('.trial-run').value };
    }
  });
  $('pane-body').addEventListener('input', (e) => {
    if (!e.target.matches('.prompt-text')) return;
    const card = e.target.closest('[data-prompt-key]');
    const v = card && shownPrompt(card.dataset.promptKey);
    if (!v) return;
    const next = v.draft ? v.draft.text : v.running;
    card.querySelector('.prompt-warn').innerHTML = promptDrift(v, e.target.value);
    card.querySelector('.prompt-status').textContent = e.target.value === next ? '' : 'unsaved changes';
  });
  // The wrench's pane: every change waiting, each leading to its machine, and
  // the restart that applies them.
  function changesPane() {
    const { changes, restart } = PR.pending;
    const running = !!(P.status && P.status.running);
    let h = '<div class="callout">A change made in the view runs only once the factory starts again: the harness applies the waiting changes itself when the watch starts.</div>';
    h += changes.length
      ? '<ul class="issues">' + changes.map((c) => {
        const at = c.stations.map(([line, station]) => `<button class="link" data-pane='${esc(JSON.stringify({ kind: 'station', line, id: station, tab: 'prompt' }))}'>${esc(line)} · ${esc(station)}</button>`).join(', ');
        return `<li><span class="n">${esc(c.kind)}</span><span class="t">${esc(c.title)} <span class="muted">on</span> ${at}</span><span class="muted">${esc(savedWhen(c.at))}</span></li>`;
      }).join('') + '</ul>'
      : '<div class="muted">Nothing waits: the factory runs every change made so far.</div>';
    if (restart) h += `<div class="callout ${restart.state === 'failed' ? '' : 'wrench'}">${esc(restart.state)} · ${esc(restart.message)} <span class="muted">(${esc(savedWhen(restart.since))})</span></div>`;
    const draining = restart && restart.state === 'draining';
    h += `<div class="prompt-bar"><button class="mini" data-restart${draining || !changes.length ? ' disabled' : ''}>${running ? 'restart the factory' : 'start the factory'}</button><span class="muted">${running ? 'a soft stop — the running tasks finish — then a start as soon as the watch is gone' : 'the watch is off: a start applies the changes'} · ${P.lanes} lane${P.lanes > 1 ? 's' : ''}</span><span class="muted restart-status"></span></div>`;
    return h;
  }
  $('pane-body').addEventListener('click', async (e) => {
    const button = e.target.closest('[data-restart]');
    if (!button) return;
    button.disabled = true;
    const status = document.querySelector('.restart-status');
    try {
      const r = await fetch('/api/restart?lanes=' + P.lanes, { method: 'POST' });
      const said = r.ok ? (await r.json()).message : await r.text();
      if (status) status.textContent = said;
    } catch (err) { if (status) status.textContent = 'cannot reach the server'; }
    loadPending();
  });

  // ---- The capabilities list: every Capability and DataCapability ------------------
  // Read once from /api/capabilities, then searched, sorted, filtered and
  // paged here: a keystroke redraws the list only, never the search box.
  const CAP_STATUS = {
    available: ['available', 'on main'],
    building: ['building', 'on a branch, not on main yet'],
    planned: ['planned', 'declared by an open task, on no branch yet'],
    bug: ['bug', 'an open issue labelled bug names it'],
  };
  const CAP_ORDER = { available: 0, building: 1, planned: 2, bug: 3 };
  const CAP_SORTS = {
    name: ['name', (a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' })],
    status: ['availability', (a, b) => CAP_ORDER[a.status] - CAP_ORDER[b.status] || a.name.localeCompare(b.name, undefined, { sensitivity: 'base' })],
    modified: ['last modified', (a, b) => String(b.modified || '').localeCompare(String(a.modified || ''))],
    created: ['created', (a, b) => String(b.created || '').localeCompare(String(a.created || ''))],
  };
  // One reading per branch shown, '' for every branch at once; S.caps is the
  // one the open pane shows.
  S.caps = null;
  S.capsBy = {};
  const capDay = (iso) => (iso ? new Date(iso).toLocaleDateString([], { year: 'numeric', month: 'short', day: 'numeric' }) : '—');
  const capDot = (status) => `<span class="cap-dot ${esc(status)}" title="${esc(CAP_STATUS[status][1])}"></span>`;
  const capTag = (status) => `<span class="tag cap-${esc(status)}">${esc(CAP_STATUS[status][0])}</span>`;
  const capKind = (k) => (k === 'data_capability' ? 'data capability' : 'capability');
  async function loadCaps(fresh, branch = '') {
    S.capsBusy = true;
    let got;
    try {
      const ask = new URLSearchParams();
      if (fresh) ask.set('fresh', 'true');
      if (branch) ask.set('branch', branch);
      const r = await fetch('/api/capabilities' + (ask.toString() ? '?' + ask : ''));
      got = r.ok ? { at: Date.now(), ...(await r.json()) } : { at: Date.now(), error: await r.text(), entries: [], branches: [] };
    } catch (err) { got = { at: Date.now(), error: 'cannot reach the server', entries: [], branches: [] }; }
    S.capsBy[branch] = got;
    S.capsBusy = false;
    if (!S.pane || (S.pane.branch || '') !== branch) {
      // The human switched while this read: the branch now shown is read next.
      const now = S.pane && S.pane.kind === 'capabilities' ? S.pane.branch || '' : null;
      if (now !== null && !S.capsBy[now]) loadCaps(false, now);
      return;
    }
    S.caps = got;
    // The list alone is redrawn: a search being typed keeps its box.
    const list = $('caps-list');
    if (S.pane && S.pane.kind === 'capabilities' && list) {
      list.innerHTML = capList(S.pane);
      const switcher = $('caps-switch');
      if (switcher) switcher.innerHTML = capSwitch(S.pane);
      const again = document.querySelector('[data-cap-refresh]');
      if (again) again.textContent = '↻';
    } else if (S.pane && S.pane.kind === 'capability') renderPane(S.pane, false);
  }
  // The rows the search, the filters and the sort leave, in order.
  function capRows(p) {
    const q = (p.q || '').trim().toLowerCase();
    const off = new Set(p.hidden || []);
    const rows = (S.caps ? S.caps.entries : []).filter((e) => !off.has(e.status)
      && (!p.unit || p.unit === e.kind)
      && (!q || e.name.toLowerCase().includes(q) || e.id.toLowerCase().includes(q)));
    const [, by] = CAP_SORTS[p.sort] || CAP_SORTS.name;
    rows.sort(by);
    if (p.desc) rows.reverse();
    return rows;
  }
  function capList(p) {
    if (!S.caps) return '<div class="muted"><span class="spin"></span>reading the product\'s branches…</div>';
    if (S.caps.error) return `<div class="callout">${esc(S.caps.error)}</div>`;
    const rows = capRows(p);
    const picked = new Set(p.pick || []);
    const head = (p.fusion ? '<th></th>' : '') + '<th></th><th>name</th><th>kind</th><th>system</th><th>status</th><th>created</th><th>modified</th>';
    const body = rows.length
      ? pagedTable(p, 'caps', 'caps', head, rows, (e) => `<tr class="cap-row ${esc(e.status)}${picked.has(e.id) ? ' picked' : ''}" data-cap="${esc(e.id)}">`
        + (p.fusion ? `<td><input type="checkbox" data-cap-pick="${esc(e.id)}" ${picked.has(e.id) ? 'checked' : ''} aria-label="pick ${esc(e.name)} for the fusion"></td>` : '')
        + `<td>${capDot(e.status)}</td><td><b>${esc(e.name)}</b><div class="muted">${esc(e.id)}</div></td>`
        + `<td>${esc(capKind(e.kind))}</td><td>${esc(e.system || '—')}</td><td>${capTag(e.status)}</td>`
        + `<td data-sort="${esc(e.created || '')}">${esc(capDay(e.created))}</td><td data-sort="${esc(e.modified || '')}">${esc(capDay(e.modified))}</td></tr>`)
      : `<div class="muted">${(p.q || '').trim() ? `nothing named “${esc(p.q.trim())}”` : 'no unit here'}</div>`;
    const count = S.caps.entries.reduce((n, e) => ({ ...n, [e.status]: (n[e.status] || 0) + 1 }), {});
    const legend = '<div class="cap-legend">' + Object.keys(CAP_STATUS).map((s) => `<button class="mini ${(p.hidden || []).includes(s) ? 'off' : ''}" data-cap-status="${s}" title="${esc(CAP_STATUS[s][1])} — click to hide or show">${capDot(s)}${esc(CAP_STATUS[s][0])} · ${count[s] || 0}</button>`).join('') + '</div>';
    const warn = (S.caps.warnings || []).length ? `<div class="callout">${S.caps.warnings.map(esc).join('<br>')}</div>` : '';
    const read = S.caps.on ? `the units on ${esc(S.caps.on)}, as that branch has them` : `${S.caps.branches.length} branch(es) read`;
    return legend + body + warn + `<div class="muted cap-foot">${read} · ${esc(when(new Date(S.caps.at).toISOString()))}</div>`;
  }
  function fusionBar(p) {
    if (!p.fusion) return '';
    const pick = p.pick || [];
    const name = (id) => { const e = S.caps && S.caps.entries.find((x) => x.id === id); return e ? e.name : id; };
    const ready = pick.length === 2;
    return `<div class="cap-fusion-bar">${pick.length ? pick.map((id) => `<span class="tag acc">${esc(name(id))}</span>`).join('⇄ ') : '<span class="muted">pick two units in the list</span>'}`
      + `<span class="spacer"></span><button class="act primary" data-cap-fuse ${ready ? '' : 'disabled'}>test the fusion</button></div><div id="caps-fusion">${fusionResult(p)}</div>`;
  }
  const VERDICT = { possible: ['possible', 'ok'], adaptable: ['possible with an adapted contract', 'warn'], impossible: ['not possible', 'bad'] };
  function fusionResult(p) {
    const f = p.fused;
    if (!f) return '';
    if (f.error && !f.comparison) return `<div class="callout">${esc(f.error)}</div>`;
    const c = f.comparison;
    const [word, cls] = VERDICT[c.verdict];
    let h = `<h3>Fusion of ${esc(c.a)} and ${esc(c.b)}: ${tag(word, cls)}</h3>`;
    h += '<div class="muted">The tooling compared the manifests; the model only words its answer from that comparison.</div>';
    if (c.differences.length) {
      h += '<table class="rows"><tr><th>aspect</th><th>' + esc(c.a) + '</th><th>' + esc(c.b) + '</th><th></th></tr>'
        + c.differences.map((d) => `<tr><td>${esc(d.aspect)}</td><td>${esc(d.a)}</td><td>${esc(d.b)}</td><td>${tag(d.weight === 'blocks' ? 'blocks' : d.weight === 'adapts' ? 'adapts' : 'note', d.weight === 'blocks' ? 'bad' : d.weight === 'adapts' ? 'warn' : '')}<div class="muted">${esc(d.why)}</div></td></tr>`).join('') + '</table>';
    }
    if (c.same.length) h += '<p><b>Identical:</b> ' + c.same.map((s) => tag(s, 'ok')).join('') + '</p>';
    if (c.callers.length) h += '<p><b>Micro-UIs to repoint:</b> ' + c.callers.map((s) => tag(s)).join('') + '</p>';
    h += '<h3>AI</h3>' + (f.answer ? `<div class="md cap-ai">${md(f.answer)}</div>` : f.asking ? '<div class="muted"><span class="spin"></span>asking the model…</div>' : `<div class="muted">${esc(f.error || 'no answer')}</div>`);
    return h;
  }
  async function fuse(p) {
    const [a, b] = p.pick || [];
    const ask = (ai) => fetch('/api/capabilities/fusion', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ a, b, ai, branch: p.branch || null }) });
    const show = () => { const el = $('caps-fusion'); if (el && S.pane === p) el.innerHTML = fusionResult(p); };
    try {
      const r = await ask(false);
      if (!r.ok) { p.fused = { error: await r.text() }; show(); return; }
      p.fused = { ...(await r.json()), asking: true };
      show();
      const ai = await ask(true);
      const got = ai.ok ? await ai.json() : { error: await ai.text() };
      if ((p.pick || []).join() !== [a, b].join()) return;
      p.fused = { ...p.fused, answer: got.answer, error: got.error, asking: false };
    } catch (err) { p.fused = { ...(p.fused || {}), asking: false, error: 'cannot reach the server' }; }
    show();
  }
  // The branches the list can be shown on, as the data pane offers them:
  // every branch at once, main, the integration branch, a milestone's.
  const featureLabel = (b) => b.replace(/^milestone\/(\d+)-?/, '#$1 ').replace(/-/g, ' ');
  function capSwitch(p) {
    const known = ((S.capsBy[''] || S.caps || {}).branches) || [];
    const features = known.filter((b) => b.startsWith('milestone/'));
    const on = p.branch || '';
    const integration = S.snap.project.integration_branch || 'main_agent';
    const button = (b, label, tip) => `<button type="button" class="${on === b ? 'on' : ''}" data-cap-branch="${esc(b)}" title="${esc(tip)}">${esc(label)}</button>`;
    const featureOn = on.startsWith('milestone/');
    let h = '<div class="dm-branches">' + button('', 'all', 'every branch at once: the colour says where each unit stands')
      + button('main', 'main', 'what a human released') + button(integration, integration, 'the agents\' version')
      + `<button type="button" class="${featureOn ? 'on' : ''}" data-cap-features ${features.length ? '' : 'disabled'} aria-haspopup="menu" title="a milestone's branch">${featureOn ? esc(featureLabel(on)) : 'feature'} ▾</button></div>`;
    if (p.featureMenu && features.length) {
      h += '<div class="cap-features" role="menu">' + features.map((b) => `<button type="button" role="menuitem" class="${on === b ? 'on' : ''}" data-cap-branch="${esc(b)}" title="${esc(b)}">${esc(featureLabel(b))}</button>`).join('') + '</div>';
    }
    return h;
  }
  function capsPane(p, refresh) {
    S.caps = S.capsBy[p.branch || ''] || null;
    const sort = p.sort || 'name';
    const bar = '<div class="cap-bar">'
      + `<input type="search" class="cap-search" data-cap-q placeholder="search by name…" value="${esc(p.q || '')}" aria-label="search the capabilities by name">`
      + '<select data-cap-sort aria-label="sort">' + Object.entries(CAP_SORTS).map(([k, [label]]) => `<option value="${k}" ${k === sort ? 'selected' : ''}>sort: ${label}</option>`).join('') + '</select>'
      + `<button class="mini" data-cap-desc title="reverse the order">${p.desc ? '↑' : '↓'}</button>`
      + '<select data-cap-kind aria-label="kind"><option value="">all kinds</option>' + ['capability', 'data_capability'].map((k) => `<option value="${k}" ${p.unit === k ? 'selected' : ''}>${capKind(k)}</option>`).join('') + '</select>'
      + `<button class="mini ${p.fusion ? 'on' : ''}" data-cap-fusion title="pick two units and ask whether they could be one">⚭ fusion</button>`
      + `<button class="mini" data-cap-refresh title="fetch the product's branches again">${S.capsBusy ? '<span class="spin"></span>' : '↻'}</button></div>`;
    const html = `<div class="cap-switch" id="caps-switch">${capSwitch(p)}</div>` + bar + fusionBar(p) + `<div id="caps-list">${capList(p)}</div>`;
    const after = (again) => {
      if (again || S.capsBusy) return;
      if (!S.caps || Date.now() - S.caps.at > 60000) loadCaps(false, p.branch || '');
      // The feature menu lists every branch: read them once when shown on one.
      else if (p.branch && !S.capsBy['']) loadCaps(false, '').then(() => { const sw = $('caps-switch'); if (sw && S.pane === p) sw.innerHTML = capSwitch(p); });
    };
    return ['Capabilities list · ' + S.snap.project.name, html, after, true];
  }
  // A branch of a unit, linked to its folder on that branch on GitHub — a
  // plain tag when the view has no GitHub target.
  function capBranch(e, branch) {
    const label = tag(branch, branch === 'main' ? 'ok' : '');
    const repo = S.snap.project.url;
    if (!repo) return label;
    const dir = e.path ? e.path.split('/').slice(0, -1).join('/') : '';
    const href = `${repo}/tree/${encodeURI(branch)}${dir ? '/' + encodeURI(dir) : ''}`;
    return `<a class="cap-branch" href="${esc(href)}" target="_blank" rel="noopener" title="open ${esc(dir || 'the repository')} on ${esc(branch)} on GitHub">${label}</a>`;
  }
  function capPane(p) {
    S.caps = S.capsBy[p.branch || ''] || null;
    const e = S.caps && S.caps.entries.find((x) => x.id === p.id);
    if (!e) return ['Capability', S.caps ? '<div class="callout">not in the list any more</div>' : '<div class="muted">reading…</div>', () => { if (!S.caps && !S.capsBusy) loadCaps(false, p.branch || ''); }];
    const c = e.contract;
    const fields = (m) => (Object.keys(m || {}).length ? '<table class="rows"><tr><th>field</th><th>type</th></tr>' + Object.entries(m).map(([k, v]) => `<tr><td>${esc(k)}</td><td>${esc(v)}</td></tr>`).join('') + '</table>' : '<div class="muted">none</div>');
    let h = kv([
      ['as on', p.branch ? tag(p.branch, 'acc') : null],
      ['status', capTag(e.status) + `<span class="muted">${esc(CAP_STATUS[e.status][1])}</span>`],
      ['kind', esc(capKind(e.kind))],
      ['system', esc(e.system || '—')],
      ['id', `<code>${esc(e.id)}</code>`],
      ['version', e.version ? esc(e.version) : null],
      ['created', esc(when(e.created))],
      ['modified', esc(when(e.modified))],
      ['manifest', e.path ? `<code>${esc(e.path)}</code>` : null],
      ['branches', e.branches.map((b) => capBranch(e, b)).join('') || null],
      ['issues', e.issues.map((i) => `<button class="link" data-issue="${i.number}">#${i.number} ${esc(i.title)}</button> ${i.bug ? tag('bug', 'bad') : ''}`).join('<br>') || null],
    ]);
    h += `<h3>Role</h3><p>${esc(e.role || '—')}</p>`;
    h += '<h3>Interface contracts</h3>';
    const ui = e.used_by.length
      ? '<table class="rows"><tr><th>Micro-UI</th><th>how</th><th>props it hands</th></tr>' + e.used_by.map((u) => `<tr><td>${esc(u.system)}.${esc(u.micro_ui)}</td><td>${u.how === 'reads' ? 'reads' : 'triggers'}</td><td>${Object.entries(u.props).map(([k, v]) => esc(`${k}: ${v}`)).join(', ') || '—'}</td></tr>`).join('') + '</table>'
      : '<div class="muted">no Micro-UI calls it</div>';
    h += `<h4>From the UI → ${e.kind === 'data_capability' ? 'payload' : 'input'}</h4>` + fields(c.input) + ui;
    if (e.kind === 'data_capability') {
      h += '<h4>To the data → DataGuard</h4>' + kv([
        ['effect', c.effect ? tag(c.effect, c.effect === 'insert' ? 'ok' : 'warn') : null],
        ['aggregate', c.target ? `<code>${esc(c.target)}</code>` : null],
        ['touches', c.touches.map((t) => tag(t)).join('') || 'nothing existing'],
        ['mode', c.mode ? esc(c.mode) : null],
        ['permissions', c.permissions.map((t) => tag(t)).join('') || null],
        ['callable by', c.callable_by.map((t) => tag(t)).join('') || null],
      ]);
    } else {
      h += '<h4>To the dataview → persisted queries</h4>' + (c.reads.length ? '<ul class="issues">' + c.reads.map((r) => `<li><code>${esc(r)}</code></li>`).join('') + '</ul>' : '<div class="muted">none declared</div>');
      h += '<h4>Returns</h4>' + kv([['output', c.output ? `<code>${esc(c.output)}</code>` : '—'], ['implements', c.implements ? tag(c.implements, 'acc') : null]]);
    }
    h += '<h3>Business rules</h3>' + (e.rules.length
      ? '<ul class="cap-rules">' + e.rules.map((r) => `<li><b>${esc(r.id)}</b> <span class="muted">· ${esc(r.from)}</span><div>${esc(r.text)}</div></li>`).join('') + '</ul>'
      : `<div class="muted">${e.kind === 'data_capability' ? 'no invariant declared' : 'no Concept implemented: its rules live in its code'}</div>`);
    return [`${e.name} · ${capKind(e.kind)}`, h];
  }
  $('pane-body').addEventListener('input', (e) => {
    if (!e.target.matches('[data-cap-q]') || !S.pane) return;
    S.pane.q = e.target.value;
    S.pane.pages = {};
    const list = $('caps-list');
    if (list) list.innerHTML = capList(S.pane);
  });
  $('pane-body').addEventListener('change', (e) => {
    if (!S.pane || S.pane.kind !== 'capabilities') return;
    if (e.target.matches('[data-cap-sort]')) { S.pane.sort = e.target.value; S.pane.pages = {}; if (S.pane.sorts) delete S.pane.sorts.caps; renderPane(S.pane, false); }
    if (e.target.matches('[data-cap-kind]')) { S.pane.unit = e.target.value; S.pane.pages = {}; renderPane(S.pane, false); }
  });
  // Clicks on the list: filters, fusion, a row's detail. Runs before the
  // generic handler, which still turns the pages.
  $('pane-body').addEventListener('click', (e) => {
    const p = S.pane;
    if (!p || p.kind !== 'capabilities') return;
    const hit = (sel) => e.target.closest(sel);
    let b;
    if ((b = hit('[data-cap-status]'))) {
      const off = new Set(p.hidden || []);
      if (off.has(b.dataset.capStatus)) off.delete(b.dataset.capStatus); else off.add(b.dataset.capStatus);
      p.hidden = [...off]; p.pages = {};
    } else if (hit('[data-cap-desc]')) p.desc = !p.desc;
    else if (hit('[data-cap-fusion]')) { p.fusion = !p.fusion; p.pick = []; p.fused = null; }
    else if (hit('[data-cap-refresh]')) { if (!S.capsBusy) { S.caps = null; S.capsBy = {}; loadCaps(true, p.branch || ''); } }
    else if ((b = hit('[data-cap-branch]'))) {
      p.branch = b.dataset.capBranch; p.featureMenu = false;
      p.pages = {}; p.pick = []; p.fused = null;
    } else if (hit('[data-cap-features]')) p.featureMenu = !p.featureMenu;
    else if ((b = hit('[data-cap-pick]'))) {
      const id = b.dataset.capPick;
      let pick = (p.pick || []).filter((x) => x !== id);
      if (b.checked) pick = [...pick, id].slice(-2);
      p.pick = pick; p.fused = null;
    } else if (hit('[data-cap-fuse]')) { fuse(p); return; }
    else if ((b = hit('[data-cap]')) && p.fusion) {
      // In fusion mode a row is picked, not opened.
      const id = b.dataset.cap;
      const was = (p.pick || []).includes(id);
      p.pick = was ? p.pick.filter((x) => x !== id) : [...(p.pick || []), id].slice(-2);
      p.fused = null;
    } else if ((b = hit('[data-cap]'))) { openPane({ kind: 'capability', id: b.dataset.cap, branch: p.branch || '' }); return; }
    else return;
    e.stopPropagation();
    renderPane(p, false);
  }, true);

  const PANES = {
    changes() { return ['Changes waiting for a restart', changesPane()]; },

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
        ['works on', worksOn(e) || null],
        ['tokens', fmtTokens(e.tokens)],
        ['time', `<span id="emp-clock">${runClock(e, p.clock)}</span>`],
        ['stage', `${esc(e.stage || '—')} ${e.model ? tag(e.model, e.model) : ''}`],
        ['task', esc(e.task || '—')], ['milestone', esc(e.milestone || '—')], ['round', esc(e.round || '—')],
        ['since', e.since ? `${hhmm(e.since)} · last write ${fmtAge(e.age_secs)} ago` : null],
        ['status', e.active ? tag('at work', 'ok') : tag('clocked out', '')],
        ['doctor', p.sick ? doctorButton(e) : null],
      ]);
      // The logs are read machine by machine: one button per stage the run
      // went through (its session in session.log), then the run's own files.
      const html = `<div id="emp-head">${head}</div><h3>Its process</h3><div class="graph" id="emp-graph"><span class="muted">loading…</span></div><h3>Machines it went through</h3><div class="tabs" id="emp-machines"><span class="muted">loading…</span></div><pre class="log" id="live-log">loading…</pre>`;
      const after = (isRefresh) => {
        if (isRefresh) { const h = $('emp-head'); if (h) h.innerHTML = head; return; }
        if (p.sick == null) sickness(p, e);
        // The process graph: a circle per gate, a square per stage, coloured
        // by what this run made of each — refreshed with the log, redrawn
        // only when it changed.
        const graph = async () => {
          try {
            const g = $('emp-graph');
            if (!g) return;
            const r = await fetch(`/api/runs/${e.workflow}/${e.run_id}/graph`);
            const data = r.ok ? await r.json() : { stations: [] };
            const html = processGraph(e.workflow, data.stations);
            if (html !== p.graphHtml) { g.innerHTML = html; p.graphHtml = html; }
          } catch (err) { /* the next pull will say */ }
        };
        const pull = async () => {
          graph();
          try {
            const r = await fetch(`/api/runs/${e.workflow}/${e.run_id}/stages`);
            const run = r.ok ? await r.json() : {};
            const stages = run.stages || [];
            const pre = $('live-log');
            const bar = $('emp-machines');
            if (!pre || !bar) return;
            const now = S.snap.employees.find((x) => x.id === p.id) || p.last || e;
            p.clock = { started_at: run.started_at, last_at: run.last_at };
            const clock = $('emp-clock');
            const clockText = runClock(now, p.clock);
            if (clock && p.clockHtml !== clockText) { clock.innerHTML = clockText; p.clockHtml = clockText; }
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
            // Each machine's time: its session's, ticking while it runs.
            const machineTime = (x, i) => {
              if (!x.opened_at) return '';
              if (x.closed_at) return ` <small>· ${esc(lasted(x.opened_at, x.closed_at))}</small>`;
              return now.active && i === last ? ` <small>· ${clockHtml(x.opened_at, null, true)}</small>` : '';
            };
            const barHtml = (stages.length ? '' : '<span class="muted">no machine reached — this run stopped before it opened a session; its run.log says why</span> ')
              + labels.map((label, i) => `<button data-stage-idx="${i}" class="${!p.file && i === p.stageIdx ? 'on' : ''}" title="${e.active && i === stages.length - 1 ? 'where the agent is now — followed live' : 'a machine it went through'}">${e.active && i === stages.length - 1 ? '● ' : ''}${esc(label)}${machineTime(stages[i], i)}</button>`).join('')
              + ['run.log', 'prompts.md'].map((f) => `<button data-file="${f}" class="muted ${f === p.file ? 'on' : ''}">${f}</button>`).join('');
            // Rewritten only when it changed: a button replaced between the
            // press and the release of a click swallows that click.
            if (barHtml !== p.barHtml) { bar.innerHTML = barHtml; p.barHtml = barHtml; }
            tickClocks();
            let text;
            if (p.file) {
              const f = await fetch(`/api/runs/${e.workflow}/${e.run_id}/${p.file}?bytes=${p.jumpTo ? 200000 : 14000}`);
              text = f.ok ? (await f.text()) || '(empty)' : `(${f.status}) ${await f.text()}`;
            } else {
              text = (stages[p.stageIdx] && stages[p.stageIdx].text) || '(empty)';
            }
            const atBottom = pre.scrollHeight - pre.scrollTop - pre.clientHeight < 40;
            if (p.jumpTo && !p.jumped && p.file) {
              // Opened from a trigger: land on the error, in its context, or
              // on the end. A run older than its halt line keeps its error in
              // session.log — looked for there before giving up on it.
              const lines = text.replace(/\s+$/, '').split('\n');
              let at = -1;
              if (p.jumpTo === 'error') {
                for (let i = lines.length - 1; i >= 0; i -= 1) if (ERROR_LINE.test(lines[i])) { at = i; break; }
                if (at < 0 && p.file === 'run.log') { p.file = 'session.log'; p.barHtml = null; pull(); return; }
              }
              if (at < 0) at = lines.length - 1;
              pre.innerHTML = lines.map((l, i) => (i === at ? `<mark class="hit">${logLine(l) || ' '}</mark>` : logLine(l))).join('\n');
              const lineHeight = pre.scrollHeight / Math.max(lines.length, 1);
              pre.scrollTop = Math.max(0, at * lineHeight - pre.clientHeight / 3);
              p.jumped = true; p.jump = false;
              return;
            }
            if (pre.textContent !== C.line(text)) {
              pre.innerHTML = logHtml(text);
              if (atBottom || p.jump) pre.scrollTop = pre.scrollHeight;
            }
            p.jump = false;
          } catch (err) { /* the next pull will say */ }
        };
        p.jump = true;
        p.barHtml = null;
        p.graphHtml = null;
        pull();
        clearInterval(S.logTimer);
        S.logTimer = setInterval(pull, 2000);
      };
      return [e.name, html, after];
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
      if (st.gates && st.gates.length) return gatesPane(p, line, st);
      const robot = isRobot(st);
      const extra = robot ? [['prompt', 'Prompt(s)']] : [];
      if (p.tab === 'prompt' && robot) return promptPane(p, line, st, paneTabs(p, st.purpose, extra));
      if (p.tab === 'recent') {
        // A machine's work: the runs that finished its stage, or stand at it.
        const here = (line.recent_work || []).filter((r) => (st.stage && r.stages.includes(st.stage)) || S.snap.employees.some((e) => e.workflow === line.id && e.run_id === r.run_id && e.station === st.id) || !st.stage);
        return [st.label, paneTabs(p, st.purpose, extra) + recentWork(line, here, st.stage)];
      }
      const bucket = st.stage ? S.snap.costs.by_stage.find((b) => b.key === st.stage) : null;
      // The sessions at work on it now, first, then the finished ones.
      const now = S.snap.employees.filter((e) => e.workflow === line.id && e.station === st.id && e.active);
      const done = st.stage ? S.snap.costs.last.filter((r) => r.stage === st.stage) : [];
      const rows = [...now.map((e) => ({ live: e })), ...done];
      const sessionClock = (e) => (e.stage_since ? clockHtml(e.stage_since, null, true) : '—');
      const took = (r) => (r.duration_ms == null ? '—' : esc(fmtDur(r.duration_ms)));
      const time = now.length
        ? (now[0].stage_since ? `session running for ${sessionClock(now[0])}` : 'its session has not opened yet')
        : done.length && done[0].duration_ms != null ? `last session took ${took(done[0])}` : null;
      let h = kv([
        ['line', esc(line.title)], ['kind', `${esc(st.kind)} — ${esc(KIND_TIP[st.kind])}`],
        ['stage', st.stage ? esc(st.stage) : '<span class="muted">a gate of the stage beside it</span>'],
        ['model', st.model ? tag(st.model, st.model) : null],
        ['state', now.length ? liveTag(now[0].id, st.state) : stateTag(st.state)],
        ['time', time],
      ]);
      if (bucket) h += '<h3>What this stage cost, all runs</h3>' + tiles([[usd(bucket.usd), 'total'], [bucket.count, 'sessions'], [usd(bucket.usd / Math.max(1, bucket.count)), 'per session']]);
      if (rows.length) {
        h += `<h3>${now.length ? 'Sessions on it — now, then the last ones' : 'Last sessions on it'}</h3>` + pagedTable(p, 'station', '', '<th>when</th><th>task</th><th>round</th><th class="num">time</th><th class="num">cost</th><th class="num">turns</th><th>outcome</th>', rows, (r) => (r.live
          ? `<tr><td>now${r.live.since ? ' · since ' + esc(hhmm(r.live.since)) : ''}</td><td>${worksOn(r.live) || ((String(r.live.task || '').match(/\d+/) || [])[0] ? '#' + String(r.live.task).match(/\d+/)[0] : '—')}</td><td>${esc(r.live.round || '—')}</td><td class="num">${sessionClock(r.live)}</td><td class="num">—</td><td class="num">—</td><td>${liveTag(r.live.id)}</td></tr>`
          : `<tr><td data-sort="${esc(r.when)}">${esc(when(r.when))}</td><td>#${esc(r.task)}</td><td>${esc(r.round)}</td><td class="num">${took(r)}</td><td class="num">${r.cost_usd == null ? '—' : usd(r.cost_usd)}</td><td class="num">${r.turns ?? '—'}</td><td>${sessionTag(r, line.id)}</td></tr>`));
      }
      if (st.kind === 'scanner') h += '<div class="callout">A gate judges and never writes: it reads the issue, the labels or the ledger, and either lets the product through, skips the stage, or halts the round.</div>';
      return [st.label, paneTabs(p, st.purpose, extra) + h];
    },

    gate(p) {
      const line = S.snap.lines.find((l) => l.id === p.line);
      const st = line && line.stations.find((s) => s.id === p.station);
      const g = st && (st.gates || []).find((x) => x.id === p.id);
      if (!g) return ['Gate', '<div class="callout">Unknown gate.</div>'];
      const head = paneTabs(p, g.purpose);
      if (p.tab === 'recent') return [g.name, head + gateRuns(line, [g.name])];
      const back = `<div class="muted" style="margin-bottom:8px">on <button class="link" data-pane='${esc(JSON.stringify({ kind: 'station', line: line.id, id: st.id }))}'>${esc(st.label)}</button> · ${esc(line.title)}</div>`;
      return [g.name, head + back + gateDetail(line, st, g)];
    },

    line(p, refresh) {
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
      let h = kv([['trigger', esc(line.trigger)], ['log folder', `<code>.llocal/logs/${esc(line.id)}/</code>`], ['runs', line.runs], ['status', line.active ? (live.length ? liveTag(live[0].id, 'at work') : tag('at work', 'ok')) : tag('idle')]]);
      if (lr) {
        const lrActive = S.snap.employees.some((e) => e.workflow === line.id && e.run_id === lr.run_id && e.active);
        h += '<h3>Latest run</h3>' + kv([['run', esc(lr.run_id)], ['time', runClock({ active: lrActive, since: lr.started_at, last_at: lr.last_at }, null)], ['started', hhmm(lr.started_at)], ['last line at', `${hhmm(lr.last_at)} · ${fmtAge(lr.age_secs)} ago`], ['task', esc(lr.task || '—')], ['round', esc(lr.round || '—')], ['last line', `<code>${esc(lr.last_line)}</code>`], ['warning', lr.warning ? `<span class="tag warn">${esc(lr.warning)}</span>` : null]]);
        const emp = S.snap.employees.find((e) => e.workflow === line.id);
        h += `<button class="link" data-pane='${esc(JSON.stringify({ kind: 'employee', id: `${line.id}/${lr.run_id}`, last: emp || null }))}'>open its logs →</button>`;
      }
      h += '<h3>Stations</h3><ul class="issues">' + line.stations.map((s) => `<li><span class="n">${esc(s.kind)}</span><span class="t"><button class="link" data-pane='${esc(JSON.stringify({ kind: 'station', line: line.id, id: s.id }))}'>${esc(s.label)}</button></span>${s.model ? tag(s.model, s.model) : ''}<span class="st ${STATE_CLASS[s.state] || 'todo'}">${s.state}</span></li>`).join('') + '</ul>';
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
        p.mainHtml = h; p.liveBarHtml = liveBar; p.whereHtml = null;
        const pull = async () => {
          const e = S.snap.employees.find((x) => x.id === p.liveRun);
          const pre = $('line-live-log');
          const where = $('line-live-where');
          if (!e || !pre) return;
          try {
            const r = await fetch(`/api/runs/${e.workflow}/${e.run_id}/stages`);
            const stages = r.ok ? (await r.json()).stages || [] : [];
            const current = stages[stages.length - 1];
            let text;
            if (current) {
              text = current.text || '(empty)';
              const at = current.closed_at ? `took ${esc(lasted(current.opened_at, current.closed_at))}` : current.opened_at ? `running for ${clockHtml(current.opened_at, null, true)}` : '';
              const whereHtml = `${esc(e.name)} — machine: ${esc(current.stage)}${at ? ' · ' + at : ''} ${worksOn(e)}`;
              if (where && p.whereHtml !== whereHtml) { where.innerHTML = whereHtml; p.whereHtml = whereHtml; tickClocks(); }
            } else {
              const f = await fetch(`/api/runs/${e.workflow}/${e.run_id}/run.log?bytes=14000`);
              text = f.ok ? (await f.text()) || '(empty)' : '(no log yet)';
              if (where) where.innerHTML = `${esc(e.name)} — no machine reached yet, its run.log ${worksOn(e)}`;
              p.whereHtml = null;
            }
            const atBottom = pre.scrollHeight - pre.scrollTop - pre.clientHeight < 40;
            if (pre.textContent !== C.line(text)) {
              pre.innerHTML = logHtml(text);
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

    capabilities: capsPane,
    capability: capPane,

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

    janitor() {
      const html = '<div id="janitor-live"><div class="muted">The janitor is looking at the yard…</div></div><div id="janitor-settings"></div><div id="janitor-lists"></div>';
      // A refresh of the plant leaves the janitor's pane as it is: its numbers
      // come from his own route, and a limit being typed is not wiped.
      return ['The janitor · the yard', html, (refresh) => { if (!refresh) janitorLoad(true); }, true];
    },

    steward(p) { return terminalPane(p.kind, p); },
    doctor(p) { return terminalPane(p.kind, p); },
    note_session(p) { return terminalPane(p.kind, p); },

    notifications(p) {
      const level = LEVELS.some(([l]) => l === p.level) ? p.level : 'error';
      p.level = level;
      p.visited = [...new Set([...(p.visited || []), level])];
      const locked = new Set(p.locked || []);
      const nav = '<nav class="screens">' + LEVELS.map(([l, label]) => {
        const unread = ofLevel(l).filter(isUnread).length;
        return `<button class="${l === level ? 'here' : ''}" data-note-level="${l}">${label}${unread ? ` · ${unread}` : ''}</button>`;
      }).join('') + '</nav>';
      const bar = `<div class="notes-bar"><button class="mini ${p.unreadOnly ? 'on' : ''}" data-notes-unread>unread only</button>`
        + '<span class="muted">click a notification to keep it unread</span>'
        + '<button class="mini" data-notes-keep title="Close without marking anything read">close, keep unread</button></div>';
      const rows = ofLevel(level).filter((n) => !p.unreadOnly || isUnread(n));
      const body = rows.length
        ? pagedTable(p, 'notes-' + level, 'notes', '<th></th><th>what</th><th class="num">times</th><th>last</th><th></th>', rows, (n) => {
          const unread = isUnread(n);
          const keep = locked.has(n.key);
          return `<tr class="note ${level} ${unread ? 'unread' : 'read'} ${keep ? 'locked' : ''}" data-note-lock="${esc(n.key)}" title="${keep ? 'kept unread when you leave — click to release' : 'click to keep it unread when you leave'}">`
            + `<td class="sign-cell">${SIGN[level]}</td>`
            + `<td><b>${esc(n.title)}</b>${keep ? ' <span class="tag acc">kept unread</span>' : unread ? ' <span class="tag">new</span>' : ''}${n.resolved ? ' <span class="tag ok">resolved</span>' : ''}<div class="muted">${esc(n.detail)}</div></td>`
            + `<td class="num">${n.count}</td><td data-sort="${esc(n.at || '')}">${n.at ? esc(when(n.at)) : '—'}</td>`
            + `<td>${n.link ? `<button class="mini" data-note-open="${esc(n.key)}">open →</button>` : ''}</td></tr>`;
        })
        : `<div class="muted">${p.unreadOnly ? 'nothing unread here' : 'nothing to tell'}</div>`;
      const title = LEVELS.find(([l]) => l === level)[1];
      return ['Notifications · ' + title, nav + bar + body];
    },

    data_model(p, refresh) {
      // The board keeps its own data and its own refresh: a new picture of
      // the plant leaves it as it is, zoom and pins included.
      const html = '<div id="dm-host" class="dm-host"></div>';
      return ['Data model · ' + (S.snap && S.snap.project ? S.snap.project.name : 'the product'), html, (again) => {
        if (again && S.canvas) return;
        teardownCanvas();
        S.canvas = CANVASES.data_model($('dm-host'), p);
        // Back from a pin's session: its turns are counted once it is closed.
        if (S.closing) { const canvas = S.canvas; S.closing.then(() => { if (canvas && canvas.reload) canvas.reload(); }).catch(() => {}); S.closing = null; }
      }, true];
    },

    // A diverging data model, diagnosed: the state of affairs, the actions
    // proposed, and the issue the human may open from it.
    data_divergence(p, refresh) {
      const html = '<div id="dv" class="dv"><div class="muted">Reading the diagnosis…</div></div>';
      return ['Data model diverges · ' + (p.branch || ''), html, (again) => { if (!again) divergenceLoad(p); }, true];
    },

    placeholder(p) {
      return [p.title || 'Coming', `<div class="callout soon">${esc(p.text || 'Not built yet.')}</div>`];
    },
  };

  // The panes, for the components beside this file (actions.js opens the doctor).
  window.HarnessPanes = { open: (p) => openPane(p) };

  // A page left with a pin's session open still keeps its turns.
  window.addEventListener('pagehide', () => { if (S.term && S.term.close) S.term.close(); });

  // ---- go --------------------------------------------------------------------
  fromHash();
  crumbs();
  loadRenderer();
  load().then(subscribe);
  // A diagnosis asked before a reload is still polled after it.
  pullDiagnoses();
})();
