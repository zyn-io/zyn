// Nap steps over a nonce another device already used. It recognises that by
// the chain's refusal text, so every refusal that means "taken" must match —
// read here from the VM's own source, so a new wording cannot slip past.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { NONCE_TAKEN } from '../src/ledger.js';

const vm = (f) => readFile(new URL(`../../../objectvm/src/${f}`, import.meta.url), 'utf8');

test('every "taken" refusal in the VM is one Nap steps over', async () => {
  const src = (await Promise.all(['lib.rs', 'market.rs', 'curve.rs', 'primary.rs'].map((f) => vm(f).catch(() => '')))).join('\n');
  const taken = [...src.matchAll(/(?:Refused|Err)\("([^"]*(?:nonce (?:was already used|is in use)|token exists)[^"]*)"\)/g)].map((m) => m[1]);
  assert.ok(taken.length >= 3, `found ${taken.join(' | ')}`);
  for (const msg of taken) assert.match(msg, NONCE_TAKEN, msg);
});

test('a refusal that is not about the nonce is not stepped over', () => {
  for (const msg of ['an output needs a nonce', 'a mint needs a nonce', 'the price changed', 'not enough to cover that']) {
    assert.doesNotMatch(msg, NONCE_TAKEN, msg);
  }
});
