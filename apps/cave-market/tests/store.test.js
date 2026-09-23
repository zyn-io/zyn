import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const root = new URL('../', import.meta.url);
const [html, css, js] = await Promise.all([
  readFile(new URL('index.html', root), 'utf8'),
  readFile(new URL('styles.css', root), 'utf8'),
  readFile(new URL('app.js', root), 'utf8'),
]);
const standaloneBuild = await readFile(new URL('../scripts/build-launchpad.mjs', import.meta.url), 'utf8');
const jadeHeaders = await readFile(new URL('../jade-headers', import.meta.url), 'utf8');
const [jadeHtml, jadeCss, jadeJs] = await Promise.all([
  readFile(new URL('../jade.html', import.meta.url), 'utf8'),
  readFile(new URL('../jade.css', import.meta.url), 'utf8'),
  readFile(new URL('../jade.js', import.meta.url), 'utf8'),
]);

test('keeps the NFT-focused Cave app separate from the Jade launchpad', () => {
  assert.match(html, /id="collection"/);
  assert.match(html, /The market should know less/);
  assert.match(html, /href="#market" data-route="market">Market/);
  assert.match(html, /href="#launch" data-route="launch">NFT Studio/);
  assert.match(html, /href="#portfolio" data-route="portfolio">Portfolio/);
  assert.doesNotMatch(html, /data-route="launchpad"/);
  assert.doesNotMatch(html + css + js, /Cave Signal|signal-token-form|token-launchpad|data-view="launchpad"/);
  assert.doesNotMatch(html, /href="#activity"|id="activity"|>Release status</);
});

