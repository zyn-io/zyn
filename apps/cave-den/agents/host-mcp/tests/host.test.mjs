import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { test } from 'node:test';
import { command, report, sshArgs, SETTINGS, TOOLS } from '../epeaky-host-mcp.mjs';

const here = (p) => new URL(p, import.meta.url);
const SERVER = 'root@203.0.113.7';

test('no tool makes, rents or buys a server, and the first one says to ask where', () => {
  assert.ok(!TOOLS.some((t) => /create|provision|buy|rent|order/i.test(t.name)));
  assert.match(TOOLS.find((t) => t.name === 'host_check').description, /Ask your human which server/);
  for (const t of TOOLS) assert.ok(t.annotations && typeof t.annotations.readOnlyHint === 'boolean', `${t.name} says whether it changes anything`);
  const changing = TOOLS.filter((t) => !t.annotations.readOnlyHint).map((t) => t.name).sort();
  assert.deepEqual(changing, ['host_approve', 'host_decline', 'host_install', 'host_invite', 'host_owner', 'host_setting', 'host_update', 'host_withdraw']);
  for (const t of TOOLS.filter((x) => !x.annotations.readOnlyHint)) assert.match(t.description, /Ask your human first/, t.name);
});

test('ssh gets the server after the end of its options, in batch mode', () => {
  assert.deepEqual(sshArgs(SERVER, 'true'), ['-o', 'BatchMode=yes', '-o', 'ConnectTimeout=15', '-o', 'StrictHostKeyChecking=accept-new', '--', SERVER, 'true']);
});

test('a server, domain, id, device or count in the wrong shape never reaches ssh', () => {
  const bad = (name, a, re) => assert.match(String(command(name, a)), re, `${name} ${JSON.stringify(a)}`);
  for (const server of ['-oProxyCommand=evil', 'root@host;id', 'root@$(id)', 'root@host name', '', 'root', 'ROOT@host', 'root@-x']) bad('host_status', { server }, /server/);
  for (const domain of ['x;id', 'a b.com', 'localhost', '$(id).com', '-x.com', 'example.com\nid']) bad('host_check', { server: SERVER, domain }, /domain/);
  bad('host_install', { server: SERVER, domain: 'chat.example.org', email: "a'b@c" }, /email/);
  for (const device of ['m-xyz123', 'm-12345', 'm-abcdef;id', 'abcdef123']) bad('host_owner', { server: SERVER, device }, /device/);
  for (const id of ['', 'zz', "a'1", '1 2']) bad('host_approve', { server: SERVER, id }, /id/);
  bad('host_invite', { server: SERVER, count: 101 }, /count/);
  bad('host_invite', { server: SERVER, count: 2, days: 91 }, /days/);
  bad('host_logs', { server: SERVER, lines: 0 }, /lines/);
});

test('the remote commands are the den host command, run as root, with checked values only', () => {
  const c = command('host_owner', { server: SERVER, device: 'm-6c8b4b0f1a' });
  assert.match(c.remote, /epeaky owner 'm-6c8b4b0f1a'/);
  assert.ok(c.root);
  assert.match(command('host_invite', { server: SERVER, count: 3, days: 7 }).remote, /epeaky invite 3 7/);
  assert.match(command('host_approve', { server: SERVER, id: 'a1b2c3d4' }).remote, /epeaky approve 'a1b2c3d4'/);
  const i = command('host_install', { server: SERVER, domain: 'chat.example.org', email: 'you@example.org' });
  assert.match(i.remote, /curl -fsSL https:\/\/epeaky\.zyn-network\.com\/install\.sh -o \/tmp\/epeaky-install\.sh/);
  assert.match(i.remote, /bash \/tmp\/epeaky-install\.sh --domain 'chat\.example\.org' --email 'you@example\.org' --yes/);
  assert.ok(!/--email/.test(command('host_install', { server: SERVER, domain: 'chat.example.org' }).remote));
});

