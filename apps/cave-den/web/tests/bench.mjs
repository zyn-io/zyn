// What a room's size costs, with the page's own MLS library (WebAssembly, as
// in the browser) and the den's own check of commits (server/native): for
// rooms of 50 to 1000 members, the time a device takes to apply a commit, to
// join, and to save its state, and the time the den takes to check a commit.
//
//   ./build.sh dist && node tests/bench.mjs [sizes…]
//   (then, for the den's side: cd ../server && mix run ../web/tests/bench.exs)
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';

const init = (await import('../dist/den/den_mls.js')).default;
const { Den } = await import('../dist/den/den_mls.js');
await init({ module_or_path: readFileSync(new URL('../dist/den/den_mls_bg.wasm', import.meta.url)) });

const sizes = process.argv.slice(2).map(Number).filter(Boolean);
const OUT = `${tmpdir()}/den-bench`;
mkdirSync(OUT, { recursive: true });
const ms = (f) => { const t = performance.now(); const r = f(); return [performance.now() - t, r]; };
const b64 = (u8) => Buffer.from(u8).toString('base64');
const rows = [];

for (const n of sizes.length ? sizes : [50, 200, 500, 1000]) {
  const room = 'general';
  const creator = new Den();
  creator.create(room);
  // Everyone else in one commit (the fastest way to a room this size).
  const others = Array.from({ length: n - 1 }, () => new Den());
  const kps = others.map((d) => JSON.parse(d.key_packages(1))[0]);
  const [addMs] = ms(() => creator.add(room, JSON.stringify(kps), JSON.stringify(others.map((d) => d.key()))));
  creator.confirm(room);
  const info = creator.group_info(room);
  // A newcomer joins by external commit; the creator (any member) applies it.
  const newcomer = new Den();
  const [joinMs, join] = ms(() => newcomer.join(room, info));
  const [applyJoinMs] = ms(() => creator.receive(room, join));
  // The newcomer refreshes its keys: a commit with a path, applied by a member.
  const update = newcomer.refresh(room);
  newcomer.confirm(room);
  const [applyUpdateMs] = ms(() => creator.receive(room, update));
  // Words: sealed by one, opened by another.
  const words = newcomer.send(room, JSON.stringify({ v: 1, t: 'msg', id: 'x', text: 'hello' }));
  const [openMs] = ms(() => creator.receive(room, words));
  // What a device keeps, written after every message it takes.
  const [saveMs, saved] = ms(() => creator.save());
  const info2 = creator.group_info(room);
  writeFileSync(`${OUT}/${n}.json`, JSON.stringify({ info: b64(info), join: b64(join), info2: b64(info2) }));
  rows.push({ members: n, add_all_ms: Math.round(addMs), join_ms: Math.round(joinMs), apply_join_ms: +applyJoinMs.toFixed(1), apply_update_ms: +applyUpdateMs.toFixed(1), open_words_ms: +openMs.toFixed(2), save_ms: +saveMs.toFixed(1), saved_kb: Math.round(saved.length / 1024), group_info_kb: Math.round(info2.length / 1024), commit_bytes: update.length });
  others.forEach((d) => d.free());
  console.log(JSON.stringify(rows.at(-1)));
}
console.table(rows);
console.log(`(fixtures for the den's side in ${OUT})`);
