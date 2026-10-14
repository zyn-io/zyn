// The background process's plumbing, on every system (CI runs this on
// Windows too): its socket or named pipe, its lock, its store.
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { alive, lock, socketPath } from '../lib/daemon.mjs';
import { fileStore } from '../lib/store.mjs';

test('Windows: a named pipe of this user and folder', () => {
  const a = socketPath('C:\\Users\\me\\.epeaky\\agents\\research', 'win32');
  assert.match(a, /^\\\\\.\\pipe\\epeaky-agent-[0-9a-f]{24}$/);
  assert.equal(socketPath('c:\\users\\me\\.epeaky\\agents\\research', 'win32'), a);
  assert.notEqual(socketPath('C:\\Users\\me\\.epeaky\\agents\\scout', 'win32'), a);
});

test('the socket (or pipe) answers while the process listens', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'ead-'));
  const path = socketPath(dir);
  const server = createServer((s) => s.end());
  await new Promise((r) => server.listen(path, r));
  assert.equal(await alive(path), true);
  await new Promise((r) => server.close(r));
  assert.equal(await alive(path), false);
  await rm(dir, { recursive: true, force: true });
});

test('one process an agent: the lock', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'eal-'));
  const unlock = await lock(dir);
  await assert.rejects(lock(dir), /already running/);
  await unlock();
  const again = await lock(dir);
  await again();
  await rm(dir, { recursive: true, force: true });
});

test('the store: writes to one key land in order, the last one kept', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'eas-'));
  const kv = fileStore(dir);
  await Promise.all(Array.from({ length: 30 }, (_, i) => kv.put('k', { i })));
  assert.deepEqual(await kv.get('k'), { i: 29 });
  await kv.del('k');
  assert.equal(await kv.get('k'), undefined);
  await kv.wipe();
});
