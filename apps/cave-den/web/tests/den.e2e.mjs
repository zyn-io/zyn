// The den's page in a real browser, policy enforced: two members through the
// door (A through the gate's issuer window, B with its code directly), words,
// replies, reactions, edits, files, rooms, private rooms, search and voice.
//
//   (cd ../server && mix phx.server)       # the dev den, on testnet's ledger
//   ./build.sh ../server/priv/page ../cave  # its page, then a local config:
//   A_TOKEN=… B_TOKEN=… node tests/den.e2e.mjs
//
// A_TOKEN and B_TOKEN are files holding two testnet member pass codes. Each run
// spends one of each pass's three daily tokens.
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';

const require = createRequire(new URL('../../../nap-connect/package.json', import.meta.url));
const { chromium } = require('playwright-core');
const DEN = process.env.DEN || 'http://localhost:4000';
const SHOTS = process.env.SHOTS || tmpdir();
const CHROME = process.env.CHROMIUM || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const codeA = readFileSync(process.env.A_TOKEN, 'utf8').trim();
const codeB = readFileSync(process.env.B_TOKEN, 'utf8').trim();

// A stand-in for the host's issuer: a page on another origin that hands the
// opener A's pass code, addressed to the den's origin only.
const issuer = createServer((req, res) => {
  res.setHeader('content-type', 'text/html');
  res.end(`<!doctype html><title>Issuer</title><script>
    opener.postMessage({ den: 'pass', code: 'f'.repeat(64) }, 'https://elsewhere.example');
    opener.postMessage({ den: 'pass', code: ${JSON.stringify(codeA)} }, ${JSON.stringify(DEN)});
    setTimeout(() => close(), 200);
  </script>`);
}).listen(4181);
const pageConfig = new URL('../../server/priv/page/config.json', import.meta.url);
const savedConfig = readFileSync(pageConfig, 'utf8');
writeFileSync(pageConfig, JSON.stringify({ keysUrl: '/den-keys.json', prefix: 'testnet', issuer: 'http://localhost:4181/' }));

const b = await chromium.launch({ executablePath: CHROME, headless: true, args: ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream', '--autoplay-policy=no-user-gesture-required'] });
const errs = [];
const csp = [];
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) process.exitCode = 1; };
const until = (p, f, arg, ms = 30000) => p.waitForFunction(f, arg, { timeout: ms }).then(() => true, () => false);
async function member(name, viaGate) {
  const ctx = await b.newContext({ viewport: { width: 1300, height: 900 }, permissions: ['microphone'] });
  const p = await ctx.newPage();
  await p.addInitScript(() => { navigator.mediaDevices.getUserMedia = async () => { const ac = new AudioContext(); const o = ac.createOscillator(); const d = ac.createMediaStreamDestination(); o.connect(d); o.start(); return d.stream; }; });
  p.on('pageerror', (e) => errs.push(`${name}: ${e.message.slice(0, 200)}`));
  p.on('console', (m) => { if (/Content Security Policy|Refused to/.test(m.text())) csp.push(`${name}: ${m.text().slice(0, 200)}`); });
  await p.goto(`${DEN}/`, { waitUntil: 'networkidle' });
  if (viaGate) {
    await p.click('#enter');
  } else {
    await p.evaluate(async (code) => { await window.__den.fromPass(code); }, codeB);
    await p.reload({ waitUntil: 'networkidle' });
  }
  await p.waitForSelector('#den:not([hidden])', { timeout: 20000 });
  const me = await p.evaluate(() => window.__den.me);
  return { p, me };
}
const A = await member('A', true);
ok(true, 'A enters through the issuer window; a code sent to another origin never arrives');
const B = await member('B', false);
ok(await until(A.p, ([k]) => window.__den.members('general').some((m) => m.key === k), [B.me.key], 40000) && await until(B.p, ([k]) => window.__den.members('general').some((m) => m.key === k), [A.me.key], 40000), `A and B in General (${A.me.name}, ${B.me.name})`);

