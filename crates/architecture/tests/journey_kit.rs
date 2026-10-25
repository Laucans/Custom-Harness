//! `tests/rules/journey/kit.ts` against node: a running rule's test fails
//! once its body ran when the real server answered a `POST /commands` with
//! the DataGuard engine's undeclared-key warning — the keys and the
//! DataCapability named, "not a rule contest" said — and passes on any
//! other warning, request or answer; a pending test stays on the plain
//! `test`, so it opens no browser. The answer is read before the page sees
//! it: one that arrives after the body returned, or in a context the test
//! opened and closed itself, still fails it. Playwright is a stub here: the
//! kit's own code runs, type-stripped by node. Skipped, loudly, when no
//! `node` on the path strips TypeScript — red instead when `CI` is set.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const KIT: &str = include_str!("../assets/rules/journey/kit.ts");
const SERVER: &str = include_str!("../assets/mock/src/server.js");
const ENGINE: &str = include_str!("../../dataguard/src/engine.rs");

/// What the kit needs of `@playwright/test`: `test`, its `skip`, its
/// `extend` — each declaration kept for the driver.
const PLAYWRIGHT: &str = r"
export const declared = [];
export const fixtures = {};
const make = (kind) => {
  const t = (title, options, body) => declared.push({ kind, title, options, body });
  t.skip = () => {};
  return t;
};
export const test = make('test');
test.extend = (definitions) => {
  Object.assign(fixtures, definitions);
  return make('journey');
};
";

const DRIVER: &str = r"
process.env.RULES_APP_URL = 'http://app';
const { rule, undeclared, offManifest, IN_FLIGHT } = await import('./kit.ts');
const { declared, fixtures } = await import('@playwright/test');
const expect = (cond, what) => { if (!cond) { console.error('FAILED: ' + what); process.exit(1); } };

rule('R1', [], 'a journey', async () => {});
rule('R2', ['micro-ui:Nowhere'], 'a pending one', async () => {});
expect(declared.length === 2 && declared[0].kind === 'journey' && declared[0].title === 'R1 a journey', 'a running test checks its payloads: ' + JSON.stringify(declared));
expect(declared[1].kind === 'test', 'a pending test opens no browser: ' + JSON.stringify(declared));

expect(undeclared(null).length === 0 && undeclared('x').length === 0 && undeclared({ warnings: 'no' }).length === 0, 'no answer, no warning');
expect(undeclared({ warnings: [3, 'payload key(s) a not declared by b.c', 'payload key(s) not declared'] }).join() === 'payload key(s) a not declared by b.c', 'only the engine words');

const [payloads, options] = fixtures.payloadsDeclared;
expect(options && options.auto === true, 'on for every running test');
const log = [];
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
// A page: the commands it has in flight, as the kit's init script counts them.
const page = (name) => {
  const p = {
    inFlight: 0,
    gone: false,
    waitForFunction: async (expression) => {
      if (p.gone) throw new Error('Target page, context or browser has been closed');
      expect(expression === '!window.__kitCommandsInFlight', 'the kit waits on its own counter: ' + expression);
      while (p.inFlight) await sleep(2);
    },
    close: async () => { log.push('page close ' + name); },
  };
  return p;
};
// A browser context as the kit sees it: `addInitScript`, `route` and
// `unroute`, `pages` and its `page` event, `close`; `send` is its page's
// request, routed `dispatch` ms later (the browser's own delay) to the first
// route whose predicate takes its URL (a URL object, as Playwright passes
// it), else to the network.
const context = (name) => {
  const routes = [];
  const listeners = new Set();
  const own = page(name);
  const ctx = {
    pageGone: false,
    scripts: [],
    addInitScript: async (script) => { ctx.scripts.push(script); },
    route: async (matcher, handler) => { routes.push([matcher, handler]); },
    unroute: async (matcher, handler) => {
      const at = routes.findIndex(([m, h]) => m === matcher && h === handler);
      if (at >= 0) routes.splice(at, 1);
    },
    pages: () => [own],
    page: own,
    on: (event, l) => { if (event === 'page') listeners.add(l); },
    off: (event, l) => listeners.delete(l),
    listeners: () => listeners.size,
    close: async () => { log.push('close ' + name); },
    routes: () => routes.length,
    send: (method, url, answer, dispatch = 0) => {
      own.inFlight++;
      return sleep(dispatch).then(() => {
        const match = routes.find(([m]) => typeof m === 'function' && m(new URL(url)));
        if (!match) return 'network';
        let seen = 'nothing';
        const route = {
          request: () => ({ method: () => method, url: () => url }),
          fallback: async () => { seen = 'fallback'; },
          abort: async (code) => { seen = 'aborted ' + code; },
          fetch: () => answer(),
          fulfill: async ({ response }) => {
            if (ctx.pageGone) throw new Error('Target page, context or browser has been closed');
            seen = response;
          },
        };
        return Promise.resolve(match[1](route)).then(() => seen);
      }).finally(() => { own.inFlight--; });
    },
  };
  return ctx;
};
// What the server answers, after `ms`; an Error body is one that is not JSON.
const reply = (body, ms = 0) => () => new Promise((resolve) => setTimeout(() => {
  log.push('answered');
  resolve({ json: () => (body instanceof Error ? Promise.reject(body) : Promise.resolve(body)) });
}, ms));
const unreachable = () => Promise.reject(new Error('ECONNREFUSED'));
const warned = (text) => ({ warnings: [text] });
const browser = () => {
  const b = { made: [], newContext: async () => { const c = context('made'); b.made.push(c); return c; } };
  return b;
};

