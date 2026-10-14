// What an agent keeps, as files in its own folder: one file a key, written
// whole, flushed and renamed into place, so a crash never leaves half a value.
// Writes to one key go one after another, in the order they were asked for,
// so the last one asked is the one on disk. The same get, put, del and wipe
// as the page's IndexedDB store.
import { randomBytes } from 'node:crypto';
import { mkdir, open, readFile, rename, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { deserialize, serialize } from 'node:v8';

export function fileStore(dir) {
  const made = mkdir(dir, { recursive: true, mode: 0o700 });
  const file = (k) => join(dir, encodeURIComponent(k));
  const queues = new Map();
  /// Run `f` after every earlier write to `k`.
  const inTurn = (k, f) => {
    const run = (queues.get(k) || made).then(f, f);
    const tail = run.catch(() => {});
    queues.set(k, tail);
    tail.then(() => { if (queues.get(k) === tail) queues.delete(k); });
    return run;
  };
  const write = async (k, v) => {
    const f = file(k);
    const tmp = `${f}.${process.pid}.${randomBytes(6).toString('hex')}.tmp`;
    const h = await open(tmp, 'w', 0o600);
    try { await h.writeFile(serialize(v)); await h.sync(); } finally { await h.close(); }
    // Windows refuses a rename over a file another process has open for a
    // moment (a reader, an indexer): try again a few times.
    for (let i = 0; ; i++) {
      try { await rename(tmp, f); break; } catch (e) {
        if (i < 5 && ['EPERM', 'EACCES', 'EBUSY'].includes(e.code)) { await new Promise((r) => setTimeout(r, 20 * (i + 1))); continue; }
        await rm(tmp, { force: true });
        throw e;
      }
    }
  };
  return {
    get: async (k) => {
      await (queues.get(k) || made);
      try { return deserialize(await readFile(file(k))); } catch (e) { if (e.code === 'ENOENT') return undefined; throw e; }
    },
    put: (k, v) => inTurn(k, () => write(k, v)),
    del: (k) => inTurn(k, () => rm(file(k), { force: true })),
    wipe: async () => { await Promise.all([...queues.values()]); await rm(dir, { recursive: true, force: true }); },
  };
}
