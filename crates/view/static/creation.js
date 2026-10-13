// The factory-creation page: connect an existing repository, or initialize
// a factory through nine steps under a breadcrumb — the repository, the kind
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
    ['review', 'Review'],
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
  const C = { dirty: {}, loaded: false, data: null, route: 'landing', auto: false, busy: '', error: '', owners: null, ownersWhy: '', timer: null, embed: null, view: { width: null, height: null }, trackHidden: false, auto_started: {}, confirm: '', parts: false, opening: '', pages: null, mock: null, reqTimer: null, pulsed: false, reading: false, again: false };
  const page = $('factory-page');

  async function call(method, url, body) {
    const r = await fetch(url, { method, headers: body ? { 'content-type': 'application/json' } : {}, body: body ? JSON.stringify(body) : undefined });
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
  const drawn = () => { const r = record(); return !!r && !!r.mocked && !running('mockups'); };

  // ---- the route, in the hash ---------------------------------------------------
  function routeFromHash() {
    const m = /^#new-factory(?:\/([a-z]+))?$/.exec(location.hash);
    if (m && m[1] === FOUND) return FOUND;
    return m && m[1] && indexOf(m[1]) >= 0 ? m[1] : 'landing';
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
    if (what === 'scope') delete C.dirty.scope;
    if (what === 'regenerate') C.dirty = {};
  };
  const start = async (what, body) => { await call('POST', `/api/creation/${what}`, body); await read(); };

  // What the current step's fields hold, as the record takes it.
  function fields() {
    const body = {};
    const description = page.querySelector('[data-cr="description"]');
    if (description) body.description = description.value;
    if (page.querySelector('[data-cr^="brief-"]')) body.brief = composeBrief((id) => page.querySelector(`[data-cr="brief-${id}"]`).value);
    const scope = page.querySelector('[data-cr="scope"]');
    if (scope) body.scope = scope.value;
    const qs = page.querySelectorAll('[data-q]');
    if (qs.length || (C.route === 'grilling' && record())) {
      body.questions = [...qs].map((q) => ({ id: Number(q.dataset.q), answer: q.querySelector('textarea').value }));
    }
    const reqs = page.querySelectorAll('[data-req]');
    if (reqs.length || (C.route === 'review' && record() && page.querySelector('.cr-reqs'))) {
      body.requirements = [...reqs].map((el) => ({
        id: Number(el.dataset.req) || 0,
        title: el.querySelector('.cr-req-title').value,
        body: el.querySelector('.cr-req-body').value,
        pages: [...el.querySelectorAll('input[data-page]:checked')].map((i) => i.dataset.page),
        components: [...el.querySelectorAll('input[data-component]:checked')].map((i) => i.dataset.component),
      }));
    }
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
      if (C.route === 'review' && (!record() || C.data.branch !== DEMO_BRANCH)) { C.data = demoBase('review'); draw(); }
      drawDemoButton();
      return;
    }
    const r = record();
    if (!r || C.busy) return;
    const once = (key, fn) => { if (C.auto_started[key]) return; C.auto_started[key] = true; fn(); };
    // The first round asks itself; every question deleted later does not bring one back.
    if (C.route === 'grilling' && !r.questions.length && !r.rounds) once(`grill-${r.regenerations}`, () => act('the AI prepares its questions', () => post('grill')));
    // A scope not written, or proposed before the grilling moved the decisions and
    // not retouched: proposed (again) by itself; a retouched one waits for the human.
    if (C.route === 'scope' && (!r.scope.trim() || (C.data.scopeStale && !r.scope_edited))) once(`scope-${r.regenerations}-${r.scope_from}`, () => act(PROPOSING, () => post('scope')));
    if (C.route === 'mockups' && !r.mocked && !running() && !(job() && job().work === 'mockups')) once(`mock-${r.regenerations}`, () => act('starting the drawing', () => start('mockups')));
    const kit = C.data.kit;
    if ((C.route === 'design' || C.route === 'brief') && kit && kit.validated && kit.waiting && !kit.running && !kit.noClaude) once(`kit-${r.regenerations}`, generateKit);
    if (C.route === 'review') {
      if (!C.mock) loadMock();
      if (!r.componentized && !running()) once(`mf-${r.regenerations}`, () => act('starting the micro-frontends', () => start('componentize')));
      else if (r.componentized && !r.requirements.length && !running()) once(`req-${r.regenerations}`, () => act('the AI reads the requirements', async () => { await post('requirements'); await loadMock(); }));
    }
  }
  // The mock-up as it stands: its pages and micro-frontends, for the review's requirements.
  async function loadMock() {
    if (!C.data || !C.data.branch) return;
    try { C.mock = await call('GET', `/api/mockup?branch=${encodeURIComponent(C.data.branch)}`); } catch (e) { C.mock = { pages: [], components: [] }; }
    if (C.route === 'review') draw();
  }

  // ---- drawing --------------------------------------------------------------------
  function crumbs(fresh) {
    const r = fresh ? null : record();
    const at = fresh ? 0 : indexOf(C.route);
    const far = r ? indexOf(C.data.reachable || 'kind') : 0;
    return `<ol class="cr-crumbs" aria-label="creation steps">${STEPS.map(([id, label], i) => {
      const state = i === at ? 'here' : (i < far || (r && i <= far && i !== at) ? 'done' : 'locked');
      const open = reachable(id, fresh);
      // While founding, the first crumb is the founding itself.
      const go = fresh && id === 'repository' ? FOUND : id;
      return `<li class="cr-crumb ${state}"><button type="button" data-cr-go="${go}"${open ? '' : ' disabled'}${i === at ? ' aria-current="step"' : ''}><span class="cr-n">${i < far && i !== at ? '✓' : i + 1}</span><span class="cr-l">${esc(label)}</span>${fresh ? '' : chip(id)}</button></li>`;
    }).join('')}</ol>`;
  }

  // What draws in the background, on the crumb of the step it is for: the
  // scope proposed, the tokens and the kit generating, a scope to redo.
  function chip(id) {
    const r = record();
    if (!r || DEMO) return '';
    const k = C.data.kit;
    if (id === 'scope') {
      if (C.busy === PROPOSING && !r.scope.trim()) return '<span class="cr-chip"><span class="cr-spin" aria-hidden="true"></span> proposing</span>';
      if (C.data.scopeStale) return '<span class="cr-chip warn" title="The decisions moved since the scope was proposed">to redo</span>';
    }
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
    if (el) el.innerHTML = jobPanel();
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
    if (!j || !stage || (j.work !== 'mockups' && j.work !== 'initialize')) return;
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
        <p class="muted">A new repository, then nine steps: what the application does, the AI's questions, the proof of concept's scope, its design system and its mock-ups — up to the factory's first roadmap task.</p>
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
  const briefParts = () => (C.data && C.data.briefParts) || [];
  const briefHeading = (q) => `### ${q.title}`;
  function composeBrief(answer) {
    return briefParts()
      .map((q) => [q, answer(q.id).trim()])
      .filter(([, text]) => text)
      .map(([q, text]) => `${briefHeading(q)}\n\n${text}`)
      .join('\n\n');
  }
  // Back from the record's text to one answer per question; a brief written
  // before the questions, without their headings, goes whole to the first.
  function splitBrief(text) {
    const parts = briefParts();
    const answers = Object.fromEntries(parts.map((q) => [q.id, []]));
    if (!parts.length) return answers;
    let current = parts[0].id;
    for (const line of (text || '').split('\n')) {
      const q = parts.find((b) => line.trim() === briefHeading(b));
      if (q) current = q.id;
      else answers[current].push(line);
    }
    return Object.fromEntries(Object.entries(answers).map(([id, lines]) => [id, lines.join('\n').trim()]));
  }

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
        <button type="button" class="cr-btn" data-cr-grill title="${open ? 'Answer the open questions first' : 'Ask the next round of questions'}"${open || C.busy ? ' disabled' : ''}>${qs.length ? 'Continue grilling' : 'Start the grilling'}</button>
        <button type="button" class="cr-btn on" data-cr-next="scope"${!qs.length && !r.grilled ? ' disabled' : ''}>Continue →</button></div>`;
  }

  function stepScope(r) {
    const stale = C.data.scopeStale && !C.busy;
    return `<p class="muted">The smallest proof of concept that shows the application's value, as the AI reads the brief and the decisions. Edit it freely.</p>
      ${C.busy === PROPOSING && !r.scope.trim() ? '<div class="callout"><span class="cr-spin" aria-hidden="true"></span> The scope is being proposed from the brief and the grilling\'s decisions.</div>' : ''}
      ${stale ? `<div class="callout">The decisions moved since this scope was proposed${r.scope_edited ? ' — and you retouched it: <b>Propose again</b> replaces your edits with a scope read from the decisions as they are now' : ': it is proposed again from them'}.</div>` : ''}
      <label class="cr-field">Scope of the proof of concept<textarea data-cr="scope" class="mono" rows="22">${esc(r.scope)}</textarea></label>
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="grilling">← Back</button>
        <span class="cr-gap"></span>
        <button type="button" class="cr-btn" data-cr-rescope>${C.confirm === 'scope' ? 'Replace my edits? Click again' : 'Propose again'}</button>
        <button type="button" class="cr-btn on" data-cr-next="design"${r.scope.trim() ? '' : ' disabled'}>Continue →</button></div>`;
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
    // While it draws: the checklist and a live preview. Drawn: the easel takes the room (see EMBEDS).
    const split = DEMO ? mine : running('mockups');
    const spinner = !DEMO && running('mockups') ? '<div class="cr-loading"><span class="cr-spin big" aria-hidden="true"></span><b>Drawing the proof of concept…</b></div>' : '';
    return `<p class="muted">A Claude Code session draws the proof of concept as interactive mock-ups, fast: one page per screen of the scope, the journeys linked, the design system's components and tokens. Drawn, it is yours to rework here — <b>edit</b> the page by hand, drop a <b>request</b> pin and apply it, add a page — before the review ties the requirements to it.</p>
      ${split ? drawSplit(C.data.branch, spinner + '<div data-cr-job>' + (mine ? jobPanel() : '') + '</div>') : ''}
      ${!split && mine && !r.mocked ? `<div data-cr-job>${jobPanel()}</div>` : ''}
      ${failed ? `<div class="cr-error" role="alert">${esc(j.why)}</div>` : ''}
      ${r.mocked && !running('mockups') ? `<div class="callout">${esc(r.mocked)}</div>` : ''}
      ${embedFor('mockups') ? '' : mockupsFoot()}`;
  }
  function mockupsFoot() {
    const r = record();
    const j = job();
    const mine = j && j.work === 'mockups';
    const failed = mine && j.state === 'failed';
    return `<div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="design"${running('mockups') ? ' disabled' : ''}>← Back</button>
        <span class="cr-gap"></span>
        ${!running('mockups') && (failed || !mine || r.mocked) ? `<button type="button" class="cr-btn" data-cr-draw>${r.mocked || failed ? 'Draw again' : 'Draw the mock-ups'}</button>` : ''}
        <button type="button" class="cr-btn on" data-cr-next="review"${r.mocked && !running('mockups') ? '' : ' disabled'}>Review →</button></div>`;
  }

  // The step's head; the mock-up's easel is mounted under it (`mountEmbed`), the choices close the step (`reviewFoot`).
  function stepReview(r) {
    const j = job();
    const making = j && j.work === 'componentize';
    return `<p class="muted">The requirements the proof of concept must meet, read off the scope and the mock-up, each tied to the pages and the micro-frontends that carry it. Retouch them, add yours, tie them to the right parts; click through the mock-up below and pin what you still want changed — then decide.</p>
      ${making && j.state !== 'done' ? `<div data-cr-job>${jobPanel()}</div>` : ''}
      ${making && j.state === 'failed' ? '<div class="cr-actions"><span></span><button type="button" class="cr-btn" data-cr-mf>Make the micro-frontends again</button></div>' : ''}
      ${r.componentized ? requirementsPanel(r) : '<p class="muted">The requirements come once the mock-up is made of micro-frontends: they are tied to them.</p>'}`;
  }
  function requirementsPanel(r) {
    const mock = C.mock || { pages: [], components: [] };
    const pages = mock.pages || [];
    const comps = mock.components || [];
    const chips = (kind, chosen, all, label) => all.map((it) => {
      const key = kind === 'page' ? it.path : it.name;
      const on = chosen.includes(key);
      const title = kind === 'page' ? (it.objective || it.title) : [it.description, it.needs && it.needs.length ? `needs ${it.needs.join(', ')}` : '', it.actions && it.actions.length ? `actions ${it.actions.join(', ')}` : ''].filter(Boolean).join(' — ');
      return `<label class="cr-chip-toggle${on ? ' on' : ''}" title="${esc(title)}"><input type="checkbox" data-${kind}="${esc(key)}"${on ? ' checked' : ''}>${esc(label(it))}</label>`;
    }).join('');
    const list = r.requirements.map((q) => `<article class="cr-req${q.human ? ' mine' : ''}" data-req="${q.id}">
        <header><span class="cr-qn">R${q.id}</span><input type="text" class="cr-req-title" value="${esc(q.title)}" placeholder="the requirement, in a few words" aria-label="requirement ${q.id}">${q.human ? '<span class="tag" title="Yours: a new reading keeps it">yours</span>' : ''}<button type="button" class="mini cr-del" data-cr-req-del="${q.id}" title="Delete this requirement" aria-label="delete requirement ${q.id}">✕</button></header>
        <textarea rows="2" class="cr-req-body" placeholder="what it asks for" aria-label="requirement ${q.id}, what it asks for">${esc(q.body)}</textarea>
        <div class="cr-chips"><span class="muted">pages</span>${chips('page', q.pages, pages, (p) => p.title || p.path) || '<span class="muted">none yet</span>'}</div>
        <div class="cr-chips"><span class="muted">micro-frontends</span>${chips('component', q.components, comps, (c) => c.name) || '<span class="muted">none yet</span>'}</div>
      </article>`).join('');
    return `<div class="cr-reqs">${list || (C.busy ? '' : '<p class="muted">No requirement yet.</p>')}</div>
      <div class="cr-actions"><span></span>
        <button type="button" class="cr-btn" data-cr-req-add>+ requirement</button>
        <button type="button" class="cr-btn" data-cr-reqs title="The AI reads the requirements again off the scope and the mock-up as it stands; yours are kept"${C.busy ? ' disabled' : ''}>${r.requirements.length ? 'Read again' : 'Read the requirements'}</button></div>`;
  }
  function reviewFoot() {
    // Both choices call the real backend (regenerate scraps the mock-up,
    // accept publishes and opens a roadmap task) — disabled in the demo,
    // the same read-only stance as the embedded mock-up pane above.
    const lock = DEMO ? ' disabled title="Disabled in the demo"' : '';
    return `<div class="cr-choices">
        <section class="cr-card"><h3>a · Regenerate</h3><p class="muted">Back to the brief with everything filled in; the grilling keeps its questions and their answers, and may ask more.</p><button type="button" class="cr-btn" data-cr-regenerate${lock}>${C.confirm === 'regenerate' ? 'The mock-ups go. Click again' : 'Regenerate'}</button></section>
        <section class="cr-card"><h3>b · Accept the proof of concept</h3><p class="muted">The record — brief, decisions, scope, requirements, pages, micro-frontends, the pins left — is written in the repository beside the mock-up; the mock-up and the design system are published, the roadmap task opened with all of it, the watch started.</p><button type="button" class="cr-btn on" data-cr-accept${lock}>Accept</button></section>
      </div>
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="mockups">← Back to the mock-up</button></div>`;
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

  const BODIES = { kind: stepKind, brief: stepBrief, grilling: stepGrilling, scope: stepScope, design: stepDesign, mockups: stepMockups, review: stepReview, initialize: stepInitialize };

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
      ${C.busy ? `<div class="cr-busy" role="status"><span class="cr-spin" aria-hidden="true"></span>${esc(C.busy)}…</div>` : ''}
      ${C.error ? `<div class="cr-error" role="alert">${esc(C.error)}</div>` : ''}
      <div class="cr-body">${body}</div>
      ${embed ? `<div class="cr-embed" id="cr-embed" data-cr-fullable data-cr-keep="${esc(embed.face)}"><div class="cr-embed-bar"><b>${esc(embed.title)}</b><button type="button" class="cr-btn" data-cr-full title="Fill the screen (Esc to leave)">⛶ Full screen</button></div><div class="cr-embed-host" id="cr-embed-host"></div></div>${embed.foot()}` : ''}
    </div>`;
    paint(html);
    if (embed) mountEmbed(embed);
    else unmountEmbed();
    syncPreview();
    showDrawn();
    restore();
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
    mockups: { kind: 'mockup', face: 'mockup', title: "The mock-up — rework it: edit, pin a request, add a page", foot: mockupsFoot, opts: () => ({ kind: 'mockup', branch: C.data.branch, readOnly: DEMO }) },
    review: { kind: 'mockup', face: 'mockup', title: "The mock-up's easel", foot: reviewFoot, opts: () => ({ kind: 'mockup', branch: C.data.branch, readOnly: DEMO }) },
  };
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
  }
  // A part of the page takes the whole screen; Esc gives it back.
  function toggleFull(el) {
    const on = el.classList.toggle('cr-full');
    document.body.classList.toggle('cr-fullscreen', !!page.querySelector('.cr-full'));
    el.querySelector('[data-cr-full]').textContent = on ? '✕ Leave full screen' : '⛶ Full screen';
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
    act('the AI prepares the next round', async () => { await save(body); await post('grill'); })
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
  page.addEventListener('click', (e) => {
    const t = e.target.closest('button');
    if (!t || t.disabled) return;
    if (t.matches('[data-cr-leave]')) { window.HarnessFactories.leave(); return; }
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
      page.querySelector(`[data-req="${t.dataset.crReqDel}"]`)?.remove();
      const body = fields();
      act('saving', () => save(body));
      return;
    }
    if (t.matches('[data-cr-req-add]')) {
      const body = fields();
      body.requirements = [...(body.requirements || []), { id: 0, title: 'New requirement', body: '', pages: [], components: [] }];
      act('saving', () => save(body));
      return;
    }
    if (t.matches('[data-cr-reqs]')) {
      const body = fields();
      act('the AI reads the requirements', async () => { await save(body); await post('requirements'); await loadMock(); });
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
      const edited = page.querySelector('[data-cr="scope"]').value !== record().scope;
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
    const keep = async () => { if (!record() || C.busy) return; try { await save(fields()); } catch (err) { C.error = err.message; draw(); } };
    if (now) keep(); else C.reqTimer = setTimeout(keep, 800);
  }
  async function keepAnswers() {
    answersOpen();
    if (!record() || C.busy) return;
    try { await save(fields()); } catch (err) { C.error = err.message; draw(); }
  }
  page.addEventListener('change', (e) => {
    if (e.target.closest('[data-q]')) keepAnswers();
    if (e.target.closest('[data-req]')) { const chip = e.target.closest('.cr-chip-toggle'); if (chip) chip.classList.toggle('on', e.target.checked); keepRequirements(!!chip); }
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
    if (e.target.closest('[data-q]')) answersOpen();
    if (e.target.closest('[data-req]')) keepRequirements(false);
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
