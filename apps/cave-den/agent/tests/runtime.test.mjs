import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileStore } from '../lib/store.mjs';

// The den's own client (built into runtime/ by build.sh), in Node.
test("the den's client starts outside a browser, from files, and comes back as the same device", async () => {
  const { DenClient } = await import('../runtime/den.js');
  const wasm = await readFile(new URL('../runtime/den/den_mls_bg.wasm', import.meta.url));
  const dir = await mkdtemp(join(tmpdir(), 'ea-'));
  try {
    const first = new DenClient({ base: 'http://localhost:0', keysUrl: 'http://localhost:0/door/keys', kv: fileStore(dir), wasm });
    const me = await first.start();
    assert.match(me.name, /^m-[0-9a-f]{20}$/);
    await first.becomeAgent();
    assert.ok(first.isAgent(me.key) && first.profile().agent === true, 'an agent says it is one, with no owner yet');
    const again = new DenClient({ base: 'http://localhost:0', keysUrl: 'http://localhost:0/door/keys', kv: fileStore(dir), wasm });
    assert.equal((await again.start()).key, me.key);
    assert.ok(again.agent, 'and still is one');
    assert.equal(again.agentOwner(me.key), null);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test('an owner counts only when its side names the agent too', async () => {
  const { DenClient } = await import('../runtime/den.js');
  const wasm = await readFile(new URL('../runtime/den/den_mls_bg.wasm', import.meta.url));
  const dir = await mkdtemp(join(tmpdir(), 'ea-'));
  try {
    const d = new DenClient({ base: 'http://localhost:0', keysUrl: 'x', kv: fileStore(dir), wasm });
    const me = await d.start();
    await d.becomeAgent();
    const owner = 'aa'.repeat(32);
    const phone = 'bb'.repeat(32);
    const other = 'cc'.repeat(32);
    d.agent.owner = owner;
    assert.ok(!d.fromOwner(owner), 'named by the agent alone: no authority');
    await d.learnAgent(owner, undefined, [me.key]);
    assert.ok(d.fromOwner(owner), 'both sides: the owner commands');
    assert.ok(!d.fromOwner(phone), "a device that only says it is the owner's: no");
    await d.learnClaims(phone, [owner]);
    assert.ok(!d.fromOwner(phone), 'one side of a link is not a link');
    await d.learnClaims(owner, [phone]);
    assert.ok(d.fromOwner(phone), "the owner's linked device commands too");
    await d.learnAgent(other, owner, undefined);
    assert.equal(d.agentOwner(other), null, 'another agent naming the owner is not confirmed by it');
    await d.learnAgent(owner, undefined, [], Date.now());
    assert.ok(!d.fromOwner(owner), 'once the owner takes it back, no authority');
    await d.learnAgent(owner, undefined, [me.key], Date.now() - 60_000);
    assert.ok(!d.fromOwner(owner), 'an older profile read late does not bring it back');
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
