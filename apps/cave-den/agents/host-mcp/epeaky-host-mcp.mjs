#!/usr/bin/env node
// Epeaky for hosts: an MCP server (stdio) that helps an agent set up and run
// a den on a server its human already has, over its human's own SSH. It holds
// no keys and makes no server: it checks one, installs Epeaky on it, and runs
// the den's `epeaky` command there. Kept apart from epeaky-mcp on purpose: an
// agent in a room should not also hold root on the server.
//
// Every value a tool takes is checked to a strict shape before it reaches
// ssh, because ssh hands its command to the server's shell as one string;
// values are then single-quoted. Tools that change the server say so
// (readOnlyHint false), so the client asks the human before each call.
//
//   EPEAKY_HOST_SSH=/path/to/ssh   (default: ssh on PATH)
//
// Claude Code:  claude mcp add epeaky-host -- npx -y epeaky-host-mcp
import { execFile } from 'node:child_process';
import { readFileSync, realpathSync } from 'node:fs';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

const VERSION = JSON.parse(readFileSync(new URL('package.json', import.meta.url), 'utf8')).version;
const SSH = process.env.EPEAKY_HOST_SSH || 'ssh';
const INSTALLER = 'https://epeaky.zyn-network.com/install.sh';
const NEEDS_PASSWORD = 77;

// ---- Shapes ----------------------------------------------------------------------

const LABEL = /^(?=[a-z0-9])[a-z0-9-]{1,63}$/i;
const isDomain = (d) => typeof d === 'string' && d.length <= 253 && d.split('.').length >= 2 && d.split('.').every((l) => LABEL.test(l) && !l.endsWith('-'));
const isIPv4 = (s) => /^(\d{1,3})(\.\d{1,3}){3}$/.test(s) && s.split('.').every((n) => Number(n) <= 255);
const isIPv6 = (s) => /^[0-9a-f:]{2,39}$/i.test(s) && s.includes(':');
/// `user@host`: a Unix user name, then a domain, an IPv4 or an IPv6 address.
function isServer(s) {
  if (typeof s !== 'string') return false;
  const m = /^([a-z_][a-z0-9_-]{0,31})@(.+)$/.exec(s);
  return !!m && (isDomain(m[2]) || isIPv4(m[2]) || isIPv6(m[2]));
}
const isEmail = (e) => typeof e === 'string' && /^[A-Za-z0-9._%+-]{1,64}@[A-Za-z0-9.-]+$/.test(e) && isDomain(e.split('@')[1]);
const isHexId = (s) => typeof s === 'string' && /^[0-9a-f]{6,64}$/.test(s);
const isDevice = (s) => typeof s === 'string' && /^m-[0-9a-f]{6,64}$/.test(s);
const int = (v, lo, hi) => Number.isInteger(v) && v >= lo && v <= hi;
/// Single-quoted for a POSIX shell.
const q = (v) => `'${String(v).replace(/'/g, `'\\''`)}'`;

/// What a setting's value may be (and an empty value removes it). Values
/// never carry quotes, backslashes, `$` or line breaks: they go into .env.
const SAFE = /^[^'"`$\\\n\r]*$/;
const LIMITS = ['message_bytes', 'file_max', 'file_bytes', 'files', 'room_members', 'invites', 'inbox', 'history_bytes', 'min_free_bytes'];
export const SETTINGS = {
  DEN_ENTRY: { rule: (v) => /^(invite|approval|external)(,(invite|approval|external))*$/.test(v), say: 'invite, approval or external, comma-separated' },
  DEN_ROOMS: { rule: (v) => /^[a-z0-9-]{1,32}:[^,:]{1,40}(,[a-z0-9-]{1,32}:[^,:]{1,40})*$/.test(v), say: 'id:Title pairs, comma-separated (ids: a-z, 0-9, -)' },
  DEN_DONATE: { rule: (v) => /^https:\/\/[A-Za-z0-9.-]+(:\d+)?(\/[A-Za-z0-9._~\/%-]*)?$/.test(v) && isDomain(new URL(v).hostname), say: 'an https:// address' },
  DEN_DONATE_LABEL: { rule: (v) => v.length >= 1 && v.length <= 40, say: '1 to 40 characters' },
  DEN_ISSUER_LABEL: { rule: (v) => v.length >= 1 && v.length <= 40, say: '1 to 40 characters' },
  DEN_SCREEN: { rule: (v) => v === 'on' || v === 'off', say: 'on or off' },
  DEN_SCREEN_BITRATE: { rule: (v) => /^\d+$/.test(v) && Number(v) >= 300000 && Number(v) <= 8000000, say: 'bits a second, 300000 to 8000000' },
  DEN_RETENTION_DAYS: { rule: (v) => /^\d+$/.test(v) && Number(v) >= 1 && Number(v) <= 3650, say: 'days, 1 to 3650' },
  DEN_LIMITS: { rule: (v) => v.split(',').every((p) => { const [k, n] = p.split('='); return LIMITS.includes(k) && /^\d{1,12}$/.test(n || ''); }), say: `name=number pairs (${LIMITS.join(', ')})` },
  DEN_PUBLIC_IP: { rule: (v) => isIPv4(v) || isIPv6(v), say: 'an IPv4 or IPv6 address' },
};

