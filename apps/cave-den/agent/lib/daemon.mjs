// The agent's background process and the way the commands reach it: one JSON
// request a line over a Unix socket in the agent's folder, one JSON answer a
// line back (or, for watch, a line per message until the command goes).
// Only this process touches the agent's MLS state.
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createWriteStream, lstatSync, mkdirSync } from 'node:fs';
import { link, mkdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { connect, createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Agent } from './agent.mjs';

/// The socket's path: in the agent's folder, or, where that path is too long
/// for a socket, in a folder of this user's own (XDG_RUNTIME_DIR, or one made
/// 0700 in the temporary folder and checked to be theirs) under a hash of it.
export function socketPath(dir) {
  const p = join(dir, 'agent.sock');
  if (p.length < 100) return p;
  const uid = process.getuid?.() ?? 0;
  const base = process.env.XDG_RUNTIME_DIR || join(tmpdir(), `epeaky-agent-${uid}`);
  try { mkdirSync(base, { recursive: true, mode: 0o700 }); } catch { /* there */ }
  const st = lstatSync(base);
  if (!st.isDirectory() || st.uid !== uid || (st.mode & 0o077)) throw new Error(`${base} is not a folder of this user's alone; set XDG_RUNTIME_DIR, or give the agent a shorter folder`);
  return join(base, `${createHash('sha256').update(dir).digest('hex').slice(0, 16)}.sock`);
}

/// One process per agent: a lock file named for it, made only if there is
/// none (link fails if it exists), and taken over only from a process that
/// is gone.
async function lock(dir) {
  const file = join(dir, 'agent.lock');
  const mine = `${file}.${process.pid}`;
  await writeFile(mine, String(process.pid), { mode: 0o600 });
  try {
    for (let tries = 0; tries < 2; tries++) {
      try { await link(mine, file); return () => rm(file, { force: true }); } catch (e) { if (e.code !== 'EEXIST') throw e; }
      const pid = Number(await readFile(file, 'utf8').catch(() => '0'));
      let living = false;
      try { process.kill(pid, 0); living = pid > 0; } catch (e) { living = e.code === 'EPERM'; }
      if (living) throw new Error('This agent is already running.');
      await rm(file, { force: true });
    }
    throw new Error('This agent is already running.');
  } finally {
    await rm(mine, { force: true });
  }
}

/// Run the agent here, in the foreground, until told to stop.
export async function serve(dir) {
  await mkdir(dir, { recursive: true, mode: 0o700 });
  const path = socketPath(dir);
  const unlock = await lock(dir);
  await rm(path, { force: true });
  const logFile = createWriteStream(join(dir, 'agent.log'), { flags: 'a', mode: 0o600 });
  const log = (t) => logFile.write(`${new Date().toISOString()} ${t}\n`);
  const agent = new Agent(dir, log);
  try { await agent.run(); } catch (e) { log(`could not start: ${e.message}`); await new Promise((r) => logFile.end(r)); await unlock(); throw e; }
  log(`up as ${agent.me.name}`);
  const server = createServer((sock) => talk(agent, sock, () => stop()));
  // The socket is this user's alone, wherever it lies.
  process.umask(0o077);
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(path, resolve); });
  let stopping = false;
  async function stop() {
    if (stopping) return;
    stopping = true;
    log('down');
    server.close();
    await agent.den.save().catch(() => {});
    await rm(path, { force: true });
    await unlock();
    process.exit(0);
  }
  process.on('SIGTERM', stop);
  process.on('SIGINT', stop);
  // In again, as at the start of a day (a new session if this one is ending).
  process.on('SIGHUP', () => agent.newDay());
  return agent;
}

