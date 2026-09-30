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

test('standalone fungible product uses the jade-yard identity and the launchpad essentials', () => {
  for (const label of ['Cave Jade', 'Pick a stone', 'Place a stone', 'Market cap', 'Bonding curve', 'Fills', 'Portfolio', 'ZEC.zy']) {
    assert.match(jadeHtml, new RegExp(label));
  }
  // Sorts, never defaulting to an empty tab; candles with timeframes; the trade panel's aids.
  for (const sort of ['movers', 'new', 'mcap', 'last', 'near', 'grad']) assert.match(jadeHtml, new RegExp(`data-sort="${sort}"`));
  assert.match(jadeHtml, /data-sort="movers" class="active"/);
  for (const tf of [60, 300, 900, 3600, 14400, 86400]) assert.match(jadeHtml, new RegExp(`data-tf="${tf}"`));
  for (const id of ['trade-presets', 'trade-slip', 'trade-min', 'trade-balance', 'curve-bar', 'coin-age', 'stone-image', 'stone-description']) assert.match(jadeHtml, new RegExp(`id="${id}"`));
  // Privacy is stated, not hidden: no holder list, no wallets on fills.
  assert.match(jadeHtml, /no wallets, by design/);
  assert.match(jadeHtml, /Holders<\/dt><dd id="coin-holders">private/);
  assert.match(jadeCss, /--jade: #06483d/);
  assert.match(jadeCss, /--metal:/);
  assert.match(jadeCss, /@media \(max-width: 760px\)/);
  assert.match(jadeHtml, /id="launch-stone" class="j-primary" type="submit" disabled>Create coin<\/button>/);
  assert.doesNotMatch(jadeHtml, /ZynZap|Cave Signal|Make the market move|landing page/i);
});

test('Jade reads the public yard from the chain and acts only through Nap (§141)', () => {
  assert.match(jadeJs, /renderGallery/);
  assert.match(jadeJs, /renderPreview/);
  assert.match(jadeJs, /\[0-9a-f\]\{64\}/);
  // Reads: the chain's public RPC, and this site's own content and price endpoints — nothing else.
  const fetched = [...jadeJs.matchAll(/fetch\(([^,)]+)/g)].map((m) => m[1].trim());
  assert.ok(fetched.length >= 3);
  for (const target of fetched) assert.match(target, /^(RPC|'\/api\/zec-usd'|`\/api\/content\/\$\{[a-z.]+\}`)$/, target);
  for (const method of ['zyn_market', 'zyn_curveStats', 'zyn_history', 'zyn_status']) assert.match(jadeJs, new RegExp(`rpc\\('${method}'`));
  assert.match(jadeJs, /credentials: 'omit'/);
  // Every write through Nap Connect, approved in Nap.
  assert.match(jadeJs, /from '\.\/nap-connect\.js'/);
  for (const method of ['zyn_curveCreate', 'zyn_curveBuy', 'zyn_curveSell', 'zyn_swap', 'zyn_collect', 'zyn_curveHoldings']) assert.match(jadeJs, new RegExp(`'${method}'`));
  assert.doesNotMatch(jadeJs, /window\.zyn/);
  // Slippage floors ride on both sides of a curve trade.
  assert.match(jadeJs, /minTokens: decimal\(/);
  assert.match(jadeJs, /minZec: decimal\(/);
  // The curve is consensus: the page mirrors it, it does not choose it.
  assert.match(jadeJs, /lots: 80_000, lotTokens: 12_500n, baseZat: 7_721n, slopeZat: 1n, graduateAt: 51_098/);
  // Chain and wallet overrides are for localhost only.
  assert.match(jadeJs, /LOCAL && params\.get\('nap'\)/);
  assert.match(jadeJs, /LOCAL && params\.get\('rpc'\)/);
  // Pictures are only ever data: images of the four accepted kinds.
  assert.match(jadeJs, /data:image\\\/\(png\|jpeg\|webp\|gif\);base64/);
  // The chart library is vendored, not fetched from a CDN.
  assert.match(jadeJs, /from '\.\/vendor\/lightweight-charts\.js'/);
});

test('standalone build emits only the dedicated Jade product', () => {
  assert.match(standaloneBuild, /jade\.html/);
  assert.match(standaloneBuild, /jade\.css/);
  assert.match(standaloneBuild, /jade\.js/);
  assert.match(standaloneBuild, /nap-connect\.js/);
  assert.match(standaloneBuild, /vendor\/lightweight-charts\.js/);
  assert.match(standaloneBuild, /LIGHTWEIGHT-CHARTS-LICENSE/);
  assert.match(jadeHtml, /launch\.cavebro\.win/);
  assert.match(jadeHtml, /1,000,000,000/);
  assert.match(jadeHtml, /51,098 of 80,000 lots/);
  assert.doesNotMatch(jadeHtml, /stone-fee|stone-dev-buy|660M/);
  // Nap's window must be able to answer the page that opened it.
  assert.match(jadeHeaders, /Cross-Origin-Opener-Policy: same-origin-allow-popups/);
  assert.match(jadeHeaders, /connect-src 'self' https:\/\/zyn-rpc\.cavebro\.win;/);
  assert.match(jadeHeaders, /script-src 'self' chrome-extension:/);
  assert.match(jadeHeaders, /frame-ancestors 'none'/);
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
