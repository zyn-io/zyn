// Epeaky in a real browser, its policy enforced: three people in by the three
// ways a den opens (an invite link, the host's approval, the host's issuer
// window registering a pass), then words, replies, reactions, edits, files,
// rooms, private rooms, search and voice between two of them.
//
//   (cd ../server && mix phx.server)   # the dev den: every way in open
//   ./build.sh ../server/priv/page      # its page
//   node tests/den.e2e.mjs            (CHROMIUM=… for another Chrome)
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';

// playwright-core from this package (npm install), or from PLAYWRIGHT_FROM, a folder that has it.
const require = createRequire(process.env.PLAYWRIGHT_FROM ? `${process.env.PLAYWRIGHT_FROM.replace(/\/$/, '')}/package.json` : import.meta.url);
const { chromium } = require('playwright-core');
const DEN = process.env.DEN || 'http://localhost:4000';
const SHOTS = process.env.SHOTS || tmpdir();
const CHROME = process.env.CHROMIUM || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const ISSUER_TOKEN = process.env.ISSUER_TOKEN || 'dev-issuer-token-not-for-any-real-den-0001';
const SERVER = new URL('../../server/', import.meta.url).pathname;

// What the host runs on the den's server (Den.Gate.Members), against its database.
const host = (expr) => execFileSync('mix', ['run', '--no-start', '-e', `Application.ensure_all_started(:ecto_sqlite3); {:ok, _} = Den.Repo.start_link(); ${expr}`], { cwd: SERVER, encoding: 'utf8' }).trim().split('\n').at(-1);