// ---- Tools -------------------------------------------------------------------------

const server = { type: 'string', description: 'The server as user@host (its address or name), reachable with your human\'s SSH key.' };
const id = { type: 'string', description: 'The id as epeaky requests / epeaky invites lists it (hex).' };
const ASK = 'This changes the server: Ask your human first, and tell them what it does.';
const read = (title) => ({ title, readOnlyHint: true, openWorldHint: true });
const write = (title, destructive = false) => ({ title, readOnlyHint: false, destructiveHint: destructive, idempotentHint: false, openWorldHint: true });

export const TOOLS = [
  {
    name: 'host_check',
    description: 'Check a server and a domain for an Epeaky den, changing nothing. Ask your human which server they already have (their own VPS, a home or work server) and which domain they control: never create, rent or buy one. Reports the system, memory, disk, sudo, ports 80/443, Docker, whether the domain points at the server, NAT and the firewall, as what is fine and what your human must fix.',
    inputSchema: { type: 'object', properties: { server, domain: { type: 'string', description: 'The domain for the den, e.g. chat.example.org.' } }, required: ['server', 'domain'], additionalProperties: false },
    annotations: read('Check a server'),
  },
  {
    name: 'host_install',
    description: `Install Epeaky on the server: Docker if it is missing (with Docker's own script), the den and Caddy in /opt/epeaky, a certificate for the domain, and TCP 80/443 and UDP 50000-50199 opened in ufw or firewalld. Run host_check first. Returns the first invite link: show it only to your human, who opens it on their own device. ${ASK} Offer the installer to read first: ${INSTALLER}`,
    inputSchema: { type: 'object', properties: { server, domain: { type: 'string' }, email: { type: 'string', description: 'For the certificate\'s expiry notices (optional).' } }, required: ['server', 'domain'], additionalProperties: false },
    annotations: write('Install Epeaky'),
  },
  {
    name: 'host_status',
    description: 'The den on the server: its owner, how many members, people asking to join, and invites. Changes nothing.',
    inputSchema: { type: 'object', properties: { server }, required: ['server'], additionalProperties: false },
    annotations: read('Den status'),
  },
  {
    name: 'host_logs',
    description: "The den's last log lines, for when something fails. Changes nothing.",
    inputSchema: { type: 'object', properties: { server, lines: { type: 'integer', minimum: 1, maximum: 1000 } }, required: ['server'], additionalProperties: false },
    annotations: read('Den logs'),
  },
  {
    name: 'host_owner',
    description: `Make a device the den's owner: the one with every power and Den settings. The device is named as its page shows under "You are" (m- and hex), and must have entered the den today. ${ASK}`,
    inputSchema: { type: 'object', properties: { server, device: { type: 'string', description: 'm- and 6 to 64 hex characters.' } }, required: ['server', 'device'], additionalProperties: false },
    annotations: write('Set the owner'),
  },
  {
    name: 'host_invite',
    description: `Make one-use invite links. Show them only to your human: whoever opens one first is in. ${ASK}`,
    inputSchema: { type: 'object', properties: { server, count: { type: 'integer', minimum: 1, maximum: 100 }, days: { type: 'integer', minimum: 1, maximum: 90, description: 'How long they are good for (default 7).' } }, required: ['server'], additionalProperties: false },
    annotations: write('Make invites'),
  },
  {
    name: 'host_withdraw',
    description: `Withdraw an invite, and the pass it let in. ${ASK}`,
    inputSchema: { type: 'object', properties: { server, id }, required: ['server', 'id'], additionalProperties: false },
    annotations: write('Withdraw an invite', true),
  },
  {
    name: 'host_approve',
    description: `Let a request to join in (see host_status for requests). ${ASK}`,
    inputSchema: { type: 'object', properties: { server, id }, required: ['server', 'id'], additionalProperties: false },
    annotations: write('Approve a request'),
  },
  {
    name: 'host_decline',
    description: `Turn a request to join down. ${ASK}`,
    inputSchema: { type: 'object', properties: { server, id }, required: ['server', 'id'], additionalProperties: false },
    annotations: write('Decline a request', true),
  },
  {
    name: 'host_setting',
    description: `Change one of the den's settings in /opt/epeaky/.env (a backup first), then restart it. An empty value removes the setting. Only these: ${Object.entries(SETTINGS).map(([k, v]) => `${k} (${v.say})`).join('; ')}. ${ASK}`,
    inputSchema: { type: 'object', properties: { server, name: { type: 'string', enum: Object.keys(SETTINGS) }, value: { type: 'string' } }, required: ['server', 'name', 'value'], additionalProperties: false },
    annotations: write('Change a setting'),
  },
  {
    name: 'host_update',
    description: `Pull the latest Epeaky and restart the den. Members reconnect by themselves. ${ASK}`,
    inputSchema: { type: 'object', properties: { server }, required: ['server'], additionalProperties: false },
    annotations: write('Update Epeaky'),
  },
];

