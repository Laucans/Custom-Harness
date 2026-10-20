//! `mock/src/server.js` against node: the mock behaves as the DataGuard
//! engine does — a DataCapability nobody handles is rejected, a value
//! declared ahead asks its author, a confirmed command applies, a parked
//! one comes back at the end, a critical field waits for its approver, an
//! invariant of the aggregate rejects on the payload, a field that merges is
//! never asked about, a derived field is refused. Skipped, loudly, when
//! `node` is not on the path.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SERVER: &str = include_str!("../assets/mock/src/server.js");
const READS: &str = include_str!("../assets/mock/src/reads.js");

const COMMANDS: &str = r"
export const COMMANDS = {
  'credit.requestLimitChange': async () => undefined,
  'credit.setStatus': async () => undefined,
  'credit.tag': async () => undefined,
  'credit.addPayment': async (server, { payload }) => (payload.amount > 0 ? undefined : ['amount-positive']),
};
";

const ACCOUNT: &str = r#"{"aggregate":"Account","fields":{"creditLimit":{"kind":"authoritative","criticality":"normal"},"status":{"kind":"authoritative","criticality":"critical","review":"role:credit-manager"},"tags":{"kind":"authoritative","criticality":"low","merge":"set"},"balance":{"kind":"derived","criticality":"normal"}},"invariants":[{"id":"limit-under-ceiling","rule":"creditLimit <= 10000","onViolation":"reject"}]}"#;

const SHIMS: &str = r"
import fs from 'node:fs/promises';
globalThis.window = globalThis;
globalThis.location = { href: new URL('./', import.meta.url).href };
globalThis.fetch = async (url) => {
  try {
    const text = await fs.readFile(new URL(url), 'utf8');
    return { ok: true, json: async () => JSON.parse(text) };
  } catch {
    return { ok: false };
  }
};
";

const DRIVER: &str = r#"
import './shims.mjs';
import { mockServer, ruleHolds } from './server.js';

const s = mockServer();
const call = async (path, body, method = 'POST') => {
  const r = await s.fetch(`http://mock${path}`, { method, body: body === undefined ? undefined : JSON.stringify(body) });
  return { status: r.status, body: await r.json() };
};
const expect = (cond, what) => { if (!cond) { console.error('FAILED: ' + what); process.exit(1); } };
const limit = (by, creditLimit, read) => ({ dataCapability: 'credit.requestLimitChange@2', payload: { creditLimit }, target: { aggregate: 'Account', id: '881' }, basedOn: { version: 1, values: { creditLimit: read } }, caller: { by } });

expect(ruleHolds('creditLimit <= 10000', { creditLimit: 5 }) && !ruleHolds('creditLimit <= 10000', { creditLimit: 20000 }), 'rules');
expect(ruleHolds('creditLimit <= 10000', {}), 'an absent field is not a violation');

const unhandled = await call('/commands', { dataCapability: 'orders.nothing', payload: {}, caller: { by: 'x' } });
expect(unhandled.status === 202 && unhandled.body.result.status === 'rejected' && unhandled.body.result.violations[0] === 'unhandled-data-capability', 'unhandled is rejected: ' + JSON.stringify(unhandled));

const alice = await call('/commands', limit('alice', 8000, 5000));
expect(alice.body.result.status === 'applied' && alice.body.result.dataVersion === 2, 'alice applied: ' + JSON.stringify(alice));
expect(alice.body.partition === 'Account/881', 'partition');
const looked = await call('/commands/' + alice.body.commandId, undefined, 'GET');
expect(looked.status === 200 && looked.body.result.status === 'applied', 'GET /commands/<id>');

const bob = await call('/commands', limit('bob', 3000, 5000));
expect(bob.body.result.status === 'awaiting_confirmation', 'bob is asked: ' + JSON.stringify(bob));
expect(bob.body.warnings.some((w) => w.includes('declared ahead')), 'warned');
const declared = s.messages.find((m) => m.message === 'ValueDeclaredAhead' && m.commandId === bob.body.commandId);
expect(declared && declared.declaredAhead.value === 8000 && declared.declaredAhead.by === '(applied)', 'the message names what moved');
const mallory = await call('/commands/' + bob.body.commandId + '/confirm', { by: 'mallory' });
expect(mallory.status === 409, 'someone else may not confirm');
const confirmed = await call('/commands/' + bob.body.commandId + '/confirm', { by: 'bob' });
expect(confirmed.body.result.status === 'applied' && confirmed.body.result.dataVersion === 3, 'bob confirmed and applied: ' + JSON.stringify(confirmed));

