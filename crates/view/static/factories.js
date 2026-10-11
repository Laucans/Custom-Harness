// The factories this machine knows: the menu under the factory's name in the
// header, and when the factory-creation page shows (its content is
// creation.js's). Talks to app.js through `window.HarnessFactories` and two
// events (`harness:factory-page` out, `harness:navigate` in).
(() => {
  'use strict';

  const $ = (id) => document.getElementById(id);
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const PAGE_HASH = '#new-factory';

  const F = { list: [], current: '', query: '', opening: '', checked: false, deciding: false };

  // ---- the menu ---------------------------------------------------------------
  const menu = $('factory-menu');
  const menuOpen = () => !menu.hidden;

  function rows() {
    const q = F.query.trim().toLowerCase();
    return F.list.filter((f) => !q || `${f.name} ${f.slug} ${f.id}`.toLowerCase().includes(q));
  }
  function draw() {
    const shown = rows();
    $('fm-list').innerHTML = shown.length
      ? shown.map((f) => `<button type="button" role="option" class="fm-row${f.id === F.current ? ' current' : ''}" data-factory="${esc(f.id)}" aria-selected="${f.id === F.current}"${F.opening ? ' disabled' : ''}>`
        + `<b>${f.id === F.current ? '✓ ' : ''}${esc(f.name)}</b><small>${f.slug ? esc(f.slug) : 'not linked to a GitHub repository'}</small></button>`).join('')
      : `<div class="fm-empty">${F.list.length ? 'No factory matches.' : 'No factory known yet.'}</div>`;
  }
  function note(text, bad) {
    const el = $('fm-note');
    el.hidden = !text;
    el.textContent = text || '';
    el.classList.toggle('bad', !!bad);
  }
  async function read() {
    try {
      const answer = await (await fetch('/api/factories')).json();
      F.list = answer.factories || [];
      F.current = answer.current || '';
      draw();
    } catch (e) { note('cannot read the factories', true); }
  }
  async function open() {
    menu.hidden = false;
    document.querySelector('#crumbs [data-factory]')?.setAttribute('aria-expanded', 'true');
    F.query = '';
    $('fm-search').value = '';
    note('');
    draw();
    $('fm-search').focus();
    await read();
  }
  function close() {
    menu.hidden = true;
    document.querySelector('#crumbs [data-factory]')?.setAttribute('aria-expanded', 'false');
  }
  function toggle() { if (menuOpen()) close(); else open(); }

  async function switchTo(id) {
    const target = F.list.find((f) => f.id === id);
    if (!target || F.opening) return;
    if (id === F.current) { close(); leavePage(); return; }
    F.opening = id;
    draw();
    note(`Opening ${target.name}…`);
    try {
      const r = await fetch(`/api/factories/${encodeURIComponent(id)}/open`, { method: 'POST' });
      if (!r.ok) throw new Error((await r.text()) || r.statusText);
      // The view starts again in that factory; the page reloads on its own
      // when the event stream comes back with a new build.
    } catch (e) {
      F.opening = '';
      draw();
      note(e.message || 'cannot open that factory', true);
    }
  }

  $('fm-search').addEventListener('input', (e) => { F.query = e.target.value; draw(); });
  $('fm-list').addEventListener('click', (e) => {
    const row = e.target.closest('[data-factory]');
    if (row) switchTo(row.dataset.factory);
  });
  $('fm-new').addEventListener('click', () => {
    close();
    // The landing, whatever step the page was on.
    history.replaceState(null, '', location.pathname + location.search + PAGE_HASH);
    openPage(false);
  });
  menu.addEventListener('keydown', (e) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const stops = [$('fm-search'), ...$('fm-list').querySelectorAll('.fm-row:not([disabled])'), $('fm-new')];
    const at = stops.indexOf(document.activeElement);
    const next = stops[(at + (e.key === 'ArrowDown' ? 1 : -1) + stops.length) % stops.length];
    e.preventDefault();
    next.focus();
    if (next.scrollIntoView) next.scrollIntoView({ block: 'nearest' });
  });
  // Escape closes the menu first; the page behind stays where it is.
  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && menuOpen()) { e.stopImmediatePropagation(); close(); document.querySelector('#crumbs [data-factory]')?.focus(); }
  }, true);
  document.addEventListener('click', (e) => {
    if (menuOpen() && !e.composedPath().includes(menu) && !e.target.closest('#crumbs [data-factory]')) close();
  });

  // ---- the creation page -------------------------------------------------------
  // Its content is creation.js's; here, when it shows and when it goes.
  const page = $('factory-page');
  const onPage = () => location.hash === PAGE_HASH || location.hash.startsWith(`${PAGE_HASH}/`);
  // Until the first picture said whether this factory is being created, the
  // scene waits: a factory being created never loads it.
  const creating = () => document.body.classList.contains('creating') || F.deciding;

  function setPage(on) {
    page.hidden = !on;
    document.body.classList.toggle('creating', on);
    window.dispatchEvent(new CustomEvent('harness:factory-page', { detail: { open: on } }));
  }
  function openPage(auto) {
    if (!onPage()) history.replaceState(null, '', location.pathname + location.search + PAGE_HASH);
    setPage(true);
    window.HarnessCreation.open({ auto });
  }
  function leavePage() {
    if (!creating()) return;
    window.HarnessCreation.close();
    setPage(false);
    if (onPage()) history.replaceState(null, '', location.pathname + location.search);
  }
  window.addEventListener('harness:navigate', leavePage);
  window.addEventListener('hashchange', () => {
    if (onPage() && !creating()) openPage(false);
    else if (!onPage() && creating()) leavePage();
    else if (onPage()) window.HarnessCreation.route();
  });

  // The first picture decides: a factory being created, or with no
  // repository, opens the page by itself (a demo has none to link).
  async function onSnapshot(snap) {
    if (F.checked) return;
    F.checked = true;
    read();
    if (snap.demo) {
      if (onPage()) openPage(false);
      return;
    }
    F.deciding = true;
    let being = false;
    try { being = await window.HarnessCreation.pending(); } catch (e) { being = false; }
    F.deciding = false;
    const unlinked = !snap.project.slug;
    if (being || unlinked) openPage(true);
    else if (onPage()) openPage(false);
    else window.dispatchEvent(new CustomEvent('harness:factory-page', { detail: { open: false } }));
  }

  window.HarnessFactories = { toggle, onSnapshot, creating, leave: leavePage };
})();