// What the check runs on the server: facts as name=value lines, nothing changed.
const CHECK = [
  'echo "os=$(uname -s)"; echo "arch=$(uname -m)"',
  '. /etc/os-release 2>/dev/null; echo "distro=$PRETTY_NAME"',
  'echo "cores=$(nproc 2>/dev/null)"; echo "mem_mb=$(free -m 2>/dev/null | awk \'/Mem/ {print $2}\')"',
  'echo "disk_free_gb=$(df -Pk / | awk \'NR==2 {print int($4/1048576)}\')"',
  'if [ "$(id -u)" = 0 ]; then echo sudo=root; elif sudo -n true 2>/dev/null; then echo sudo=ok; else echo sudo=password; fi',
  'echo "ports=$(ss -ltnH 2>/dev/null | awk \'{print $4}\' | grep -E \':(80|443)$\' | tr \'\\n\' \' \')"',
  'if command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then echo docker=yes; else echo docker=no; fi',
  'echo "addrs=$(ip -o addr show scope global 2>/dev/null | awk \'{print $4}\' | cut -d/ -f1 | tr \'\\n\' \' \')"',
  'echo "public=$(curl -fsS -m 8 https://api.ipify.org 2>/dev/null)"',
  'S=""; [ "$(id -u)" = 0 ] || S="sudo -n"; if command -v ufw >/dev/null 2>&1; then echo "firewall=ufw $($S ufw status 2>/dev/null | head -1 | sed \'s/Status: //\')"; elif command -v firewall-cmd >/dev/null 2>&1; then echo "firewall=firewalld $($S firewall-cmd --state 2>/dev/null)"; else echo firewall=none; fi',
  'if [ -d /opt/epeaky ]; then echo installed=yes; else echo installed=no; fi',
].join('; ');