function talk(agent, sock, stop) {
  let buf = '';
  // A reader that falls far behind is let go rather than held in memory.
  const send = (o) => { if (sock.destroyed) return; sock.write(`${JSON.stringify(o)}\n`); if (sock.writableLength > 4 << 20) sock.destroy(); };
  sock.on('error', () => {});
  sock.on('data', async (chunk) => {
    buf += chunk;
    if (buf.length > 1 << 20) return sock.destroy();
    const nl = buf.indexOf('\n');
    if (nl < 0) return;
    const line = buf.slice(0, nl);
    buf = '';
    let req;
    try { req = JSON.parse(line); } catch { send({ ok: false, error: 'not a request' }); return sock.end(); }
    try {
      if (req.cmd === 'watch') return watch(agent, sock, req.args || {}, send);
      send({ ok: true, data: await answer(agent, req.cmd, req.args || {}) });
      sock.end();
      if (req.cmd === 'down') setTimeout(stop, 50);
    } catch (e) {
      send({ ok: false, error: e.message, code: e.code || null });
      sock.end();
    }
  });
}

async function answer(agent, cmd, a) {
  if (cmd !== 'status' && cmd !== 'down') await agent.settled();
  switch (cmd) {
    case 'status': return agent.status();
    case 'rooms': return agent.rooms();
    case 'read': return agent.read(a.room, a);
    case 'send': return agent.send(a.room, a.text, a.reply);
    case 'search': return agent.search(a.q);
    case 'next': return agent.next(a);
    case 'pins': return agent.pins(a.room);
    case 'owner':
      if (a.clear) { await agent.clearOwner(); return null; }
      if (a.code) return agent.own(a.code);
      return agent.ownerOf();
    case 'down': return null;
    default: throw new Error(`no command ${cmd}`);
  }
}

/// New messages as they arrive, each a line, until the command goes; with
/// once, only the first that matches.
async function watch(agent, sock, a, send) {
  await agent.settled();
  const room = a.room ? agent.room(a.room).id : null;
  send({ ok: true, data: 'watching' });
  const f = (r) => {
    if (room && r.room !== room) return;
    if (a.mentions && !r.mention) return;
    if (a.orders && r.authority !== 'owner') return;
    send({ message: r });
    if (a.once) { agent.listeners.delete(f); sock.end(); }
  };
  agent.listeners.add(f);
  sock.on('close', () => agent.listeners.delete(f));
}

export async function alive(path) {
  return new Promise((resolve) => {
    const s = connect(path);
    s.once('connect', () => { s.end(); resolve(true); });
    s.once('error', () => resolve(false));
  });
}

/// The background process for `dir`, started if it is not running.
export async function ensure(dir, bin) {
  const path = socketPath(dir);
  if (await alive(path)) return path;
  try { await stat(join(dir, 'agent.json')); } catch { throw Object.assign(new Error('No agent here yet: epeaky-agent join <invite link>'), { code: 'none' }); }
  const child = spawn(process.execPath, [bin, 'up', '--dir', dir], { detached: true, stdio: 'ignore', env: process.env });
  child.unref();
  const end = Date.now() + 30_000;
  while (Date.now() < end) {
    await new Promise((r) => setTimeout(r, 150));
    if (await alive(path)) return path;
    if (child.exitCode !== null) break;
  }
  throw Object.assign(new Error(`The agent's background process did not start; see ${join(dir, 'agent.log')}`), { code: 'start' });
}

/// One request to the background process: its answer, or, for watch, each
/// line to `each` until the process or the caller ends it.
export function request(path, cmd, args, each = null) {
  return new Promise((resolve, reject) => {
    const s = connect(path);
    let buf = '';
    let first = true;
    s.on('connect', () => s.write(`${JSON.stringify({ cmd, args })}\n`));
    s.on('data', (chunk) => {
      buf += chunk;
      let nl;
      while ((nl = buf.indexOf('\n')) >= 0) {
        const o = JSON.parse(buf.slice(0, nl));
        buf = buf.slice(nl + 1);
        if (first) {
          first = false;
          if (!o.ok) return reject(Object.assign(new Error(o.error), { code: o.code }));
          if (!each) return resolve(o.data);
        } else if (each && o.message) each(o.message);
      }
    });
    s.on('error', reject);
    s.on('close', () => resolve(null));
  });
}