const quiet = context('quiet');
const quietClose = quiet.close;
const quietPageClose = quiet.page.close;
const quietBrowser = browser();
const original = quietBrowser.newContext;
const seen = [];
await payloads({ context: quiet, browser: quietBrowser }, async () => {
  expect(quiet.routes() === 1 && quietBrowser.newContext !== original, 'the default context and new ones are watched');
  expect(quiet.scripts.length === 1 && quiet.scripts[0] === IN_FLIGHT && quiet.listeners() === 1 && quiet.page.close !== quietPageClose, 'its pages count their commands, and a page closing waits for them');
  const declared = warned('a value is declared ahead: confirm or cancel');
  seen.push(await quiet.send('POST', 'http://app/commands', reply(declared)));
  seen.push(await quiet.send('GET', 'http://app/commands', reply(warned('payload key(s) x not declared by a.b'))));
  seen.push(await quiet.send('POST', 'http://app/capabilities/a.read', reply(warned('payload key(s) x not declared by a.b'))));
  seen.push(await quiet.send('POST', 'http://app/commands/c1/confirm', reply(warned('payload key(s) x not declared by a.b'))));
  seen.push(await quiet.send('POST', 'http://app/commands', reply(new Error('no JSON'))));
  seen.push(await quiet.send('POST', 'http://app/commands', reply({ warnings: [], then: [{ submitted: warned('payload key(s) seal not declared by a.c') }] })));
  seen.push(await quiet.send('POST', 'http://app/commands', unreachable));
  expect(seen[0] && typeof seen[0].json === 'function', 'the page gets the server\'s answer: ' + JSON.stringify(seen[0]));
  expect(seen.slice(1, 4).join() === 'fallback,network,network', 'only a POST /commands is answered through the kit: ' + seen.slice(1, 4).join());
  expect(seen[6] === 'aborted failed', 'a request the server never answered fails for the page: ' + seen[6]);
});
expect(quiet.routes() === 0 && quiet.close === quietClose && quiet.page.close === quietPageClose && quiet.listeners() === 0 && quietBrowser.newContext === original, 'the kit goes with the test');

