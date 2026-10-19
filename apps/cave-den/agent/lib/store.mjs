// What an agent keeps, as files in its own folder: one file a key, written
// whole, flushed and renamed into place, so a crash never leaves half a value.
// Writes to one key go one after another, in the order they were asked for,
// so the last one asked is the one on disk. The same get, put, del and wipe
// as the page's IndexedDB store.
//
// A value that cannot be read stops what needs it, naming its file, unless
// it is one the agent can make again (`remake`: its cursors and journal):
// that one is put aside as NAME.corrupt and starts again empty. Half-written
// files left by a process that is gone are cleared when the store opens.
import { randomBytes } from 'node:crypto';
import { mkdir, open, readdir, readFile, rename, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { deserialize, serialize } from 'node:v8';

const living = (pid) => {
  if (pid === process.pid) return true;
  try { process.kill(pid, 0); return true; } catch (e) { return e.code === 'EPERM'; }
};

export function fileStore(dir, { remake = [] } = {}) {
  const made = mkdir(dir, { recursive: true, mode: 0o700 }).then(async () => {
    for (const n of await readdir(dir)) {
      const m = /\.(\d+)\.[0-9a-f]{12}\.tmp$/.exec(n);
      if (m && !living(Number(m[1]))) await rm(join(dir, n), { force: true });
    }
  });
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
    // The rename itself, on disk (Windows cannot open a folder: its rename is).
    try { const d = await open(dir, 'r'); try { await d.sync(); } finally { await d.close(); } } catch { /* Windows */ }
  };
  const read = async (k) => {
    let raw;
    try { raw = await readFile(file(k)); } catch (e) { if (e.code === 'ENOENT') return undefined; throw e; }
    try { return deserialize(raw); } catch {
      if (!remake.includes(k)) throw Object.assign(new Error(`${file(k)} cannot be read; move it from the agent's folder only if you know what it held`), { code: 'corrupt' });
      await rename(file(k), `${file(k)}.corrupt`).catch(() => {});
      return undefined;
    }
  };
  return {
    get: async (k) => {
      await (queues.get(k) || made);
      return remake.includes(k) ? inTurn(k, () => read(k)) : read(k);
    },
    put: (k, v) => inTurn(k, () => write(k, v)),
    del: (k) => inTurn(k, () => rm(file(k), { force: true })),
    wipe: async () => { await Promise.all([...queues.values()]); await rm(dir, { recursive: true, force: true }); },
  };
}
