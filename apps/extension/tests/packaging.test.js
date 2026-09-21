import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

const read = async (p) => JSON.parse(await readFile(new URL(p, import.meta.url), 'utf8'));

/// Two stores, two background models, one extension.
///
/// Chrome's MV3 runs the background as a service worker; Firefox runs it as an
/// event page and wants an explicit add-on id. Those are the *only* legitimate
/// differences, and they are the ones a build is allowed to introduce. Anything
/// else that diverges is drift: a permission added for one store and not the
/// other ships two different extensions under one name, and the store that got
/// the smaller manifest is the one that breaks in the user's hands.
test('the two builds differ only in how the background runs', async () => {
    const chrome = await read('../dist/manifest.json');
    const firefox = await read('../dist-firefox/manifest.json');

    assert.deepEqual(chrome.background, { service_worker: 'background.js' });
    assert.deepEqual(firefox.background, { scripts: ['background.js'] });
    assert.equal(firefox.browser_specific_settings.gecko.id, 'nap-wallet@cavebro.win');
    assert.equal(chrome.browser_specific_settings, undefined);

    const allowed = new Set(['background', 'browser_specific_settings']);
    for (const key of new Set([...Object.keys(chrome), ...Object.keys(firefox)])) {
        if (allowed.has(key)) continue;
        assert.deepEqual(firefox[key], chrome[key], `${key} drifted between the two builds`);
    }
});

/// Every host the extension reaches must be declared.
///
/// An undeclared cross-origin fetch survives only while the server stays
/// CORS-permissive, and a reviewer reading the manifest should see the whole
/// network surface. The popup defaults its art gateway to cavebro.win, so that
/// host belongs here next to the local app.
test('every host the popup reaches is declared', async () => {
    const manifest = await read('../dist/manifest.json');
    const popup = await readFile(new URL('../dist/popup.js', import.meta.url), 'utf8');

    const declared = manifest.host_permissions.map((p) => new URL(p.replace('/*', '/')).origin);
    const reached = new Set(
        [...popup.matchAll(/https?:\/\/[a-zA-Z0-9._:-]+/g)].map((m) => new URL(m[0]).origin)
    );

    for (const origin of reached) {
        assert.ok(declared.includes(origin), `${origin} is fetched but not in host_permissions`);
    }
});

/// The popup must never open onto a bare failure.
///
/// This extension holds no keys: without the desktop app every call fails, and
/// a wallet whose first screen is an error reads as broken to a new user and to
/// a store reviewer alike. The gate is what makes that state explain itself.
test('the popup ships the first-run gate', async () => {
    const html = await readFile(new URL('../dist/popup.html', import.meta.url), 'utf8');
    const gate = await readFile(new URL('../dist/agent-gate.js', import.meta.url), 'utf8');

    assert.match(html, /<script src="agent-gate\.js"><\/script>/);
    assert.ok(html.indexOf('popup.js') < html.indexOf('agent-gate.js'), 'the gate reads ZYN_API_BASE, so it must load after the app script');
    assert.match(gate, /Nap Wallet isn't running/);
    assert.match(gate, /nap-gate-retry/);
});

/// The wallet may not claim a privacy tier it is not on.
///
/// Zyn holdings are visible to the operator and its verifiers, and until DA
/// sealing is deployed to anyone who can fetch a bundle. The wallet shows them
/// beside shielded ZEC, which is genuinely private, so any sentence implying
/// one property covers both is false in the most misleading place available —
/// and this page ships to a store listing, where it becomes a public claim.
///
/// DECISIONS §130.8d-e, §130.12. The visibility vocabulary is `--private`
/// (only this key), `--operator` (this key and us), `--record` (anyone).
test('the popup claims no privacy it does not have', async () => {
    const html = await readFile(new URL('../dist/popup.html', import.meta.url), 'utf8');
    const prose = html.replace(/<style[\s\S]*?<\/style>/g, '').replace(/<script[\s\S]*?<\/script>/g, '');

    for (const claim of [
        'Held on Zyn, privately',
        'Your holdings stay private',
        'private, instant',
    ]) {
        assert.ok(!prose.includes(claim), `retracted claim is back: "${claim}"`);
    }

    // The correction, not merely the absence of the claim.
    assert.match(prose, /visible to the operator — not private/);
    assert.match(prose, /instant, not private/);
});

/// Emphasis must not borrow a visibility colour.
///
/// `section.lead` is the panel that leads a tab — six of them, across both the
/// Zcash and Zyn sides. While it took the `--private` tint, the wallet was
/// making a privacy claim every time it wanted a panel to stand out.
test('the lead panel does not borrow the privacy tint', async () => {
    const css = await readFile(new URL('../dist/popup.html', import.meta.url), 'utf8');

    assert.match(css, /section\.lead \{ background: linear-gradient\(145deg, var\(--lead\)/);
    assert.match(css, /--operator:/, 'the middle visibility category must exist');
    assert.ok(
        /\.nft-send-panel \{[^}]*var\(--operator\)/.test(css),
        'Zyn state must take the operator tint, not the private one'
    );
});