const loud = context('loud');
let failure;
try {
  await payloads({ context: loud, browser: browser() }, async () => {
    await loud.send('POST', 'http://app/api/commands', reply(warned('payload key(s) conditions not declared by table.setTokenCondition')));
    await loud.send('POST', 'http://app/commands', reply(warned('payload key(s) conditions not declared by table.setTokenCondition')));
    await loud.send('POST', 'http://app/commands', reply({ warnings: ['invariant(s) at risk: x', 'payload key(s) position, roll not declared by table.reorderInitiative'] }));
  });
} catch (e) {
  failure = e;
}
expect(failure instanceof Error, 'an undeclared key fails the test');
const lines = failure.message.split('\n');
expect(lines[0] === 'payload off the manifest — fix the payload, this is not a rule contest: payload key(s) conditions not declared by table.setTokenCondition', 'the first line, what the rules gate shows: ' + lines[0]);
expect(lines.length === 4, 'each warning once, then what to do: ' + failure.message);
expect(lines[1] === '  - conditions sent to table.setTokenCondition: its data-capability.json (crates/*/data-capabilities/*/, the one whose dataCapability is table.setTokenCondition) does not declare it', lines[1]);
expect(lines[2].startsWith('  - position, roll sent to table.reorderInitiative:'), lines[2]);
expect(lines[3].includes('by its owner'), lines[3]);
expect(offManifest(['payload key(s) a not declared by b.c']).split('\n').length === 3, 'one warning, three lines');

// The body's last gesture is a command nothing awaits: still on its way to
// the route when the body returns, answered after.
const late = context('late');
let lateFailure;
try {
  await payloads({ context: late, browser: browser() }, async () => {
    void late.send('POST', 'http://app/commands', reply(warned('payload key(s) token not declared by table.moveToken'), 30), 10);
  });
} catch (e) {
  lateFailure = e;
}
expect(lateFailure instanceof Error && lateFailure.message.includes('payload key(s) token not declared by table.moveToken'), 'an answer after the body still fails it: ' + lateFailure);

// A context the test opens itself, and closes while an answer is on its way.
log.length = 0;
const first = context('first');
const second = browser();
const secondOriginal = second.newContext;
let secondFailure;
try {
  await payloads({ context: first, browser: second }, async () => {
    const other = await second.newContext();
    expect(other.routes() === 1, 'a context the test opens is watched');
    void other.send('POST', 'http://app/commands', reply(warned('payload key(s) conditions not declared by table.resetBattlemap'), 30), 10);
    await other.close();
  });
} catch (e) {
  secondFailure = e;
}
expect(secondFailure instanceof Error && secondFailure.message.includes('not declared by table.resetBattlemap'), 'a warning in a second context fails the test: ' + secondFailure);
expect(log.join() === 'answered,close made', 'closing a context waits for the answer it is reading: ' + log.join());
expect(second.newContext === secondOriginal && second.made[0].routes() === 0, 'browser.newContext is the browser\'s again');

// A page gone before the kit could ask it: the answers being read are
// still awaited.
const gone = context('gone');
let goneFailure;
try {
  await payloads({ context: gone, browser: browser() }, async () => {
    void gone.send('POST', 'http://app/commands', reply(warned('payload key(s) seal not declared by table.sealMap'), 30));
    await sleep(5);
    gone.page.gone = true;
    gone.pageGone = true;
  });
} catch (e) {
  goneFailure = e;
}
expect(goneFailure instanceof Error && goneFailure.message.includes('not declared by table.sealMap'), 'an answer read while the page went still fails the test: ' + goneFailure);

// A page the test closes right after its last gesture.
log.length = 0;
const closing = context('closing');
let closingFailure;
try {
  await payloads({ context: closing, browser: browser() }, async () => {
    void closing.send('POST', 'http://app/commands', reply(warned('payload key(s) roll not declared by table.rollInitiative'), 20), 10);
    await closing.page.close();
    closing.pageGone = true;
  });
} catch (e) {
  closingFailure = e;
}
expect(closingFailure instanceof Error && closingFailure.message.includes('not declared by table.rollInitiative'), 'a page closed first still fails the test: ' + closingFailure);
expect(log.join() === 'answered,page close closing', 'closing a page waits for its commands: ' + log.join());