const carol = await call('/commands', limit('carol', 4500, 5000));
expect(carol.body.result.status === 'awaiting_confirmation', 'carol is asked');
s.parkAfterMs = 0;
await s.drive();
const parked = await call('/commands/' + carol.body.commandId, undefined, 'GET');
expect(parked.body.result.status === 'parked', 'unconfirmed at the head: parked, got ' + parked.body.result.status);
s.parkAfterMs = 30000;
const again = await call('/commands/' + carol.body.commandId + '/requeue', { by: 'carol' });
expect(again.body.commandId === carol.body.commandId + '-r' && again.body.result.status === 'awaiting_confirmation', 'requeued, asked again: ' + JSON.stringify(again));
const cancelled = await call('/commands/' + again.body.commandId + '/cancel', { by: 'carol' });
expect(cancelled.body.result.status === 'cancelled', 'cancelled');

const over = await call('/commands', limit('dave', 20000, 3000));
expect(over.body.result.status === 'rejected' && over.body.result.violations[0] === 'limit-under-ceiling', 'the invariant rejects: ' + JSON.stringify(over));
expect(over.body.warnings[0].includes('limit-under-ceiling'), 'and warned at enqueue');

const close = await call('/commands', { dataCapability: 'credit.setStatus', payload: { status: 'closed' }, target: { aggregate: 'Account', id: '881' }, caller: { by: 'erin' } });
expect(close.body.result.status === 'awaiting_review' && close.body.result.reviewId, 'a critical field waits: ' + JSON.stringify(close));
expect(s.messages.some((m) => m.message === 'ReviewRequired' && m.to[0] === 'role:approver'), 'the approver is told');
const approved = await call('/commands/' + close.body.commandId + '/approve', { by: 'erin' });
expect(approved.body.result.status === 'applied', 'approved, applied');

const pay = { dataCapability: 'credit.addPayment', payload: { amount: 12 }, idempotencyKey: 'k1', caller: { by: 'alice' } };
const first = await call('/commands', pay);
const replay = await call('/commands', pay);
expect(first.body.result.status === 'applied' && replay.body.replayed && replay.body.commandId === first.body.commandId, 'idempotency replays');
const bad = await call('/commands', { dataCapability: 'credit.addPayment', payload: { amount: 0 }, caller: { by: 'alice' } });
expect(bad.body.result.status === 'rejected' && bad.body.result.violations[0] === 'amount-positive', 'the handler judges');

const read = await call('/capabilities/account', { variables: {} });
expect(read.status === 200 && read.body.data.id === '881' && read.body.asOf === s.dataVersion, 'a fixture read');
expect(s.messages.filter((m) => m.message === 'CommandApplied').length === 4, 'four applications told');

const tag = (by, tags, read) => ({ dataCapability: 'credit.tag@1', payload: { tags }, target: { aggregate: 'Account', id: '881' }, basedOn: { version: 1, values: { tags: read } }, caller: { by } });
const firstTag = await call('/commands', tag('alice', ['a'], []));
const movedTag = await call('/commands', tag('bob', ['b'], ['stale']));
expect(firstTag.body.result.status === 'applied' && movedTag.body.result.status === 'applied', 'a merging field is never asked: ' + JSON.stringify(movedTag));
const derived = await call('/commands', { dataCapability: 'credit.tag@1', payload: { balance: 3 }, target: { aggregate: 'Account', id: '881' }, caller: { by: 'eve' } });
expect(derived.status === 403 && derived.body.error.includes('derived field(s) balance'), 'a derived field is refused: ' + JSON.stringify(derived));
console.log('ok');
"#;

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

#[test]
fn the_mock_server_behaves_as_the_engine() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("mock_server: node is not on the path, skipped");
        return;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let dir =
        Dir(std::env::temp_dir().join(format!("harness-mock-{}-{stamp}", std::process::id())));
    let root = &dir.0;
    write(root, "src/server.js", SERVER);
    write(root, "src/reads.js", READS);
    write(root, "src/commands.js", COMMANDS);
    write(root, "src/shims.mjs", SHIMS);
    write(root, "src/driver.mjs", DRIVER);
    write(root, "fixtures/aggregates/Account.json", ACCOUNT);
    write(
        root,
        "fixtures/account.json",
        r#"{"id":"881","creditLimit":5000}"#,
    );
    let output = Command::new("node")
        .arg(root.join("src/driver.mjs"))
        .output()
        .expect("node runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("ok"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
}