/// The command for a tool: what runs on the server, and whether as root. A
/// string is a refusal, said to the model as it is.
export function command(name, a = {}) {
  if (!isServer(a.server)) return 'server must be user@host: a user name, then the server\'s name or address';
  const root = (remote, timeout = 120) => ({ server: a.server, remote, root: true, timeout });
  switch (name) {
    case 'host_check':
      if (!isDomain(a.domain) || a.domain.endsWith('.local')) return 'domain must be a public name such as chat.example.org';
      return { server: a.server, remote: CHECK, root: false, timeout: 60, domain: a.domain };
    case 'host_install': {
      if (!isDomain(a.domain)) return 'domain must be a public name such as chat.example.org';
      if (a.email !== undefined && a.email !== '' && !isEmail(a.email)) return 'email is not an address';
      const email = a.email ? ` --email ${q(a.email)}` : '';
      return root(`curl -fsSL ${INSTALLER} -o /tmp/epeaky-install.sh && bash /tmp/epeaky-install.sh --domain ${q(a.domain)}${email} --yes`, 900);
    }
    case 'host_status':
      return root('echo "== owner"; epeaky owner; echo "== members"; epeaky members; echo "== requests"; epeaky requests; echo "== invites"; epeaky invites');
    case 'host_logs': {
      const n = a.lines ?? 100;
      if (!int(n, 1, 1000)) return 'lines is 1 to 1000';
      return root(`epeaky logs ${n}`);
    }
    case 'host_owner':
      if (!isDevice(a.device)) return 'device must be its name as the page shows it: m- and 6 to 64 hex characters';
      return root(`epeaky owner ${q(a.device)}`);
    case 'host_invite': {
      const n = a.count ?? 1;
      const d = a.days ?? 7;
      if (!int(n, 1, 100)) return 'count is 1 to 100';
      if (!int(d, 1, 90)) return 'days is 1 to 90';
      return root(`epeaky invite ${n} ${d}`);
    }
    case 'host_withdraw':
    case 'host_approve':
    case 'host_decline':
      if (!isHexId(a.id)) return 'id must be hex, as epeaky requests or epeaky invites shows it';
      return root(`${{ host_withdraw: 'epeaky withdraw', host_approve: 'epeaky approve', host_decline: 'epeaky decline' }[name]} ${q(a.id)}`);
    case 'host_setting': {
      const s = SETTINGS[a.name];
      if (!s) return `${a.name} is not a setting this tool changes; it changes ${Object.keys(SETTINGS).join(', ')}`;
      const v = typeof a.value === 'string' ? a.value.trim() : null;
      if (v === null || !SAFE.test(v) || (v !== '' && !s.rule(v))) return `${a.name} must be ${s.say}${SAFE.test(v ?? '') ? '' : ', with no quotes, $, backslashes or line breaks'}`;
      const add = v === '' ? '' : ` && printf '%s\\n' ${q(`${a.name}=${v}`)} >> .env.new`;
      return root(`cd /opt/epeaky && cp .env .env.bak-$(date +%Y%m%d-%H%M%S) && grep -v '^${a.name}=' .env > .env.new; true${add} && mv .env.new .env && docker compose up -d`);
    }
    case 'host_update':
      return root('epeaky update', 600);
    default:
      return `no tool named ${name}`;
  }
}

/// As root: directly, or by sudo when it asks for no password; otherwise
/// the exit code that means "your human must run this".
const asRoot = (s) => `if [ "$(id -u)" = 0 ]; then sh -c ${q(s)}; elif sudo -n true 2>/dev/null; then sudo -n sh -c ${q(s)}; else exit ${NEEDS_PASSWORD}; fi`;

export function sshArgs(server, remote) {
  return ['-o', 'BatchMode=yes', '-o', 'ConnectTimeout=15', '-o', 'StrictHostKeyChecking=accept-new', '--', server, remote];
}

