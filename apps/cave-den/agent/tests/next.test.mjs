// `next` hands each message once per filter, in each room's own order, and
// never skips one: a cursor per filter, moved only over what it returned.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Agent } from '../lib/agent.mjs';

const ME = 'a'.repeat(64), OWNER = 'b'.repeat(64), SAM = 'c'.repeat(64);
function agent(log, store = {}, { journal = false } = {}) {
  const a = Object.create(Agent.prototype);
  a.me = { key: ME, name: 'm-aaaa', nick: 'me' };
  a.listeners = new Set();
  a.visible = () => true;
  a.log = () => {};
  if (!journal) a.handedOrders = async () => {};
  a.den = {
    agent: { owner: OWNER, since: 0 },
    rules: () => null,
    fromOwner: (k) => k === OWNER,
    isAgent: () => false,
    agentOwner: (k) => (k === ME ? OWNER : null),
    nickFor: () => '',
    rooms: new Map([['general', { id: 'general' }]]),
    log: async () => log,
    kv: { get: async (k) => store[k], put: async (k, v) => { store[k] = structuredClone(v); } },
  };
  return a;
}
const msg = (seq, key, at, text = 'hi', extra = {}) => ({ t: 'msg', seq, key, at, id: String(seq).padStart(16, '0'), text, ...extra });

test('handing tasks does not move the cursor past an order that --orders has not handed', async () => {
  const log = [msg(10, OWNER, 100, 'deploy it'), msg(11, SAM, 101, 'look at this', { task: { to: ME } })];
  const a = agent(log);
  assert.deepEqual((await a.next({ tasks: true })).map((r) => r.seq), [11]);
  assert.deepEqual((await a.next({ orders: true })).map((r) => r.seq), [10], 'the order is still there');
  assert.deepEqual((await a.next({ orders: true })), [], 'and handed once');
});

test("a limit takes a room's messages in its own order, so a sender's skewed clock skips nothing", async () => {
  const log = [msg(19, OWNER, 5000, 'the order'), msg(20, SAM, 1), msg(21, SAM, 1), msg(22, SAM, 1)];
  const a = agent(log);
  assert.deepEqual((await a.next({ limit: 3 })).map((r) => r.seq), [19, 20, 21]);
  assert.deepEqual((await a.next({ limit: 3 })).map((r) => r.seq), [22]);
  const b = agent(log);
  assert.deepEqual((await b.next({ orders: true, limit: 1 })).map((r) => r.seq), [19]);
});

test('the cursor from before (one for all filters) carries over: nothing old comes again', async () => {
  const log = [msg(5, OWNER, 50, 'old order'), msg(6, OWNER, 60, 'new order')];
  const a = agent(log, { handed: { general: 5 } });
  assert.deepEqual((await a.next({ orders: true })).map((r) => r.seq), [6]);
  assert.deepEqual((await a.next({})).map((r) => r.seq), [6], 'each filter from the old cursor');
});

test('an order watch has handed is not handed again by next', async () => {
  const log = [msg(7, OWNER, 70, 'deploy it')];
  const a = agent(log, {}, { journal: true });
  await a.handedOrders([a.recordOf('general', log[0], null, log)]);
  assert.deepEqual(await a.next({ orders: true }), []);
  assert.deepEqual(await a.next({}), []);
});

test('an edit to a message not handed yet comes once: the message as it now stands', async () => {
  const log = [msg(8, SAM, 80, 'new words', { edited: true })];
  const a = agent(log);
  await a.noteChange('general', { id: log[0].id, text: 'new words' }, 'edit');
  const out = await a.next({ changes: true });
  assert.deepEqual(out.map((r) => [r.seq, r.change ?? null]), [[8, null]]);
  assert.deepEqual(await a.next({ changes: true }), [], 'and the edit is not handed after it');
});

test('a deleted task for this agent is one change, seen by --tasks and by --changes', async () => {
  const task = { to: ME, state: 'submitted', id: 't1' };
  const log = [msg(9, OWNER, 90, '', { deleted: true, task })];
  const a = agent(log, { handedBy: { t: { rooms: { general: 9 }, change: 0 }, c: { rooms: { general: 9 }, change: 0 } } });
  a.event('message', { room: 'general', kind: 'del', message: { ...log[0] }, mine: false });
  await a.changing;
  assert.equal((await a.den.kv.get('changes')).list.length, 1);
  assert.deepEqual((await a.next({ tasks: true })).map((r) => r.change), ['deleted']);
  assert.deepEqual((await a.next({ changes: true })).map((r) => r.change), ['deleted']);
});
