// Every clock the page shows, read in the time zone of the machine that runs
// the harness — not the browser's: a page opened from another device still
// reads the harness's hours. The traces stay in UTC (`2026-10-09T19:27:16Z`,
// they sort as they read); only what a human reads is converted, here.
//
// The zone comes with the snapshot (`time_zone`); until it lands, or when the
// server could not name one, the browser's.
(() => {
  'use strict';

  let zone;
  let parts = null;
  let monthDay = null;

  // A clock in the zone, as its fields: `{year, month, day, hour, minute, second}`.
  function fields(ms) {
    parts ||= new Intl.DateTimeFormat('en-CA', {
      timeZone: zone, hourCycle: 'h23',
      year: 'numeric', month: '2-digit', day: '2-digit',
      hour: '2-digit', minute: '2-digit', second: '2-digit',
    });
    const o = {};
    for (const p of parts.formatToParts(new Date(ms))) o[p.type] = p.value;
    return o;
  }

  // An ISO clock, or seconds' worth of milliseconds, as milliseconds; NaN when unreadable.
  const toMs = (at) => (typeof at === 'number' ? at : Date.parse(at));

  function split(at) {
    const ms = toMs(at);
    if (at == null || at === '' || !Number.isFinite(ms)) return null;
    const f = fields(ms);
    return { ms, day: `${f.year}-${f.month}-${f.day}`, time: `${f.hour}:${f.minute}:${f.second}`, f };
  }

  // How far the zone is ahead of UTC at that instant, in milliseconds.
  function offset(ms) {
    const f = fields(ms);
    return Date.UTC(+f.year, +f.month - 1, +f.day, +f.hour, +f.minute, +f.second) - Math.floor(ms / 1000) * 1000;
  }

  const utc = (ms) => new Date(ms).toISOString().slice(0, 19) + 'Z';

  // A wall clock in the zone (`2026-10-09T15:27`) as the trace clock it is.
  function fromInput(local) {
    if (!local) return null;
    const guess = Date.parse(local.length === 16 ? local + ':00Z' : local + 'Z');
    if (!Number.isFinite(guess)) return null;
    // Twice: the first guess may sit on the other side of a DST change.
    return utc(guess - offset(guess - offset(guess)));
  }

  const STAMP = /\[(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ)\]/g;

  window.HarnessClock = {
    // The zone the server named; an empty or missing one keeps the browser's.
    zone(name) {
      const next = name || undefined;
      if (next === zone) return;
      try { new Intl.DateTimeFormat('en-CA', { timeZone: next }); } catch (e) { return; }
      zone = next; parts = null; monthDay = null;
    },
    // What to call it in a sentence.
    label: () => zone || 'the browser\'s time zone',
    // `15:27:16`.
    time: (at) => { const s = split(at); return s ? s.time : '—'; },
    // `2026-10-09`.
    day: (at) => { const s = split(at); return s ? s.day : '—'; },
    // `2026-10-09 15:27:16`.
    stamp: (at) => { const s = split(at); return s ? `${s.day} ${s.time}` : '—'; },
    // The time alone today, `Oct 9 15:27:16` another day.
    when(at) {
      const s = split(at);
      if (!s) return '—';
      if (s.day === split(Date.now()).day) return s.time;
      monthDay ||= new Intl.DateTimeFormat('en-US', { timeZone: zone, month: 'short', day: 'numeric' });
      return `${monthDay.format(new Date(s.ms))} ${s.time}`;
    },
    // A trace clock as a `datetime-local` value in the zone, and back.
    toInput: (at) => { const s = split(at); return s ? `${s.day}T${s.time.slice(0, 5)}` : ''; },
    fromInput,
    // A run id (`20261009-192716`, its start in UTC) as the trace clock it is; null otherwise.
    ofRunId(id) {
      const m = /^(\d{4})(\d\d)(\d\d)-(\d\d)(\d\d)(\d\d)/.exec(String(id ?? ''));
      return m ? `${m[1]}-${m[2]}-${m[3]}T${m[4]}:${m[5]}:${m[6]}Z` : null;
    },
    // The last midnight in the zone, as a trace clock.
    midnight: () => fromInput(`${split(Date.now()).day}T00:00`),
    // Log text with its `[2026-10-09T19:27:16Z]` stamps read in the zone.
    line: (text) => String(text ?? '').replace(STAMP, (_, at) => `[${window.HarnessClock.stamp(at)}]`),
  };
})();
