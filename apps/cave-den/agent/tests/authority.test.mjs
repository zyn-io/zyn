// Delegated authority: a task from another agent is an order only while that
// agent and this one have the same owner, under rules that owner set.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Agent } from '../lib/agent.mjs';

const ME = 'a'.repeat(64), OWNER = 'b'.repeat(64), BOT = 'd'.repeat(64), OTHER = 'e'.repeat(64);
function agent({ botOwner = OWNER, rules = { mode: 'all', takes_from: [BOT], files: true, by: OWNER, at: 0 } } = {}) {
  const a = Object.create(Agent.prototype);
  a.me = { key: ME, name: 'm-aaaa', nick: 'me' };
  a.den = {
    agent: { owner: OWNER, since: 0 },
    rules: () => rules,
    fromOwner: (k) => k === OWNER,
    isAgent: (k) => k === BOT,
    agentOwner: (k) => (k === ME ? OWNER : k === BOT ? botOwner : null),
    nickFor: () => '',
  };
  return a;
}
const task = { t: 'task', key: BOT, at: 1000, seq: 5, id: '1'.repeat(16), task: { id: 't1', to: ME, text: 'run the tests' } };

test("a task from an agent of the same owner, under the owner's rules, is delegated", () => {
  const r = agent().recordOf('general', task);
  assert.equal(r.authority, 'delegated');
  assert.equal(r.on_behalf_of, OWNER);
});

test('once the delegating agent belongs to someone else, its tasks are information', () => {
  const r = agent({ botOwner: OTHER }).recordOf('general', task);
  assert.notEqual(r.authority, 'delegated');
  assert.equal(agent({ botOwner: null }).recordOf('general', task).authority === 'delegated', false);
});