/// From the check's facts and the domain's records: what is fine, and what
/// your human has to fix before installing.
export function report(f, dns) {
  const ok = [];
  const fix = [];
  const PRIVATE = /^(10\.|172\.(1[6-9]|2\d|3[01])\.|192\.168\.|100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.|fe80:|f[cd])/i;
  if (f.os !== 'Linux') fix.push(`The server runs ${f.os || 'an unknown system'}; Epeaky needs Linux.`);
  else if (!['x86_64', 'amd64', 'aarch64', 'arm64'].includes(f.arch)) fix.push(`The server's processor is ${f.arch}; the image is built for x86-64 and arm64.`);
  else ok.push(`Linux on ${f.arch}${f.distro ? ` (${f.distro})` : ''}.`);
  const mem = Number(f.mem_mb);
  if (mem && mem < 1000) fix.push(`Only ${mem} MB of memory; give it at least 1 GB (2 cores and 4 GB carry one room of 200 devices).`);
  else if (mem) ok.push(`${f.cores || '?'} cores, ${mem} MB of memory.`);
  const disk = Number(f.disk_free_gb);
  if (Number.isFinite(disk) && disk < 5) fix.push(`Only ${disk} GB of free disk; leave at least 5 GB.`);
  else if (Number.isFinite(disk)) ok.push(`${disk} GB of free disk.`);
  if (f.sudo === 'password') fix.push('sudo asks for a password here: your human runs host_install\'s command themselves, or uses root.');
  else ok.push(f.sudo === 'root' ? 'Connected as root.' : 'sudo works without a password.');
  const used = (f.ports || '').trim();
  if (used && f.installed !== 'yes') fix.push(`Something already listens on ${used.split(/\s+/).map((p) => p.split(':').pop()).join(' and ')}; Caddy needs ports 80 and 443. Stop or move it first.`);
  ok.push(f.docker === 'yes' ? 'Docker is installed.' : 'No Docker yet: the installer installs it.');
  const mine = (f.addrs || '').split(/\s+/).filter(Boolean);
  const pub = mine.filter((x) => !PRIVATE.test(x) && !x.startsWith('172.17.'));
  const records = [...(dns?.a || []), ...(dns?.aaaa || [])];
  if (!records.length) fix.push('The domain has no A or AAAA record yet: point it at the server\'s public address.');
  else if (!records.some((r) => mine.includes(r) || r === f.public)) fix.push(`The domain does not point at this server: it has ${records.join(', ')}, the server is ${f.public || mine.join(', ')}.`);
  else ok.push('The domain points at this server.');
  if (!pub.length) fix.push('The server has only private addresses: it is behind NAT. A cloud\'s 1:1 NAT works as it is; a home router must forward TCP 80, 443 and UDP 50000-50199 to it.');
  if (/active/.test(f.firewall || '') && !/inactive/.test(f.firewall)) ok.push(`Firewall: ${f.firewall}; the installer opens the ports in it.`);
  ok.push('If the provider has a firewall in its web panel, open TCP 80, 443 and UDP 50000-50199 there too.');
  if (f.installed === 'yes') ok.push('Epeaky is already installed here: use host_status, host_update and host_setting.');
  return { ok, fix };
}

// ---- Running -----------------------------------------------------------------------

function ssh(c) {
  return new Promise((resolve) => {
    execFile(SSH, sshArgs(c.server, c.root ? asRoot(c.remote) : c.remote), { timeout: c.timeout * 1000, maxBuffer: 8 << 20 }, (err, stdout, stderr) => {
      resolve({ code: err ? (typeof err.code === 'number' ? err.code : 1) : 0, out: String(stdout), err: String(stderr).trim() });
    });
  });
}

async function resolveName(domain) {
  const ask = async (type) => {
    try {
      const r = await fetch(`https://cloudflare-dns.com/dns-query?name=${encodeURIComponent(domain)}&type=${type}`, { headers: { accept: 'application/dns-json' }, signal: AbortSignal.timeout(8000) });
      const j = await r.json();
      return (j.Answer || []).filter((x) => x.type === (type === 'A' ? 1 : 28)).map((x) => x.data);
    } catch { return []; }
  };
  return { a: await ask('A'), aaaa: await ask('AAAA') };
}

