import assert from 'node:assert/strict';
import { test } from 'node:test';
import { answer, find, keep, open, query, question } from '../epeaky-librarian.mjs';

const rec = (room, seq, text, extra = {}) => ({ room, seq, id: `i${seq}`, at: '2026-10-14T10:00:00.000Z', kind: 'member', from: 'm-1', nick: 'Sam', text, ...extra });

test('it keeps words, replaces edits and drops deletions', () => {
  const db = open(':memory:');
  keep(db, rec('general', 1, 'we deploy on friday'));
  keep(db, rec('general', 2, 'lunch at noon'));
  assert.equal(find(db, 'deploy', 'general').length, 1);
  keep(db, rec('general', 1, 'we deploy on monday', { edited: true }));
  assert.match(find(db, 'deploy', 'general')[0].text, /monday/);
  assert.equal(find(db, 'friday', 'general').length, 0, 'the edit replaced the old words');
  keep(db, rec('general', 1, '', { deleted: true }));
  assert.equal(find(db, 'deploy', 'general').length, 0, 'a deletion is forgotten');
});

test('answers stay in their room unless asked for every room', () => {
  const db = open(':memory:');
  keep(db, rec('general', 1, 'the budget is fine'));
  keep(db, rec('p-0123456789abcdef0123456789abcdef', 5, 'the budget secret plan'));
  assert.equal(find(db, 'budget', 'general').length, 1);
  assert.equal(find(db, 'budget').length, 2);
});

test('a question cannot carry query operators', () => {
  assert.equal(query('deploy OR NEAR(x) "a" * -b'), '"deploy" "or" "near" "x" "a" "b"');
  const db = open(':memory:');
  keep(db, rec('general', 1, 'deploy now'));
  assert.doesNotThrow(() => find(db, 'deploy) OR (1', 'general'));
});

test('the name and "everywhere" come off the question', () => {
  assert.deepEqual(question('@librarian deploy friday', ['librarian']), { words: 'deploy friday', everywhere: false });
  assert.deepEqual(question('librarian: everywhere budget', ['m-ab12', 'librarian']), { words: 'budget', everywhere: true });
  assert.match(answer([], 'x', false), /Nothing I hold matches "x" in this room/);
});

test('every word first, then the most words', () => {
  const db = open(':memory:');
  keep(db, rec('general', 1, 'the vault keys rotate in november'));
  keep(db, rec('general', 2, 'the vault is closed'));
  assert.equal(find(db, 'vault keys', 'general').length, 1, 'both words: only the one with both');
  assert.match(find(db, 'vault keys please', 'general')[0].text, /november/, 'a filler word does not sink the best match');
  assert.equal(find(db, 'zebra', 'general').length, 0);
});

test('the question is not its own answer; questions and its answers are not kept', () => {
  const db = open(':memory:');
  keep(db, rec('ops', 10, 'the vault keys rotate in november'));
  keep(db, rec('general', 19, '@librarian vault', { mention: true }));
  keep(db, rec('general', 20, 'vault please'));
  keep(db, rec('general', 21, 'ops 10 · vault keys', { kind: 'self' }));
  const hits = find(db, 'vault please', null, { not: { room: 'general', seq: 20 } });
  assert.deepEqual(hits.map((h) => `${h.room} ${h.seq}`), ['ops 10']);
});

test('answers mention nobody, and "everywhere" is answered only in a room of two', async () => {
  const { quiet, everywhereHere, limiter } = await import('../epeaky-librarian.mjs');
  const db = open(':memory:');
  keep(db, rec('general', 1, 'deploy is @scout and @helper', { nick: '@ops' }));
  const text = answer(find(db, 'deploy', 'general'), '@helper deploy', false);
  assert.ok(!/@[^⁠]/.test(text), 'every @ is followed by a word joiner');
  assert.equal(quiet('@a@b'), '@⁠a@⁠b');
  assert.equal(everywhereHere({ kind: 'private', members: 2 }), true);
  assert.equal(everywhereHere({ kind: 'private', members: 3 }), false);
  assert.equal(everywhereHere({ kind: 'open', members: 2 }), false);
  assert.equal(everywhereHere(undefined), false);
  const ok = limiter(2, 1000);
  assert.deepEqual([ok('a', 0), ok('a', 1), ok('a', 2), ok('b', 2), ok('a', 1001)], [true, true, false, true, true]);
});

test('rooms it is no longer in are not searched', () => {
  const db = open(':memory:');
  keep(db, rec('general', 1, 'the budget is fine'));
  keep(db, rec('p-left', 2, 'the budget secret'));
  assert.deepEqual(find(db, 'budget', null, { rooms: ['general'] }).map((h) => h.room), ['general']);
  assert.equal(find(db, 'budget', null, { rooms: [] }).length, 0);
});