test('only known settings, each to its own rule, written with a backup and a restart', () => {
  const s = (name, value) => command('host_setting', { server: SERVER, name, value });
  assert.match(String(s('DEN_SECRET_KEY_BASE', 'x')), /not a setting this tool changes/);
  assert.match(String(s('PHX_HOST', 'evil.com')), /not a setting/);
  assert.match(String(s('DEN_ENTRY', 'invite,everyone')), /DEN_ENTRY/);
  assert.match(String(s('DEN_DONATE', 'http://x.org')), /DEN_DONATE/);
  assert.match(String(s('DEN_DONATE_LABEL', "it's")), /DEN_DONATE_LABEL/);
  assert.match(String(s('DEN_DONATE_LABEL', 'pay $HOME')), /DEN_DONATE_LABEL/);
  assert.match(String(s('DEN_ROOMS', 'general:General\nPHX_HOST=x')), /DEN_ROOMS/);
  assert.match(String(s('DEN_SCREEN_BITRATE', '9000000')), /DEN_SCREEN_BITRATE/);
  const ok = s('DEN_ENTRY', 'invite,approval');
  assert.match(ok.remote, /cp \.env \.env\.bak-/);
  assert.match(ok.remote, /grep -v '\^DEN_ENTRY=' \.env > \.env\.new/);
  assert.match(ok.remote, /printf '%s\\n' 'DEN_ENTRY=invite,approval' >> \.env\.new/);
  assert.match(ok.remote, /docker compose up -d/);
  assert.ok(!/printf/.test(s('DEN_DONATE', '').remote), 'an empty value removes the setting');
  assert.ok(s('DEN_ROOMS', 'general:General,news:News and notes').remote);
});

test('the check report says what is fine and what the human must fix', () => {
  const good = report({ os: 'Linux', arch: 'aarch64', cores: '2', mem_mb: '3805', disk_free_gb: '33', sudo: 'root', ports: '', docker: 'no', addrs: '203.0.113.10 2001:db8::1 172.17.0.1', public: '203.0.113.10', installed: 'no', firewall: 'inactive' }, { a: ['203.0.113.10'], aaaa: [] });
  assert.deepEqual(good.fix, []);
  assert.ok(good.ok.some((l) => /points at this server/.test(l)));
  const bad = report({ os: 'Linux', arch: 'armv7l', cores: '1', mem_mb: '900', disk_free_gb: '2', sudo: 'password', ports: '0.0.0.0:80', docker: 'yes', addrs: '10.0.0.5', public: '203.0.113.9', installed: 'no', firewall: 'inactive' }, { a: ['198.51.100.1'], aaaa: [] });
  for (const re of [/arm|x86/i, /80/, /does not point/, /behind NAT|router/i, /password/, /memory/i, /disk/i]) assert.ok(bad.fix.some((l) => re.test(l)), `${re} in ${JSON.stringify(bad.fix)}`);
});

// Drift: what the tool runs and changes must exist in the installer and the den.
const installer = ['../../../install.sh', '../../../../../scripts/den/root/install.sh'].map(here).find((u) => existsSync(u));
test('every epeaky command and setting the tool uses exists', () => {
  const install = readFileSync(installer, 'utf8');
  const runtime = readFileSync(here('../../../server/config/runtime.exs'), 'utf8');
  const commands = new Set([...install.matchAll(/^\s+([a-z]+)\)\s/gm)].map((m) => m[1]));
  const src = readFileSync(here('../epeaky-host-mcp.mjs'), 'utf8');
  for (const [, c] of src.matchAll(/epeaky ([a-z]+)\b/g)) assert.ok(commands.has(c), `epeaky ${c}`);
  const read = new Set([...runtime.matchAll(/System\.get_env\("(DEN_[A-Z_]+)"/g)].map((m) => m[1]));
  for (const name of Object.keys(SETTINGS)) assert.ok(read.has(name), name);
  for (const f of ['--domain', '--email', '--yes']) assert.match(install, new RegExp(`^\\s+${f}[|)]`, 'm'), f);
});

test('the MCP Registry entry names this package, at this version', () => {
  const pkg = JSON.parse(readFileSync(here('../package.json'), 'utf8'));
  const srv = JSON.parse(readFileSync(here('../server.json'), 'utf8'));
  assert.equal(pkg.mcpName, 'io.github.zyn-io/epeaky-host-mcp');
  assert.equal(srv.name, pkg.mcpName);
  assert.equal(srv.version, pkg.version);
  assert.deepEqual(srv.packages.map((p) => [p.registryType, p.identifier, p.version, p.transport.type]), [['npm', 'epeaky-host-mcp', pkg.version, 'stdio']]);
});