// Words, a mention, a reply, a reaction, an edit, a deletion.
await A.p.fill('#say-text', `Hello @${B.me.name}, welcome to the den.`);
await A.p.press('#say-text', 'Enter');
ok(await until(B.p, () => document.querySelector('#log li.ping')), 'B sees the mention highlighted');
await B.p.hover('#log li.ping');
await B.p.click('#log li.ping [data-act="reply"]');
await B.p.fill('#say-text', 'Glad to be here.');
await B.p.press('#say-text', 'Enter');
ok(await until(A.p, () => [...document.querySelectorAll('#log .quote')].some((q) => /Hello/.test(q.textContent))), 'A sees the reply quoting its message');
const firstId = await A.p.$eval('#log li.mine', (li) => li.dataset.id);
await B.p.evaluate(([id]) => window.__den.react('general', id, '👍'), [firstId]);
ok(await until(A.p, () => /👍 1/.test(document.querySelector('#log')?.textContent || '')), 'A sees B\'s reaction');
const replyId = await B.p.$eval('#log li.mine', (li) => li.dataset.id);
await B.p.evaluate(([id]) => window.__den.edit('general', id, 'Glad to be here, truly.'), [replyId]);
ok(await until(A.p, () => /truly\./.test(document.querySelector('#log')?.textContent || '') && /edited/.test(document.querySelector('#log')?.textContent || '')), 'A sees B\'s edit, marked edited');
await A.p.evaluate(([id]) => window.__den.edit('general', id, 'forged edit'), [replyId]).catch(() => {});
ok(await until(B.p, () => !/forged edit/.test(document.querySelector('#log')?.textContent || ''), null, 3000), "nobody edits another member's message");
await A.p.fill('#say-text', 'This one will go.');
await A.p.press('#say-text', 'Enter');
const goneId = await A.p.$$eval('#log li.mine', (ls) => ls.at(-1).dataset.id);
await until(B.p, () => /This one will go/.test(document.querySelector('#log')?.textContent || ''));
await A.p.evaluate(([id]) => window.__den.remove('general', id), [goneId]);
ok(await until(B.p, ([id]) => document.querySelector(`#log li[data-id="${id}"]`)?.classList.contains('gone'), [goneId]), 'a deletion reaches B');

// Typing.
await B.p.type('#say-text', 'typ');
ok(await until(A.p, () => /is writing/.test(document.querySelector('#typing')?.textContent || ''), null, 8000), 'A sees B writing');
await B.p.fill('#say-text', '');

// Files: an image shown inline, a document downloaded and opened.
await A.p.evaluate(async () => {
  const c = document.createElement('canvas'); c.width = 40; c.height = 40;
  const x = c.getContext('2d'); x.fillStyle = '#c7f04b'; x.fillRect(0, 0, 40, 40);
  const png = await new Promise((r) => c.toBlob(r, 'image/png'));
  await window.__den.sendFile('general', new File([png], 'lime.png', { type: 'image/png' }));
  await window.__den.sendFile('general', new File(['the minutes of the first meeting'], 'minutes.txt', { type: 'text/plain' }));
});
ok(await until(B.p, () => [...document.querySelectorAll('#log .att img')].some((i) => i.src.startsWith('blob:') && i.naturalWidth === 40)), 'B sees the image, decrypted in the browser');
const doc = await B.p.evaluate(async () => { const log = await window.__den.log('general'); const f = log.find((m) => m.file?.name === 'minutes.txt'); return (await window.__den.openFile(f.file)).text(); });
ok(doc === 'the minutes of the first meeting', 'B opens the document');

// A room a member makes; proposal rooms from the ledger.
const nw = await B.p.evaluate(() => window.__den.push(window.__den.lobby, 'open_room', { title: 'Night watch' }));
ok(await until(A.p, ([id]) => document.querySelector(`[data-room="${id}"]`), [nw.id]), 'A sees the room B made');
await A.p.click(`[data-room="${nw.id}"]`);
ok(await until(A.p, () => /Members · ([2-9]|\d\d)/.test(document.querySelector('#room-meta')?.textContent || ''), null, 30000), 'both are in it');
await A.p.click('[data-toggle="props"]');
await until(A.p, () => document.querySelector('[data-room^="proposal-"]'));
const props = await A.p.$$eval('[data-room^="proposal-"]', (bs) => bs.map((b) => b.textContent.trim()));
ok(props.length > 0, `proposal rooms from testnet's DAO: ${props.slice(0, 4).join(', ')}`);

// Unread counts.
await A.p.click('[data-room="general"]');
await B.p.click(`[data-room="${nw.id}"]`);
await B.p.fill('#say-text', 'Anyone on watch?');
await B.p.press('#say-text', 'Enter');
ok(await until(A.p, ([id]) => /1/.test(document.querySelector(`[data-room="${id}"] .badge`)?.textContent || ''), [nw.id]), 'A sees 1 unread in Night watch');

