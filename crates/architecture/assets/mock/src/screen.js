// The screen host of the mock-up: what a frozen screen composes its
// micro-frontends with (contract E). A page injects a micro-frontend as
// `<div data-component="<Name>" data-props='{"campagneId": "$ctx.campagne"}'>`;
// `$ctx.<name>` reads the screen's context, which a micro-frontend sets
// (`setContext`) and the others follow (`compose` updates them). The real
// composer host keeps this shape; this file is installed by `harness
// init-repo` and is not meant to change per product.

const context = () => (window.__mockContext = window.__mockContext || {});

/** Sets one value of the screen's context and tells every micro-frontend. */
export function setContext(name, value) {
  const ctx = context();
  if (ctx[name] === value) return;
  ctx[name] = value;
  document.dispatchEvent(new CustomEvent('mock:context', { detail: { name, value } }));
}

/** One value of the screen's context; `undefined` while nothing set it. */
export function getContext(name) {
  return context()[name];
}

/** Listens to the context; answers how to stop. */
export function onContext(listener) {
  const handler = (e) => listener(e.detail.name, e.detail.value);
  document.addEventListener('mock:context', handler);
  return () => document.removeEventListener('mock:context', handler);
}

/** The `data-component` element this bundle was loaded for, its root to mount in, and its raw props. */
export function mountPoint() {
  const script = document.currentScript;
  const component = script && script.closest('[data-component]');
  const host = component ? component.querySelector('.mf-root') || component : document.body;
  let props = {};
  try { props = JSON.parse((component && component.dataset.props) || '{}'); } catch (e) { props = {}; }
  return { component, host, props };
}

/** The raw props with every `$ctx.<name>` resolved; `undefined` while the context has no value. */
export function resolveProps(raw) {
  const out = {};
  for (const [key, value] of Object.entries(raw || {})) {
    out[key] = typeof value === 'string' && value.startsWith('$ctx.') ? getContext(value.slice(5)) : value;
  }
  return out;
}

/**
 * Mounts a micro-frontend with its resolved props and keeps them current:
 * `mount(props)` answers what the Micro-UI's own mount answers — an object
 * with `update(props)` when it follows its props. Answers that object.
 */
export function compose(raw, mount) {
  const follows = Object.values(raw || {})
    .filter((v) => typeof v === 'string' && v.startsWith('$ctx.'))
    .map((v) => v.slice(5));
  const mounted = mount(resolveProps(raw));
  if (follows.length) onContext((name) => { if (follows.includes(name) && mounted && mounted.update) mounted.update(resolveProps(raw)); });
  return mounted;
}
