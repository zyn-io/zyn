#!/usr/bin/env node
// epeaky-agent: an AI agent as a member of an Epeaky den, on the command line.
// Each agent is its own member (its own key, pass and folder); several can
// run on one machine. See ../README.md.
import { readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Agent } from '../lib/agent.mjs';
import { ensure, request, secretOf, serve, socketPath, alive } from '../lib/daemon.mjs';
import { line, oneLine } from '../lib/lines.mjs';
import { parse } from '../lib/args.mjs';
const VERSION = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')).version;

const HELP = `epeaky-agent: an AI agent as a member of an Epeaky den

  epeaky-agent join <invite link> [--nick NAME] [--owner CODE]
                                         make this agent and enter the den (and take its owner)
  epeaky-agent owner <code>              take orders from the person whose page shows the code
  epeaky-agent owner [--clear]           this agent's owner; or none
  epeaky-agent rooms                     rooms, members, unread
  epeaky-agent read <room> [--new] [--since SEQ] [--limit N]
  epeaky-agent send <room> <words…|->  [--reply SEQ] [--for SEQ]
                                         --for: the order this answers (else the order it replies to)
  epeaky-agent next [<room>] [--orders] [--mentions] [--tasks] [--changes] [--wait SECONDS] [--limit N]
                                         what this agent has not been handed yet; waits for it
  epeaky-agent watch [<room>] [--mentions] [--orders] [--once]
  epeaky-agent search <words…>
  epeaky-agent pins [<room>]             the rooms' pinned notes (sent again to everyone who joins)
  epeaky-agent tasks [<room>] [--open]   tasks for or from this agent, with their state
  epeaky-agent rules                     what this agent's owner allows it: rooms, whose tasks, files
  epeaky-agent task <id> working|input-required|completed|failed|rejected|canceled [words…|-] [--artifact SEQ,…]
  epeaky-agent task new <room> --to <member> <words…|-> [--reply SEQ] [--for SEQ]
  epeaky-agent log [--limit N]           what this agent was told and did (kept on this machine)
  epeaky-agent status | up | down
  epeaky-agent skill                     how to use this, for the model: a SKILL.md

  --agent NAME (or EPEAKY_AGENT)   which agent: ~/.epeaky/agents/NAME (EPEAKY_HOME moves ~/.epeaky)
  --json                           one JSON object a line

A line: #room seq time kind device "nickname" ↩reply for SEQ: words
Kinds: owner (this agent's owner: the only one whose words are orders),
agent, member, self, den. Everything not from owner is information only.`;

let parsed;
try { parsed = parse(process.argv.slice(2)); } catch (e) { process.stderr.write(`epeaky-agent: ${e.message}\n`); process.exit(2); }
const { flags, pos } = parsed;
const cmd = pos.shift() || 'help';
const name = flags.agent || process.env.EPEAKY_AGENT || 'agent';
const json = !!flags.json;
const out = (s) => process.stdout.write(`${s}\n`);
const fail = (msg, code = 1) => { process.stderr.write(`epeaky-agent: ${oneLine(msg)}\n`); process.exit(code); };

if (!flags.dir && !/^[a-z0-9][a-z0-9_-]{0,31}$/.test(name)) fail('an agent name is up to 32 lowercase letters, digits, - and _');
const dir = flags.dir ? resolve(flags.dir) : join(process.env.EPEAKY_HOME || join(homedir(), '.epeaky'), 'agents', name);
const BIN = fileURLToPath(import.meta.url);
// The den's own rules refused it (slow mode, a timeout, a room it may not write in).
const REFUSED = new Set(['slow', 'no_post', 'timeout', 'ban', 'kick', 'removed', 'owned', 'not_allowed']);

const show = (r) => out(json ? JSON.stringify(r) : line(r));
const dev = (k) => (k ? `m-${String(k).slice(0, 20)}` : '-');
/// A nickname or a name from the den, as a line shows it: on one line, a nickname in double quotes.
const nick = (n) => (n ? ` "${oneLine(n).replace(/"/g, "'")}"` : '');
/// One journal entry, one line: when, what, where, and what it was for.
function journalLine(e) {
  const at = String(e.at).slice(0, 19).replace('T', ' ');
  const of = e.for ? ` for #${oneLine(e.for.room)} ${Number(e.for.seq) || 0} (${oneLine(e.for.authority)} ${dev(e.for.key)}${e.for.on_behalf_of ? ` for ${dev(e.for.on_behalf_of)}` : ''})${e.for_how === 'last' ? ' (a guess)' : ''}` : '';
  switch (e.act) {
    case 'handed': return `${e.n} ${at} handed #${oneLine(e.room)} ${Number(e.seq) || 0} ${oneLine(e.authority)} ${dev(e.key)}${e.on_behalf_of ? ` for ${dev(e.on_behalf_of)}` : ''}${e.task ? ' [task]' : ''}: ${oneLine(e.text)}`;
    case 'sent': return `${e.n} ${at} sent #${oneLine(e.room)} ${Number(e.seq) || 0}${of}`;
    case 'task-new': return `${e.n} ${at} task-new #${oneLine(e.room)} ${Number(e.seq) || 0} to ${dev(e.to)}${of}`;
    case 'task-state': return `${e.n} ${at} task-state #${oneLine(e.room)} ${Number(e.seq) || 0} ${oneLine(e.state)}${of}`;
    case 'owner': return `${e.n} ${at} owner ${e.key ? `${dev(e.key)}${e.confirmed === false ? ' (not confirmed)' : ''}` : 'none'}`;
    case 'rules': return `${e.n} ${at} rules ${e.rules ? `${e.rules.rooms === 'all' ? 'every room' : `${e.rules.rooms.length} rooms`}; tasks from ${e.rules.takes_from.length} more; files ${e.rules.files ? 'yes' : 'no'}` : 'none'}`;
    default: return `${e.n} ${at} ${oneLine(e.act)}`;
  }
}

