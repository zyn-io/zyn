// The host skill (skill/epeaky-host/SKILL.md) teaches an agent to set up and
// run a den. What it names must exist: a command or setting renamed in the
// installer or the server fails here, not in front of someone's server.
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { test } from 'node:test';

const here = (p) => new URL(p, import.meta.url);
const skill = readFileSync(here('../skill/epeaky-host/SKILL.md'), 'utf8');
// The installer: at the repository's root in zyn-io/epeaky, under scripts/den/root where it is made.
const installer = ['../../install.sh', '../../../../scripts/den/root/install.sh'].map(here).find((u) => existsSync(u));
const install = readFileSync(installer, 'utf8');
const runtime = readFileSync(here('../../server/config/runtime.exs'), 'utf8');

test('every epeaky command the host skill names is one the installed command has', () => {
  const known = new Set([...install.matchAll(/^\s+([a-z]+)\)\s/gm)].map((m) => m[1]));
  const named = new Set([...skill.matchAll(/`epeaky ([a-z]+)\b/g)].map((m) => m[1]));
  assert.ok(named.size >= 8, `the skill names the day-to-day commands (${[...named]})`);
  for (const c of named) assert.ok(known.has(c), `epeaky ${c} is not a command of install.sh's epeaky`);
});

test('every setting the host skill names is one the den reads', () => {
  const known = new Set([...runtime.matchAll(/System\.get_env\("(DEN_[A-Z_]+)"/g)].map((m) => m[1]));
  const named = new Set([...skill.matchAll(/\b(DEN_[A-Z_]+)\b/g)].map((m) => m[1]));
  assert.ok(named.size >= 4);
  for (const s of named) assert.ok(known.has(s), `${s} is not a setting the den reads`);
});

test('the installer flags the skill uses are the installer\'s own', () => {
  const flags = new Set([...install.matchAll(/^\s+(--[a-z]+)[|)]/gm)].map((m) => m[1]));
  for (const f of new Set([...skill.matchAll(/install\.sh[^\n`]*?(--[a-z]+)/g)].map((m) => m[1]))) assert.ok(flags.has(f), `install.sh has no ${f}`);
});

test('the skill asks where before anything, never buys a server, and asks before every change', () => {
  assert.match(skill, /Never create, rent or buy a server/);
  assert.match(skill, /Ask before every command that changes the server/);
  assert.match(skill, /only to your human/i);
});