// The init script, in a page of its own: it counts the POST /commands of
// fetch and of XMLHttpRequest until each settles, and nothing else.
const { runInNewContext } = await import('node:vm');
const settles = [];
const later = () => new Promise((resolve, reject) => settles.push([resolve, reject]));
class Xhr {
  constructor() { this.listeners = []; }
  addEventListener(event, l) { if (event === 'loadend') this.listeners.push(l); }
}
Xhr.prototype.open = function () {};
Xhr.prototype.send = function () {};
const sandbox = { location: { href: 'http://app/screens/x' }, URL, Request: class {}, fetch: later, XMLHttpRequest: Xhr };
sandbox.window = sandbox;
runInNewContext(IN_FLIGHT, sandbox);
runInNewContext(IN_FLIGHT, sandbox);
const count = () => sandbox.__kitCommandsInFlight;
expect(count() === 0, 'installed once, from zero');
const posted = sandbox.fetch('/commands', { method: 'post' });
void sandbox.fetch('/commands');
void sandbox.fetch('/api/commands/c1/confirm', { method: 'POST' });
void sandbox.fetch('http://[bad', { method: 'POST' });
expect(count() === 1, 'only a POST /commands is counted: ' + count());
const failed = sandbox.fetch('http://other/x/commands', { method: 'POST' });
failed.catch(() => undefined);
expect(count() === 2, 'wherever it is mounted: ' + count());
settles[0][0]('answered');
expect(await posted === 'answered', 'the page gets its answer');
settles[4][1](new Error('aborted'));
await failed.catch(() => undefined);
await sleep(0);
expect(count() === 0, 'a settled or failed command is no longer in flight: ' + count());
const xhr = new sandbox.XMLHttpRequest();
xhr.open('POST', '/commands');
xhr.send('{}');
const read = new sandbox.XMLHttpRequest();
read.open('GET', '/commands');
read.send();
expect(count() === 1 && read.listeners.length === 0, 'an XHR POST /commands is counted, a GET is not: ' + count());
xhr.listeners.forEach((l) => l());
await sleep(0);
expect(count() === 0, 'until its loadend: ' + count());
console.log('ok');
";

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

/// Whether `node` strips TypeScript with the flag (node 22.6 and later).
fn strips_types() -> bool {
    Command::new("node")
        .args([
            "--experimental-strip-types",
            "-e",
            "process.exit(process.features.typescript ? 0 : 1)",
        ])
        .output()
        .is_ok_and(|out| out.status.success())
}

#[test]
fn a_journey_fails_on_the_engine_s_undeclared_key_warning_and_only_on_it() {
    if !strips_types() {
        // CI pins node (`.github/workflows/ci.yml`): a missing runtime there
        // would make this gate vacuous, so it is red, never skipped.
        assert!(
            std::env::var_os("CI").is_none(),
            "journey_kit: no node on the path strips TypeScript (node 22.6 or later), and CI is set"
        );
        eprintln!("journey_kit: no node on the path strips TypeScript, skipped");
        return;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let dir = Dir(std::env::temp_dir().join(format!(
        "harness-journey-kit-{}-{stamp}",
        std::process::id()
    )));
    let root = &dir.0;
    write(root, "package.json", r#"{"type":"module"}"#);
    write(
        root,
        "node_modules/@playwright/test/package.json",
        r#"{"name":"@playwright/test","type":"module","exports":"./index.js"}"#,
    );
    write(root, "node_modules/@playwright/test/index.js", PLAYWRIGHT);
    write(root, "tests/rules/journey/kit.ts", KIT);
    write(root, "tests/rules/journey/driver.mjs", DRIVER);
    let output = Command::new("node")
        .arg("--experimental-strip-types")
        .arg("--no-warnings")
        .arg(root.join("tests/rules/journey/driver.mjs"))
        .output()
        .expect("node runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("ok"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
}

/// The kit and the mock quote the engine's words: a change to the
/// warning in `crates/dataguard` that they do not follow is red here.
#[test]
fn the_kit_and_the_mock_quote_the_engine_s_warning_word_for_word() {
    assert!(
        ENGINE.contains(r#""payload key(s) {} not declared by {}","#)
            && ENGINE.contains(r#"undeclared.join(", "),"#),
        "the engine's undeclared-key warning moved: follow it in kit.ts and server.js"
    );
    assert!(
        KIT.contains(r"/^payload key\(s\) (.+) not declared by (\S+)$/"),
        "kit.ts reads the engine's words"
    );
    assert!(
        SERVER.contains("`payload key(s) ${keys.join(', ')} not declared by ${dataCapability}`"),
        "server.js says the engine's words"
    );
}