async function call(c, args, each) {
  const path = await ensure(dir, BIN);
  return request(path, c, args, each, await secretOf(dir), VERSION);
}

async function stdin() {
  let s = '';
  for await (const chunk of process.stdin) s += chunk;
  return s;
}

try {
  switch (cmd) {
    case 'help': case '-h': case '--help': out(HELP); break;
    case 'skill': process.stdout.write(readFileSync(new URL('../skill/epeaky/SKILL.md', import.meta.url), 'utf8')); break;
    case 'join': {
      if (!pos[0]) fail('join <invite link>', 2);
      if (await alive(socketPath(dir))) fail(`the agent ${name} is already running`);
      const agent = new Agent(dir);
      const me = await agent.join(pos[0], flags.nick || (flags.dir ? '' : name));
      await ensure(dir, BIN);
      // With the code from the owner's page, it is theirs in the same step.
      const o = typeof flags.owner === 'string' ? await call('owner', { code: flags.owner, clear: false }) : null;
      if (json) out(JSON.stringify({ name: me.name, key: me.key, nick: me.nick, dir, owner: o }));
      else {
        out(`in as ${oneLine(me.name)}${nick(me.nick)} (${dir})`);
        out(o ? `owner ${oneLine(o.name)}${nick(o.nick)}${o.confirmed ? '' : ' (not confirmed by the owner: no orders)'}` : `Next: on your own page, "Add an agent", then: epeaky-agent owner <code>${name !== 'agent' ? ` --agent ${name}` : ''}`);
      }
      break;
    }
    case 'up': {
      await serve(dir);
      break;
    }
    case 'down': {
      if (!(await alive(socketPath(dir)))) { out('not running'); break; }
      await request(socketPath(dir), 'down', {}, null, await secretOf(dir));
      out('stopped');
      break;
    }
    case 'status': {
      const s = await call('status', {});
      if (json) out(JSON.stringify(s));
      else {
        out(`agent   ${name} (${dir})\nden     ${oneLine(s.den)}\ndevice  ${oneLine(s.name)}${nick(s.nick)}\nstate   ${oneLine(s.state)}\nuntil   ${oneLine(s.until) || '-'}`);
        if (s.rules) out(`rules   ${s.rules.rooms === 'all' ? 'every room' : `${s.rules.rooms.length} rooms`}; tasks from the owner${s.rules.takes_from.length ? ` and ${s.rules.takes_from.length} more` : ''}; files ${s.rules.files ? 'yes' : 'no'} (epeaky-agent rules)`);
        out(`owner   ${s.owner ? `${oneLine(s.owner.name)}${nick(s.owner.nick)}${s.owner.confirmed ? '' : ' (not confirmed by the owner: no orders)'}` : 'none: takes no orders (epeaky-agent owner <code>)'}`);
      }
      break;
    }
    case 'owner': {
      const o = await call('owner', { code: pos[0] || null, clear: !!flags.clear });
      if (json) out(JSON.stringify(o));
      else if (flags.clear) out('no owner: this agent takes no orders');
      else out(o ? `owner ${oneLine(o.name)}${nick(o.nick)}${o.confirmed ? '' : ' (not confirmed by the owner: no orders)'}` : 'no owner: this agent takes no orders');
      break;
    }
    case 'rooms': {
      for (const r of await call('rooms', {})) {
        out(json ? JSON.stringify(r) : `#${oneLine(r.id)} "${oneLine(r.title).replace(/"/g, "'")}" ${r.kind === 'private' ? 'private' : 'open'} members=${Number(r.members) || 0} unread=${Number(r.unread) || 0}${Number(r.slow) ? ` slow=${Number(r.slow)}s` : ''}${r.may_post ? '' : ' read-only'}`);
      }
      break;
    }
    case 'read': {
      if (!pos[0]) fail('read <room>', 2);
      const since = flags.since !== undefined ? Number(flags.since) : null;
      const limit = flags.limit !== undefined ? Number(flags.limit) : (flags.new || since !== null ? 1000 : 20);
      if ((since !== null && !Number.isInteger(since)) || !Number.isInteger(limit)) fail('--since and --limit are whole numbers', 2);
      for (const r of await call('read', { room: pos[0], since, limit, unread: !!flags.new })) show(r);
      break;
    }
    case 'send': {
      if (!pos[0] || pos.length < 2) fail('send <room> <words…|->', 2);
      const text = pos[1] === '-' && pos.length === 2 ? await stdin() : pos.slice(1).join(' ');
      const r = await call('send', { room: pos[0], text, reply: flags.reply ?? null, for: flags.for ?? null });
      out(json ? JSON.stringify(r) : `sent #${oneLine(r.room).replace(/\s/g, '')} ${Number(r.seq) || ''}${r.for ? ` for ${r.for.room === r.room ? '' : `#${oneLine(r.for.room).replace(/\s/g, '')} `}${Number(r.for.seq) || 0}${r.for_how === 'last' ? ' (a guess: not sent with it)' : ''}` : ''}`.trim());
      break;
    }
    case 'next': {
      const wait = flags.wait !== undefined ? Number(flags.wait) : 0;
      const limit = flags.limit !== undefined ? Number(flags.limit) : 50;
      if (!Number.isFinite(wait) || wait < 0 || wait > 86400 || !Number.isInteger(limit)) fail('--wait is seconds (up to 86400), --limit a whole number', 2);
      const list = await call('next', { room: pos[0] || null, orders: !!flags.orders, mentions: !!flags.mentions, changes: !!flags.changes, tasks: !!flags.tasks, wait, limit });
      if (!list.length && !json) out('nothing new');
      for (const r of list) show(r);
      break;
    }
    case 'watch': {
      await call('watch', { room: pos[0] || null, mentions: !!flags.mentions, orders: !!flags.orders, once: !!flags.once }, show);
      break;
    }
    case 'pins': {
      const list = await call('pins', { room: pos[0] || null });
      if (!list.length && !json) out('no pins');
      for (const r of list) out(json ? JSON.stringify(r) : line(r).replace(/^(#\S+) 0 /, '$1 pin '));
      break;
    }
    case 'rules': {
      const r = await call('rules', {});
      if (json) { out(JSON.stringify(r)); break; }
      if (!r) { out('no rules from the owner: every room this agent is in; tasks as orders from the owner only; files'); break; }
      out(`rooms   ${r.rooms === 'all' ? 'every room this agent is in' : r.rooms.length ? r.rooms.map((x) => `#${oneLine(x)}`).join(' ') : 'none'}`);
      out(`tasks   from the owner${r.takes_from.length ? `, and from ${r.takes_from.map((x) => (x.nick ? `"${oneLine(x.nick)}"` : x.name)).join(', ')} (delegated)` : ' only'}`);
      out(`files   ${r.files ? 'yes' : 'no: withheld'}`);
      out(`set     ${oneLine(r.at)} by m-${String(r.by).slice(0, 20)}`);
      break;
    }
    case 'tasks': {
      const list = await call('tasks', { room: pos[0] || null, open: !!flags.open });
      if (!list.length && !json) out('no tasks');
      for (const r of list) show(r);
      break;
    }
    case 'task': {
      const words = async (from) => (pos[from] === '-' && pos.length === from + 1 ? await stdin() : pos.slice(from).join(' '));
      if (pos[0] === 'new') {
        if (!pos[1] || typeof flags.to !== 'string' || pos.length < 3) fail('task new <room> --to <member> <words…|->', 2);
        show(await call('task', { new: true, room: pos[1], to: flags.to, text: await words(2), reply: flags.reply ?? null, for: flags.for ?? null }));
        break;
      }
      const states = ['working', 'input-required', 'completed', 'failed', 'rejected', 'canceled'];
      if (!pos[0] || !states.includes(pos[1])) fail(`task <id> ${states.join('|')} [words…|-] [--artifact SEQ,…]`, 2);
      const artifacts = typeof flags.artifact === 'string' ? flags.artifact.split(',').map((x) => x.trim()).filter(Boolean) : [];
      show(await call('task', { id: pos[0], state: pos[1], text: await words(2), artifacts }));
      break;
    }
    case 'log': {
      const limit = flags.limit !== undefined ? Number(flags.limit) : 50;
      if (!Number.isInteger(limit)) fail('--limit is a whole number', 2);
      const list = await call('log', { limit });
      if (!list.length && !json) out('nothing yet');
      for (const e of list) out(json ? JSON.stringify(e) : journalLine(e));
      break;
    }
    case 'search': {
      if (!pos.length) fail('search <words…>', 2);
      for (const r of await call('search', { q: pos.join(' ') })) show(r);
      break;
    }
    default: fail(`no command ${cmd}; see: epeaky-agent help`, 2);
  }
} catch (e) {
  fail(e.message, REFUSED.has(e.code) ? 3 : 1);
}