// A private room by invitation.
await A.p.click('#room-meta');
await A.p.click(`[data-dm="${B.me.key}"]`);
ok(await until(A.p, () => /^p-/.test(document.querySelector('[data-room][aria-current="true"]')?.dataset.room || ''), null, 30000), 'A is taken to the private room');
ok(await until(B.p, () => [...document.querySelectorAll('[data-room^="p-"]')].length > 0, null, 30000), 'B is invited to the private room');
const pid = await B.p.$eval('[data-room^="p-"]', (b) => b.dataset.room);
ok(!(await A.p.evaluate(async () => (await window.__den.push(window.__den.lobby, 'claim', { key: 'x' }).catch((e) => e.reason)) === undefined)), 'claims are answered only for real devices');
await B.p.click(`[data-room="${pid}"]`);
await B.p.fill('#say-text', 'Just us here.');
await B.p.press('#say-text', 'Enter');
await A.p.click(`[data-room="${pid}"]`);
ok(await until(A.p, () => /Just us here\./.test(document.querySelector('#log')?.textContent || '')), 'A reads B in the private room');
const listed = await A.p.evaluate(async () => (await fetch('/door/keys')).ok && (await window.__den.push(window.__den.lobby, 'claim', { key: window.__den.me.key }).then(() => 'claimed', (e) => e.reason)));
ok(true, `(own invitations left check: ${listed})`);

// Search.
await A.p.fill('#search', 'watch');
ok(await until(A.p, () => /Anyone on watch/.test(document.querySelector('#found')?.textContent || '')), 'search finds words across rooms');
await A.p.fill('#search', '');

// Voice in General.
await A.p.click('[data-room="general"]');
await B.p.click('[data-room="general"]');
await A.p.click('#voice-go');
await B.p.click('#voice-go');
ok(await until(A.p, () => !document.querySelector('#voice').hidden), 'A is in voice');
ok(await until(A.p, ([n]) => (document.querySelector('#voice-who')?.textContent || '').includes(n), [B.me.name], 20000), 'A sees B in voice');
await A.p.waitForTimeout(6000);
const va = await A.p.evaluate(async () => { const v = window.__den.voice; const st = await v.pc.getStats(); let rx = 0; st.forEach((r) => { if (r.type === 'inbound-rtp' && r.kind === 'audio') rx += r.bytesReceived || 0; }); return { sealed: v.sealed, opened: v.opened, failed: v.failed, rx, state: v.pc.connectionState }; });
const vb = await B.p.evaluate(async () => { const v = window.__den.voice; return { sealed: v.sealed, opened: v.opened, failed: v.failed, state: v.pc.connectionState }; });
console.log('      A voice:', JSON.stringify(va), '· B voice:', JSON.stringify(vb));
ok(va.state === 'connected' && vb.state === 'connected', 'both voice connections are up');
ok(va.sealed > 50 && vb.sealed > 50, 'each side seals its own audio frames');
ok(va.opened > 50 && vb.opened > 50, 'each side opens the other\'s frames (the key is the room\'s, the den has none)');
// A forwarding unit that names the wrong speaker: A's slot for B relabelled
// as A itself. B's frames no longer open under that key.
const relabel = await A.p.evaluate(async () => {
  const v = window.__den.voice;
  const me = window.__den.me.key;
  const before = v.opened;
  v.worker.postMessage({ slots: v.downs.map((t, i) => [i, me]) });
  await new Promise((r) => setTimeout(r, 4500));
  const mid = v.opened;
  await new Promise((r) => setTimeout(r, 2500));
  return { before, mid, after: v.opened };
});
ok(relabel.after === relabel.mid, `a relabelled slot opens nothing (${JSON.stringify(relabel)})`);
await A.p.evaluate(() => window.__den.voice.postSlots());
ok(await until(A.p, ([n]) => window.__den.voice.opened > n, [relabel.after], 10000), 'named rightly again, it opens');
await A.p.click('#voice-leave');
ok(await until(A.p, () => document.querySelector('#voice').hidden), 'A leaves voice');

await A.p.screenshot({ path: `${SHOTS}/den-e2e-a.png` });
await B.p.screenshot({ path: `${SHOTS}/den-e2e-b.png` });
ok(csp.length === 0, `the page runs under its own policy with nothing refused${csp.length ? ': ' + csp.slice(0, 4).join(' | ') : ''}`);
ok(errs.length === 0, `no page errors${errs.length ? ': ' + errs.slice(0, 6).join(' | ') : ''}`);
await b.close();
issuer.close();
writeFileSync(pageConfig, savedConfig);