const text = (t, extra = {}) => ({ content: [{ type: 'text', text: t }], ...extra });
const oops = (t) => ({ isError: true, ...text(t) });

async function call(name, a) {
  const c = command(name, a);
  if (typeof c === 'string') return oops(c);
  const r = await ssh(c);
  if (r.code === NEEDS_PASSWORD) {
    return oops(`sudo asks for a password on ${c.server}. Ask your human to run this themselves (in Claude Code: ! and the command):\nssh -t ${c.server} ${q(`sudo sh -c ${q(c.remote)}`)}`);
  }
  if (r.code === 255) return oops(`ssh could not reach ${c.server}: ${r.err || 'no answer'}. Your human may need to add their key to the server, or check its address.`);
  if (name === 'host_check') {
    if (r.code !== 0) return oops(r.err || r.out || `exit ${r.code}`);
    const facts = Object.fromEntries(r.out.split('\n').map((l) => l.match(/^([a-z_]+)=(.*)$/)).filter(Boolean).map((m) => [m[1], m[2].trim()]));
    const dns = await resolveName(c.domain);
    const out = report(facts, dns);
    const body = [out.fix.length ? 'To fix before installing:' : 'Nothing to fix before installing.', ...out.fix.map((l) => `- ${l}`), 'Fine:', ...out.ok.map((l) => `- ${l}`)].join('\n');
    return text(body, { structuredContent: { ...out, facts, dns } });
  }
  const body = (r.out + (r.err ? `\n${r.err}` : '')).trim() || 'done';
  if (r.code !== 0) return oops(body);
  const invites = [...body.matchAll(/https:\/\/\S+#invite=[0-9a-f]{64}/g)].map((m) => m[0]);
  const note = invites.length ? '\n\nShow these invite links only to your human: whoever opens one first is in. Do not post, log or store them.' : '';
  return text(body + note, { structuredContent: { output: body, invites } });
}

// ---- MCP over stdio: one JSON-RPC message a line ------------------------------

const send = (msg) => process.stdout.write(`${JSON.stringify(msg)}\n`);
const reply = (mid, result) => send({ jsonrpc: '2.0', id: mid, result });
const fail = (mid, code, message) => send({ jsonrpc: '2.0', id: mid, error: { code, message } });

async function handle(m) {
  if (m?.jsonrpc !== '2.0' || typeof m.method !== 'string') {
    if (m && 'id' in m) fail(m.id ?? null, -32600, 'not a JSON-RPC 2.0 request');
    return;
  }
  switch (m.method) {
    case 'initialize':
      return reply(m.id, {
        protocolVersion: typeof m.params?.protocolVersion === 'string' ? m.params.protocolVersion : '2025-06-18',
        capabilities: { tools: {} },
        serverInfo: { name: 'epeaky-host', version: VERSION },
        instructions: 'These tools set up and run an Epeaky den on a server your human already has, over their SSH. Ask them which server and domain first; never create, rent or buy a server, never change DNS yourself. Run host_check before host_install. Ask before every tool that changes the server. Invite links are for your human only.',
      });
    case 'ping':
      return reply(m.id, {});
    case 'tools/list':
      return reply(m.id, { tools: TOOLS });
    case 'tools/call': {
      const a = m.params?.arguments ?? {};
      if (typeof a !== 'object' || Array.isArray(a)) return reply(m.id, oops('arguments is an object'));
      return reply(m.id, await call(m.params?.name, a));
    }
    default:
      if ('id' in m) fail(m.id, -32601, `no method ${m.method}`);
  }
}

// Serve only when run, not when imported (the tests import the pure parts).
const main = (() => { try { return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url)); } catch { return false; } })();
if (main) {
  const rl = createInterface({ input: process.stdin });
  rl.on('line', (line) => {
    if (!line.trim()) return;
    let m;
    try { m = JSON.parse(line); } catch { return fail(null, -32700, 'not JSON'); }
    handle(m).catch((e) => { if (m && 'id' in m) fail(m.id, -32603, String(e?.message || e)); });
  });
}
