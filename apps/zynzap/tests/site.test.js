import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const provider = readFileSync(new URL('../../extension/src/inpage.js', import.meta.url), 'utf8');
const background = readFileSync(new URL('../../extension/src/background.js', import.meta.url), 'utf8');
const headers = readFileSync(new URL('../_headers', import.meta.url), 'utf8');

test('is a standalone mainnet bridge rather than a blank app shell', () => {
  assert.match(html, /ZEC in\./);
  assert.match(html, /ZEC\.zy/);
  assert.match(html, /MAINNET_CHAIN = 26460/);
  assert.match(html, /s\.network !== 'mainnet'/);
  assert.match(html, /id="connection-gate"/);
  assert.match(html, /<div id="app-shell">/);
  assert.match(html, /Read-only preview/);
  assert.match(html, /function showPreview\(message\)/);
  assert.doesNotMatch(html, /\$\('app-shell'\)\.hidden = true/);
  assert.match(html, /showTab\('bridge'\)/);
});

test('mainnet hides Solana, not the Zcash bridge', () => {
  assert.match(html, /<section id="zcash-panel">\s*<h2>Zcash/);
  assert.match(html, /<section id="solana-panel">\s*<h2>Solana/);
  assert.match(html, /\$\('solana-panel'\)\.hidden = S\.network === 'mainnet'/);
  assert.doesNotMatch(html, /\$\('zcash-panel'\)\.hidden/);
});

test('hosted mode uses only the injected provider', () => {
  assert.match(html, /window\.zyn && window\.zyn\.isZynZap/);
  assert.match(html, /if \(sameOrigin\) return http/);
  assert.match(html, /throw new Error\('Install the Nap Wallet extension/);
  assert.doesNotMatch(html, /zec-rpc\.cavebro\.win/);
  assert.doesNotMatch(html, /zyn-rpc\.cavebro\.win/);
});

test('ships self-hosted fonts and no tracking origins', () => {
  assert.match(html, /\/fonts\/BricolageGrotesque\.ttf/);
  assert.doesNotMatch(html, /fonts\.googleapis\.com|fonts\.gstatic\.com/);
  assert.doesNotMatch(html, /analytics|gtag|segment\.com/i);
});

test('provider and extension retain approval-gated bridge and launch methods', () => {
  assert.match(provider, /isZynZap: true/);
  for (const method of ['zyn_deposit', 'zyn_withdraw', 'zyn_bind', 'zyn_launchCurve', 'zyn_buyCurve', 'zyn_sellCurve']) {
    assert.match(background, new RegExp(`${method}:\\s+\\{ read: false`));
  }
  for (const method of ['zyn_deposit', 'zyn_withdraw', 'zyn_bind']) assert.match(html, new RegExp(`m: '${method}'`));
  for (const method of ['zyn_curves', 'zyn_curve', 'zyn_curveQuote']) assert.match(background, new RegExp(`${method}:\\s+\\{ read: true`));
  assert.match(background, /Pay 0\.01 ZEC\.zy launch fee/);
});

test('CSP permits provider injection but no page-originated network calls', () => {
  assert.match(headers, /script-src[^\n]+chrome-extension:/);
  assert.match(headers, /script-src[^\n]+moz-extension:/);
  assert.match(headers, /connect-src 'none'/);
});

test('does not render the superseded bridge-fee and halving launch', () => {
  assert.doesNotMatch(html, /id="launch"|id="markets"/);
  assert.doesNotMatch(html, /halving_blocks|genesis pot|rebated in ZYN|z\.launch/);
  assert.match(html, /\['Protocol bridge fee', 'none'\]/);
});
