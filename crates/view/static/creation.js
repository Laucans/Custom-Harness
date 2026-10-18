// The factory-creation page: connect an existing repository, or initialize
// a factory through ten steps under a breadcrumb — the repository, the kind
// of factory, the brief (with the design system's material: logo, moodboard,
// design brief), the grilling (the tokens and the kit draw meanwhile), the
// proof of concept's scope (proposed once the grilling is over), the design
// system, the mock-ups
// (drawn, then reworked in the easel), their review (the requirements tied
// to the pages and the micro-frontends), the initialization. Steps 2 to 9
// run in the new factory's own view; the design system's easel (on the brief
// step its material face: the sources it is generated from, no sheet and no
// canvas; whole on the design step) and the mock-up's (mock-ups and review
// steps) are mounted in the page itself, never in a pane over it. Driven by factories.js through
// `window.HarnessCreation`.
(() => {
  'use strict';

  const $ = (id) => document.getElementById(id);
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const HASH = '#new-factory';
  const POLL_MS = 2000;
  // A burst of beats (several kit components generating at once) reads at most this often.
  const SETTLE_MS = 250;
  // A frozen, looping replay of the mock-ups/initialize split view, with no
  // server poll and no AI call — `?demo=1` (before the hash, e.g.
  // `/?demo=1#new-factory/mockups`) over a factory whose mock-up is already
  // drawn on disk, so the right-hand preview is real, navigable content.
  const DEMO = new URLSearchParams(location.search).get('demo') === '1';
  // What the page shows while a grilling round is asked.
  const GRILL_FIRST = 'the AI prepares its questions';
  const GRILL_NEXT = 'the AI prepares the next round';
  const DEMO_PAGES = ['Accueil de la maquette', 'Campagnes', 'Joueurs', 'Éléments de campagne', 'Image et grille hex', 'Préparation de séance', 'Vue MJ — mode séance', 'Vue joueur'];
  const DEMO_BRANCH = 'main_agent';
  let demoTimer = null;
  let demoWork = null;
  let demoPlaying = false;

  const STEPS = [
    ['repository', 'Repository'],
    ['kind', 'Factory type'],
    ['brief', 'Brief'],
    ['grilling', 'Grilling'],
    ['scope', 'POC scope'],
    ['design', 'Design system'],
    ['mockups', 'Mock-ups'],
    ['rules', 'Rules'],
    ['data', 'Data'],
    ['ports', 'Ports'],
    ['initialize', 'Initialize'],
  ];
  const KINDS = [
    { id: 'vertical', label: 'Vertical architecture development factory', text: 'Vertical slices with their contracts, the architecture init-repo installs — what this plant builds today.' },
    { id: 'hexagonal', label: 'Hexagonal architecture development factory', text: 'Ports and adapters around a domain core.', soon: true },
  ];
  const indexOf = (step) => STEPS.findIndex(([id]) => id === step);
  // Founding another factory from a factory that has a creation of its own:
  // the repository step, read as if there were no record — the record on the
  // desk belongs to the factory open here, never to the one being founded.
  const FOUND = 'found';

  // The busy label while the AI proposes the scope.
  const PROPOSING = 'the AI proposes the scope';
  const C = { helps: [], dirty: {}, loaded: false, data: null, route: 'landing', auto: false, busy: '', error: '', owners: null, ownersWhy: '', timer: null, embed: null, view: { width: null, height: null }, trackHidden: false, auto_started: {}, confirm: '', parts: false, opening: '', pages: null, mock: null, reqTimer: null, pulsed: false, reading: false, again: false };
  const page = $('factory-page');

  async function call(method, url, body) {
    // A change of the creation says its mock-up's branch: the rules' part
    // of it goes in that mock-up's history (Cmd+Z).
    const branch = method !== 'GET' && C.data && C.data.branch ? { 'x-harness-branch': C.data.branch } : {};
    const r = await fetch(url, { method, headers: Object.assign(body ? { 'content-type': 'application/json' } : {}, branch), body: body ? JSON.stringify(body) : undefined });
    const text = await r.text();
    if (!r.ok) throw new Error(text || r.statusText);
    return text ? JSON.parse(text) : null;
  }
  async function read() {
    try { C.data = await call('GET', '/api/creation'); } catch (e) { C.data = null; C.error = e.message; }
    C.loaded = true;
    return C.data;
  }
  const record = () => (C.data && C.data.record) || null;
  const job = () => (C.data && C.data.job) || null;
  const running = (work) => { const j = job(); return !!j && j.state === 'running' && (!work || j.work === work); };
  // The kit generates in the background from the brief step on: followed on every step.
  const kitBusy = () => { const k = C.data && C.data.kit; return !!k && (k.running || (k.waiting > 0 && !k.noClaude && k.validated)); };
  // Drawn, and not rewritten as micro-frontends at this very moment: the easel may open.
  const drawn = () => { const r = record(); return !!r && !!r.mocked && !running('mockups') && !running('componentize'); };

  // ---- the route, in the hash ---------------------------------------------------
  function routeFromHash() {
    const m = /^#new-factory(?:\/([a-z]+))?$/.exec(location.hash);
    if (m && m[1] === FOUND) return FOUND;
    // The rules step was called the review: an old link still lands on it.
    const step = m && m[1] === 'review' ? 'rules' : m && m[1];
    return step && indexOf(step) >= 0 ? step : 'landing';
  }
  function setRoute(route) {
    C.route = route;
    C.confirm = '';
    const hash = route === 'landing' ? HASH : `${HASH}/${route}`;
    if (location.hash !== hash) history.replaceState(null, '', location.pathname + location.search + hash);
    draw();
  }

  // A step is open when the gates reach it; without a record only the first
  // is — and a founding reads no record, so it stops at the first too.
  function reachable(step, fresh) {
    const r = fresh ? null : record();
    if (!r) return step === 'repository';
    return indexOf(step) <= indexOf(C.data.reachable || 'kind');
  }

  // ---- following the server ---------------------------------------------------------
  // The server beats a pulse on the event stream whenever the product's
  // workspace or a desk's job moved (`harness:pulse`, from app.js): the page
  // reads again then, and redraws only what the reading changed. Until a
  // pulse is heard — a view built before it, a stream not up yet — the page
  // asks every POLL_MS instead, while something moves: a job, the kit, the
  // design system the brief and the design step follow.
  const designOpen = () => (C.route === 'design' || C.route === 'brief') && !!record();
  window.addEventListener('harness:pulse', () => {
    C.pulsed = true;
    follow();
  });
  function poll() {
    if (DEMO || C.pulsed) return;
    clearTimeout(C.timer);
    if (!running() && !designOpen() && !kitBusy()) return;
    C.timer = setTimeout(follow, POLL_MS);
  }
  // One reading at a time, never under an action: a beat heard meanwhile
  // reads once more after it (`resume`).
  async function follow() {
    if (DEMO || page.hidden || !C.loaded) return;
    if (C.reading || C.busy) { C.again = true; return; }
    C.reading = true;
    try {
      await refresh();
    } finally {
      setTimeout(() => { C.reading = false; resume(); }, SETTLE_MS);
    }
  }
  function resume() {
    if (C.again) { C.again = false; follow(); } else poll();
  }
  // What was read is compared with what is drawn: the same, nothing is
  // redrawn; only the job moved, only the job's panel and the preview are.
  async function refresh() {
    const before = C.data;
    const was = job();
    await read();
    const outside = (data) => JSON.stringify(data && { ...data, job: null });
    if (JSON.stringify(C.data) === JSON.stringify(before)) return;
    const now = job();
    if (was && was.state === 'running' && now && now.state !== 'running') { onJobEnd(was, now); return; }
    if (outside(C.data) === outside(before)) { drawJob(); drawPreview(); return; }
    draw();
    drawPreview();
    if (!running()) enter();
  }
  const generateKit = () => act('starting the kit', async () => { await call('POST', '/api/kit/generate', {}); await read(); });
  function onJobEnd(before, after) {
    if (after.work === 'found' && after.state === 'done' && after.factory) return openFactory(after.factory, 'kind');
    if (after.work === 'connect' && after.state === 'done' && after.factory) return openFactory(after.factory, null);
    draw();
    // A step whose job just ended starts what comes next by itself: the
    // mock-up's easel once drawn, the requirements once the micro-frontends are made.
    enter();
    return undefined;
  }
  async function openFactory(id, step) {
    C.opening = id;
    drawJob();
    // The new view reads the hash after its reload: the step it opens on.
    const hash = step ? `${HASH}/${step}` : '';
    history.replaceState(null, '', location.pathname + location.search + hash);
    try {
      await call('POST', `/api/factories/${encodeURIComponent(id)}/open`);
      // The page reloads on its own when the event stream comes back with a new build.
    } catch (e) {
      C.opening = '';
      C.error = e.message;
      draw();
    }
  }

  // ---- actions --------------------------------------------------------------------
  async function act(label, fn) {
    if (C.busy) return;
    C.busy = label;
    C.error = '';
    draw();
    try { await fn(); } catch (e) { C.error = e.message || String(e); }
    C.busy = '';
    draw();
    resume();
  }
  // What was typed and not saved yet, by field: a redraw puts it back.
  const save = async (body) => { C.data = await call('PUT', '/api/creation', body); C.dirty = {}; };
  const post = async (what) => {
    C.data = await call('POST', `/api/creation/${what}`);
    if (what === 'scope') for (const key of Object.keys(C.dirty)) if (key.startsWith('scope-')) delete C.dirty[key];
    if (what === 'regenerate') C.dirty = {};
  };
  const start = async (what, body) => { await call('POST', `/api/creation/${what}`, body); await read(); };

  // What the current step's fields hold, as the record takes it.
  function fields() {
    const body = {};
    const description = page.querySelector('[data-cr="description"]');
    if (description) body.description = description.value;
    if (page.querySelector('[data-cr^="brief-"]')) body.brief = composeBrief((id) => page.querySelector(`[data-cr="brief-${id}"]`).value);
    if (page.querySelector('[data-cr^="scope-"]')) body.scope = composeScope((id) => (page.querySelector(`[data-cr="scope-${id}"]`) || { value: '' }).value);
    const qs = page.querySelectorAll('[data-q]');
    if (qs.length || (C.route === 'grilling' && record())) {
      body.questions = [...qs].map((q) => ({ id: Number(q.dataset.q), answer: q.querySelector('textarea').value }));
    }
    // The rules are kept as the page edits them, not read back off the cards.
    if (C.route === 'rules' && C.rules) body.requirements = C.rules;
    return body;
  }

  async function next(step) {
    // Read before `act` redraws the step.
    const body = { ...fields(), step };
    const from = C.route;
    await act('saving', async () => {
      await flushEmbed();
      await save(body);
      C.route = step;
      setRoute(step);
    });
    // The brief closed: what it allows draws meanwhile — the tokens, the kit.
    if (from === 'brief' && step === 'grilling' && !DEMO) {
      try { await call('POST', '/api/creation/prepare'); await read(); draw(); } catch (e) { /* another job runs, or no brief: the steps say */ }
    }
    enter();
    resume();
  }

  // What a step starts on its own the first time it is entered.
  function enter() {
    if (DEMO) {
      const want = C.route === 'mockups' ? 'mockups' : C.route === 'initialize' ? 'initialize' : null;
      if (want && demoWork !== want) { demoWork = want; showDemoFinal(want); }
      else if (!want) { clearInterval(demoTimer); demoTimer = null; demoWork = null; demoPlaying = false; }
      // Review has no job to animate — just the real pages, read-only (see
      // EMBEDS.review.opts and mockup.js's own readOnly guard).
      if (C.route === 'rules' && (!record() || C.data.branch !== DEMO_BRANCH)) { C.data = demoBase('rules'); draw(); }
      drawDemoButton();
      return;
    }
    const r = record();
    if (!r || C.busy) return;
    const once = (key, fn) => { if (C.auto_started[key]) return; C.auto_started[key] = true; fn(); };
    // The first round asks itself; every question deleted later does not bring one back.
    if (C.route === 'grilling' && !r.questions.length && !r.rounds) once(`grill-${r.regenerations}`, () => act(GRILL_FIRST, () => post('grill')));
    // A scope not written, or proposed before the grilling moved the decisions and
    // not retouched: proposed (again) by itself; a retouched one waits for the human.
    if (C.route === 'scope' && (!r.scope.trim() || (C.data.scopeStale && !r.scope_edited))) once(`scope-${r.regenerations}-${r.scope_from}`, () => act(PROPOSING, () => post('scope')));
    // The scope settled: the proof of concept draws ahead while the human
    // settles the design system — mock-ups, micro-frontends, requirements.
    const ahead = !r.mocked && r.scope.trim() && indexOf(r.step) >= indexOf('design') && C.route !== 'mockups';
    if (ahead && !running() && !(job() && job().work === 'mockups')) once(`ahead-${r.regenerations}`, () => start('ahead').then(draw, () => { /* another job runs, or nothing can draw: the mock-up step says */ }));
    if (C.route === 'mockups' && !r.mocked && !running() && !(job() && job().work === 'mockups')) once(`mock-${r.regenerations}`, () => act('starting the drawing', () => start('mockups')));
    const kit = C.data.kit;
    if ((C.route === 'design' || C.route === 'brief') && kit && kit.validated && kit.waiting && !kit.running && !kit.noClaude) once(`kit-${r.regenerations}`, generateKit);
    if (C.route === 'data') {
      if (!C.mock) loadMock();
      if ((!r.data || (C.data.dataStale && !r.data.validated)) && r.rules_validated && !running()) once(`data-${r.regenerations}`, () => act('the AI refines the data model off the rules', async () => { await post('data'); reloadEmbed(); }));
    }
    if (C.route === 'ports' && !r.ports && r.data && r.data.validated && !running()) once(`ports-${r.regenerations}`, () => act('the AI lists the ports', () => post('ports')));
    if (C.route === 'rules') {
      if (!C.mock) loadMock();
      if (!r.componentized && !running()) once(`mf-${r.regenerations}`, () => act('starting the micro-frontends', () => start('componentize')));
      else if (r.componentized && !r.requirements.length && !running()) once(`req-${r.regenerations}`, () => act('the AI reads the rules', async () => { await post('requirements'); await loadMock(); }));
      // The mock-up was cut again since the rules were read: read again, the human's kept and tied again.
      else if (r.componentized && C.data.rulesStale && !running()) once(`reread-${r.regenerations}-${r.componentized.length}-${r.componentized.slice(0, 60)}`, () => act('the mock-up changed: the AI reads the rules again', async () => { C.rules = null; await post('requirements'); await loadMock(); }));
      // The guard reads the cut once by itself; again only when asked.
      // (The data step drafts its model by itself the same way, below.)
      else if (r.componentized && r.requirements.length && !r.audit && !running()) once(`audit-${r.regenerations}`, () => act('the guard reads the architecture', () => post('audit')));
    }
  }
  // The mock-up as it stands: its pages and micro-frontends, for the review's requirements.
  async function loadMock() {
    if (!C.data || !C.data.branch) return;
    try { C.mock = await call('GET', `/api/mockup?branch=${encodeURIComponent(C.data.branch)}`); } catch (e) { C.mock = { pages: [], components: [] }; }
    if (C.route === 'rules') draw();
  }

  // ---- drawing --------------------------------------------------------------------
  // Each crumb wears its step's mark, read from the server: green done,
  // yellow to review (what it was made from moved), red blocked (a gate
  // before where the human went fails), grey locked; a spinner chip says
  // what draws for it.
  const MARK_SIGN = { done: '✓', stale: '↻', blocked: '!' };
  const MARK_TITLE = { stale: 'To review: what it was made from moved since', blocked: 'Blocked: a gate before where you went fails here', locked: 'Not reachable yet' };
  function crumbs(fresh) {
    const r = fresh ? null : record();
    const at = fresh ? 0 : indexOf(C.route);
    const marks = r && C.data.marks ? Object.fromEntries(C.data.marks) : null;
    // The demo's frozen record has no marks: how far it reaches says them.
    const far = r ? indexOf(C.data.reachable || 'kind') : 0;
    const guess = (i) => (i < far ? 'done' : i === far ? 'open' : 'locked');
    return `<ol class="cr-crumbs" aria-label="creation steps">${STEPS.map(([id, label], i) => {
      const mark = fresh ? (i === 0 ? 'open' : 'locked') : (marks ? marks[id] || 'locked' : guess(i));
      const open = reachable(id, fresh);
      // While founding, the first crumb is the founding itself.
      const go = fresh && id === 'repository' ? FOUND : id;
      const title = MARK_TITLE[mark] ? ` title="${MARK_TITLE[mark]}"` : '';
      return `<li class="cr-crumb ${mark}${i === at ? ' here' : ''}"><button type="button" data-cr-go="${go}"${open ? '' : ' disabled'}${i === at ? ' aria-current="step"' : ''}${title}><span class="cr-n">${i !== at && MARK_SIGN[mark] ? MARK_SIGN[mark] : i + 1}</span><span class="cr-l">${esc(label)}</span>${fresh ? '' : chip(id)}</button></li>`;
    }).join('')}</ol>`;
  }

  // What draws in the background, on the crumb of the step it is for: the
  // scope proposed, the tokens and the kit generating, a scope to redo, the
  // mock-ups drawing, the micro-frontends made for the review.
  function chip(id) {
    const r = record();
    if (!r || DEMO) return '';
    const k = C.data.kit;
    if (id === 'scope') {
      if (C.busy === PROPOSING && !r.scope.trim()) return '<span class="cr-chip"><span class="cr-spin" aria-hidden="true"></span> proposing</span>';
      if (C.data.scopeStale) return '<span class="cr-chip warn" title="The decisions moved since the scope was proposed">to redo</span>';
    }
    if (id === 'mockups') {
      if (running('mockups')) return '<span class="cr-chip"><span class="cr-spin" aria-hidden="true"></span> drawing</span>';
      if (C.data.mockupStale) return '<span class="cr-chip warn" title="The scope moved since the mock-ups were drawn: draw them again">to redraw</span>';
      if (r.mocked && indexOf(C.route) < indexOf('mockups')) return '<span class="cr-chip">ready</span>';
    }
    // The micro-frontends and the requirements are the review's: their spinner sits on its crumb.
    if (id === 'rules' && running('componentize')) return '<span class="cr-chip"><span class="cr-spin" aria-hidden="true"></span> micro-frontends</span>';
    if (id === 'design' && k) {
      if (k.running || (running('prepare') && !k.validated && !k.generated)) return '<span class="cr-chip"><span class="cr-spin" aria-hidden="true"></span> generating</span>';
      if (k.failed) return `<span class="cr-chip warn">${k.failed} failed</span>`;
      if (k.validated && !k.waiting && C.route !== 'design') return '';
      if (!k.validated && k.generated + k.waiting > 0) return '<span class="cr-chip">to validate</span>';
    }
    return '';
  }

  function jobPanel() {
    const j = job();
    if (!j) return '';
    const title = { found: 'Founding the factory', connect: 'Connecting the repository', prepare: 'Preparing the design system', mockups: 'Drawing the mock-ups', componentize: 'Making the micro-frontends', initialize: 'Initializing the factory' }[j.work] || j.work;
    const state = j.state === 'running' ? '<span class="cr-spin" aria-hidden="true"></span> under way' : j.state === 'done' ? '<span class="tag ok">done</span>' : '<span class="tag bad">stopped</span>';
    // Every step but the last one of a still-running job is done — the
    // last is whatever is under way right now, so it stays plain.
    const last = j.steps.length - 1;
    return `<div class="cr-job ${esc(j.state)}" role="status"><div class="cr-job-head"><b>${esc(title)}</b> ${state}</div>
      <ul>${j.steps.map((s, i) => `<li>${j.state !== 'running' || i < last ? '<span class="cr-ok">✓</span> ' : ''}${esc(s)}</li>`).join('')}</ul>
      ${j.why ? `<p class="cr-bad">${esc(j.why)}</p>` : ''}
      ${j.report ? `<details><summary>init-repo's report</summary><pre>${esc(j.report)}</pre></details>` : ''}
      ${C.opening ? `<p>Opening the factory… the page reloads when it is up.</p>` : ''}</div>`;
  }
  function drawJob() {
    const el = page.querySelector('[data-cr-job]');
    if (!el) return;
    const kept = jobScroll();
    el.innerHTML = jobPanel();
    jobScrolled(kept);
  }
  // The job's checklist scrolls in its own box: left at its bottom, it
  // follows the new lines; scrolled up, it stays where the human put it.
  function jobScroll() {
    const list = page.querySelector('[data-cr-job] .cr-job ul');
    if (!list) return null;
    return { bottom: list.scrollTop + list.clientHeight >= list.scrollHeight - 4, top: list.scrollTop };
  }
  function jobScrolled(kept) {
    const list = page.querySelector('[data-cr-job] .cr-job ul');
    if (!list) return;
    list.scrollTop = !kept || kept.bottom ? list.scrollHeight : kept.top;
  }

  // A live, navigable preview of the mock-up as it stands right now: the
  // same /mock/{branch}/{*path} route the review step's easel reads pages
  // through, which already serves whatever is on disk — committed or still
  // mid-session — with no gate on the job being done. Only reloaded when
  // job.steps actually grew (counted on the frame's stage, which a redraw
  // keeps, so a freshly drawn pane starts correctly whatever ran before it
  // existed), and at
  // its *current* path rather than forced back to the index, so clicking
  // into another page survives the next tick.
  function drawPreview() {
    const j = job();
    const host = page.querySelector('[data-cr-preview]');
    const stage = host && host.querySelector('.cr-draw-stage');
    if (!j || !stage || (j.work !== 'mockups' && j.work !== 'componentize' && j.work !== 'initialize')) return;
    const seen = Number(stage.dataset.steps || 0);
    if (j.steps.length <= seen) return;
    stage.dataset.steps = String(j.steps.length);
    const frame = host.querySelector('.cr-draw-frame');
    if (!frame) return;
    if (frame.hidden) {
      frame.hidden = false;
      host.querySelector('.cr-draw-empty')?.setAttribute('hidden', '');
      const slug = (host.dataset.crPreview || '').replace(/\//g, '__');
      frame.src = `/mock/${encodeURIComponent(slug)}/index.html`;
    } else {
      frame.src = frame.src.replace(/\?.*$/, '') + '?v=' + Date.now();
    }
  }
  // Two columns while the job runs: the checklist on the left, the
  // preview on the right — a placeholder until the first page lands.
  function drawSplit(branch, track) {
    return `<div class="cr-draw-split${C.trackHidden ? ' cr-track-hidden' : ''}">
      <div class="cr-draw-track">
        ${C.trackHidden ? '' : '<div class="cr-draw-track-head"><button type="button" class="cr-btn" data-cr-track-hide title="Hide the checklist — more room for the preview">◂ Hide</button></div>'}
        ${track}
      </div>
      <div class="cr-draw-preview" data-cr-fullable data-cr-preview="${esc(branch)}">
        ${C.trackHidden ? '<button type="button" class="cr-draw-show" data-cr-track-show title="Show the checklist">▸</button>' : ''}
        <div class="cr-draw-head"><h3>Taking shape</h3>
          ${window.Viewport.markup({ fit: true })}
          <button type="button" class="cr-btn" data-cr-full title="Fill the screen (Esc to leave)">⛶ Full screen</button></div>
        <p class="muted cr-draw-empty" data-cr-keep="${esc(branch)}">Nothing drawn yet — the first page appears here once it is written.</p>
        <div class="cr-draw-stage" data-cr-keep="${esc(branch)}"><div class="cr-draw-sizer"${C.view.width || C.view.height ? ` style="${C.view.width ? `width:${C.view.width}px;` : ''}${C.view.height ? `height:${C.view.height}px;` : ''}"` : ''}>
          <iframe class="cr-draw-frame" title="the proof of concept, as it is drawn" hidden></iframe>
          <div class="cr-draw-handle" title="drag to resize"></div></div></div>
      </div>
    </div>`;
  }
  // The preview's picker follows the width kept; a width is a number, null is "fit".
  function syncPreview() {
    const head = page.querySelector('.cr-draw-head');
    if (head) window.Viewport.sync(head, C.view.width);
  }
  function setPreviewWidth(width) {
    C.view.width = width;
    const sizer = page.querySelector('.cr-draw-sizer');
    if (sizer) sizer.style.width = width ? `${width}px` : '';
    syncPreview();
  }
  // Set only by the corner handle's own drag — the width/height picker
  // never touches height, so a preset stays width-only as it already was.
  function setPreviewHeight(height) {
    C.view.height = height;
    const sizer = page.querySelector('.cr-draw-sizer');
    if (sizer) sizer.style.height = height ? `${height}px` : '';
  }

  // The demo: the finished job, against a branch already drawn for real —
  // the right-hand iframe is genuine, navigable content on disk, nothing
  // here starts a session or calls the server. The default view is the
  // end state; the persistent "simulate" button (bottom-left) replays the
  // step-by-step animation the real poller would have produced, once, and
  // stops on its own at the end rather than looping.
  const DEMO_PRELUDE = {
    mockups: ["opening the integration branch's mock-up", "a Claude Code session draws the proof of concept's pages"],
    initialize: ['opening the accepted mock-up', 'a Claude Code session makes the mock-up again of micro-frontends'],
  };
  function demoSteps(work) {
    const making = work === 'initialize';
    const total = DEMO_PAGES.length;
    const items = DEMO_PAGES.map((name, i) => (making ? `made the "${name}" micro-frontend real (${i + 1} of ${total})` : `drew the mock-up's "${name}" page (${i + 1} of ${total})`));
    return [...DEMO_PRELUDE[work], ...items, making ? 'every page is made of micro-frontends' : 'the mock-up is drawn'];
  }
  // A completed factory has no creation record any more — fabricate a
  // minimal one, just enough for each step and the crumbs to render; only
  // the branch (and what is on disk under it) needs to be real.
  function demoBase(step) {
    return {
      branch: DEMO_BRANCH,
      reachable: 'initialize',
      record: { step, mocked: '', componentized: '', issue: '', regenerations: 0, questions: [], scope: '', brief: '', kind: 'vertical', slug: 'Laucans/DND2024_DMSystem', rounds: 0, grilled: false, requirements: [], scope_from: '', scope_edited: false, mockup_at: '', design_at: '', design_refused: '' },
      kit: { validated: true, waiting: 0, generated: DEMO_PAGES.length, failed: 0, running: false, noClaude: false },
    };
  }
  function setDemoJob(work, steps, state) {
    C.data = { ...demoBase(work), job: { work, state, steps } };
  }
  // A floating button over the page, never part of a step's own markup —
  // it survives `paint()` wiping `page.innerHTML` on every redraw, and
  // only exists while ?demo=1 is on. Shown while the route has a demo
  // job (mockups/initialize); it replays that job's animation on click.
  function demoFloat() {
    let el = document.getElementById('cr-demo-float');
    if (el) return el;
    el = document.createElement('button');
    el.type = 'button';
    el.id = 'cr-demo-float';
    el.className = 'cr-demo-float';
    el.hidden = true;
    el.addEventListener('click', () => { if (demoWork) playDemo(demoWork); });
    document.body.appendChild(el);
    return el;
  }
  function drawDemoButton() {
    if (!DEMO) return;
    const el = demoFloat();
    el.hidden = !demoWork;
    el.disabled = demoPlaying;
    el.textContent = demoPlaying ? 'Simulating…' : '▶ Simulate the drawing';
  }
  function showDemoFinal(work) {
    clearInterval(demoTimer);
    demoTimer = null;
    demoPlaying = false;
    setDemoJob(work, demoSteps(work), 'done');
    draw();
    drawPreview();
  }
  function playDemo(work) {
    clearInterval(demoTimer);
    const full = demoSteps(work);
    const start = DEMO_PRELUDE[work].length;
    const host = page.querySelector('[data-cr-preview]');
    if (host) {
      const stage = host.querySelector('.cr-draw-stage');
      if (stage) stage.dataset.steps = '0';
      const frame = host.querySelector('.cr-draw-frame');
      if (frame) frame.hidden = true;
      host.querySelector('.cr-draw-empty')?.removeAttribute('hidden');
    }
    demoPlaying = true;
    setDemoJob(work, full.slice(0, start), 'running');
    draw();
    drawDemoButton();
    let i = start;
    demoTimer = setInterval(() => {
      i += 1;
      const done = i >= full.length;
      setDemoJob(work, full.slice(0, i), done ? 'done' : 'running');
      drawJob();
      drawPreview();
      if (done) { clearInterval(demoTimer); demoTimer = null; demoPlaying = false; }
      drawDemoButton();
    }, 1600);
  }

  function landing() {
    const r = record();
    const j = job();
    const connecting = j && j.work === 'connect';
    return `<div class="cr-landing">
      ${r ? `<div class="callout cr-resume">This factory is being created — step ${indexOf(r.step) + 1}, ${esc(STEPS[indexOf(r.step)][1].toLowerCase())}. <button type="button" class="cr-btn" data-cr-go="${esc(r.step)}">Resume</button></div>` : ''}
      <section class="cr-card">
        <h2>Connect an existing repository</h2>
        <p class="muted">A GitHub repository you already have: the harness prepares it (labels, the integration branch, its architecture's files) and the factory opens on its plant.</p>
        <form class="cr-row" data-cr-form="connect">
          <input type="text" name="url" placeholder="https://github.com/owner/repository or owner/repository" aria-label="repository URL" required autocomplete="off">
          <button type="submit" class="cr-btn on"${running() ? ' disabled' : ''}>Connect</button>
        </form>
        ${connecting ? '<div data-cr-job>' + jobPanel() + '</div>' : ''}
      </section>
      <section class="cr-card">
        <h2>Initialize a factory</h2>
        <p class="muted">A new repository, then nine steps: what the application does, the AI's questions, the proof of concept's scope, its design system, its mock-ups, its rules and its data — up to the factory's first roadmap task.</p>
        <div class="cr-row"><button type="button" class="cr-btn on" data-cr-go="${FOUND}">Start</button></div>
      </section>
    </div>`;
  }

  function stepRepository() {
    const r = record();
    if (r) {
      return `<p>The repository <a href="https://github.com/${esc(r.slug)}" target="_blank" rel="noopener">${esc(r.slug)} ↗</a> is created, prepared by <code>init-repo</code>, and this factory runs in its own folder.</p>
        <div class="cr-actions"><span></span><button type="button" class="cr-btn on" data-cr-next="kind">Continue →</button></div>`;
    }
    return foundForm();
  }

  // The repository of the factory being founded. Shown whatever the factory
  // open here is doing: its own creation, when it has one, is left alone —
  // the view switches to the new factory once `init-repo` is done.
  function foundForm() {
    const j = job();
    const founding = j && j.work === 'found';
    if (C.owners === null && !C.ownersWhy) loadOwners();
    const owners = C.owners || [];
    const beside = record();
    return `${beside ? `<div class="callout">This founds a factory beside <code>${esc(beside.slug)}</code>, the one open here: its creation stays where it is, and the view moves to the new factory once <code>init-repo</code> is done.</div>` : ''}
      <p class="muted">The factory's repository on GitHub. The harness creates it with a first commit, makes the factory's folder beside the register (<code>~/.harness/factories/</code>), runs <code>init-repo</code> there, then opens the new factory.</p>
      <form class="cr-form" data-cr-form="found">
        <label>Name<input type="text" name="name" required autocomplete="off" placeholder="my-application" pattern="[A-Za-z0-9._-]+"></label>
        <label>Owner<select name="owner" required>${owners.length ? owners.map((o) => `<option>${esc(o)}</option>`).join('') : `<option value="">${C.ownersWhy ? 'cannot read the accounts' : 'reading the accounts…'}</option>`}</select></label>
        ${C.ownersWhy ? `<p class="cr-bad">${esc(C.ownersWhy)}</p>` : ''}
        <label>Description<input type="text" name="description" autocomplete="off" placeholder="What it is, in one line"></label>
        <fieldset class="cr-radios"><legend>Visibility</legend>
          <label><input type="radio" name="visibility" value="private" checked> private</label>
          <label><input type="radio" name="visibility" value="public"> public</label>
        </fieldset>
        <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="landing">← Back</button><button type="submit" class="cr-btn on"${running() || C.opening ? ' disabled' : ''}>Create the factory</button></div>
      </form>
      ${founding ? '<div data-cr-job>' + jobPanel() + '</div>' : ''}`;
  }

  function stepKind(r) {
    return `<p class="muted">The kind of factory decides the architecture the products are built on, and the rooms of the plant.</p>
      <div class="cr-kinds" role="radiogroup" aria-label="factory type">${KINDS.map((k) => `<label class="cr-kind${k.soon ? ' soon' : ''}">
        <input type="radio" name="kind" value="${k.id}"${(r.kind || 'vertical') === k.id && !k.soon ? ' checked' : ''}${k.soon ? ' disabled' : ''}>
        <b>${esc(k.label)}</b><span class="muted">${esc(k.text)}</span>${k.soon ? '<span class="tag">later</span>' : ''}</label>`).join('')}</div>
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="repository">← Back</button><button type="button" class="cr-btn on" data-cr-kind>Continue →</button></div>`;
  }

  // The brief: a description told at large, which the AI sorts into a few
  // parts the human then rereads (`briefParts`, as the server names them).
  // The record keeps one text: the parts answered, each under its heading.
  // A text kept as parts under fixed headings: written from one field per
  // part, the empty ones left out, and split back into them.
  function composeParts(parts, heading, answer) {
    return parts
      .map((q) => [q, answer(q.id).trim()])
      .filter(([, text]) => text)
      .map(([q, text]) => `${heading(q)}\n\n${text}`)
      .join('\n\n');
  }
  // Text before any known heading goes to `home`; `stray`, given, turns a
  // heading no part knows into a line of `home`.
  function splitParts(parts, heading, text, home, stray) {
    const answers = Object.fromEntries(parts.map((q) => [q.id, []]));
    if (!parts.length) return answers;
    let current = home;
    for (const line of (text || '').split('\n')) {
      const q = parts.find((b) => line.trim() === heading(b));
      const moved = q ? null : stray && stray(line);
      if (q) current = q.id;
      else if (moved) { current = home; answers[home].push(moved); }
      else answers[current].push(line);
    }
    return Object.fromEntries(Object.entries(answers).map(([id, lines]) => [id, lines.join('\n').trim()]));
  }
  const briefParts = () => (C.data && C.data.briefParts) || [];
  const briefHeading = (q) => `### ${q.title}`;
  const composeBrief = (answer) => composeParts(briefParts(), briefHeading, answer);
  // A brief written before the questions, without their headings, goes
  // whole to the first.
  const splitBrief = (text) => splitParts(briefParts(), briefHeading, text, (briefParts()[0] || {}).id);

  // The parts show once the description was split, or the human chose to
  // fill them alone.
  const split = (r) => !!r.brief.trim() || C.parts;
  function stepBrief(r) {
    const parts = briefParts();
    const answers = splitBrief(r.brief);
    const shown = split(r);
    const sorting = C.busy === SORTING;
    const button = shown
      ? `<button type="button" class="cr-btn" data-cr-decompose${sorting ? ' disabled' : ''}>${C.confirm === 'decompose' ? 'The parts are replaced. Click again' : 'Split again'}</button>`
      : `<button type="button" class="cr-btn on" data-cr-decompose${sorting ? ' disabled' : ''}>Split into the ${parts.length} parts</button><button type="button" class="cr-btn" data-cr-parts>Fill them myself</button>`;
    return `<p class="muted">What the application does, told at large: the need, who uses it, what they do with it, what it keeps. The AI then splits it into ${parts.length} parts you reread — the more context, the better its questions, the scope and the mock-ups.</p>
      ${r.regenerations ? '<div class="callout">Sent back from the review: everything written is still here — change what the mock-ups got wrong.</div>' : ''}
      <label class="cr-field cr-brief-q"><b>The application, in your words</b><textarea data-cr="description" rows="${shown ? 4 : 10}">${esc(r.description)}</textarea></label>
      <div class="cr-actions">${button}</div>
      ${shown ? '' : `<p class="muted">Left for another part of the page — the design system's material below — for ${AUTO_SPLIT_MS / 1000} seconds, the description is split on its own.</p>`}
      ${shown ? parts.map((q, i) => `<label class="cr-field cr-brief-q"><b>${i + 1} · ${esc(q.title)}</b><span class="muted">${esc(q.hint)}</span><textarea data-cr="brief-${q.id}" rows="4">${esc(answers[q.id])}</textarea></label>`).join('') : ''}
      <p class="muted">Below, the design system's material — the logo, the moodboard, a design brief. On <b>Continue</b>, the tokens and the kit draw from it while you answer the grilling; you find them generated at their step. The scope waits for the grilling's decisions.</p>`;
  }
  function briefFoot() {
    const j = job();
    return `${j && j.work === 'prepare' ? `<div data-cr-job>${jobPanel()}</div>` : ''}
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="kind">← Back</button>${record() && split(record()) ? '<button type="button" class="cr-btn on" data-cr-next="grilling">Continue →</button>' : ''}</div>`;
  }
  // The description sent to the AI, its parts written back in their fields.
  const SORTING = 'the AI splits the description into its parts';
  function decompose() {
    const field = page.querySelector('[data-cr="description"]');
    const description = field ? field.value : '';
    if (!description.trim()) { C.error = 'describe the application first'; draw(); return; }
    disarmSplit();
    act(SORTING, async () => {
      C.data = await call('POST', '/api/creation/decompose', { description });
      for (const q of briefParts()) delete C.dirty[`brief-${q.id}`];
      if (C.dirty.description === description) delete C.dirty.description;
    });
  }
  // The description left for another part of the page — the design
  // system's material, a field — splits on its own after AUTO_SPLIT_MS,
  // before the first split only: a later one would overwrite the human's
  // rereading. Counted only while this page keeps the focus: another tab or
  // another window cancels it (a frame of the page keeps it, so the easel's
  // own fields count as the page).
  const AUTO_SPLIT_MS = 10000;
  let splitTimer = null;
  function disarmSplit() { clearInterval(splitTimer); splitTimer = null; }
  function armSplit() {
    disarmSplit();
    const since = Date.now();
    splitTimer = setInterval(() => {
      if (document.hidden || !document.hasFocus() || C.route !== 'brief' || page.hidden) { disarmSplit(); return; }
      if (Date.now() - since < AUTO_SPLIT_MS) return;
      disarmSplit();
      const r = record();
      const field = page.querySelector('[data-cr="description"]');
      if (!r || split(r) || C.busy || !field || !field.value.trim() || document.activeElement === field) return;
      decompose();
    }, 500);
  }
  page.addEventListener('focusout', (e) => { if (e.target.matches('[data-cr="description"]')) armSplit(); });
  page.addEventListener('focusin', (e) => { if (e.target.matches('[data-cr="description"]')) disarmSplit(); });

  function stepGrilling(r) {
    const qs = r.questions;
    // While a round is asked, its button spins.
    const asking = C.busy === GRILL_FIRST || C.busy === GRILL_NEXT;
    const open = qs.filter((q) => !q.answer.trim()).length;
    // The last round's questions stand out until answered; an answered one greys.
    const typed = (q) => C.dirty[`q${q.id}`] ?? q.answer;
    const look = (q) => (typed(q).trim() ? ' done' : q.round === r.rounds ? ' new' : '');
    const card = (q) => `<article class="cr-q${look(q)}" data-q="${q.id}">
        <header><span class="cr-qn">Q${q.id}</span><b>${esc(q.title)}</b>${q.round ? `<span class="tag">round ${q.round}</span>` : ''}<button type="button" class="mini cr-del" data-cr-del="${q.id}" title="Delete this question" aria-label="delete question ${q.id}">✕</button></header>
        ${q.body ? `<p class="cr-qb">${esc(q.body)}</p>` : ''}
        ${q.recommended ? `<p class="cr-rec">➡️ ${esc(q.recommended)} <button type="button" class="mini" data-cr-take="${q.id}">use it</button></p>` : ''}
        <textarea rows="3" aria-label="answer to question ${q.id}" placeholder="Your answer">${esc(q.answer)}</textarea>
      </article>`;
    // One card per topic the AI filed its questions under, the general one first.
    const topics = [''];
    for (const q of qs) if (!topics.includes(q.topic || '')) topics.push(q.topic || '');
    const list = topics.map((t) => {
      const under = qs.filter((q) => (q.topic || '') === t);
      if (!under.length) return '';
      const left = under.filter((q) => !q.answer.trim()).length;
      return `<section class="cr-topic"><header><b>${t ? esc(t) : 'General'}</b><span class="tag">${left ? `${left} open · ` : ''}${under.length} question${under.length === 1 ? '' : 's'}</span></header>${under.map(card).join('')}</section>`;
    }).join('');
    const j = job();
    return `<p class="muted">The AI grills the brief round after round: every decision it can ask now, each with its recommended answer. Answer, retouch, or delete a question — or take the recommendations as they are.</p>
      ${j && j.work === 'prepare' && j.state === 'running' ? '<div class="callout cr-aside"><span class="cr-spin" aria-hidden="true"></span> Meanwhile: the design system\'s tokens and the kit draw in the background.</div>' : ''}
      ${j && j.work === 'prepare' && j.state === 'failed' ? `<div class="callout cr-aside">The preparation stopped: ${esc(j.why)} — the design system waits at its step.</div>` : ''}
      ${list || (C.busy ? '' : '<p class="muted">No question yet.</p>')}
      ${r.grilled ? '<div class="callout">The AI has no more questions.</div>' : ''}
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="brief">← Back</button>
        <span class="cr-gap"></span>
        ${open ? `<button type="button" class="cr-btn" data-cr-take-all title="Every open question takes its recommended answer">Use the ${open} recommendation${open === 1 ? '' : 's'}</button>` : ''}
        ${asking
    ? '<button type="button" class="cr-btn busy" data-cr-grill disabled><span class="cr-spin" aria-hidden="true"></span> Generating the round…</button>'
    : `<button type="button" class="cr-btn" data-cr-grill title="${open ? 'Answer the open questions first' : 'Ask the next round of questions'}"${open || C.busy ? ' disabled' : ''}>${qs.length ? 'Continue grilling' : 'Start the grilling'}</button>`}
        <button type="button" class="cr-btn on" data-cr-next="scope"${!qs.length && !r.grilled ? ' disabled' : ''}>Continue →</button></div>`;
  }

  // The scope: one field per section the server names (`scopeSections`),
  // each with its `?`. The record keeps one text, each section under its
  // `## ` heading; a heading no section knows goes to the free one, as a
  // sub-section.
  const scopeSections = () => (C.data && C.data.scopeSections) || [];
  const scopeHeading = (s) => `## ${s.title}`;
  const SCOPE_FREE = 'notes';
  const strayHeading = (line) => (/^##\s+\S/.test(line) ? `### ${line.replace(/^##\s+/, '').trim()}` : null);
  const composeScope = (answer) => composeParts(scopeSections(), scopeHeading, answer);
  const splitScope = (text) => splitParts(scopeSections(), scopeHeading, text, SCOPE_FREE, strayHeading);
  // The required sections still empty, as the fields hold them.
  const scopeMissing = () => scopeSections().filter((s) => s.required).filter((s) => {
    const field = page.querySelector(`[data-cr="scope-${s.id}"]`);
    return !(field ? field.value : splitScope(record().scope)[s.id]).trim();
  });
  function stepScope(r) {
    const stale = C.data.scopeStale && !C.busy;
    const answers = splitScope(r.scope);
    const missing = scopeSections().filter((s) => s.required && !(C.dirty[`scope-${s.id}`] ?? answers[s.id]).trim());
    const section = (s) => {
      const open = C.helps.includes(`scope-${s.id}`);
      return `<div class="cr-field cr-brief-q">
        <div class="cr-scope-head"><b>${esc(s.title)}</b>${s.required ? '<span class="tag">required</span>' : ''}<button type="button" class="help-btn${open ? ' on' : ''}" data-cr-help="scope-${s.id}" title="What is this section for?" aria-expanded="${open}">?</button></div>
        ${open ? `<div class="callout purpose">${esc(s.help)}</div>` : ''}
        <span class="muted">${esc(s.hint)}</span>
        <textarea class="cr-fit" data-cr="scope-${s.id}" rows="2" aria-label="${esc(s.title)}">${esc(answers[s.id] || '')}</textarea></div>`;
    };
    return `<p class="muted">The smallest proof of concept that shows the application's value, as the AI reads the brief and the decisions. Edit it freely, section by section — <b>?</b> says what each one is for.</p>
      ${C.busy === PROPOSING && !r.scope.trim() ? '<div class="callout"><span class="cr-spin" aria-hidden="true"></span> The scope is being proposed from the brief and the grilling\'s decisions.</div>' : ''}
      ${stale ? `<div class="callout">The decisions moved since this scope was proposed${r.scope_edited ? ' — and you retouched it: <b>Propose again</b> replaces your edits with a scope read from the decisions as they are now' : ': it is proposed again from them'}.</div>` : ''}
      ${scopeSections().map(section).join('')}
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="grilling">← Back</button>
        <span class="cr-gap"></span>
        <button type="button" class="cr-btn" data-cr-rescope>${C.confirm === 'scope' ? 'Replace my edits? Click again' : 'Propose again'}</button>
        <button type="button" class="cr-btn on" data-cr-next="design"${missing.length ? ` disabled title="Fill ${esc(missing.map((s) => s.title).join(', '))} first"` : ''}>Continue →</button></div>`;
  }

  // The step's head: the design system's state and the kit's. The easel
  // itself is mounted under it (`mountKit`), the actions close the step (`designFoot`).
  function stepDesign() {
    const k = C.data.kit || { validated: C.data.designValidated, waiting: 0, generated: 0, failed: 0, running: false, noClaude: false };
    const kitLine = !k.validated ? '<span class="muted">The kit is generated once the design system is validated.</span>'
      : `<span class="tag ${k.waiting ? 'warn' : 'ok'}">kit: ${k.generated} generated${k.waiting ? ` · ${k.waiting} waiting` : ''}${k.failed ? ` · ${k.failed} failed` : ''}</span>${k.running ? '<span class="cr-spin" aria-hidden="true"></span><span class="muted">generating…</span>' : ''}`;
    return `<p class="muted">The design system the mock-ups and the product wear: a moodboard and the logo, a brief, the tokens an AI draws from them, validated by you — then the kit of components, generated from it. It lives on the <code>design-system</code> branch.</p>
      <div class="cr-card"><div class="cr-row"><span>${k.validated ? '<span class="tag ok">validated</span>' : '<span class="tag warn">not validated yet</span>'}</span>
        ${kitLine}
        ${k.validated && (k.waiting || k.failed) && !k.running ? `<button type="button" class="cr-btn" data-cr-kit${k.noClaude ? ' disabled title="No Claude at hand"' : ''}>${k.failed ? 'Generate the missing components' : 'Generate the kit'}</button>` : ''}</div>
        ${k.failed && !k.waiting ? '<p class="muted">Some components failed: the mock-ups do without them, or generate them again.</p>' : ''}</div>`;
  }
  function designFoot() {
    const k = C.data.kit || { validated: C.data.designValidated, waiting: 0 };
    const ready = k.validated && !k.waiting;
    return `<div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="scope">← Back</button><button type="button" class="cr-btn on" data-cr-next="mockups"${ready ? '' : ` disabled title="${k.validated ? 'Wait for the kit' : 'Validate the design system first'}"`}>Continue →</button></div>`;
  }

  function stepMockups(r) {
    const j = job();
    const mine = j && j.work === 'mockups';
    const failed = mine && j.state === 'failed';
    // Drawn ahead, it may still be cut into micro-frontends: the easel waits for it.
    const making = !DEMO && running('componentize');
    // While it draws: the checklist and a live preview. Drawn: the easel takes the room (see EMBEDS).
    const split = DEMO ? mine : running('mockups') || making;
    const spinner = !DEMO && running('mockups') ? '<div class="cr-loading"><span class="cr-spin big" aria-hidden="true"></span><b>Drawing the proof of concept…</b></div>'
      : making ? '<div class="cr-loading"><span class="cr-spin big" aria-hidden="true"></span><b>Making the micro-frontends…</b></div>' : '';
    const track = `${spinner}<div data-cr-files>${filesPanel()}</div><div data-cr-job>${mine || making ? jobPanel() : ''}</div>`;
    return `${split ? drawSplit(C.data.branch, track) : `${mine && !r.mocked ? `<div data-cr-job>${jobPanel()}</div>` : ''}<div data-cr-files>${filesPanel()}</div>`}
      ${failed ? `<div class="cr-error" role="alert">${esc(j.why)}</div>` : ''}
      ${C.data.mockupStale && !running() ? '<div class="callout">Drawn from an earlier scope: the scope moved since — draw them again to follow it.</div>' : ''}
      ${r.mocked && !running('mockups') ? `<div class="callout">${esc(r.mocked)}</div>` : ''}
      ${embedFor('mockups') ? '' : mockupsFoot()}`;
  }
  // What the mock-up holds, as a session writes it: pages, micro-frontends,
  // fixtures, each ticked with when it landed — read every few seconds while
  // the step is open, whatever started the drawing.
  const FILE_KINDS = [['page', 'Pages'], ['micro-frontend', 'Micro-frontends'], ['fixture', 'Fixtures']];
  const FILES_MS = 4000;
  const clock = (at) => (at ? new Date(at * 1000).toTimeString().slice(0, 5) : '');
  function filesPanel() {
    const files = C.files || [];
    // A micro-frontend's draft is a proposal on the canvas, not a session drawing.
    const last = files.filter((f) => f.kind !== 'draft').reduce((m, f) => Math.max(m, f.at || 0), 0);
    // A file written in the last minute and a half: a session is at work.
    const writing = running('mockups') || running('componentize') || (last && Date.now() / 1000 - last < 90);
    const notice = writing ? '<div class="callout">Drawing the mock-up takes <b>several tens of minutes</b>: the fixtures and the micro-frontends come first, the pages last. You may leave this page — the work goes on.</div>' : '';
    const count = (kind) => files.filter((f) => f.kind === kind).length;
    const counts = FILE_KINDS.map(([kind, label]) => `<span><b>${count(kind)}</b> ${label.toLowerCase()}</span>`).join('');
    const timer = drawingTime();
    const head = files.length || timer ? `<div class="cr-files-head">${counts}<span class="muted">${files.length} files${last ? ` · last written ${clock(last)}` : ''}${timer ? ` · <span data-cr-timer title="How long the drawing has been at work">${timer}</span>` : ''}</span></div>` : '';
    return notice + head;
  }
  // How long the drawing (or the cut into micro-frontends) has been at
  // work — frozen at its end; empty when no such job is known.
  function drawingTime() {
    const j = job();
    if (!j || !j.started || (j.work !== 'mockups' && j.work !== 'componentize')) return '';
    const secs = Math.max(0, (j.ended || Math.floor(Date.now() / 1000)) - j.started);
    const h = Math.floor(secs / 3600);
    const mm = String(Math.floor((secs % 3600) / 60)).padStart(h ? 2 : 1, '0');
    const ss = String(secs % 60).padStart(2, '0');
    return `${j.ended ? 'took' : 'drawing for'} ${h ? `${h}:` : ''}${mm}:${ss}`;
  }
  async function loadFiles() {
    if (C.route !== 'mockups' || !C.data || !C.data.branch) { clearInterval(C.filesTimer); C.filesTimer = null; return; }
    try {
      const got = await call('GET', `/api/mockup/files?branch=${encodeURIComponent(C.data.branch)}`);
      C.files = got.files || [];
    } catch (e) { /* no mock-up read here: nothing to list */ }
    const host = page.querySelector('[data-cr-files]');
    if (host) host.innerHTML = filesPanel();
  }
  function followFiles() {
    if (C.route === 'mockups' && !C.filesTimer && !DEMO) { loadFiles(); C.filesTimer = setInterval(loadFiles, FILES_MS); }
    // The drawing's time ticks every second, on its own span only.
    if (C.route === 'mockups' && !C.clockTimer && !DEMO) {
      C.clockTimer = setInterval(() => {
        if (C.route !== 'mockups') { clearInterval(C.clockTimer); C.clockTimer = null; return; }
        const span = page.querySelector('[data-cr-timer]');
        if (span) span.textContent = drawingTime();
      }, 1000);
    }
  }
  function mockupsFoot() {
    const r = record();
    const j = job();
    const mine = j && j.work === 'mockups';
    const failed = mine && j.state === 'failed';
    return `<div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="design"${running('mockups') ? ' disabled' : ''}>← Back</button>
        <span class="cr-gap"></span>
        ${!running() && (failed || !mine || r.mocked) ? `<button type="button" class="cr-btn" data-cr-draw>${r.mocked || failed ? 'Draw again' : 'Draw the mock-ups'}</button>` : ''}
        <button type="button" class="cr-btn on" data-cr-next="rules"${r.mocked && !running('mockups') ? '' : ' disabled'}>Rules →</button></div>`;
  }

  // The rules step: the rules beside the mock-up's easel (`mountEmbed`),
  // the choices under them (`rulesFoot`). A rule picked shows what carries
  // it on the easel: its micro-frontends alone in the list, marked on the
  // page, its first page shown.
  const FAMILIES = [['journey', 'Journey', 'A step of a user\'s journey: what the user does and sees'], ['business', 'Business', 'An invariant, a computation, what the application keeps — the data step makes invariants of them']];
  const rules = () => C.rules || (record() ? record().requirements : []);
  // The simple view, for a client who is no developer: no architecture tab,
  // the blocking findings said in the rules tab. A per-viewer preference.
  const SIMPLE = 'harness.creation.simple';
  const simple = () => { try { return localStorage.getItem(SIMPLE) === '1'; } catch (e) { return false; } };
  const CHECKS = { duplicate: 'Duplicate', naming: 'Naming', system_size: 'System size', mixed_systems: 'Reaches into another system', write_as_read: 'A write passed off as a read', capability_calls: 'A Capability leaning on another' };
  // A change to the rules: made on the page's copy, kept a moment later.
  function editRules(fn, now) {
    C.rules = C.rules || JSON.parse(JSON.stringify(record().requirements));
    fn(C.rules);
    C.rulesRev = (C.rulesRev || 0) + 1;
    keepRequirements(now);
  }
  const ruleOf = (id) => rules().find((q) => q.id === id);
  function stepRules(r) {
    const j = job();
    const making = j && j.work === 'componentize';
    const head = `<p class="muted">The rules the proof of concept must meet, read off the scope and the mock-up: each a step of a <b>journey</b> or a <b>business</b> rule, each with the criterion a test checks, each carried by pages and micro-frontends. Pick one to see what carries it on the mock-up.</p>
      ${making && j.state !== 'done' ? `<div data-cr-job>${jobPanel()}</div>` : ''}
      ${making && j.state === 'failed' ? '<div class="cr-actions"><span></span><button type="button" class="cr-btn" data-cr-mf>Make the micro-frontends again</button></div>' : ''}`;
    if (!r.componentized) return head + '<p class="muted">The rules come once the mock-up is made of micro-frontends: they are tied to them.</p>';
    const blocking = r.audit ? r.audit.findings.filter((f) => f.check === 'write_as_read').length : 0;
    const architecture = C.rulesTab === 'architecture' && !simple();
    const tabs = `<div class="cr-tabs" role="tablist">
        <button type="button" role="tab" class="${architecture ? '' : 'on'}" data-cr-tab="rules" aria-selected="${!architecture}">Rules</button>
        ${simple() ? '' : `<button type="button" role="tab" class="${architecture ? 'on' : ''}" data-cr-tab="architecture" aria-selected="${architecture}">Architecture${r.audit && r.audit.findings.length ? ` <span class="tag${blocking ? ' bad' : ''}">${r.audit.findings.length}</span>` : ''}</button>`}
        <span class="spacer"></span>
        <label class="cr-simple" title="For a client who is no developer: the architecture tab hidden, only what blocks said in plain words"><input type="checkbox" data-cr-simple${simple() ? ' checked' : ''}> simple view</label></div>`;
    return head + tabs + (architecture ? architecturePanel(r) : (simple() ? plainBlockers(r) : '') + rulesPanel(r));
  }
  // What blocks, in plain words, for the simple view.
  function plainBlockers(r) {
    const list = r.audit ? r.audit.findings.filter((f) => f.check === 'write_as_read') : [];
    return list.map((f) => `<div class="cr-error" role="alert">${esc(f.why)} <button type="button" class="cr-btn" data-cr-finding="${f.id}" data-how="apply"${C.busy ? ' disabled' : ''}>Fix it</button></div>`).join('');
  }
  // The architecture's guard: what it found, then the cut, system by system.
  function architecturePanel(r) {
    const mock = C.mock || { pages: [], components: [] };
    const audit = r.audit;
    const effects = (audit && audit.effects) || {};
    const comps = mock.components || [];
    const systems = [...new Set(comps.map((c) => c.system || '—'))].sort();
    const reads = new Set(comps.flatMap((c) => c.needs || []));
    const writes = new Set(comps.flatMap((c) => c.actions || []));
    const parallel = [...writes].filter((w) => effects[w] === 'insert').length;
    const sequential = [...writes].filter((w) => effects[w] && effects[w] !== 'insert').length;
    const head = `<div class="cr-rules-head">
        <span class="muted">${systems.length} systems · ${comps.length} micro-frontends · ${reads.size} Capabilities · ${writes.size} DataCapabilities${audit ? ` (${parallel} parallel · ${sequential} sequential)` : ''}</span>
        <span class="spacer"></span>
        ${C.data.auditStale ? '<span class="cr-chip warn" title="The cut into micro-frontends moved since the guard read it">to redo</span>' : ''}
        <button type="button" class="cr-btn" data-cr-audit${C.busy ? ' disabled' : ''} title="The architecture's guard reads the cut: names, systems, duplicates, writes passed off as reads">${audit ? 'Audit again' : 'Audit'}</button></div>`;
    const findings = audit
      ? (audit.findings.length ? audit.findings.map((f) => `<article class="cr-finding${f.check === 'write_as_read' ? ' bad' : ''}">
          <header><b>${esc(CHECKS[f.check] || f.check || 'finding')}</b>${f.check === 'write_as_read' ? ' <span class="tag bad">blocks</span>' : ''}<span class="spacer"></span>
            <button type="button" class="mini" data-cr-finding="${f.id}" data-how="apply"${C.busy ? ' disabled' : ''} title="A Claude Code session applies the fix to the mock-up">apply</button>
            ${f.check === 'write_as_read' ? '' : `<button type="button" class="mini" data-cr-finding="${f.id}" data-how="dismiss"${C.busy ? ' disabled' : ''} title="Keep it as it is">dismiss</button>`}</header>
          <div class="cr-chips">${(f.units || []).map((u) => `<code>${esc(u)}</code>`).join(' ')}</div>
          <p>${esc(f.why)}</p>${f.fix ? `<p class="muted">→ ${esc(f.fix)}</p>` : ''}</article>`).join('') : '<p class="muted">The guard found nothing to say.</p>')
      : '<p class="muted">The guard has not read the cut yet.</p>';
    const tagOf = (w) => (effects[w] ? (effects[w] === 'insert' ? '<span class="tag ok" title="A pure addition: its task runs beside the others">parallel</span>' : `<span class="tag" title="${esc(effects[w])}: its task runs alone">sequential</span>`) : '');
    const slices = systems.map((system) => {
      const own = comps.filter((c) => (c.system || '—') === system);
      return `<section class="cr-slice"><h3>${esc(system)} <small>${own.length}</small></h3>${own.map((c) => `<div class="cr-slice-mf"><b>${esc(c.name)}</b>
          ${(c.needs || []).length ? `<div><span class="muted">reads</span> ${c.needs.map((n) => `<code>${esc(n)}</code>`).join(' ')}</div>` : ''}
          ${(c.actions || []).length ? `<div><span class="muted">writes</span> ${c.actions.map((a) => `<code>${esc(a)}</code>${tagOf(a)}`).join(' ')}</div>` : ''}</div>`).join('')}</section>`;
    }).join('');
    return head + `<div class="cr-findings">${findings}</div><h3 class="cr-sub">The cut</h3><div class="cr-slices">${slices}</div>`;
  }
  // The rules the filters let through: this page only, the micro-frontend
  // locked on the easel, the words searched (a rule, a page, a micro-frontend).
  function shownRules(list, mock) {
    const pages = mock.pages || [];
    const here = C.rulesHere ? C.canvasPage : '';
    const onHere = new Set(here ? (pages.find((p) => p.path === here) || {}).components || [] : []);
    const titleOf = (path) => (pages.find((p) => p.path === path) || {}).title || '';
    const words = String(C.rulesQuery || '').trim().toLowerCase().split(/\s+/).filter(Boolean);
    return list.filter((q) => {
      if (here && !q.pages.includes(here) && !q.components.some((c) => onHere.has(c))) return false;
      // The open pin's micro-frontends stand for the lock while it is open.
      const pinned = C.pinMfs || [];
      if (pinned.length ? !q.components.some((c) => pinned.includes(c)) : C.rulesMf && !q.components.includes(C.rulesMf)) return false;
      const text = [`r${q.id}`, q.title, ...q.pages, ...q.pages.map(titleOf), ...q.components].join(' ').toLowerCase();
      return words.every((w) => text.includes(w));
    });
  }
  function rulesFilter(shown, total) {
    const pinned = C.pinMfs || [];
    const filtering = (C.rulesHere && C.canvasPage) || C.rulesMf || pinned.length || String(C.rulesQuery || '').trim();
    return `<div class="cr-rules-filter">
        <input type="search" class="cr-rules-search" data-cr-rules-q value="${esc(C.rulesQuery || '')}" placeholder="Filter by micro-frontend, page or rule" aria-label="Filter the rules">
        <label class="cr-simple" title="Only the rules tied to the page the mock-up shows, or to one of its micro-frontends"><input type="checkbox" data-cr-rules-here${C.rulesHere ? ' checked' : ''}${C.canvasPage ? '' : ' disabled'}> this page only</label>
        ${pinned.length ? `<span class="cr-tie pin" title="The micro-frontends the open pin touches: close the pin to let go"><span class="cr-tie-name">pin: ${pinned.map(esc).join(', ')}</span></span>` : ''}
        ${!pinned.length && C.rulesMf ? `<span class="cr-tie on" title="Locked on the mock-up: click it there again to let go"><span class="cr-tie-name">${esc(C.rulesMf)}</span><button type="button" class="mini" data-cr-rules-unlock title="Let go of this micro-frontend" aria-label="let go of ${esc(C.rulesMf)}">✕</button></span>` : ''}
        ${filtering ? `<span class="muted">${shown} of ${total}</span>` : ''}</div>`;
  }
  function rulesPanel(r) {
    const mock = C.mock || { pages: [], components: [] };
    const all = rules();
    const list = shownRules(all, mock);
    const gaps = (q) => { const d = danglingOf(q); return [critMissing(q).length ? 'criterion to complete' : '', !(q.pages.length || q.components.length) ? 'carried by nothing' : '', d.pages.length + d.components.length ? 'broken links' : ''].filter(Boolean); };
    const broken = danglingGaps().length;
    const lacking = all.filter((q) => gaps(q).length).length;
    const proposals = r.proposals || [];
    const changing = new Map(proposals.filter((p) => p.id).map((p) => [p.id, p]));
    const fresh = shownRules(proposals.filter((p) => !p.id), mock);
    const group = ([family, label, hint]) => {
      const items = list.filter((q) => (q.family || 'journey') === family);
      const added = fresh.filter((p) => (p.family || 'journey') === family);
      const cards = items.map((q) => (changing.has(q.id) ? proposedCard(changing.get(q.id)) : ruleCard(q, gaps(q), mock))).join('') + added.map(proposedCard).join('');
      return `<section class="cr-rule-group"><h3 title="${esc(hint)}">${label} <small>${items.length + added.length}</small></h3>${cards || '<p class="muted">none</p>'}</section>`;
    };
    refreshRulesStatus(r);
    return `<div class="cr-rules-head">
        <span data-cr-rules-summary>${rulesSummary()}</span>
        ${proposals.length ? `<span class="tag warn">${proposals.length} change${proposals.length > 1 ? 's' : ''} proposed</span>` : ''}
        ${r.rules_validated ? '<span class="tag ok">validated</span>' : lacking ? `<span class="tag bad">${lacking} rule${lacking > 1 ? 's' : ''} to complete</span>` : all.length ? '<span class="tag">ready to validate</span>' : ''}
        <span class="spacer"></span>
        ${broken ? `<button type="button" class="cr-btn" data-cr-unbreak title="Untie every rule from the pages and micro-frontends the mock-up has not">Remove ${broken} broken link${broken > 1 ? 's' : ''}</button>` : ''}
        <button type="button" class="cr-btn" data-cr-req-add>+ rule</button>
        <button type="button" class="cr-btn" data-cr-reqs title="The AI reads the rules again off the scope and the mock-up as it stands; yours are kept"${C.busy ? ' disabled' : ''}>${all.length ? 'Read all again' : 'Read the rules'}</button></div>
      ${all.length ? rulesFilter(list.length, all.length) : ''}
      <div class="cr-rules">${all.length || proposals.length ? FAMILIES.map(group).join('') : (C.busy ? '' : '<p class="muted">No rule yet.</p>')}</div>`;
  }
  // A rule the AI proposes to change (or to add) for a request on the
  // mock-up: its new version in place, outlined until validated or refused,
  // the rule as it is now folded under it.
  function proposedCard(p) {
    const was = p.id ? ruleOf(p.id) : null;
    const crit = (c) => `<div class="cr-crit">${PARTS.map(([k, label]) => `<div class="cr-crit-line"><b>${label}</b><span>${esc((c || {})[k] || '') || '<i class="muted">to write</i>'}</span></div>`).join('')}</div>`;
    const ties = [...(p.pages || []), ...(p.components || [])];
    return `<article class="cr-rule proposed">
        <header><span class="cr-qn">${p.id ? 'R' + p.id : 'new'}</span><span class="tag warn" title="${esc(p.about)}">${p.id ? 'changed' : 'new'} by a request</span><span class="spacer"></span>
          <button type="button" class="cr-btn on" data-cr-proposal="${p.key}" data-how="accept"${C.busy ? ' disabled' : ''}>Validate</button>
          <button type="button" class="cr-btn" data-cr-proposal="${p.key}" data-how="refuse"${C.busy ? ' disabled' : ''}>Refuse</button></header>
        <b class="cr-proposed-title">${esc(p.title)}</b>
        ${p.why ? `<p class="muted">${esc(p.why)}</p>` : ''}
        ${p.body ? `<p>${esc(p.body)}</p>` : ''}
        ${crit(p.criterion)}
        <div class="cr-rule-cover">${ties.length ? ties.map((x) => `<code>${esc(x)}</code>`).join(' ') : '<span class="tag bad">carried by nothing</span>'}</div>
        ${was ? `<details><summary>R${was.id} as it is now</summary><p><b>${esc(was.title)}</b>${was.body ? ' — ' + esc(was.body) : ''}</p>${crit(was.criterion)}</details>` : ''}
      </article>`;
  }
  // A rule's ties to what the mock-up has not (gone with a discard or a new
  // drawing): shown red, counted as gaps, removed on a click.
  function danglingOf(q) {
    const mock = C.mock;
    if (!mock) return { pages: [], components: [] };
    const pages = new Set((mock.pages || []).map((p) => p.path));
    const comps = new Set((mock.components || []).map((c) => c.name));
    return { pages: q.pages.filter((p) => !pages.has(p)), components: q.components.filter((c) => !comps.has(c)) };
  }
  const danglingGaps = () => rules().flatMap((q) => { const d = danglingOf(q); return [...d.pages.map((p) => `R${q.id} points at ${p}, which the mock-up has not`), ...d.components.map((c) => `R${q.id} is tied to ${c}, which the mock-up has not`)]; });
  // A criterion's three parts, and those still to write.
  const PARTS = [['given', 'Given'], ['when', 'When'], ['then', 'Then']];
  const critOf = (q) => Object.assign({ given: '', when: '', then: '' }, typeof q.criterion === 'object' && q.criterion ? q.criterion : {});
  const critMissing = (q) => { const c = critOf(q); return PARTS.filter(([k]) => !String(c[k] || '').trim()).map(([k]) => k); };
  // The criterion, three fields on the open card; a folded one shows its description only.
  function criterionBox(q) {
    const c = critOf(q);
    const missing = critMissing(q);
    return `<div class="cr-crit open${missing.length ? ' bad' : ''}">${PARTS.map(([k, label]) => `<label class="cr-crit-line"><b>${label}</b><textarea rows="1" class="cr-fit${missing.includes(k) ? ' bad' : ''}" data-cr-rule-field="criterion.${k}" placeholder="${k === 'given' ? 'the situation before' : k === 'when' ? 'what happens' : 'what must then be true'}" aria-label="rule ${q.id}, ${k}">${esc(c[k])}</textarea></label>`).join('')}</div>`;
  }
  // The parts tied to a rule, and a + that lists the others to add.
  function ties(q, kind, all, keyOf, labelOf, extra) {
    const on = kind === 'page' ? q.pages : q.components;
    const chips = on.map((key) => {
      const it = all.find((x) => keyOf(x) === key);
      const label = it ? labelOf(it) : key;
      const pick = !it ? ' title="The mock-up has not this any more"' : kind === 'page' ? ` data-cr-rule-page="${esc(key)}" title="Show this page"` : '';
      return `<span class="cr-tie${kind === 'page' && C.rulePage === key ? ' on' : ''}${it ? '' : ' dangling'}"><button type="button" class="cr-tie-name"${pick}>${esc(label)}</button>${extra && it ? extra(key) : ''}<button type="button" class="mini" data-cr-untie="${kind}" data-key="${esc(key)}" title="Not carried by it" aria-label="untie ${esc(label)}">✕</button></span>`;
    }).join('');
    const rest = all.filter((x) => !on.includes(keyOf(x)));
    const open = C.addOpen && C.addOpen.rule === q.id && C.addOpen.kind === kind;
    const add = rest.length ? `<span class="cr-add"><button type="button" class="cr-tie-add" data-cr-add="${kind}" aria-expanded="${open}">+ ${kind === 'page' ? 'page' : 'micro-frontend'}</button>${open ? `<div class="cr-add-list" role="menu">${rest.map((x) => `<button type="button" role="menuitem" data-cr-tie="${kind}" data-key="${esc(keyOf(x))}">${esc(labelOf(x))}</button>`).join('')}</div>` : ''}</span>` : '';
    return `<div class="cr-chips"><span class="muted">${kind === 'page' ? 'pages' : 'micro-frontends'}</span>${chips || '<span class="muted">none yet</span>'}${add}</div>`;
  }
  // ---- each rule's state, as the factory last judged it ----------------------
  // Once the factory runs, the CI's rules' gate says of each rule's tests:
  // green, red, pending (a unit it needs is not built yet) — and a test a
  // development contested. Read at most once a minute, shown in place.
  const RULES_EVERY = 60000;
  function issueUrl(r, number) {
    return String(r.rules_issue || '').replace(/\/issues\/\d+$/, `/issues/${number}`);
  }
  function ruleStateTag(id) {
    const st = C.rulesStatus && C.rulesStatus.rules[`R${id}`];
    if (!st) return C.rulesStatus ? '<span class="tag" title="No run of the rules\' gate judged it yet">untested</span>' : '';
    const r = record();
    if (st.contested) return `<a class="tag warn" href="${esc(issueUrl(r, st.contested))}" target="_blank" rel="noopener" title="A development contests one of its tests: the rules' task decides">contested ↗</a>`;
    const why = st.why ? ` title="${esc(st.why)}"` : '';
    if (st.state === 'red') return st.conform ? `<span class="tag warn"${why}>to conform</span>` : `<span class="tag bad"${why}>red</span>`;
    if (st.state === 'green') return '<span class="tag ok">green</span>';
    if (st.state === 'pending') return `<span class="tag"${why}>pending</span>`;
    return `<span class="tag"${why}>untested</span>`;
  }
  function rulesSummary() {
    const st = C.rulesStatus;
    if (!st) return '';
    const states = Object.values(st.rules);
    const count = (f) => states.filter(f).length;
    const red = count((x) => x.state === 'red' && !x.conform);
    const conform = count((x) => x.state === 'red' && x.conform);
    const green = count((x) => x.state === 'green');
    const contested = count((x) => x.contested);
    const run = st.run_url ? `<a href="${esc(st.run_url)}" target="_blank" rel="noopener" title="${esc(st.at)}">the factory's last run ↗</a>` : '<span class="muted">no run of the rules\' gate yet</span>';
    const waits = st.tasks.length ? `<span class="tag warn" title="No development task starts while a rules' task is open">development waits on ${st.tasks.map((n) => `#${n}`).join(', ')}</span>` : '';
    return `${green ? `<span class="tag ok">${green} green</span>` : ''}${red ? `<span class="tag bad">${red} red</span>` : ''}${conform ? `<span class="tag warn">${conform} to conform</span>` : ''}${contested ? `<span class="tag warn">${contested} contested</span>` : ''}${waits} ${run}`;
  }
  function refreshRulesStatus(r) {
    if (DEMO || !r || !r.rules_issue) return;
    if (C.rulesStatusAt && Date.now() - C.rulesStatusAt < RULES_EVERY) return;
    C.rulesStatusAt = Date.now();
    call('GET', '/api/creation/rules/status').then((st) => {
      C.rulesStatus = st;
      // In place: a redraw would take the focus from a rule being typed.
      page.querySelectorAll('[data-cr-rule-state]').forEach((el) => { el.innerHTML = ruleStateTag(el.dataset.crRuleState); });
      const summary = page.querySelector('[data-cr-rules-summary]');
      if (summary) summary.innerHTML = rulesSummary();
    }).catch(() => {});
  }

  function ruleCard(q, gaps, mock) {
    const sel = C.ruleSel === q.id;
    const titleOf = (path) => (mock.pages.find((p) => p.path === path) || {}).title || path.replace(/^pages\//, '');
    const lost = danglingOf(q);
    const pages = q.pages.map((path) => (lost.pages.includes(path)
      ? `<span class="cr-rule-page dangling" title="The mock-up has no such page any more">${esc(path)}</span>`
      : `<button type="button" class="cr-rule-page${sel && C.rulePage === path ? ' on' : ''}" data-cr-rule-page="${esc(path)}" title="Show this page">${esc(titleOf(path))}</button>`)).join('');
    const cover = q.pages.length || q.components.length
      ? `${pages}<span class="muted">${q.components.length} micro-frontend${q.components.length === 1 ? '' : 's'}</span>${lost.components.length ? `<span class="tag bad" title="${esc(lost.components.join(', '))}">${lost.components.length} missing</span>` : ''}`
      : '<span class="tag bad">carried by nothing</span>';
    const family = FAMILIES.map(([id, label]) => `<option value="${id}"${(q.family || 'journey') === id ? ' selected' : ''}>${label}</option>`).join('');
    const open = sel ? `<textarea rows="1" class="cr-req-body cr-fit" data-cr-rule-field="body" placeholder="what it asks for" aria-label="rule ${q.id}, what it asks for">${esc(q.body)}</textarea>
        ${criterionBox(q)}
        ${ties(q, 'page', mock.pages, (p) => p.path, (p) => p.title || p.path)}
        ${ties(q, 'component', mock.components, (c) => c.name, (c) => c.name, (name) => `<button type="button" class="mini" data-cr-mf-redo="${esc(name)}" title="Generate this micro-frontend again from its spec">↻</button>`)}` : `<p class="cr-rule-desc">${q.body ? esc(q.body) : '<i class="muted">no description yet</i>'}</p>
        <div class="cr-rule-cover">${cover}${critMissing(q).length ? '<span class="tag bad" title="Open the rule to write its Given, When, Then">criterion to write</span>' : ''}</div>`;
    return `<article class="cr-rule${sel ? ' sel' : ''}${q.human ? ' mine' : ''}${gaps.length ? ' gap' : ''}" data-rule="${q.id}">
        <header><span class="cr-qn">R${q.id}</span><input type="text" class="cr-req-title" data-cr-rule-field="title" value="${esc(q.title)}" placeholder="the rule, in a few words" aria-label="rule ${q.id}">
          <select data-cr-rule-field="family" aria-label="rule ${q.id}, family">${family}</select>
          ${q.human ? '<span class="tag" title="Yours: a new reading keeps it">yours</span>' : ''}
          <span data-cr-rule-state="${q.id}">${ruleStateTag(q.id)}</span>
          <button type="button" class="mini" data-cr-rule-redo="${q.id}" title="The AI reads this rule again off its supports"${C.busy ? ' disabled' : ''}>↻</button>
          <button type="button" class="mini cr-del" data-cr-req-del="${q.id}" title="Delete this rule" aria-label="delete rule ${q.id}">✕</button></header>
        ${open}
      </article>`;
  }
  // A textarea as tall as what it holds.
  function fit(el) { el.style.height = 'auto'; el.style.height = el.scrollHeight + 'px'; }
  // The rule picked, shown on the easel.
  function syncFocus() {
    const canvas = C.embed && C.embed.canvas;
    if (!canvas || typeof canvas.focus !== 'function') return;
    const q = C.route === 'rules' && C.ruleSel ? ruleOf(C.ruleSel) : null;
    canvas.focus(q ? { names: q.components, page: C.rulePage || (q.pages.includes(C.canvasPage) ? C.canvasPage : q.pages[0]) || '' } : null);
  }
  function rulesFoot() {
    // Both outward choices call the real backend (regenerate scraps the
    // mock-up, accept publishes and opens a roadmap task) — disabled in the
    // demo, the same read-only stance as the embedded mock-up pane above.
    const r = record();
    const waiting = !r || !r.componentized || running() || !!C.busy;
    const gaps = [...((C.data && C.data.ruleGaps) || []), ...danglingGaps()];
    const lock = DEMO ? ' disabled title="Disabled in the demo"'
      : waiting ? ' disabled title="Wait for the micro-frontends and the rules"' : '';
    const validated = !!(r && r.rules_validated);
    const shown = gaps.slice(0, 4).map((g) => `<li>${esc(g)}</li>`).join('') + (gaps.length > 4 ? `<li>and ${gaps.length - 4} more</li>` : '');
    return `<div class="cr-choices">
        <section class="cr-card"><h3>a · Validate the rules</h3>${validated
          ? '<p><span class="tag ok">validated</span> Nothing is published: changing a rule takes it back.</p>'
          : `<p class="muted">Every rule with its criterion and carried by the mock-up. Nothing is published.</p>${gaps.length ? `<ul class="cr-gaps">${shown}</ul>` : ''}<button type="button" class="cr-btn on" data-cr-validate${lock || (gaps.length ? ' disabled title="Complete the rules first"' : '')}>Validate</button>`}</section>
        <section class="cr-card"><h3>b · Regenerate</h3><p class="muted">Back to the brief with everything filled in; the grilling keeps its questions and their answers, and may ask more.</p><button type="button" class="cr-btn" data-cr-regenerate${lock}>${C.confirm === 'regenerate' ? 'The mock-ups go. Click again' : 'Regenerate'}</button></section>
        <section class="cr-card"><h3>c · The data</h3><p class="muted">The data model the rules need, drafted by the AI — the business rules become its invariants.</p><button type="button" class="cr-btn on" data-cr-next="data"${validated && !waiting ? '' : ' disabled title="Validate the rules first"'}>Data →</button></section>
        ${rulesSendCard(r, validated)}
      </div>
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="mockups">← Back to the mock-up</button></div>`;
  }

  // Once the factory runs, a rule changed here reaches it only when sent:
  // a rules' task rewrites its tests ahead of the code, a conformity task
  // brings the code to them.
  function rulesSendCard(r, validated) {
    const moved = C.data && C.data.rulesMoved;
    if (!r || !r.rules_issue || !moved) return '';
    const [changed, removed] = moved;
    const ids = (list) => list.map((id) => 'R' + id).join(', ');
    const sent = (r.rules_changes || []).map((c) => `<li><a href="${esc(c.rules)}" target="_blank" rel="noopener">rules' task ↗</a> · <a href="${esc(c.conformity)}" target="_blank" rel="noopener">conformity ↗</a></li>`).join('');
    const what = [changed.length ? `changed: ${ids(changed)}` : '', removed.length ? `removed: ${ids(removed)}` : ''].filter(Boolean).join(' · ');
    const lock = DEMO ? ' disabled title="Disabled in the demo"'
      : !what ? ' disabled title="No rule changed since the factory received them"'
      : !validated ? ' disabled title="Validate the rules first"'
      : running() || C.busy ? ' disabled' : '';
    return `<section class="cr-card" data-cr-rules-sent><h3>d · Send to the factory</h3>
        <p class="muted">The factory runs on the rules it received (<a href="${esc(r.rules_issue)}" target="_blank" rel="noopener">their task ↗</a>). ${what ? `Since then — ${esc(what)}. Sending opens a rules' task that rewrites their tests ahead of the code, and a conformity task that brings the code to them; development waits on the first.` : 'No rule changed since.'}</p>
        ${sent ? `<ul class="cr-gaps">${sent}</ul>` : ''}
        <button type="button" class="cr-btn on" data-cr-send-rules${lock}>Send the changed rules</button></section>`;
  }

  // ---- the data step ------------------------------------------------------------
  // The steps whose body sits beside their easel.
  const SPLIT = ['mockups', 'rules', 'data'];
  const CRITICALITY = ['low', 'normal', 'critical'];
  const ON_DELETE = ['restrict', 'cascade', 'nullify', 'archive', 'review'];
  const ON_UPDATE = ['propagate', 'restrict'];
  const options = (all, on) => `<option value=""${on ? '' : ' selected'}>—</option>` + all.map((v) => `<option value="${v}"${v === on ? ' selected' : ''}>${v}</option>`).join('');
  function stepData(r) {
    const d = r.data;
    const gaps = (C.data && C.data.dataGaps) || [];
    const head = `<p class="muted">The data model the proof of concept needs, laid by the AI before the mock-up, then refined off the rules, the micro-frontends and the mock-up's example data: the tables (drawn beside), then each aggregate's integrity rules — what each field's criticality is, the invariants that carry the business rules, what a link does when the row it points at goes. Ask the AI to change the structure; retouch the words and pick the policies here.</p>`;
    if (!d) return head + (C.busy ? '' : '<div class="cr-actions"><span></span><button type="button" class="cr-btn on" data-cr-data-draft>Draft the data model</button></div>');
    const business = new Map(rules().filter((q) => q.family === 'business').map((q) => [q.id, q]));
    const status = d.validated && !C.data.dataStale ? '<span class="tag ok">validated</span>' : gaps.length ? `<span class="tag bad">${gaps.length} gap${gaps.length > 1 ? 's' : ''}</span>` : '<span class="tag">ready to validate</span>';
    const aggregates = (d.aggregates || []).map((a) => `<section class="cr-agg" data-agg="${esc(a.aggregate)}"><h3>${esc(a.aggregate)} <small>${esc((a.tables || []).join(', '))}</small></h3>
        <div class="cr-agg-fields">${Object.entries(a.fields || {}).map(([field, rule]) => `<label class="${rule.criticality ? '' : 'bad'}"><code>${esc(field)}</code><span class="muted">${esc(rule.kind || '')}</span><select data-cr-crit="${esc(field)}" aria-label="${esc(field)} criticality">${options(CRITICALITY, rule.criticality)}</select></label>`).join('') || '<span class="tag bad">no field</span>'}</div>
        ${(a.invariants || []).length ? `<ul class="cr-agg-inv">${a.invariants.map((i) => `<li><code>${esc(i.id)}</code> ${esc(i.rule)}${(i.rules || []).length ? ` <span class="muted">carries ${i.rules.map((n) => `<span title="${esc((business.get(n) || {}).title || '')}">R${n}</span>`).join(', ')}</span>` : ''}</li>`).join('')}</ul>` : ''}
        ${(a.relations || []).map((rel) => `<div class="cr-agg-rel${rel.onDelete && rel.onUpdate ? '' : ' bad'}" data-rel="${esc(rel.from)}"><code>${esc(rel.from)}</code> on delete <select data-cr-rel="onDelete" aria-label="${esc(rel.from)} on delete">${options(ON_DELETE, rel.onDelete)}</select> on update <select data-cr-rel="onUpdate" aria-label="${esc(rel.from)} on update">${options(ON_UPDATE, rel.onUpdate)}</select></div>`).join('')}
      </section>`).join('');
    const tables = (d.tables || []).map((t) => `<details class="cr-table"><summary><code>${esc(t.name)}</code> <span class="muted">${(t.fields || []).length} fields</span></summary>
        <input type="text" data-cr-purpose="${esc(t.name)}" data-field="" value="${esc(t.purpose)}" placeholder="what the table is for" aria-label="${esc(t.name)}, purpose">
        ${(t.fields || []).map((f) => `<label><code>${esc(f.name)}</code> <span class="muted">${esc(f.type)}${f.references ? ' → ' + esc(f.references) : ''}</span><input type="text" data-cr-purpose="${esc(t.name)}" data-field="${esc(f.name)}" value="${esc(f.purpose)}" placeholder="what it is for" aria-label="${esc(t.name)}.${esc(f.name)}, purpose"></label>`).join('')}</details>`).join('');
    return head + `<div class="cr-rules-head">${status}${C.data.dataStale ? ' <span class="cr-chip warn" title="The rules moved since the model was drafted">to redo</span>' : ''}<span class="spacer"></span><button type="button" class="cr-btn" data-cr-data-draft${C.busy ? ' disabled' : ''} title="The AI drafts the whole model again off the rules as they are">Draft again</button></div>
      <div class="cr-data-ask"><textarea rows="2" data-cr="data-request" placeholder="Ask the AI to change the structure: a table, a field, a link, an aggregate…" aria-label="what to change in the data model"></textarea><button type="button" class="cr-btn" data-cr-data-change${C.busy ? ' disabled' : ''}>Ask the AI</button></div>
      <h3 class="cr-sub">Aggregates</h3>${aggregates || '<p class="muted">No aggregate.</p>'}
      <h3 class="cr-sub">Tables</h3>${tables}`;
  }
  // What the volume of the proof of concept is, before it is accepted.
  function volumes(r) {
    const mock = C.mock || { pages: [], components: [] };
    const comps = mock.components || [];
    const d = r.data || { tables: [], aggregates: [] };
    const count = (n, one) => `<b>${n}</b> ${one}${n === 1 ? '' : 's'}`;
    return [
      count((mock.pages || []).length, 'page'),
      count(comps.length, 'micro-frontend'),
      count(new Set(comps.flatMap((c) => c.needs || [])).size, 'Capability').replace('Capabilitys', 'Capabilities'),
      count(new Set(comps.flatMap((c) => c.actions || [])).size, 'DataCapability').replace('DataCapabilitys', 'DataCapabilities'),
      count((d.tables || []).length, 'table'),
      count((d.aggregates || []).length, 'aggregate'),
      count((d.aggregates || []).reduce((n, a) => n + (a.invariants || []).length, 0), 'invariant'),
    ].join(' · ');
  }
  function dataFoot() {
    const r = record();
    const gaps = (C.data && C.data.dataGaps) || [];
    const waiting = !r || running() || !!C.busy;
    const lock = DEMO ? ' disabled title="Disabled in the demo"' : waiting ? ' disabled title="Wait for the AI"' : '';
    const validated = !!(r && r.data && r.data.validated && !C.data.dataStale);
    const shown = gaps.slice(0, 4).map((g) => `<li>${esc(g)}</li>`).join('') + (gaps.length > 4 ? `<li>and ${gaps.length - 4} more</li>` : '');
    return `<div class="cr-choices">
        <section class="cr-card"><h3>a · Validate the data model</h3>${validated
          ? '<p><span class="tag ok">validated</span> Nothing is published: changing it takes it back.</p>'
          : `<p class="muted">Every business rule carried, every link with its policies, every write on a field the model has, every field with its criticality.</p>${gaps.length ? `<ul class="cr-gaps">${shown}</ul>` : ''}<button type="button" class="cr-btn on" data-cr-data-validate${lock || (gaps.length ? ' disabled title="Close the gaps first"' : '')}>Validate</button>`}</section>
        <section class="cr-card"><h3>b · Regenerate</h3><p class="muted">Back to the brief with everything filled in; the grilling keeps its questions and their answers, and may ask more.</p><button type="button" class="cr-btn" data-cr-regenerate${lock}>${C.confirm === 'regenerate' ? 'The mock-ups go. Click again' : 'Regenerate'}</button></section>
        <section class="cr-card"><h3>c · The ports</h3><p class="muted">Everything the product reaches outside itself — payment, mail, storage, an LLM, its own PostgreSQL — each behind a port, its provider chosen.</p><button type="button" class="cr-btn on" data-cr-next="ports"${lock || (validated ? '' : ' disabled title="Validate the data model first"')}>Ports →</button></section>
      </div>
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="rules">← Back to the rules</button></div>`;
  }

  // ---- the ports step -------------------------------------------------------------
  // Every dependency outside the product, behind a port named by its business
  // function; the human picks each provider. PostgreSQL is always there.
  const TECH_KINDS = ['sdk', 'rest', 'graphql', 'sql', 'smtp', 'grpc', 'other'];
  function stepPorts(r) {
    const p = r.ports;
    const gaps = (C.data && C.data.portsGaps) || [];
    const head = '<p class="muted">What the product reaches outside itself — a third-party API, payment, mail, object storage, an LLM, identity, its own PostgreSQL — each behind a <b>port</b> named by its business function. Pick the provider of each: switching later is a new adapter behind the same port. A port that <b>keeps state</b> is written only by DataCapabilities, our data staying the truth. Secrets are named here; a human provides their values, encrypted with sops.</p>';
    if (!p) return head + (C.busy ? '' : '<div class="cr-actions"><span></span><button type="button" class="cr-btn on" data-cr-ports-draft>List the ports</button></div>');
    const status = p.validated && !C.data.portsStale ? '<span class="tag ok">validated</span>' : gaps.length ? `<span class="tag bad">${gaps.length} gap${gaps.length > 1 ? 's' : ''}</span>` : '<span class="tag">ready to validate</span>';
    const card = (port, i) => {
      const db = port.id === 'database';
      const lock = db ? ' disabled title="The product\'s own PostgreSQL: listed, not abstracted"' : '';
      const field = (key, label, value, hint) => `<label class="cr-port-f"><span class="muted">${label}</span><input type="text" data-cr-port="${i}" data-key="${key}" value="${esc(value || '')}" placeholder="${esc(hint)}"${lock}></label>`;
      const alts = (port.alternatives || []).filter(Boolean);
      return `<article class="cr-rule cr-port" data-port="${i}">
        <header><code>${esc(port.id || '?')}</code> <span class="muted">${esc(port.purpose || '')}</span><span class="spacer"></span>
          <label class="cr-simple"><input type="checkbox" data-cr-port="${i}" data-key="stateful"${port.stateful ? ' checked' : ''}${lock}> keeps state</label>
          ${db ? '' : `<button type="button" class="mini cr-del" data-cr-port-del="${i}" title="Remove this port" aria-label="remove the ${esc(port.id)} port">✕</button>`}</header>
        <div class="cr-port-grid">
          ${db ? '' : field('id', 'port', port.id, 'payments')}
          ${field('provider', 'provider', port.provider, 'Stripe')}
          <label class="cr-port-f"><span class="muted">wraps</span><select data-cr-port="${i}" data-key="tech_kind"${lock}>${TECH_KINDS.map((k) => `<option${k === port.tech_kind ? ' selected' : ''}>${k}</option>`).join('')}</select></label>
          ${field('tech_name', 'SDK / API', port.tech_name, 'async-stripe')}
          ${field('tech_version', 'version', port.tech_version, '0.39')}
          ${field('secrets', 'secrets', (port.secrets || []).join(', '), 'STRIPE_SECRET_KEY')}
          ${field('cost', 'cost', port.cost, 'per transaction')}
          ${field('used_by', 'used by', (port.used_by || []).join(', '), 'billing.chargeSubscription')}
        </div>
        ${alts.length && !db ? `<p class="muted">Other providers: ${alts.map((a) => `<button type="button" class="mini" data-cr-port-alt="${i}" data-alt="${esc(a)}" title="Pick ${esc(a)}">${esc(a)}</button>`).join(' ')}</p>` : ''}
      </article>`;
    };
    return head + `<div class="cr-rules-head">${status}${C.data.portsStale ? ' <span class="cr-chip warn" title="The rules or the writes moved since the ports were listed">to redo</span>' : ''}<span class="spacer"></span><button type="button" class="cr-btn" data-cr-port-add>+ port</button><button type="button" class="cr-btn" data-cr-ports-draft${C.busy ? ' disabled' : ''} title="The AI lists the ports again off the rules and the data model">List again</button></div>
      <div class="cr-rules">${(p.ports || []).map(card).join('')}</div>${portsFoot(r, gaps)}`;
  }
  function portsFoot(r, gaps) {
    const waiting = !r || running() || !!C.busy;
    const lock = DEMO ? ' disabled title="Disabled in the demo"' : waiting ? ' disabled title="Wait for the AI"' : '';
    const validated = !!(r.ports && r.ports.validated && !C.data.portsStale);
    const shown = gaps.slice(0, 4).map((g) => `<li>${esc(g)}</li>`).join('') + (gaps.length > 4 ? `<li>and ${gaps.length - 4} more</li>` : '');
    return `<div class="cr-choices">
        <section class="cr-card"><h3>a · Validate the ports</h3>${validated
          ? '<p><span class="tag ok">validated</span> Nothing is published: changing a port takes it back.</p>'
          : `<p class="muted">Every port named by its function, with its provider and what wraps it; nothing that keeps state read by a Capability.</p>${gaps.length ? `<ul class="cr-gaps">${shown}</ul>` : ''}<button type="button" class="cr-btn on" data-cr-ports-validate${lock || (gaps.length ? ' disabled title="Close the gaps first"' : '')}>Validate</button>`}</section>
        <section class="cr-card"><h3>b · Accept the proof of concept</h3><p class="muted">${volumes(r)} · <b>${(r.ports && r.ports.ports || []).length}</b> ports</p><p class="muted">The record — brief, decisions, scope, rules, pages, micro-frontends, the data model, the ports, the pins left — is written in the repository beside the mock-up, with <code>ports.json</code>; the mock-up and the design system are published, the rules' task, a task per port's secrets and the roadmap task opened, the watch started.</p><button type="button" class="cr-btn on" data-cr-accept${lock || (validated ? '' : ' disabled title="Validate the ports first"')}>Accept</button></section>
      </div>
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="data">← Back to the data</button></div>`;
  }
  // The ports as the page holds them, one field changed, sent back whole.
  function editPorts(change) {
    const r = record();
    const ports = JSON.parse(JSON.stringify((r.ports && r.ports.ports) || []));
    change(ports);
    act('saving', async () => { C.data = await call('PUT', '/api/creation/ports', { ports }); });
  }
  // A retouch of the data model, kept at once.
  function editData(edit) {
    act('saving', async () => { C.data = await call('PUT', '/api/creation/data', edit); reloadEmbed(); });
  }
  function reloadEmbed() {
    const canvas = C.embed && C.embed.canvas;
    if (canvas && typeof canvas.reload === 'function') canvas.reload();
  }

  function stepInitialize(r) {
    const j = job();
    const mine = j && j.work === 'initialize';
    const failed = mine && j.state === 'failed';
    const done = !!r.issue && !running('initialize');
    const split = DEMO ? mine : running('initialize');
    return `<p class="muted">The accepted mock-up made again of micro-frontends — you find them in the mock-up's micro-frontends view — then the design system and the mock-up published on the integration branch, the proof of concept's roadmap task opened, the watch started: the planner takes it from there.</p>
      ${split ? drawSplit(C.data.branch, '<div data-cr-job>' + jobPanel() + '</div>') : ''}
      ${!split && mine ? `<div data-cr-job>${jobPanel()}</div>` : ''}
      ${r.componentized ? `<div class="callout">${esc(r.componentized)}</div>` : ''}
      ${r.mockup_at ? `<p>The mock-up and the record of the creation (<code>mock/proof-of-concept.json</code>): ${r.mockup_at.startsWith('mockup/') ? `on <code>${esc(r.mockup_at)}</code> — the integration branch refused them` : `on the integration branch (<code>${esc(r.mockup_at)}</code>)`}.</p>` : ''}
      ${r.design_refused ? `<div class="cr-error" role="alert">The design system is published on its branch but not carried onto the integration branch: ${esc(r.design_refused)}. The roadmap task says so.</div>` : ''}
      ${r.issue ? `<p>The roadmap task: <a href="${esc(r.issue)}" target="_blank" rel="noopener">${esc(r.issue)} ↗</a></p>` : ''}
      <div class="cr-actions"><span></span>
        ${failed || (!mine && !r.issue) ? '<button type="button" class="cr-btn" data-cr-accept>Initialize again</button>' : ''}
        <button type="button" class="cr-btn on" data-cr-finish${done ? '' : ' disabled'}>Open the plant →</button></div>`;
  }

  const BODIES = { kind: stepKind, brief: stepBrief, grilling: stepGrilling, scope: stepScope, design: stepDesign, mockups: stepMockups, rules: stepRules, data: stepData, ports: stepPorts, initialize: stepInitialize };

  function draw() {
    if (page.hidden) { unmountEmbed(); return; }
    if (!C.loaded) { unmountEmbed(); page.innerHTML = '<div class="cr"><p class="muted">Reading the creation…</p></div>'; return; }
    const r = record();
    // A step the gates do not reach yet falls back on the furthest they do
    // — except in the demo, which fabricates its own record after this.
    // A founding is read as a factory with nothing done yet, whatever the
    // record on the desk holds: the steps ahead are the new factory's.
    const fresh = C.route === FOUND;
    if (!DEMO && C.route !== 'landing' && !fresh && !reachable(C.route)) C.route = r ? C.data.reachable || r.step : 'repository';
    const at = fresh ? 0 : indexOf(C.route);
    const title = C.route === 'landing' ? 'New factory' : `${at + 1} · ${STEPS[at][1]}`;
    const body = C.route === 'landing' ? landing()
      : fresh ? foundForm()
        : C.route === 'repository' ? stepRepository()
          : r ? BODIES[C.route](r) : '<p class="muted">Reading the creation…</p>';
    const lead = C.auto && !r ? '<p class="muted">This factory is not linked to a GitHub repository.</p>' : '';
    const embed = fresh ? undefined : r ? embedFor(C.route) : undefined;
    // The split view (mockups/initialize) wants the same room as an embed.
    const drawing = C.route === 'mockups' || C.route === 'initialize';
    const html = `<div class="cr${embed ? ' cr-wide' : ''}${drawing ? ' cr-wider' : ''}">
      <div class="cr-top"><h1>${esc(title)}</h1><button type="button" class="fp-back" data-cr-leave>${C.auto ? '← open the plant anyway' : '← back to the plant'}</button></div>
      ${lead}
      ${C.route === 'landing' ? '' : crumbs(fresh)}
      ${proposalsNotice(r)}
      ${C.busy ? `<div class="cr-busy" role="status"><span class="cr-spin" aria-hidden="true"></span>${esc(C.busy)}…</div>` : ''}
      ${C.error ? `<div class="cr-error" role="alert">${esc(C.error)}</div>` : ''}
      ${SPLIT.includes(C.route) && embed ? '<div class="cr-rules-split">' : ''}<div class="cr-body">${body}</div>
      ${embed ? `<div class="cr-embed" id="cr-embed" data-cr-fullable data-cr-keep="${esc(embed.face)}"><div class="cr-embed-bar"><b>${esc(embed.title)}</b><button type="button" class="cr-btn" data-cr-full title="Fill the screen (Esc to leave)">⛶ Full screen</button></div><div class="cr-embed-host" id="cr-embed-host"></div></div>${SPLIT.includes(C.route) ? '</div>' : ''}${embed.foot()}` : ''}
    </div>`;
    const kept = jobScroll();
    paint(html);
    jobScrolled(kept);
    if (embed) mountEmbed(embed);
    else unmountEmbed();
    sizeEmbed();
    syncFocus();
    followFiles();
    syncPreview();
    showDrawn();
    restore();
    // After the drafts are put back: each box as tall as what it holds.
    page.querySelectorAll('.cr-rule.sel textarea.cr-fit, [data-cr^="scope-"].cr-fit').forEach(fit);
  }
  // A drawing that is over (just ended, or the step revisited): its first page shows at once.
  function showDrawn() {
    if (DEMO || C.route !== 'mockups' || C.busy || running('mockups') || embedFor('mockups')) return;
    const host = page.querySelector('[data-cr-preview]');
    const frame = host && host.querySelector('.cr-draw-frame');
    if (!frame || !frame.hidden) return;
    frame.hidden = false;
    host.querySelector('.cr-draw-empty')?.setAttribute('hidden', '');
    frame.src = `/mock/${encodeURIComponent((host.dataset.crPreview || '').replace(/\//g, '__'))}/index.html`;
  }
  // The page is morphed into the new markup, not rewritten (morphdom): an
  // element that did not change stays the same node, so a field keeps its
  // focus, its caret and its scroll. Two are left exactly as they are: the
  // field being typed in, and an element marked `data-cr-keep` whose mark
  // did not change — the easel mounted in the step (its iframes would
  // reload, its canvas lose its place), the preview's frame — whose live
  // state is the script's, not the markup's.
  const TYPED = 'textarea, select, input:not([type="checkbox"]):not([type="radio"]):not([type="button"]):not([type="submit"])';
  function paint(html) {
    const want = embedFor(C.route);
    if (C.embed && (!want || want.face !== C.embed.face || !page.contains(C.embed.wrap))) unmountEmbed();
    // Without the vendor script the page is rewritten whole, the easel mounted again.
    if (!window.morphdom) { unmountEmbed(); page.innerHTML = html; return; }
    const next = document.createElement('div');
    next.innerHTML = html;
    window.morphdom(page, next, { childrenOnly: true, onBeforeElUpdated: (from, to) => !kept(from, to) });
  }
  function kept(from, to) {
    if (from.dataset.crKeep !== undefined && from.dataset.crKeep === to.dataset.crKeep) return true;
    return from === document.activeElement && from.matches(TYPED);
  }
  // The easels that live in a step: the design system's, the mock-up's.
  // `face` tells two mountings of the same pane apart — the brief's easel is
  // the material only, the design step's is the whole one — so moving from
  // one step to the other mounts it again instead of keeping the other face.
  const EMBEDS = {
    brief: { kind: 'kit', face: 'kit-material', title: "The design system's material — logo, moodboard, design brief", foot: briefFoot, opts: () => ({ kind: 'kit', tab: 'system', material: true }) },
    design: { kind: 'kit', face: 'kit', title: "The design system's easel", foot: designFoot, opts: () => ({ kind: 'kit', tab: 'system' }) },
    // The mock-ups step reworks the pages: no micro-frontends' list beside them.
    mockups: { kind: 'mockup', face: 'mockup-pages', title: "The mock-up — rework it: edit, pin a request, add a page", foot: mockupsFoot, opts: () => ({ kind: 'mockup', branch: C.data.branch, readOnly: DEMO, strip: false }) },
    data: { kind: 'data_model', face: 'data-draft', title: 'The data model drafted for the proof of concept', foot: dataFoot, opts: () => ({ kind: 'data_model', source: '/api/creation/data-model' }) },
    rules: { kind: 'mockup', face: 'mockup', title: "The mock-up — pick a rule to see what carries it, click a micro-frontend to filter by it", foot: rulesFoot, opts: () => ({ kind: 'mockup', branch: C.data.branch, readOnly: DEMO }) },
  };
  // Elsewhere than the rules step: the changes waiting there, one line.
  function proposalsNotice(r) {
    const waiting = r && C.route !== 'rules' ? (r.proposals || []).length : 0;
    if (!waiting) return '';
    return `<div class="callout cr-proposals-notice"><b>${waiting}</b> change${waiting > 1 ? 's' : ''} to the rules proposed by your requests on the mock-up. <button type="button" class="cr-btn" data-cr-go="rules">Review in Rules →</button></div>`;
  }
  // The rules moved under the page (mockup.js's `harness:creation-moved`:
  // a request proposed, validated or taken back on the mock-up): read again,
  // what was typed here kept first.
  document.addEventListener('harness:creation-moved', async () => {
    await flushRules();
    await read();
    C.rules = null;
    if (page.offsetParent && !C.busy) draw();
  });
  // The easel tells the rules step what it shows (mockup.js's events): the
  // page, the micro-frontend locked by a click on it.
  page.addEventListener('harness:mockup-page', (e) => {
    C.canvasPage = e.detail.page || '';
    if (C.route === 'rules') draw();
  });
  // The mock-up's history asks whether the rules are on screen, and goes
  // there when they are not.
  page.addEventListener('harness:where', (e) => { if (e.detail && C.route === 'rules') e.detail.rules = true; });
  page.addEventListener('harness:goto', (e) => {
    const to = e.detail && e.detail.route;
    if (to && to !== C.route) setRoute(to);
  });
  // A pin open on the mock-up: the rules of the micro-frontends it touches.
  page.addEventListener('harness:pin-mfs', (e) => {
    C.pinMfs = (e.detail && e.detail.names) || [];
    if (C.route === 'rules') draw();
  });
  page.addEventListener('harness:mf-lock', (e) => {
    C.rulesMf = e.detail.name || '';
    if (C.route === 'rules') draw();
  });
  // The easel a step mounts right now: the mock-up's only once it is drawn.
  function embedFor(route) {
    if (!record()) return undefined;
    if (route === 'mockups') return !DEMO && drawn() ? EMBEDS.mockups : undefined;
    return EMBEDS[route];
  }
  // What the easel holds and the server has not: the draft's steps, written
  // into the workspace before anything reads it.
  async function flushEmbed() {
    if (C.embed && C.embed.canvas && typeof C.embed.canvas.flush === 'function') await C.embed.canvas.flush();
  }
  // The easel ends at nine tenths of the screen, wherever it stands — at
  // the top of the step or held at the top while the page scrolls (sticky):
  // its height follows its top, measured once a frame while scrolling.
  const EMBED_END = 0.9;
  const EMBED_MIN = 480;
  let sizing = 0;
  function sizeEmbed() {
    sizing = 0;
    const el = $('cr-embed');
    if (!el || el.classList.contains('cr-full')) return;
    const top = Math.max(8, el.getBoundingClientRect().top);
    el.style.setProperty('--embed-h', Math.max(EMBED_MIN, Math.round(window.innerHeight * EMBED_END - top)) + 'px');
  }
  const sizeSoon = () => { if (!sizing) sizing = requestAnimationFrame(sizeEmbed); };
  // The page scrolls inside its own box (#factory-page), not the window:
  // a scroll of any box is heard, in the capture phase.
  document.addEventListener('scroll', sizeSoon, { passive: true, capture: true });
  window.addEventListener('resize', sizeSoon);
  function mountEmbed(embed) {
    if (C.embed && C.embed.face === embed.face) return;
    unmountEmbed();
    const wrap = $('cr-embed');
    const host = $('cr-embed-host');
    if (!host || !window.HarnessPanes) return;
    const canvas = window.HarnessPanes.mount(embed.kind, host, embed.opts());
    if (canvas) C.embed = { kind: embed.kind, face: embed.face, wrap, canvas };
  }
  function unmountEmbed() {
    if (!C.embed) return;
    try { C.embed.canvas.destroy(); } catch (e) { /* already gone */ }
    C.embed = null;
    // What the easel told is gone with it.
    C.canvasPage = '';
    C.rulesMf = '';
    C.pinMfs = [];
  }
  // A part of the page takes the whole screen; Esc gives it back.
  function toggleFull(el) {
    const on = el.classList.toggle('cr-full');
    document.body.classList.toggle('cr-fullscreen', !!page.querySelector('.cr-full'));
    el.querySelector('[data-cr-full]').textContent = on ? '✕ Leave full screen' : '⛶ Full screen';
    sizeEmbed();
  }
  document.addEventListener('keydown', (e) => {
    const full = e.key === 'Escape' && page.querySelector('.cr-full');
    if (full) toggleFull(full);
  });
  // Typing survives a redraw: what was typed and not saved goes back in its field.
  function restore() {
    for (const [key, value] of Object.entries(C.dirty)) {
      const field = key.startsWith('q') ? page.querySelector(`[data-q="${key.slice(1)}"] textarea`) : page.querySelector(`[data-cr="${key}"]`);
      if (field && field.value !== value) field.value = value;
    }
  }
  page.addEventListener('input', (e) => {
    const named = e.target.closest('[data-cr]');
    if (named) C.dirty[named.dataset.cr] = named.value;
    // A required section emptied or filled opens or closes the way on at once.
    if (named && named.dataset.cr.startsWith('scope-')) {
      fit(named);
      const on = page.querySelector('[data-cr-next="design"]');
      const missing = scopeMissing();
      if (on) { on.disabled = missing.length > 0; on.title = missing.length ? `Fill ${missing.map((s) => s.title).join(', ')} first` : ''; }
    }
    const q = e.target.closest('[data-q]');
    if (q && e.target.tagName === 'TEXTAREA') { C.dirty[`q${q.dataset.q}`] = e.target.value; markAnswered(); }
  });
  // A question with text in its answer greys at once, before any save.
  function markAnswered() {
    for (const el of page.querySelectorAll('[data-q]')) {
      const field = el.querySelector('textarea');
      if (field) el.classList.toggle('done', field.value.trim() !== '');
    }
  }

  async function loadOwners() {
    C.owners = [];
    try {
      C.owners = (await call('GET', '/api/creation/owners')).owners || [];
    } catch (e) { C.owners = null; C.ownersWhy = e.message; }
    // Only the two forms that hold the owner select: a founding, or the first step without a record.
    if (C.route !== FOUND && (C.route !== 'repository' || record())) return;
    // The list alone changes: what the human typed meanwhile stays.
    const select = page.querySelector('select[name="owner"]');
    if (select && C.owners && C.owners.length) select.innerHTML = C.owners.map((o) => `<option>${esc(o)}</option>`).join('');
    else draw();
  }

  // A small advisory dialog: two named choices, never a silent default.
  // Resolves 'primary', 'secondary', or null (Escape, the backdrop, or the
  // page leaving while it is open) — nothing happens on its own.
  function confirmModal({ title, body, primary, secondary }) {
    return new Promise((resolve) => {
      const host = document.createElement('div');
      host.className = 'cr-modal-backdrop';
      host.innerHTML = `<div class="cr-modal" role="alertdialog" aria-modal="true" aria-label="${esc(title)}">
        <h3>${esc(title)}</h3>
        <p>${esc(body)}</p>
        <div class="cr-modal-actions">
          <button type="button" class="cr-btn" data-pick="secondary">${esc(secondary)}</button>
          <button type="button" class="cr-btn on" data-pick="primary">${esc(primary)}</button>
        </div>
      </div>`;
      document.body.appendChild(host);
      const done = (pick) => { host.remove(); window.removeEventListener('keydown', onKey, true); resolve(pick); };
      host.addEventListener('click', (e) => {
        if (e.target === host) { done(null); return; }
        const b = e.target.closest('[data-pick]');
        if (b) done(b.dataset.pick);
      });
      function onKey(e) { if (e.key === 'Escape') done(null); }
      window.addEventListener('keydown', onKey, true);
      host.querySelector('[data-pick="primary"]').focus();
    });
  }
  window.addEventListener('harness:navigate', () => document.querySelectorAll('.cr-modal-backdrop').forEach((h) => h.remove()));

  // The next grilling round, asked.
  function askGrill() {
    const body = fields();
    // The new round lands in any topic: the page goes to its first open question.
    act(GRILL_NEXT, async () => { await save(body); await post('grill'); })
      .then(() => { if (!C.error) firstOpen()?.scrollIntoView({ block: 'start', behavior: 'smooth' }); });
  }
  function firstOpen() {
    return [...page.querySelectorAll('[data-q]')].find((el) => !el.querySelector('textarea')?.value.trim());
  }

  // ---- gestures -------------------------------------------------------------------
  page.addEventListener('submit', (e) => {
    const form = e.target.closest('[data-cr-form]');
    if (!form) return;
    e.preventDefault();
    const data = new FormData(form);
    if (form.dataset.crForm === 'connect') {
      act('connecting', () => start('connect', { url: String(data.get('url') || '') }));
    } else {
      act('creating the repository', () => start('found', {
        name: String(data.get('name') || ''),
        owner: String(data.get('owner') || ''),
        private: data.get('visibility') !== 'public',
        description: String(data.get('description') || ''),
      }));
    }
  });
  // A rule's card clicked outside its fields: picked, or let go.
  page.addEventListener('click', (e) => {
    // A + list left open closes on a click anywhere else.
    if (C.addOpen && !e.target.closest('.cr-add')) { C.addOpen = null; draw(); return; }
    const card = e.target.closest('[data-rule]');
    if (!card || e.target.closest('button, input, select, textarea, label')) return;
    const id = Number(card.dataset.rule);
    C.ruleSel = C.ruleSel === id ? null : id;
    C.rulePage = '';
    draw();
  });
  page.addEventListener('click', (e) => {
    const t = e.target.closest('button');
    if (!t || t.disabled) return;
    if (t.matches('[data-cr-leave]')) { window.HarnessFactories.leave(); return; }
    if (t.dataset.crHelp) {
      const key = t.dataset.crHelp;
      C.helps = C.helps.includes(key) ? C.helps.filter((k) => k !== key) : [...C.helps, key];
      draw();
      return;
    }
    if (t.dataset.crGo) {
      if (t.dataset.crGo !== 'landing' && t.dataset.crGo !== FOUND && t.dataset.crGo !== C.route && record()) {
        // Moving keeps what the fields hold.
        const body = fields();
        const to = t.dataset.crGo;
        if (Object.keys(body).length && to !== 'repository') { next(to); return; }
      }
      setRoute(t.dataset.crGo);
      enter();
      return;
    }
    if (t.dataset.crNext) {
      const rounds = C.route === 'grilling' && record() ? record().rounds : null;
      const open = C.route === 'grilling' ? [...page.querySelectorAll('[data-q] textarea')].filter((x) => !x.value.trim()).length : 0;
      if (t.dataset.crNext === 'scope' && open) {
        confirmModal({
          title: `${open} question${open === 1 ? '' : 's'} still open`,
          body: 'An open question takes its recommended answer when you continue — or stay and answer it yourself.',
          primary: 'Use the recommendations and continue',
          secondary: 'Stay',
        }).then((pick) => { if (pick === 'primary') { takeAll(); next('scope'); } });
        return;
      }
      if (t.dataset.crNext === 'scope' && rounds !== null && rounds < 3 && !record().grilled) {
        confirmModal({
          title: rounds ? `Only ${rounds} round${rounds === 1 ? '' : 's'} so far` : 'No round asked yet',
          body: 'Grilling at least three rounds usually surfaces the decisions that matter before the scope is drafted.',
          primary: 'Keep grilling',
          secondary: 'Continue anyway',
        }).then((pick) => { if (pick === 'secondary') next('scope'); });
        return;
      }
      next(t.dataset.crNext);
      return;
    }
    if (t.matches('[data-cr-decompose]')) {
      const edited = [...page.querySelectorAll('[data-cr^="brief-"]')].some((x) => x.value.trim());
      if (edited && C.confirm !== 'decompose') { C.confirm = 'decompose'; draw(); return; }
      C.confirm = '';
      decompose();
      return;
    }
    if (t.matches('[data-cr-parts]')) { C.parts = true; draw(); return; }
    if (t.matches('[data-cr-kind]')) {
      const kind = page.querySelector('input[name="kind"]:checked');
      if (!kind) return;
      act('saving', async () => { await save({ kind: kind.value, step: 'brief' }); C.route = 'brief'; setRoute('brief'); });
      return;
    }
    if (t.dataset.crTake) {
      const q = page.querySelector(`[data-q="${t.dataset.crTake}"]`);
      const rec = record().questions.find((x) => String(x.id) === t.dataset.crTake);
      if (q && rec) { q.querySelector('textarea').value = rec.recommended; C.dirty[`q${rec.id}`] = rec.recommended; }
      markAnswered();
      keepAnswers();
      return;
    }
    if (t.dataset.crDel) {
      page.querySelector(`[data-q="${t.dataset.crDel}"]`)?.remove();
      const body = fields();
      act('saving', () => save(body));
      return;
    }
    if (t.matches('[data-cr-take-all]')) { takeAll(); markAnswered(); keepAnswers(); return; }
    if (t.dataset.crReqDel) {
      const id = Number(t.dataset.crReqDel);
      if (C.ruleSel === id) C.ruleSel = null;
      editRules((list) => list.splice(list.findIndex((q) => q.id === id), 1), true);
      draw();
      return;
    }
    if (t.matches('[data-cr-req-add]')) {
      editRules((list) => list.push({ id: 0, title: 'New rule', body: '', family: 'journey', criterion: { given: '', when: '', then: '' }, pages: [], components: [] }), true);
      return;
    }
    if (t.dataset.crRuleRedo) { const id = Number(t.dataset.crRuleRedo); act(`the AI reads R${id} again`, async () => { await flushRules(); await post(`rules/${id}/reread`); }); return; }
    if (t.dataset.crUntie || t.dataset.crTie) {
      const kind = t.dataset.crUntie || t.dataset.crTie;
      const key = t.dataset.key;
      const field = kind === 'page' ? 'pages' : 'components';
      const tie = !!t.dataset.crTie;
      C.addOpen = null;
      editRules((list) => { const q = list.find((x) => x.id === C.ruleSel); if (q) q[field] = tie ? [...q[field].filter((k) => k !== key), key] : q[field].filter((k) => k !== key); }, true);
      draw();
      return;
    }
    if (t.matches('[data-cr-unbreak]')) {
      editRules((list) => list.forEach((q) => { const d = danglingOf(q); q.pages = q.pages.filter((p) => !d.pages.includes(p)); q.components = q.components.filter((c) => !d.components.includes(c)); }), true);
      draw();
      return;
    }
    if (t.dataset.crAdd) {
      const open = C.addOpen && C.addOpen.rule === C.ruleSel && C.addOpen.kind === t.dataset.crAdd;
      C.addOpen = open ? null : { rule: C.ruleSel, kind: t.dataset.crAdd };
      draw();
      return;
    }
    if (t.dataset.crProposal) {
      const key = t.dataset.crProposal;
      const how = t.dataset.how;
      act(how === 'accept' ? 'accepting the change' : 'refusing the change', async () => {
        await flushRules();
        C.data = await call('POST', `/api/creation/rules/proposals/${key}/${how}`);
        C.rules = null;
      });
      return;
    }
    if (t.matches('[data-cr-rules-unlock]')) {
      C.rulesMf = '';
      if (C.embed && C.embed.canvas && typeof C.embed.canvas.lock === 'function') C.embed.canvas.lock(null);
      draw();
      return;
    }
    if (t.dataset.crRulePage) { const card = t.closest('[data-rule]'); C.ruleSel = Number(card.dataset.rule); C.rulePage = t.dataset.crRulePage; draw(); return; }
    if (t.dataset.crMfRedo) {
      const canvas = C.embed && C.embed.canvas;
      if (canvas && typeof canvas.regenerate === 'function') canvas.regenerate(t.dataset.crMfRedo);
      return;
    }
    if (t.dataset.crTab) { C.rulesTab = t.dataset.crTab; draw(); return; }
    if (t.matches('[data-cr-data-draft]')) { act('the AI drafts the data model', async () => { await post('data'); reloadEmbed(); }); return; }
    if (t.matches('[data-cr-data-change]')) {
      const field = page.querySelector('[data-cr="data-request"]');
      const request = field ? field.value : '';
      act('the AI changes the data model', async () => { C.data = await call('POST', '/api/creation/data/change', { request }); delete C.dirty['data-request']; if (field) field.value = ''; reloadEmbed(); });
      return;
    }
    if (t.matches('[data-cr-ports-draft]')) { act('the AI lists the ports', () => post('ports')); return; }
    if (t.matches('[data-cr-ports-validate]')) { act('validating the ports', () => post('ports/validate')); return; }
    if (t.matches('[data-cr-port-add]')) { editPorts((ports) => ports.push({ id: 'new-port', purpose: '', stateful: false, provider: '', tech_kind: 'rest', tech_name: '', tech_version: '', secrets: [], cost: '', used_by: [], alternatives: [] })); return; }
    if (t.matches('[data-cr-port-del]')) { const i = +t.dataset.crPortDel; editPorts((ports) => ports.splice(i, 1)); return; }
    if (t.matches('[data-cr-port-alt]')) { const i = +t.dataset.crPortAlt; const alt = t.dataset.alt; editPorts((ports) => { const p = ports[i]; p.alternatives = [p.provider, ...(p.alternatives || []).filter((a) => a !== alt)].filter(Boolean); p.provider = alt; p.tech_name = ''; }); return; }
    if (t.matches('[data-cr-data-validate]')) { act('validating the data model', () => post('data/validate')); return; }
    if (t.matches('[data-cr-audit]')) { act('the guard reads the architecture', () => post('audit')); return; }
    if (t.dataset.crFinding) {
      const how = t.dataset.how;
      act(how === 'apply' ? 'a session applies the fix to the mock-up' : 'dismissing', async () => { await post(`audit/${t.dataset.crFinding}/${how}`); await loadMock(); if (C.embed && C.embed.canvas && C.embed.canvas.reload) C.embed.canvas.reload(); });
      return;
    }
    if (t.matches('[data-cr-send-rules]')) { act('sending the changed rules', async () => { await flushRules(); await post('rules/send'); }); return; }
    if (t.matches('[data-cr-validate]')) { act('validating the rules', async () => { await flushRules(); await post('rules/validate'); }); return; }
    if (t.matches('[data-cr-reqs]')) {
      const body = fields();
      act('the AI reads the rules', async () => { await save(body); C.rules = null; await post('requirements'); await loadMock(); });
      return;
    }
    if (t.matches('[data-cr-mf]')) { act('starting the micro-frontends', () => start('componentize')); return; }
    if (t.matches('[data-cr-grill]')) {
      const rounds = record().rounds;
      if (rounds >= 6) {
        confirmModal({
          title: `${rounds} rounds already`,
          body: 'Six rounds usually covers the ground; asking for more adds little and spends more of the model\'s budget for a shrinking return.',
          primary: 'Continue to the scope',
          secondary: 'Keep grilling anyway',
        }).then((pick) => { if (pick === 'primary') next('scope'); else if (pick === 'secondary') askGrill(); });
        return;
      }
      askGrill();
      return;
    }
    if (t.matches('[data-cr-rescope]')) {
      const edited = fields().scope !== record().scope;
      if (edited && C.confirm !== 'scope') { C.confirm = 'scope'; draw(); return; }
      C.confirm = '';
      act(PROPOSING, () => post('scope'));
      return;
    }
    if (t.matches('[data-cr-full]')) { toggleFull(t.closest('[data-cr-fullable]')); return; }
    if (t.matches('[data-cr-track-hide]')) { C.trackHidden = true; draw(); drawPreview(); return; }
    if (t.matches('[data-cr-track-show]')) { C.trackHidden = false; draw(); drawPreview(); return; }
    if (t.matches('[data-cr-kit]')) { generateKit(); return; }
    if (t.matches('[data-cr-draw]')) {
      if (!record().mocked) { act('starting the drawing', () => start('mockups')); return; }
      confirmModal({
        title: 'Draw the mock-up again?',
        body: 'The pages go, with every edit and every request applied to them; the pins stay where they were, and point at parts that may no longer exist. The scope and the design system stay.',
        primary: 'Draw again',
        secondary: 'Keep the mock-up',
      }).then((pick) => { if (pick === 'primary') { C.auto_started = {}; act('starting the drawing', () => start('mockups')); } });
      return;
    }
    if (t.matches('[data-cr-regenerate]')) {
      if (C.confirm !== 'regenerate') { C.confirm = 'regenerate'; draw(); return; }
      C.auto_started = {};
      act('back to the brief', async () => { await post('regenerate'); setRoute('brief'); });
      return;
    }
    if (t.matches('[data-cr-accept]')) { act('initializing', async () => { await flushEmbed(); await save(fields()); await post('accept'); await read(); setRoute('initialize'); }); return; }
    if (t.matches('[data-cr-finish]')) {
      act('opening the plant', async () => { await call('POST', '/api/creation/finish'); await read(); window.HarnessFactories.leave(); });
    }
  });
  // An answer is kept as soon as the field is left, and the buttons it opens follow.
  // No redraw: the field the human moved to keeps its focus.
  function answersOpen() {
    const open = [...page.querySelectorAll('[data-q] textarea')].filter((t) => !t.value.trim()).length;
    const any = page.querySelectorAll('[data-q]').length;
    const grill = page.querySelector('[data-cr-grill]');
    if (grill) grill.disabled = !!open || !!C.busy;
    const next = page.querySelector('[data-cr-next="scope"]');
    if (next) next.disabled = !any && !(record() && record().grilled);
    const all = page.querySelector('[data-cr-take-all]');
    if (all) { all.hidden = !open; all.textContent = `Use the ${open} recommendation${open === 1 ? '' : 's'}`; }
  }
  // Every open question takes its recommended answer, in its field.
  function takeAll() {
    const r = record();
    if (!r) return;
    for (const el of page.querySelectorAll('[data-q]')) {
      const field = el.querySelector('textarea');
      const q = r.questions.find((x) => String(x.id) === el.dataset.q);
      if (field && q && !field.value.trim() && q.recommended) { field.value = q.recommended; C.dirty[`q${q.id}`] = q.recommended; }
    }
    answersOpen();
  }
  // A requirement retouched is kept a moment after the last keystroke; a chip at once.
  function keepRequirements(now) {
    clearTimeout(C.reqTimer);
    if (now) flushRules(true); else C.reqTimer = setTimeout(() => flushRules(true), 800);
  }
  // The page's copy of the rules saved; let go once the record holds them
  // and nothing changed meanwhile. `redraw` paints what the save answered.
  async function flushRules(redraw) {
    clearTimeout(C.reqTimer);
    if (!record() || !C.rules) return;
    const sent = C.rulesRev;
    try { await save(fields()); if (C.rulesRev === sent) C.rules = null; } catch (err) { C.error = err.message; }
    if (redraw && !C.busy) draw();
  }
  async function keepAnswers() {
    answersOpen();
    if (!record() || C.busy) return;
    try { await save(fields()); } catch (err) { C.error = err.message; draw(); }
  }
  page.addEventListener('change', (e) => {
    if (e.target.closest('[data-q]')) keepAnswers();
    if (e.target.dataset.crCrit !== undefined && e.target.value) {
      const agg = e.target.closest('[data-agg]');
      editData({ criticalities: [{ aggregate: agg.dataset.agg, field: e.target.dataset.crCrit, criticality: e.target.value }] });
      return;
    }
    if (e.target.dataset.crRel && e.target.value) {
      const rel = e.target.closest('[data-rel]');
      editData({ relations: [{ from: rel.dataset.rel, [e.target.dataset.crRel]: e.target.value }] });
      return;
    }
    if (e.target.dataset.crPurpose !== undefined) {
      editData({ texts: [{ table: e.target.dataset.crPurpose, field: e.target.dataset.field || '', purpose: e.target.value }] });
      return;
    }
    if (e.target.dataset.crPort !== undefined) {
      const i = +e.target.dataset.crPort;
      const key = e.target.dataset.key;
      const value = key === 'stateful' ? e.target.checked
        : key === 'secrets' || key === 'used_by' ? e.target.value.split(',').map((x) => x.trim()).filter(Boolean)
        : e.target.value;
      editPorts((ports) => { if (ports[i]) ports[i][key] = value; });
      return;
    }
    if (e.target.matches('[data-cr-rules-here]')) { C.rulesHere = e.target.checked; draw(); return; }
    if (e.target.matches('[data-cr-simple]')) {
      try { localStorage.setItem(SIMPLE, e.target.checked ? '1' : '0'); } catch (err) { /* a preference, not a record */ }
      draw();
      return;
    }
    const card = e.target.closest('[data-rule]');
    if (card) {
      const id = Number(card.dataset.rule);
      if (e.target.dataset.crRuleField === 'family') { editRules((list) => { const q = list.find((x) => x.id === id); if (q) q.family = e.target.value; }, true); draw(); }
    }
    if (e.target.closest('.cr-draw-head')) window.Viewport.change(e, setPreviewWidth);
  });
  // The preview's corner, dragged: width grows from the center (the sizer
  // is centered in its stage, so its edge only moves half as far as the
  // pointer, unless it already fills the stage), height follows the
  // pointer directly — a plain corner resize, not a centered one.
  let drag = null;
  const MIN_PREVIEW_HEIGHT = 240;
  // Back/Continue (and the demo's own floating button, bottom-right, when
  // it's there) must stay reachable: the box's bottom edge never drags
  // past this much clearance from the bottom of the viewport, wherever the
  // box itself starts.
  const PREVIEW_BOTTOM_RESERVE = 110;
  const dragSize = (e) => {
    const maxH = Math.max(MIN_PREVIEW_HEIGHT, window.innerHeight - drag.top0 - PREVIEW_BOTTOM_RESERVE);
    return {
      w: window.Viewport.clamp(Math.round((drag.w0 + (e.clientX - drag.x0) * (drag.centered ? 2 : 1)) / 10) * 10),
      h: Math.min(maxH, Math.max(MIN_PREVIEW_HEIGHT, Math.round((drag.h0 + (e.clientY - drag.y0)) / 10) * 10)),
    };
  };
  page.addEventListener('pointerdown', (e) => {
    const handle = e.target.closest('.cr-draw-handle');
    if (!handle || e.button !== 0) return;
    const sizer = handle.parentElement;
    const r = sizer.getBoundingClientRect();
    drag = { x0: e.clientX, y0: e.clientY, w0: r.width, h0: r.height, top0: r.top, sizer, centered: r.width < sizer.parentElement.clientWidth };
    handle.setPointerCapture(e.pointerId);
    sizer.classList.add('cr-dragging');
    e.preventDefault();
  });
  page.addEventListener('pointermove', (e) => {
    if (!drag) return;
    const { w, h } = dragSize(e);
    drag.sizer.style.width = `${w}px`;
    drag.sizer.style.height = `${h}px`;
    const field = page.querySelector('.cr-draw-head .mk-width');
    if (field) field.value = w;
  });
  page.addEventListener('pointerup', (e) => {
    if (!drag) return;
    const { w, h } = dragSize(e);
    drag.sizer.classList.remove('cr-dragging');
    drag = null;
    setPreviewWidth(w);
    setPreviewHeight(h);
  });
  page.addEventListener('input', (e) => {
    if (e.target.matches('[data-cr-rules-q]')) { C.rulesQuery = e.target.value; draw(); return; }
    if (e.target.closest('[data-q]')) answersOpen();
    const field = e.target.dataset && e.target.dataset.crRuleField;
    const card = e.target.closest('[data-rule]');
    if (card && field && field !== 'family') {
      const id = Number(card.dataset.rule);
      const part = field.startsWith('criterion.') ? field.slice(10) : '';
      editRules((list) => {
        const q = list.find((x) => x.id === id);
        if (!q) return;
        if (part) q.criterion = Object.assign(critOf(q), { [part]: e.target.value });
        else q[field] = e.target.value;
      }, false);
      if (part) e.target.classList.toggle('bad', !e.target.value.trim());
      if (e.target.classList.contains('cr-fit')) fit(e.target);
    }
  });

  // ---- the page's life ----------------------------------------------------------------
  async function open({ auto }) {
    C.auto = auto;
    C.route = routeFromHash();
    C.error = '';
    draw();
    await read();
    const r = record();
    // A factory being created opens on its step; the landing stays the landing.
    if (r && auto && C.route === 'landing') C.route = r.step;
    setRoute(C.route);
    enter();
    resume();
  }
  function close() {
    clearTimeout(C.timer);
    clearTimeout(C.reqTimer);
    unmountEmbed();
  }
  function route() {
    const to = routeFromHash();
    if (to !== C.route) { C.route = to; draw(); enter(); resume(); }
  }
  // Whether this factory is being created: the page opens by itself then.
  async function pending() {
    const data = await read();
    return !!(data && data.record);
  }

  window.HarnessCreation = { open, close, route, pending };
})();
