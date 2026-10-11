// The factory-creation page: connect an existing repository, or initialize
// a factory through nine steps under a breadcrumb — the repository, the kind
// of factory, the brief, the grilling, the proof of concept's scope, the
// design system, the mock-ups, their review, the initialization. Steps 2 to 9
// run in the new factory's own view; the design system's easel and the
// mock-up's open over the page (`harness:open-pane`, body.easel) and give it
// back when they close (`harness:pane-closed`). Driven by factories.js
// through `window.HarnessCreation`.
(() => {
  'use strict';

  const $ = (id) => document.getElementById(id);
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const HASH = '#new-factory';
  const POLL_MS = 2000;

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

  const C = { dirty: {}, loaded: false, data: null, route: 'landing', auto: false, busy: '', error: '', owners: null, ownersWhy: '', timer: null, auto_started: {}, confirm: '', opening: '', pages: null };
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

  // ---- the route, in the hash ---------------------------------------------------
  function routeFromHash() {
    const m = /^#new-factory(?:\/([a-z]+))?$/.exec(location.hash);
    return m && m[1] && indexOf(m[1]) >= 0 ? m[1] : 'landing';
  }
  function setRoute(route) {
    C.route = route;
    C.confirm = '';
    const hash = route === 'landing' ? HASH : `${HASH}/${route}`;
    if (location.hash !== hash) history.replaceState(null, '', location.pathname + location.search + hash);
    draw();
  }

  // A step is open when the gates reach it; without a record only the first is.
  function reachable(step) {
    const r = record();
    if (!r) return step === 'repository';
    return indexOf(step) <= indexOf(C.data.reachable || 'kind');
  }

  // ---- the polling while a job runs ------------------------------------------------
  // The kit generating on the design step: its counts follow.
  const kitBusy = () => C.route === 'design' && !!(C.data && C.data.kit && C.data.kit.running);
  function poll() {
    clearTimeout(C.timer);
    if (!running() && !kitBusy()) return;
    C.timer = setTimeout(async () => {
      if (kitBusy() && !running()) {
        await read();
        if (C.route === 'design' && !C.busy) draw();
        poll();
        return;
      }
      const before = job();
      await read();
      const after = job();
      drawJob();
      if (after && after.state !== 'running') onJobEnd(before, after);
      else poll();
    }, POLL_MS);
  }
  const generateKit = () => act('starting the kit', async () => { await call('POST', '/api/kit/generate', {}); await read(); });
  function onJobEnd(before, after) {
    if (after.work === 'found' && after.state === 'done' && after.factory) return openFactory(after.factory, 'kind');
    if (after.work === 'connect' && after.state === 'done' && after.factory) return openFactory(after.factory, null);
    if (after.work === 'mockups' && after.state === 'done') { setRoute('review'); enter(); return undefined; }
    draw();
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
    poll();
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
    const brief = page.querySelector('[data-cr="brief"]');
    if (brief) body.brief = brief.value;
    const scope = page.querySelector('[data-cr="scope"]');
    if (scope) body.scope = scope.value;
    const qs = page.querySelectorAll('[data-q]');
    if (qs.length || (C.route === 'grilling' && record())) {
      body.questions = [...qs].map((q) => ({ id: Number(q.dataset.q), answer: q.querySelector('textarea').value }));
    }
    return body;
  }

  async function next(step) {
    // Read before `act` redraws the step.
    const body = { ...fields(), step };
    await act('saving', async () => {
      await save(body);
      C.route = step;
      setRoute(step);
    });
    enter();
  }

  // What a step starts on its own the first time it is entered.
  function enter() {
    const r = record();
    if (!r || C.busy) return;
    const once = (key, fn) => { if (C.auto_started[key]) return; C.auto_started[key] = true; fn(); };
    if (C.route === 'grilling' && !r.questions.length) once(`grill-${r.regenerations}`, () => act('the AI prepares its questions', () => post('grill')));
    if (C.route === 'scope' && !r.scope.trim()) once(`scope-${r.regenerations}`, () => act('the AI proposes the scope', () => post('scope')));
    if (C.route === 'mockups' && !r.mocked && !running() && !(job() && job().work === 'mockups')) once(`mock-${r.regenerations}`, () => act('starting the drawing', () => start('mockups')));
    const kit = C.data.kit;
    if (C.route === 'design' && kit && kit.validated && kit.waiting && !kit.running && !kit.noClaude) once(`kit-${r.regenerations}`, generateKit);
    if (C.route === 'review') readPages();
  }

  async function readPages() {
    try {
      const state = await call('GET', `/api/mockup?branch=${encodeURIComponent(C.data.branch)}`);
      C.pages = state.pages || [];
    } catch (e) { C.pages = []; }
    if (C.route === 'review') drawPages();
  }

  // An easel over the page: the design system's or the mock-up's.
  function easel(detail) {
    document.body.classList.add('easel');
    window.dispatchEvent(new CustomEvent('harness:open-pane', { detail }));
  }
  window.addEventListener('harness:pane-closed', async () => {
    if (!document.body.classList.contains('easel')) return;
    document.body.classList.remove('easel');
    await read();
    draw();
    enter();
    poll();
  });

  // ---- drawing --------------------------------------------------------------------
  function crumbs() {
    const r = record();
    const at = indexOf(C.route);
    const far = r ? indexOf(C.data.reachable || 'kind') : 0;
    return `<ol class="cr-crumbs" aria-label="creation steps">${STEPS.map(([id, label], i) => {
      const state = i === at ? 'here' : (i < far || (r && i <= far && i !== at) ? 'done' : 'locked');
      const open = reachable(id);
      return `<li class="cr-crumb ${state}"><button type="button" data-cr-go="${id}"${open ? '' : ' disabled'}${i === at ? ' aria-current="step"' : ''}><span class="cr-n">${i < far && i !== at ? '✓' : i + 1}</span><span class="cr-l">${esc(label)}</span></button></li>`;
    }).join('')}</ol>`;
  }

  function jobPanel() {
    const j = job();
    if (!j) return '';
    const title = { found: 'Founding the factory', connect: 'Connecting the repository', mockups: 'Drawing the mock-ups', initialize: 'Initializing the factory' }[j.work] || j.work;
    const state = j.state === 'running' ? '<span class="cr-spin" aria-hidden="true"></span> under way' : j.state === 'done' ? '<span class="tag ok">done</span>' : '<span class="tag bad">stopped</span>';
    return `<div class="cr-job ${esc(j.state)}" role="status"><div class="cr-job-head"><b>${esc(title)}</b> ${state}</div>
      <ul>${j.steps.map((s) => `<li>${esc(s)}</li>`).join('')}</ul>
      ${j.why ? `<p class="cr-bad">${esc(j.why)}</p>` : ''}
      ${j.report ? `<details><summary>init-repo's report</summary><pre>${esc(j.report)}</pre></details>` : ''}
      ${C.opening ? `<p>Opening the factory… the page reloads when it is up.</p>` : ''}</div>`;
  }
  function drawJob() {
    const el = page.querySelector('[data-cr-job]');
    if (el) el.innerHTML = jobPanel();
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
        <div class="cr-row"><button type="button" class="cr-btn on" data-cr-go="repository">Start</button></div>
      </section>
    </div>`;
  }

  function stepRepository() {
    const r = record();
    if (r) {
      return `<p>The repository <a href="https://github.com/${esc(r.slug)}" target="_blank" rel="noopener">${esc(r.slug)} ↗</a> is created, prepared by <code>init-repo</code>, and this factory runs in its own folder.</p>
        <div class="cr-actions"><span></span><button type="button" class="cr-btn on" data-cr-next="kind">Continue →</button></div>`;
    }
    const j = job();
    const founding = j && j.work === 'found';
    if (C.owners === null && !C.ownersWhy) loadOwners();
    const owners = C.owners || [];
    return `<p class="muted">The factory's repository on GitHub. The harness creates it with a first commit, makes the factory's folder beside the register (<code>~/.harness/factories/</code>), runs <code>init-repo</code> there, then opens the new factory.</p>
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

  function stepBrief(r) {
    return `<p class="muted">What the application does: who uses it, for what, what it keeps. The AI's questions start from here.</p>
      ${r.regenerations ? '<div class="callout">Sent back from the review: everything written is still here — change what the mock-ups got wrong.</div>' : ''}
      <label class="cr-field">What the application does<textarea data-cr="brief" rows="12" placeholder="An application that…">${esc(r.brief)}</textarea></label>
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="kind">← Back</button><button type="button" class="cr-btn on" data-cr-next="grilling">Continue →</button></div>`;
  }

  function stepGrilling(r) {
    const qs = r.questions;
    const open = qs.filter((q) => !q.answer.trim()).length;
    const list = qs.map((q) => `<article class="cr-q" data-q="${q.id}">
        <header><span class="cr-qn">Q${q.id}</span><b>${esc(q.title)}</b>${q.round ? `<span class="tag">round ${q.round}</span>` : ''}<button type="button" class="mini cr-del" data-cr-del="${q.id}" title="Delete this question" aria-label="delete question ${q.id}">✕</button></header>
        ${q.body ? `<p class="cr-qb">${esc(q.body)}</p>` : ''}
        ${q.recommended ? `<p class="cr-rec">➡️ ${esc(q.recommended)} <button type="button" class="mini" data-cr-take="${q.id}">use it</button></p>` : ''}
        <textarea rows="3" aria-label="answer to question ${q.id}" placeholder="Your answer">${esc(q.answer)}</textarea>
      </article>`).join('');
    return `<p class="muted">The AI grills the brief round after round: every decision it can ask now, each with its recommended answer. Answer, retouch, or delete a question.</p>
      ${list || (C.busy ? '' : '<p class="muted">No question yet.</p>')}
      ${r.grilled ? '<div class="callout">The AI has no more questions.</div>' : ''}
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="brief">← Back</button>
        <span class="cr-gap"></span>
        <button type="button" class="cr-btn" data-cr-grill title="${open ? 'Answer the open questions first' : 'Ask the next round of questions'}"${open || C.busy ? ' disabled' : ''}>${qs.length ? 'Continue grilling' : 'Start the grilling'}</button>
        <button type="button" class="cr-btn on" data-cr-next="scope"${!qs.length || open ? ' disabled' : ''}>Continue →</button></div>`;
  }

  function stepScope(r) {
    return `<p class="muted">The smallest proof of concept that shows the application's value, as the AI reads the brief and the decisions. Edit it freely.</p>
      <label class="cr-field">Scope of the proof of concept<textarea data-cr="scope" class="mono" rows="22">${esc(r.scope)}</textarea></label>
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="grilling">← Back</button>
        <span class="cr-gap"></span>
        <button type="button" class="cr-btn" data-cr-rescope>${C.confirm === 'scope' ? 'Replace my edits? Click again' : 'Propose again'}</button>
        <button type="button" class="cr-btn on" data-cr-next="design"${r.scope.trim() ? '' : ' disabled'}>Continue →</button></div>`;
  }

  function stepDesign() {
    const k = C.data.kit || { validated: C.data.designValidated, waiting: 0, generated: 0, failed: 0, running: false, noClaude: false };
    const ready = k.validated && !k.waiting;
    const kitLine = !k.validated ? '<span class="muted">The kit is generated once the design system is validated.</span>'
      : `<span class="tag ${k.waiting ? 'warn' : 'ok'}">kit: ${k.generated} generated${k.waiting ? ` · ${k.waiting} waiting` : ''}${k.failed ? ` · ${k.failed} failed` : ''}</span>${k.running ? '<span class="cr-spin" aria-hidden="true"></span><span class="muted">generating…</span>' : ''}`;
    return `<p class="muted">The design system the mock-ups and the product wear: a moodboard and the logo, a brief, the tokens an AI draws from them, validated by you — then the kit of components, generated from it. It lives on the <code>design-system</code> branch.</p>
      <div class="cr-card"><div class="cr-row"><span>${k.validated ? '<span class="tag ok">validated</span>' : '<span class="tag warn">not validated yet</span>'}</span>
        <button type="button" class="cr-btn on" data-cr-easel="kit">Open the design system's easel</button></div>
        <div class="cr-row cr-kit">${kitLine}
          ${k.validated && (k.waiting || k.failed) && !k.running ? `<button type="button" class="cr-btn" data-cr-kit${k.noClaude ? ' disabled title="No Claude at hand"' : ''}>${k.failed ? 'Generate the missing components' : 'Generate the kit'}</button>` : ''}</div>
        ${k.failed && !k.waiting ? '<p class="muted">Some components failed: the mock-ups do without them, or generate them again.</p>' : ''}</div>
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="scope">← Back</button><button type="button" class="cr-btn on" data-cr-next="mockups"${ready ? '' : ` disabled title="${k.validated ? 'Wait for the kit' : 'Validate the design system first'}"`}>Continue →</button></div>`;
  }

  function stepMockups(r) {
    const j = job();
    const mine = j && j.work === 'mockups';
    const failed = mine && j.state === 'failed';
    return `<p class="muted">A Claude Code session draws the proof of concept as interactive mock-ups, fast: one page per screen of the scope, the journeys linked, the design system's components and tokens. The micro-frontends come once you accept it.</p>
      ${running('mockups') ? '<div class="cr-loading"><span class="cr-spin big" aria-hidden="true"></span><b>Drawing the proof of concept…</b></div>' : ''}
      <div data-cr-job>${mine ? jobPanel() : ''}</div>
      ${r.mocked && !running('mockups') ? `<div class="callout">${esc(r.mocked)}</div>` : ''}
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="design"${running('mockups') ? ' disabled' : ''}>← Back</button>
        <span class="cr-gap"></span>
        ${!running('mockups') && (failed || !mine || r.mocked) ? `<button type="button" class="cr-btn" data-cr-draw>${r.mocked || failed ? 'Draw again' : 'Draw the mock-ups'}</button>` : ''}
        <button type="button" class="cr-btn on" data-cr-go="review"${r.mocked && !running('mockups') ? '' : ' disabled'}>Review →</button></div>`;
  }

  function drawPages() {
    const el = page.querySelector('[data-cr-pages]');
    if (!el) return;
    const pages = C.pages;
    el.innerHTML = pages === null ? '<p class="muted">Reading the pages…</p>' : pages.length
      ? `<ul class="cr-pages">${pages.map((p) => `<li><button type="button" class="cr-link" data-cr-page="${esc(p.path)}"><b>${esc(p.title || p.path)}</b><span class="muted">${esc(p.objective || p.path)}</span></button></li>`).join('')}</ul>`
      : '<p class="muted">The mock-up lists no page.</p>';
  }

  function stepReview(r) {
    return `<p class="muted">The proof of concept, page by page. Click through it, then decide.</p>
      ${r.mocked ? `<div class="callout">${esc(r.mocked)}</div>` : ''}
      <div data-cr-pages></div>
      <div class="cr-choices">
        <section class="cr-card"><h3>a · Request changes</h3><p class="muted">Pin requests on the pages, apply them and save — the mock-up's easel, with its pins.</p><button type="button" class="cr-btn" data-cr-easel="mockup">Open the mock-up</button></section>
        <section class="cr-card"><h3>b · Regenerate</h3><p class="muted">Back to the brief with everything filled in; the grilling keeps its questions and their answers, and may ask more.</p><button type="button" class="cr-btn" data-cr-regenerate>${C.confirm === 'regenerate' ? 'The mock-ups go. Click again' : 'Regenerate'}</button></section>
        <section class="cr-card"><h3>c · Accept the proof of concept</h3><p class="muted">An agent makes the mock-up again of micro-frontends, keeping what you saw; then the design system and the mock-up are published, the roadmap task opened, the watch started.</p><button type="button" class="cr-btn on" data-cr-accept>Accept</button></section>
      </div>
      <div class="cr-actions"><button type="button" class="cr-btn" data-cr-go="mockups">← Back</button></div>`;
  }

  function stepInitialize(r) {
    const j = job();
    const mine = j && j.work === 'initialize';
    const failed = mine && j.state === 'failed';
    const done = !!r.issue && !running('initialize');
    return `<p class="muted">The accepted mock-up made again of micro-frontends — you find them in the mock-up's micro-frontends view — then the design system and the mock-up published on the integration branch, the proof of concept's roadmap task opened, the watch started: the planner takes it from there.</p>
      ${r.componentized ? `<div class="callout">${esc(r.componentized)}</div>` : ''}
      <div data-cr-job>${mine ? jobPanel() : ''}</div>
      ${r.issue ? `<p>The roadmap task: <a href="${esc(r.issue)}" target="_blank" rel="noopener">${esc(r.issue)} ↗</a></p>` : ''}
      <div class="cr-actions"><span></span>
        ${failed || (!mine && !r.issue) ? '<button type="button" class="cr-btn" data-cr-accept>Initialize again</button>' : ''}
        <button type="button" class="cr-btn on" data-cr-finish${done ? '' : ' disabled'}>Open the plant →</button></div>`;
  }

  const BODIES = { kind: stepKind, brief: stepBrief, grilling: stepGrilling, scope: stepScope, design: stepDesign, mockups: stepMockups, review: stepReview, initialize: stepInitialize };

  function draw() {
    if (page.hidden) return;
    if (!C.loaded) { page.innerHTML = '<div class="cr"><p class="muted">Reading the creation…</p></div>'; return; }
    const r = record();
    // A step the gates do not reach yet falls back on the furthest they do.
    if (C.route !== 'landing' && !reachable(C.route)) C.route = r ? C.data.reachable || r.step : 'repository';
    const title = C.route === 'landing' ? 'New factory' : `${indexOf(C.route) + 1} · ${STEPS[indexOf(C.route)][1]}`;
    const body = C.route === 'landing' ? landing()
      : C.route === 'repository' ? stepRepository()
        : r ? BODIES[C.route](r) : '<p class="muted">Reading the creation…</p>';
    const lead = C.auto && !r ? '<p class="muted">This factory is not linked to a GitHub repository.</p>' : '';
    page.innerHTML = `<div class="cr">
      <div class="cr-top"><h1>${esc(title)}</h1><button type="button" class="fp-back" data-cr-leave>${C.auto ? '← open the plant anyway' : '← back to the plant'}</button></div>
      ${lead}
      ${C.route === 'landing' ? '' : crumbs()}
      ${C.busy ? `<div class="cr-busy" role="status"><span class="cr-spin" aria-hidden="true"></span>${esc(C.busy)}…</div>` : ''}
      ${C.error ? `<div class="cr-error" role="alert">${esc(C.error)}</div>` : ''}
      <div class="cr-body">${body}</div>
    </div>`;
    restore();
    if (C.route === 'review') drawPages();
  }
  // Typing survives a redraw: what was typed and not saved goes back in its field.
  function restore() {
    for (const [key, value] of Object.entries(C.dirty)) {
      const field = key.startsWith('q') ? page.querySelector(`[data-q="${key.slice(1)}"] textarea`) : page.querySelector(`[data-cr="${key}"]`);
      if (field) field.value = value;
    }
  }
  page.addEventListener('input', (e) => {
    const named = e.target.closest('[data-cr]');
    if (named) C.dirty[named.dataset.cr] = named.value;
    const q = e.target.closest('[data-q]');
    if (q && e.target.tagName === 'TEXTAREA') C.dirty[`q${q.dataset.q}`] = e.target.value;
  });

  async function loadOwners() {
    C.owners = [];
    try {
      C.owners = (await call('GET', '/api/creation/owners')).owners || [];
    } catch (e) { C.owners = null; C.ownersWhy = e.message; }
    if (C.route !== 'repository' || record()) return;
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
    act('the AI prepares the next round', async () => { await save(body); await post('grill'); });
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
    const t = e.target.closest('button, [data-cr-page]');
    if (!t || t.disabled) return;
    if (t.matches('[data-cr-leave]')) { window.HarnessFactories.leave(); return; }
    if (t.dataset.crGo) {
      if (t.dataset.crGo !== 'landing' && t.dataset.crGo !== C.route && record()) {
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
      if (t.dataset.crNext === 'scope' && rounds !== null && rounds < 3) {
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
      keepAnswers();
      return;
    }
    if (t.dataset.crDel) {
      page.querySelector(`[data-q="${t.dataset.crDel}"]`)?.remove();
      const body = fields();
      act('saving', () => save(body));
      return;
    }
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
      act('the AI proposes the scope', () => post('scope'));
      return;
    }
    if (t.dataset.crEasel === 'kit') { easel({ kind: 'kit', title: 'design system', tab: 'system' }); return; }
    if (t.dataset.crEasel === 'mockup') { easel({ kind: 'mockup', title: 'mock-up', branch: C.data.branch }); return; }
    if (t.dataset.crPage) { easel({ kind: 'mockup', title: 'mock-up', branch: C.data.branch, page: t.dataset.crPage }); return; }
    if (t.matches('[data-cr-kit]')) { generateKit(); return; }
    if (t.matches('[data-cr-draw]')) { act('starting the drawing', () => start('mockups')); return; }
    if (t.matches('[data-cr-regenerate]')) {
      if (C.confirm !== 'regenerate') { C.confirm = 'regenerate'; draw(); return; }
      C.auto_started = {};
      act('back to the brief', async () => { await post('regenerate'); C.pages = null; setRoute('brief'); });
      return;
    }
    if (t.matches('[data-cr-accept]')) { act('initializing', async () => { await post('accept'); await read(); setRoute('initialize'); }); return; }
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
    if (next) next.disabled = !any || !!open;
  }
  async function keepAnswers() {
    answersOpen();
    if (!record() || C.busy) return;
    try { await save(fields()); } catch (err) { C.error = err.message; draw(); }
  }
  page.addEventListener('change', (e) => { if (e.target.closest('[data-q]')) keepAnswers(); });
  page.addEventListener('input', (e) => { if (e.target.closest('[data-q]')) answersOpen(); });

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
    poll();
  }
  function close() {
    clearTimeout(C.timer);
    document.body.classList.remove('easel');
  }
  function route() {
    const to = routeFromHash();
    if (to !== C.route) { C.route = to; draw(); enter(); }
  }
  // Whether this factory is being created: the page opens by itself then.
  async function pending() {
    const data = await read();
    return !!(data && data.record);
  }

  window.HarnessCreation = { open, close, route, pending };
})();
