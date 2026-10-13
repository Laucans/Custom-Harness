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
  // The confirmation dialog: which factory, delink or destroy, what must be typed, a call under way.
  const C = { f: null, kind: '', must: '', busy: false };

  // ---- the menu ---------------------------------------------------------------
  const menu = $('factory-menu');
  const menuOpen = () => !menu.hidden;

  function rows() {
    const q = F.query.trim().toLowerCase();
    return F.list.filter((f) => !q || `${f.name} ${f.slug} ${f.id}`.toLowerCase().includes(q));
  }
  // The view runs in the current factory's folder: it can be taken down from another one only.
  const HERE = 'open another factory first';
  function row(f) {
    const here = f.id === F.current;
    const off = F.opening || C.busy ? ' disabled' : '';
    const act = (kind, icon, tip) => `<button type="button" class="fm-act${kind === 'destroy' ? ' danger' : ''}" data-${kind}="${esc(f.id)}"`
      + ` title="${esc(here ? `${tip} — ${HERE}` : tip)}" aria-label="${esc(`${kind} ${f.name}`)}"${here || off ? ' disabled' : ''}>${icon}</button>`;
    return '<div class="fm-item">'
      + `<button type="button" role="option" class="fm-row${here ? ' current' : ''}" data-factory="${esc(f.id)}" aria-selected="${here}"${off}>`
      + `<b>${here ? '✓ ' : ''}${esc(f.name)}</b><small>${f.slug ? esc(f.slug) : 'not linked to a GitHub repository'}</small></button>`
      + `<span class="fm-acts">${act('delink', '⛓', 'Delink — the harness lets go, the GitHub repository stays')}`
      + `${act('destroy', '🗑', 'Destroy — the GitHub repository is deleted')}</span></div>`;
  }
  function draw() {
    const shown = rows();
    $('fm-list').innerHTML = shown.length
      ? shown.map(row).join('')
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

  // ---- taking a factory down -----------------------------------------------------
  // Delink: the harness lets go (watch, folder, register), the repository stays.
  // Destroy: the same, and the repository is deleted — the name must be typed, and the
  // server checks it again.
  const dialog = $('factory-confirm');
  const dialogOpen = () => !dialog.hidden;
  const must = (f) => f.slug || f.name;
  const folderLine = (f) => (f.owned
    ? `Its folder <code>${esc(f.root)}</code> is removed, with the traces and the clones in it.`
    : `Its folder <code>${esc(f.root)}</code> stays on disk: the view did not make it.`);

  function dialogNote(text, bad) {
    const el = $('fc-note');
    el.hidden = !text;
    el.textContent = text || '';
    el.classList.toggle('bad', !!bad);
  }
  function dialogReady() {
    const typed = C.kind !== 'destroy' || $('fc-input').value === C.must;
    $('fc-go').disabled = C.busy || !typed;
    $('fc-cancel').disabled = C.busy;
    $('fc-input').disabled = C.busy;
  }
  function askToTakeDown(kind, id) {
    const f = F.list.find((x) => x.id === id);
    if (!f || f.id === F.current || F.opening || C.busy) return;
    Object.assign(C, { f, kind, must: must(f), busy: false });
    const destroy = kind === 'destroy';
    const box = dialog.querySelector('.fc-box');
    box.classList.toggle('danger', destroy);
    $('fc-title').textContent = `${destroy ? 'Destroy' : 'Delink'} ${f.name}?`;
    const repo = f.slug ? `<code>${esc(f.slug)}</code>` : 'no GitHub repository';
    $('fc-text').innerHTML = destroy
      ? (f.slug
        ? `This permanently deletes the GitHub repository ${repo}: its code, issues and pull requests. It cannot be undone.`
        : `${esc(f.name)} is linked to no GitHub repository: only the harness's side of it is destroyed. It cannot be undone.`)
      : `The harness lets go of ${esc(f.name)}. Its GitHub repository (${repo}) is not touched.`;
    $('fc-list').innerHTML = [
      destroy ? 'Its watch is killed now.' : 'Its watch is asked to stop: the running tasks finish, none starts.',
      folderLine(f),
      'It leaves the list of factories.',
      ...(destroy ? [] : [f.slug ? 'Connect the repository again from “+ new factory” whenever you want.' : '']),
    ].filter(Boolean).map((l) => `<li>${l}</li>`).join('');
    $('fc-typed').hidden = $('fc-input').hidden = !destroy;
    $('fc-name').textContent = C.must;
    $('fc-input').value = '';
    $('fc-go').textContent = destroy ? 'Destroy this factory' : 'Delink';
    $('fc-go').classList.toggle('danger', destroy);
    dialogNote('');
    dialogReady();
    close();
    dialog.hidden = false;
    (destroy ? $('fc-input') : $('fc-cancel')).focus();
  }
  function closeDialog() {
    if (C.busy) return;
    dialog.hidden = true;
    C.f = null;
    document.querySelector('#crumbs [data-factory]')?.focus();
  }
  async function takeDown() {
    if (!C.f || C.busy) return;
    const { f, kind } = C;
    if (kind === 'destroy' && $('fc-input').value !== C.must) return;
    C.busy = true;
    dialogReady();
    dialogNote(kind === 'destroy' ? `Destroying ${f.name}…` : `Delinking ${f.name}…`);
    try {
      const r = await fetch(`/api/factories/${encodeURIComponent(f.id)}/${kind}`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ confirm: $('fc-input').value }),
      });
      if (!r.ok) throw new Error((await r.text()) || r.statusText);
      const done = await r.json();
      C.busy = false;
      dialog.hidden = true;
      C.f = null;
      await open();
      note(summary(f, done));
    } catch (e) {
      C.busy = false;
      dialogReady();
      dialogNote(e.message || `cannot ${kind} that factory`, true);
    }
  }
  function summary(f, done) {
    const parts = [];
    if (done.watch === 'draining') parts.push('its watch drains');
    if (done.watch === 'killed') parts.push('its watch is killed');
    if (done.repository === 'deleted') parts.push('the GitHub repository is deleted');
    if (done.repository === 'kept') parts.push('the GitHub repository stays');
    parts.push(done.folder === 'removed' ? 'its folder is removed' : 'its folder stays');
    const text = `${f.name} ${done.repository === 'kept' ? 'delinked' : 'destroyed'}: ${parts.join(', ')}.`;
    return text.charAt(0).toUpperCase() + text.slice(1);
  }

  $('fc-input').addEventListener('input', dialogReady);
  $('fc-input').addEventListener('keydown', (e) => { if (e.key === 'Enter' && !$('fc-go').disabled) takeDown(); });
  $('fc-go').addEventListener('click', takeDown);
  $('fc-cancel').addEventListener('click', closeDialog);
  dialog.addEventListener('mousedown', (e) => { if (e.target === dialog) closeDialog(); });
  // Escape closes the dialog before anything behind it hears it; Tab stays inside.
  window.addEventListener('keydown', (e) => {
    if (!dialogOpen()) return;
    if (e.key === 'Escape') { e.stopImmediatePropagation(); e.preventDefault(); closeDialog(); return; }
    if (e.key !== 'Tab') return;
    const stops = [...dialog.querySelectorAll('input, button')].filter((el) => !el.hidden && !el.disabled);
    if (!stops.length) return;
    const at = stops.indexOf(document.activeElement);
    const next = stops[(at + (e.shiftKey ? -1 : 1) + stops.length) % stops.length];
    e.preventDefault();
    next.focus();
  }, true);

  $('fm-search').addEventListener('input', (e) => { F.query = e.target.value; draw(); });
  $('fm-list').addEventListener('click', (e) => {
    const gesture = e.target.closest('[data-delink], [data-destroy]');
    if (gesture) { askToTakeDown(gesture.dataset.delink ? 'delink' : 'destroy', gesture.dataset.delink || gesture.dataset.destroy); return; }
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