test('launch configurator separates quantity policy from identity assignment', () => {
  for (const label of ['Shape the drop', 'Fixed', 'Weighted draw', 'Buyer chooses', 'Concealed shuffle', 'Instant random', 'Sequential']) {
    assert.match(html, new RegExp(label));
  }
  assert.match(html, /name="quantity-mode"/);
  assert.match(html, /name="identity-mode"/);
  assert.match(html, /Run 10,000-entry preview/);
  assert.match(html, /Connect backend to publish/);
  assert.doesNotMatch(js, /fetch\(|window\.zyn/);
});

test('launch policy preview is functional but cannot impersonate the backend draw', () => {
  assert.match(js, /version: 'cave-allocation\/v1'/);
  assert.match(js, /crypto\.getRandomValues/);
  assert.match(js, /Browser simulation \/ not the draw/);
  assert.match(js, /policy\.totalWeight !== 100/);
  assert.match(html, /<button type="button" disabled>Connect backend to publish<\/button>/);
});

test('market has familiar catalog controls without fabricating inventory', () => {
  for (const label of ['Search collections or token ID', 'Items', 'Collections', 'Activity', 'Recently listed', 'Filters']) {
    assert.match(html, new RegExp(label));
  }
  assert.match(html, /<b>0<\/b> live listings/);
  assert.match(html, /The market is sealed/);
  assert.match(html, /SECONDARY FEES/);
  assert.match(html, /50%[\s\S]*Future ZYN POL/);
  assert.match(html, /40%[\s\S]*Collection floor/);
  assert.match(html, /10%[\s\S]*Creator/);
  assert.match(html, /Treasury 26460/);
  assert.match(html, /Atomic rounding remainder goes to future ZYN POL/);
  assert.doesNotMatch(html, /owl-0057|mole-0102|shielded-byte|data-price|Buy now<\/button>/);
});

test('portfolio provides a private gallery without fake owned items', () => {
  for (const label of ['Your collection', 'Collected', 'Created', 'Gallery', 'Connect wallet', 'Nothing to show yet']) {
    assert.match(html, new RegExp(label));
  }
  assert.match(html, /private pieces are never published automatically/);
  assert.match(html, /id="portfolio" hidden/);
  assert.doesNotMatch(html, /portfolio-card|owned-token|token-id/);
});

test('hash routing preserves four NFT marketplace views', () => {
  assert.match(js, /location\.hash === '#market'/);
  assert.match(js, /location\.hash === '#launch'/);
  assert.match(js, /location\.hash === '#portfolio'/);
  assert.match(js, /portfolio\.hidden = view !== 'portfolio'/);
  assert.match(js, /launch\.hidden = view !== 'launch'/);
  assert.doesNotMatch(js, /launchpad/);
  assert.match(js, /window\.scrollTo\(0, 0\)/);
  assert.match(css, /\.market-layout/);
  assert.match(css, /\.portfolio-gallery/);
  assert.match(css, /\.launch-layout/);
  assert.match(css, /\.collection-intro/);
});

test('standalone fungible product uses the jade-yard identity and phase-one rules', () => {
  for (const label of ['Cave Jade', 'The Jade Gallery', 'Pick your stone', 'Rough', 'Open a window', 'Reveal the jade', 'Fixed supply', 'ZEC.zy', 'Curve sells stay open']) {
    assert.match(jadeHtml, new RegExp(label));
  }
  assert.match(jadeCss, /--jade: #06483d/);
  assert.match(jadeCss, /--metal:/);
  assert.match(jadeHtml, /id="launch-stone" type="submit" disabled>Place in the yard<\/button>/);
  assert.doesNotMatch(jadeHtml, /Cave Signal|Make the market move|landing page/i);
});

test('Jade reads live launches only through the injected provider', () => {
  assert.match(jadeJs, /renderGallery/);
  assert.match(jadeJs, /renderPreview/);
  assert.match(jadeJs, /\[0-9a-f\]\{64\}/);
  assert.match(jadeJs, /Nothing was published/);
  assert.match(jadeJs, /window\.zyn/);
  for (const method of ['zyn_curves', 'zyn_curve', 'zyn_curveQuote', 'zyn_launchCurve', 'zyn_buyCurve', 'zyn_sellCurve', 'zyn_quote', 'zyn_swap']) {
    assert.match(jadeJs, new RegExp(method));
  }
  assert.doesNotMatch(jadeJs, /fetch\(/);
  assert.doesNotMatch(jadeHtml, /Illustrative market data|Illustrative quote|seven day price/i);
  assert.match(jadeHtml, /deterministic bonding curve/i);
});

test('standalone build emits only the dedicated Jade product', () => {
  assert.match(standaloneBuild, /jade\.html/);
  assert.match(standaloneBuild, /jade\.css/);
  assert.match(standaloneBuild, /jade\.js/);
  assert.match(jadeHtml, /launch\.cavebro\.win/);
  assert.match(jadeHtml, /value="1,000,000,000" readonly/);
  assert.match(jadeHtml, /Ticker · exact Unicode/);
  assert.match(jadeHtml, /660M sold · 100 ZEC FDV/);
  assert.match(jadeJs, /feeBps: Math\.round/);
  assert.match(jadeJs, /bytes\.length <= 32/);
  assert.match(jadeJs, /0\.009 seeds the pair and 0\.001 goes to Treasury 26460/);
  assert.match(jadeJs, /OFFICIAL_ASSETS/);
  assert.match(jadeHeaders, /connect-src 'none'/);
  assert.match(jadeHeaders, /chrome-extension:/);
  assert.match(standaloneBuild, /jade-headers/);
  assert.doesNotMatch(standaloneBuild, /sourceHtml|sourceCss|launchpadMarkup|app\.js/);
});

test('trading goes through Nap and nowhere else', async () => {
  const { readFileSync } = await import('node:fs');
  const trade = readFileSync(new URL('../trade.js', import.meta.url), 'utf8');
  // The page itself reads one thing: the chain's public order book, which
  // names no holder. Everything about the holder, and every write, is a
  // request to their Nap, and every write is one Nap asks them to approve.
  assert.equal((trade.match(/fetch\(/g) || []).length, 1, 'one fetch, for the public order book');
  assert.match(trade, /method: 'zyn_offers'/);
  assert.match(trade, /credentials: 'omit'/);
  assert.doesNotMatch(trade, /XMLHttpRequest|WebSocket|window\.zyn/);
  for (const m of ['zyn_nftItems', 'zyn_owed', 'zyn_nftList', 'zyn_nftBuy', 'zyn_nftCancel', 'zyn_nftRedeem', 'zyn_collect']) {
    assert.match(trade, new RegExp(m));
  }
  // What the provider says is shape-checked and escaped before it is shown.
  assert.match(trade, /isHex\(o\.item\)/);
  assert.match(trade, /esc\(itemName/);
  assert.match(html, /trade\.js/);
});


test('the store connects through Nap Connect, the same file Nap ships', async () => {
  const { readFileSync } = await import('node:fs');
  const here = readFileSync(new URL('../nap-connect.js', import.meta.url), 'utf8');
  const canonical = readFileSync(new URL('../../nap-connect/nap-connect.js', import.meta.url), 'utf8');
  assert.equal(here, canonical, 'apps/cave-market/nap-connect.js must be a verbatim copy of apps/nap-connect/nap-connect.js');
  const wallet = readFileSync(new URL('../wallet.js', import.meta.url), 'utf8');
  assert.match(wallet, /from '\.\/nap-connect\.js'/);
  assert.match(html, /data-wallet-dialog[^>]*>\s*<span><\/span> Connect wallet/);
  // Nothing the wallet reports is put on the page unescaped.
  assert.doesNotMatch(wallet, /\$\{(?!esc\()[^}]*(session|info)\.[a-z]+/);
});