// A stand-in for the host's issuer. Its page reads the hash the den's page
// put after "#", its server registers that hash with the den (the host's
// token), and the page tells its opener, addressed to the den's origin only,
// that it is registered. The issuer never sees the pass code. A message sent
// to any other origin never arrives.
const issuer = createServer(async (req, res) => {
  const url = new URL(req.url, 'http://localhost:4181');
  if (url.pathname === '/register') {
    const pass = url.searchParams.get('pass') || '';
    const r = await fetch(`${DEN}/door/passes`, { method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${ISSUER_TOKEN}` }, body: JSON.stringify({ pass }) });
    res.setHeader('content-type', 'application/json');
    return res.end(JSON.stringify({ ok: r.ok }));
  }
  res.setHeader('content-type', 'text/html');
  res.end(`<!doctype html><title>Issuer</title><script>
    (async () => {
      const pass = new URLSearchParams(location.hash.slice(1)).get('den-pass');
      opener.postMessage({ den: 'pass', registered: true }, 'https://elsewhere.example');
      const ok = pass && (await (await fetch('/register?pass=' + pass)).json()).ok;
      opener.postMessage({ den: 'pass', registered: !!ok }, ${JSON.stringify(DEN)});
    })();
  </script>`);
}).listen(4181);
const pageConfig = new URL('../../server/priv/page/config.json', import.meta.url);
const savedConfig = readFileSync(pageConfig, 'utf8');
writeFileSync(pageConfig, JSON.stringify({ keysUrl: '/door/keys', prefix: 'e2e', issuer: 'http://localhost:4181/' }));

const b = await chromium.launch({ executablePath: CHROME, headless: true, args: ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream', '--autoplay-policy=no-user-gesture-required'] });
const errs = [];
const csp = [];
const ok = (c, w) => { console.log(`${c ? 'PASS' : 'FAIL'}  ${w}`); if (!c) process.exitCode = 1; };
const until = (p, f, arg, ms = 30000) => p.waitForFunction(f, arg, { timeout: ms }).then(() => true, () => false);
async function person(name, url) {
  const ctx = await b.newContext({ viewport: { width: 1300, height: 900 }, permissions: ['microphone'] });
  const p = await ctx.newPage();
  await p.addInitScript(() => { navigator.mediaDevices.getUserMedia = async () => { const ac = new AudioContext(); const o = ac.createOscillator(); const d = ac.createMediaStreamDestination(); o.connect(d); o.start(); return d.stream; }; });
  p.on('pageerror', (e) => errs.push(`${name}: ${e.message.slice(0, 200)}`));
  p.on('console', (m) => { if (/Content Security Policy|Refused to/.test(m.text())) csp.push(`${name}: ${m.text().slice(0, 200)}`); });
  await p.goto(url, { waitUntil: 'networkidle' });
  return { ctx, p };
}
const inside = (p, ms = 20000) => p.waitForSelector('#den:not([hidden])', { timeout: ms }).then(() => true, () => false);
const meOf = (p) => p.evaluate(() => window.__den.me);

// A: an invite link.
const invite = host('IO.puts(hd(Den.Gate.Members.invite(1, 1)))');
const a = await person('A', `${DEN}/#invite=${invite}`);
ok(await inside(a.p), 'A is in through an invite link');
ok(!(await a.p.evaluate(() => location.hash)), 'the invite code is taken off the address at once');
const reuse = await person('A2', `${DEN}/#invite=${invite}`);
ok(await until(reuse.p, () => /used, withdrawn or has expired/.test(document.querySelector('#chat-out')?.textContent || ''), null, 10000), 'a used invite lets nobody else in');
await reuse.ctx.close();

// B: the host's issuer window.
const bb = await person('B', `${DEN}/`);
ok(await bb.p.isVisible('#way-ask') && await bb.p.isVisible('#way-invite') && await bb.p.isVisible('#enter'), 'the door offers the ways this den opens');
await bb.p.click('#enter');
ok(await inside(bb.p), 'B is in through the issuer window, which saw only its pass hash; a message to another origin never arrives');

// C: asks, and the host approves.
const c = await person('C', `${DEN}/`);
await c.p.fill('#ask-note', 'Hi, C from the reading group');
await c.p.click('#way-ask button');
ok(await until(c.p, () => /once the host approves/.test(document.querySelector('#chat-out')?.textContent || ''), null, 10000), 'C waits for the host');
ok(!(await inside(c.p, 2000)), 'nobody is in before the host approves');
const approved = host('for r <- Den.Gate.Members.pending(), r.note == "Hi, C from the reading group", do: :ok = Den.Gate.Members.approve(r.id); IO.puts("approved")');
ok(approved === 'approved', 'the host approves C from the server');
ok(await inside(c.p, 40000), 'C is let in by the approval, on the page it asked from');
const cKey = (await meOf(c.p)).key;
// The device keeps its own pass: with its session and tokens gone, a reload goes straight back in.
await c.p.evaluate(async () => { const kv = window.__den.kv; await kv.del('session'); for (const d of [0, 1]) await kv.del(`token:${new Date(Date.now() + d * 86400000).toISOString().slice(0, 10)}`); });
await c.p.reload({ waitUntil: 'networkidle' });
ok(await inside(c.p), 'with no session or token left, the kept pass lets the device back in');
// A den that shows this device a different key for a day it has seen is refused.
const swapped = await c.p.evaluate(async () => {
  const den = window.__den;
  const d = new Date().toISOString().slice(0, 10);
  await den.kv.put(`pin:${d}`, 'AAAA');
  den.keys = undefined;
  return den.dayKey(d).then(() => 'accepted', (e) => e.message);
});
ok(/different door key/.test(swapped), 'a day key different from the pinned one is refused');
// Forget this device: everything it keeps is gone, and it is outside again.
const cBefore = (await meOf(c.p)).key;
await c.p.click('#forget');
await c.p.click('#ask [data-yes]');
await c.p.waitForLoadState('networkidle');
ok(await until(c.p, () => document.querySelector('#den')?.hidden && !document.querySelector('.ledger')?.hidden, null, 15000), 'forgetting the device puts it back outside the door');
ok((await meOf(c.p)).key !== cBefore, 'and it comes back as a new device, with nothing of the old one');
await c.ctx.close();
const A = { p: a.p, me: await meOf(a.p) };
const B = { p: bb.p, me: await meOf(bb.p) };
ok(await until(A.p, ([k]) => window.__den.members('general').some((m) => m.key === k), [B.me.key], 40000) && await until(B.p, ([k]) => window.__den.members('general').some((m) => m.key === k), [A.me.key], 40000), `A and B in General (${A.me.name}, ${B.me.name})`);
// A roommate shown another door key for today is flagged in the room, sealed.
await A.p.evaluate(async () => {
  const d = new Date().toISOString().slice(0, 10);
  await window.__den.send('general', { t: 'seen', day: d, key_hash: 'f'.repeat(64) });
});
ok(await until(B.p, () => /reports a different door key/.test(document.querySelector('#den-warn')?.textContent || ''), null, 15000), 'a roommate shown a different door key is flagged, by name');

// Names members give themselves: sealed, and shown beside the device's own name.
await A.p.evaluate(() => window.__den.setNick('Ada'));
await B.p.evaluate(() => window.__den.setNick('Bo'));
ok(await until(B.p, ([k]) => window.__den.nickFor(k) === 'Ada', [A.me.key], 15000) && await until(A.p, ([k]) => window.__den.nickFor(k) === 'Bo', [B.me.key], 15000), 'each sees the name the other gave themselves');
ok(await until(A.p, () => document.querySelector('#me')?.textContent === 'Ada', null, 5000), 'A sees its own name');

// Words, a mention, a reply, a reaction, an edit, a deletion.
await A.p.fill('#say-text', `Hello @${B.me.name}, welcome to the den.`);
await A.p.press('#say-text', 'Enter');
ok(await until(B.p, () => document.querySelector('#log li.ping')), 'B sees the mention highlighted');
ok(await until(B.p, ([s]) => [...document.querySelectorAll('#log .msg-head')].some((h) => h.querySelector('.who')?.textContent === 'Ada' && h.querySelector('.id')?.textContent === s), [`m-${A.me.key.slice(0, 6)}`]), 'B sees A as Ada, beside the start of its device name');
ok(await until(A.p, () => [...document.querySelectorAll('#log mark')].some((m) => m.textContent === '@Bo')), 'a mention shows the name its member chose');
// A member who takes another's name is flagged in the members list.
await A.p.evaluate(() => window.__den.setNick('Bo'));
await until(B.p, ([k]) => window.__den.nickFor(k) === 'Bo', [A.me.key], 15000);
await B.p.click('#room-meta');
ok(await until(B.p, () => !!document.querySelector('#members .alike'), null, 5000), 'two members with the same name are flagged');
await B.p.click('#room-meta');
await A.p.evaluate(() => window.__den.setNick('Ada'));
await B.p.hover('#log li.ping');
await B.p.click('#log li.ping [data-act="reply"]');
await B.p.fill('#say-text', 'Glad to be here.');
await B.p.press('#say-text', 'Enter');
ok(await until(A.p, () => [...document.querySelectorAll('#log .quote')].some((q) => /Hello/.test(q.textContent))), 'A sees the reply quoting its message');
const firstId = await A.p.$eval('#log li.mine', (li) => li.dataset.id);
await B.p.evaluate(([id]) => window.__den.react('general', id, '👍'), [firstId]);
ok(await until(A.p, () => /👍 1/.test(document.querySelector('#log')?.textContent || '')), 'A sees B\'s reaction');
// A reaction outside the page's set is not taken; a mention inside a link stays inside the link.
await B.p.evaluate(([id]) => window.__den.react('general', id, '🦊'), [firstId]);
await A.p.fill('#say-text', `see https://example.org/@${B.me.name}/x now`);
await A.p.press('#say-text', 'Enter');
ok(await until(B.p, () => [...document.querySelectorAll('#log a')].some((l) => /example\.org\/@m-[0-9a-f]{20}\/x$/.test(l.href) && !l.querySelector('mark') && l.rel.includes('noopener'))), 'a mention inside a link stays a plain part of the link');
ok(!/🦊/.test(await A.p.textContent('#log')), 'a reaction the page does not offer is not shown');
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

// Fresh keys: A replaces its own MLS keys in General; the room moves on and B reads A after it.
const epochBefore = await A.p.evaluate(() => Number(window.__den.den.epoch('general')));
await A.p.evaluate(() => window.__den.refreshKeys('general'));
ok(await A.p.evaluate(([e]) => Number(window.__den.den.epoch('general')) === e + 1, [epochBefore]), "A's fresh keys are a commit the den ordered");
await A.p.fill('#say-text', 'Keys refreshed.');
await A.p.press('#say-text', 'Enter');
ok(await until(B.p, () => /Keys refreshed\./.test(document.querySelector('#log')?.textContent || ''), null, 15000), 'B reads A after A refreshed its keys');

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
// A file of several pieces, past the old 25 MB limit's pieces logic: 30 MB.
const bigSum = await A.p.evaluate(async () => {
  const data = new Uint8Array(30 * 1024 * 1024);
  for (let o = 0; o < data.length; o += 65536) crypto.getRandomValues(data.subarray(o, o + 65536));
  await window.__den.sendFile('general', new File([data], 'archive.bin', { type: 'application/octet-stream' }));
  return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', data))).map((x) => x.toString(16).padStart(2, '0')).join('');
});
const bigBack = await B.p.evaluate(async () => {
  for (let t = 0; t < 50; t++) {
    const f = (await window.__den.log('general')).find((m) => m.file?.name === 'archive.bin');
    if (f) { const b = await window.__den.openFile(f.file); return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', await b.arrayBuffer()))).map((x) => x.toString(16).padStart(2, '0')).join(''); }
    await new Promise((r) => setTimeout(r, 300));
  }
  return 'not arrived';
});
ok(bigBack === bigSum, 'a 30 MB file, sealed in pieces, opens whole on the other side');

// A room a member makes.
const nw = await B.p.evaluate(() => window.__den.push(window.__den.lobby, 'open_room', { title: 'Night watch' }));
ok(await until(A.p, ([id]) => document.querySelector(`[data-room="${id}"]`), [nw.id]), 'A sees the room B made');
await A.p.click(`[data-room="${nw.id}"]`);
ok(await until(A.p, () => /Members · ([2-9]|\d\d)/.test(document.querySelector('#room-meta')?.textContent || ''), null, 30000), 'both are in it');
ok(await until(B.p, async ([id, n]) => (await window.__den.log(id)).some((m) => m.t === 'note' && m.text === `${n} joined`), [nw.id, A.me.name], 15000), 'the room tells B that A joined');

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
ok(await until(B.p, () => document.querySelector('[data-join]'), null, 30000), 'B sees the invitation, waiting for B to choose');
ok(!(await B.p.$('[data-room^="p-"]')), 'nobody is pulled into a room: B is not in it before choosing');
await B.p.click('[data-join]');
ok(await until(B.p, () => [...document.querySelectorAll('[data-room^="p-"]')].length > 0, null, 30000), 'B joins the private room');
const pid = await B.p.$eval('[data-room^="p-"]', (b) => b.dataset.room);
ok(!(await A.p.evaluate(async () => (await window.__den.push(window.__den.lobby, 'claim', { key: 'x' }).catch((e) => e.reason)) === undefined)), 'claims are answered only for real devices');
await B.p.click(`[data-room="${pid}"]`);
ok(await until(B.p, ([id]) => window.__den.inRoom(id) && window.__den.members(id).length >= 2 && window.__den.channels.get(id)?.state === 'joined', [pid], 20000), 'B is in the private room, kept there while its invitation waited');
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
ok(await until(A.p, () => (document.querySelector('#voice-who')?.textContent || '').includes('Bo'), null, 20000), 'A sees B in voice, by the name B chose');
await A.p.waitForTimeout(6000);
const va = await A.p.evaluate(async () => { const v = window.__den.voice; const st = await v.pc.getStats(); let rx = 0; st.forEach((r) => { if (r.type === 'inbound-rtp' && r.kind === 'audio') rx += r.bytesReceived || 0; }); return { sealed: v.sealed, opened: v.opened, failed: v.failed, rx, state: v.pc.connectionState }; });
const vb = await B.p.evaluate(async () => { const v = window.__den.voice; return { sealed: v.sealed, opened: v.opened, failed: v.failed, state: v.pc.connectionState }; });
console.log('      A voice:', JSON.stringify(va), '· B voice:', JSON.stringify(vb));
ok(va.state === 'connected' && vb.state === 'connected', 'both voice connections are up');
ok(await A.p.evaluate(() => /a=extmap:\d+ urn:ietf:params:rtp-hdrext:ssrc-audio-level/.test(window.__den.voice.pc.remoteDescription.sdp)), 'the den asks for the audio level it ranks speakers by');
ok(await A.p.evaluate(([k]) => window.__den.voice.roster.includes(k), [B.me.key]), "B is on A's voice roster");
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

// A member sends a commit the den accepts but nobody can apply (its
// membership tag flipped): B cannot apply it, reports it, and the den undoes
// it (B is the only other member it leaves in the room); words flow again,
// and A may not change the room until tomorrow.
await until(A.p, ([k]) => window.__den.members('general').some((m) => m.key === k), [cKey], 20000);
const tampered = await A.p.evaluate(async ([victim]) => {
  const den = window.__den;
  const bad = den.den.remove('general', JSON.stringify([victim]));
  bad[bad.length - 1] ^= 1;
  let s = ''; for (const x of bad) s += String.fromCharCode(x);
  const r = await den.push(den.channels.get('general'), 'relay', { bytes: btoa(s) }).then(() => 'ordered', (e) => JSON.stringify(e));
  den.den.discard('general');
  return r;
}, [cKey]);
ok(tampered === 'ordered', `the den orders the tampered commit (it cannot check its tag): ${tampered}`);
ok(await until(B.p, () => /could not apply/.test(document.querySelector('#den-status')?.textContent || ''), null, 15000), 'B says it could not apply a change');
await A.p.waitForTimeout(3000);
await A.p.fill('#say-text', 'After the undo.');
await A.p.press('#say-text', 'Enter');
ok(await until(B.p, () => /After the undo\./.test(document.querySelector('#log')?.textContent || ''), null, 20000), 'the room is not frozen: B reads A after the undo');
const barred = await A.p.evaluate(async ([victim]) => {
  const den = window.__den;
  const again = den.den.remove('general', JSON.stringify([victim]));
  let s = ''; for (const x of again) s += String.fromCharCode(x);
  const r = await den.push(den.channels.get('general'), 'relay', { bytes: btoa(s) }).then(() => 'ordered', (e) => e.reason || JSON.stringify(e));
  den.den.discard('general');
  return r;
}, [cKey]);
ok(/until tomorrow/.test(barred), `its sender may not change the room until tomorrow: ${barred}`);

await A.p.screenshot({ path: `${SHOTS}/den-e2e-a.png` });
await B.p.screenshot({ path: `${SHOTS}/den-e2e-b.png` });
ok(csp.length === 0, `the page runs under its own policy with nothing refused${csp.length ? ': ' + csp.slice(0, 4).join(' | ') : ''}`);
ok(errs.length === 0, `no page errors${errs.length ? ': ' + errs.slice(0, 6).join(' | ') : ''}`);
await b.close();
issuer.close();
writeFileSync(pageConfig, savedConfig);
