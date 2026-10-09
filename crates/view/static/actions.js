// What the human asks of the page — a build, a sweep, an issue opened, the
// plant started — followed the same way everywhere:
//
// - the button just pressed gets a small log button (≡) inside it: a click
//   opens a log window with what the action said, live;
// - an action that fails puts a red card at the top of the page, which says
//   what failed and offers "Ask doctor": the doctor's terminal opens with the
//   action, its error and its log already asked.
//
//   const a = HarnessActions.start({ key, label, button, log });
//   a.note('…'); a.ok('…'); a.fail('…');
//   HarnessActions.chip(key)   // the log button, for a renderer to put back
(() => {
  'use strict';

  const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const clock = () => new Date().toTimeString().slice(0, 8);
  // How many actions a page remembers; the oldest are forgotten.
  const KEPT = 50;
  const POLL_MS = 2000;

  const A = { list: [], seq: 0, open: null, timer: null };

  const byId = (id) => A.list.find((a) => a.id === Number(id));
  const byKey = (key) => A.list.filter((a) => a.key === key).pop();

  function chip(key) {
    const a = byKey(key);
    if (!a) return '';
    return `<span class="act-log ${a.state}" role="button" tabindex="0" data-action-log="${a.id}" title="${esc(a.label)} — its log">≡</span>`;
  }

  // Puts the log button inside the pressed button, or refreshes it there.
  function mark(a) {
    if (!a.button || !a.button.isConnected) return;
    const old = a.button.querySelector('[data-action-log]');
    if (old) old.remove();
    a.button.insertAdjacentHTML('beforeend', chip(a.key));
  }

  function start({ key, label, button, log }) {
    const a = {
      id: ++A.seq, key, label, button: button || null, log: log || null,
      state: 'running', lines: [], output: '', error: '', dismissed: false,
    };
    a.note = (text) => { a.lines.push(`${clock()}  ${text}`); refresh(a); };
    a.ok = (text) => { a.state = 'done'; a.note(text ? `done — ${text}` : 'done'); mark(a); };
    a.fail = (text) => {
      a.state = 'failed';
      a.error = String(text || 'failed').trim();
      a.note(`failed — ${a.error}`);
      mark(a);
      cards();
    };
    A.list.push(a);
    if (A.list.length > KEPT) A.list.shift();
    a.note(`asked: ${label}`);
    mark(a);
    return a;
  }

  // ---- the red cards ---------------------------------------------------------------
  function cards() {
    const box = document.getElementById('action-errors');
    if (!box) return;
    const failed = A.list.filter((a) => a.state === 'failed' && !a.dismissed);
    box.innerHTML = failed.map((a) => `<div class="act-error" role="alert" data-action="${a.id}">`
      + `<div class="act-error-text"><b>⚠ ${esc(a.label)} failed</b><span>${esc(a.error.split('\n')[0])}</span></div>`
      + '<div class="act-error-buttons">'
      + `<button type="button" class="act-doctor" data-action-doctor="${a.id}" title="Open the doctor with this error and its log">🦞 Ask doctor</button>`
      + `<button type="button" data-action-log="${a.id}" title="What the action said">log</button>`
      + `<button type="button" data-action-dismiss="${a.id}" aria-label="dismiss" title="Dismiss">×</button>`
      + '</div></div>').join('');
  }

  // ---- the log window --------------------------------------------------------------
  function text(a) {
    const said = a.lines.join('\n');
    return a.output ? `${said}\n\n--- its output ---\n${a.output}` : said;
  }
  function refresh(a) {
    if (A.open !== a.id) return;
    const win = document.getElementById('action-log');
    if (!win) return;
    win.querySelector('.act-log-title').textContent = a.label;
    const state = win.querySelector('.act-log-state');
    state.textContent = a.state;
    state.className = 'act-log-state ' + a.state;
    const pre = win.querySelector('pre');
    const atEnd = pre.scrollTop + pre.clientHeight >= pre.scrollHeight - 8;
    pre.textContent = text(a);
    if (atEnd) pre.scrollTop = pre.scrollHeight;
  }
  async function pull(a) {
    if (!a.log) return;
    try {
      const out = await a.log();
      if (typeof out === 'string') a.output = out;
    } catch (e) { /* asked again at the next tick */ }
    refresh(a);
  }
  function openLog(id) {
    const a = byId(id);
    const win = document.getElementById('action-log');
    if (!a || !win) return;
    A.open = a.id;
    win.hidden = false;
    refresh(a);
    clearInterval(A.timer);
    pull(a);
    // Followed live while it runs and the window is open.
    A.timer = setInterval(() => {
      const now = byId(A.open);
      if (!now || win.hidden) { clearInterval(A.timer); return; }
      pull(now);
      if (now.state !== 'running') clearInterval(A.timer);
    }, POLL_MS);
  }
  function closeLog() {
    const win = document.getElementById('action-log');
    if (win) win.hidden = true;
    A.open = null;
    clearInterval(A.timer);
  }

  // ---- asking the doctor ---------------------------------------------------------
  async function askDoctor(id, button) {
    const a = byId(id);
    if (!a) return;
    await pull(a);
    if (button) { button.disabled = true; button.textContent = '🦞 asking…'; }
    try {
      const res = await fetch('/api/doctor/ask', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: a.label, error: a.error, log: text(a) }),
      });
      if (!res.ok) throw new Error(await res.text());
      a.note('asked the doctor');
      if (window.HarnessPanes) window.HarnessPanes.open({ kind: 'doctor' });
      if (button) button.textContent = '🦞 asked';
    } catch (e) {
      if (button) { button.disabled = false; button.textContent = '🦞 Ask doctor'; }
      a.note(`the doctor could not be asked — ${e.message || e}`);
    }
  }

  // ---- events: before anything else, so a log button inside another button
  // is not that button's click ---------------------------------------------------------
  document.addEventListener('click', (e) => {
    const log = e.target.closest('[data-action-log]');
    if (log) { e.stopPropagation(); e.preventDefault(); openLog(log.dataset.actionLog); return; }
    const doctor = e.target.closest('[data-action-doctor]');
    if (doctor) { e.stopPropagation(); askDoctor(doctor.dataset.actionDoctor, doctor); return; }
    const dismiss = e.target.closest('[data-action-dismiss]');
    if (dismiss) { const a = byId(dismiss.dataset.actionDismiss); if (a) a.dismissed = true; cards(); return; }
    if (e.target.closest('[data-action-close]')) closeLog();
  }, true);
  document.addEventListener('keydown', (e) => {
    const log = e.target.closest && e.target.closest('[data-action-log]');
    if (log && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); e.stopPropagation(); openLog(log.dataset.actionLog); }
    if (e.key === 'Escape' && A.open !== null) { e.stopPropagation(); closeLog(); }
  }, true);

  window.HarnessActions = { start, chip };
})();
