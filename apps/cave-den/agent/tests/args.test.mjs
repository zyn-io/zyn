// The command line's flags: a flag that takes a value never takes the next
// flag, nor nothing.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { parse } from '../lib/args.mjs';

test('a value after its flag, or after =, kept whole', () => {
  assert.deepEqual(parse(['send', 'general', '--reply', '42', 'hi']), { flags: { reply: '42' }, pos: ['send', 'general', 'hi'] });
  assert.deepEqual(parse(['join', 'x', '--nick=a=b']).flags, { nick: 'a=b' });
  assert.deepEqual(parse(['read', 'general', '--new', '--json']).flags, { new: true, json: true });
});

test('a flag missing its value stops the command', () => {
  assert.throws(() => parse(['status', '--agent']), /--agent needs a value/);
  assert.throws(() => parse(['send', 'general', '--reply', '--for', '7', 'hi']), /--reply needs a value/);
});

test('after -- everything is words', () => {
  assert.deepEqual(parse(['send', 'general', '--', '--agent', 'x']), { flags: {}, pos: ['send', 'general', '--agent', 'x'] });
});
