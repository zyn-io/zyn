// Builds dist/ (Chrome, Edge, Brave) and dist-firefox/ from src/ plus the
// shared page. Extension pages forbid inline scripts, so the page's script
// becomes popup.js with the app's address set first; the rest of the page is
// the popup as-is.
import { readFileSync, writeFileSync, mkdirSync, cpSync, copyFileSync, rmSync } from 'node:fs';

const CHROME = 'dist';
const FIREFOX = 'dist-firefox';

function stage(dir) {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  cpSync('src', dir, { recursive: true });
  cpSync('icons', `${dir}/icons`, { recursive: true });
  cpSync('../../zynzapd/ui/fonts', `${dir}/fonts`, { recursive: true });
  // The page and its assets travel together. The daemon serves the mark from
  // its own route; in the popup it has to be a real file next to the page, or
  // the header renders an empty frame.
  copyFileSync('../../zynzapd/ui/nap-mark.png', `${dir}/nap-mark.png`);

  let html = readFileSync('../../zynzapd/ui/index.html', 'utf8');
  const m = html.match(/<script>([\s\S]*?)<\/script>\s*<\/body>/);
  if (!m) throw new Error('no inline script in the page');
  const js = "window.ZYN_API_BASE = 'http://127.0.0.1:8977';\n" + m[1];
  // The gate runs after the app's own script so it can read ZYN_API_BASE, and
  // covers the page when the desktop app is unreachable.
  html = html.replace(
    m[0],
    '<script src="popup.js"></script>\n<script src="agent-gate.js"></script>\n</body>'
  );
  // A popup is at most 800px wide; the page lays itself out to that.
  html = html.replace(
    '</style>',
    'html, body { width: 420px; min-height: 560px; } main { padding: 14px; grid-template-columns: 1fr; gap: 12px; }\n</style>'
  );
  writeFileSync(`${dir}/popup.html`, html);
  // The ZIP-321 parser the browser wallet uses, as a classic script in front
  // of the background: service workers and Firefox event pages both load it
  // without module support, and there is one parser to get right.
  const zip321 = readFileSync('../nap-web/src/zip321.js', 'utf8').replace(/^export /gm, '');
  writeFileSync(`${dir}/background.js`, `// --- zip321.js (from apps/nap-web) ---\n${zip321}\n// --- background.js ---\n${readFileSync('src/background.js', 'utf8')}`);
  writeFileSync(`${dir}/popup.js`, js);
}

const manifest = JSON.parse(readFileSync('src/manifest.json', 'utf8'));

stage(CHROME);

// Firefox implements MV3 with event pages rather than service workers, and
// requires an explicit add-on id. Everything else in the manifest is shared,
// so the two builds cannot drift apart: this is the whole difference.
stage(FIREFOX);
const firefox = structuredClone(manifest);
delete firefox.background.service_worker;
firefox.background = { scripts: ['background.js'] };
firefox.browser_specific_settings = {
  gecko: { id: 'nap-wallet@cavebro.win', strict_min_version: '121.0' },
};
writeFileSync(`${FIREFOX}/manifest.json`, JSON.stringify(firefox, null, 2) + '\n');

console.log(`${CHROME}/ ready — load it unpacked from chrome://extensions`);
console.log(`${FIREFOX}/ ready — load it from about:debugging`);
