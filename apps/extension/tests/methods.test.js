import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';

const background = await readFile(new URL('../src/background.js', import.meta.url), 'utf8');
const agent = await readFile(new URL('../../../zynzapd/src/app.rs', import.meta.url), 'utf8');

/// Every `app('/path', …)` call the provider makes, and the verb it implies.
///
/// `app()` sends a POST whenever it is given a body and a GET otherwise, so the
/// presence of a second argument *is* the verb. That is the whole bug this file
/// exists for: `app('/api/deposit-address', {})` looked like "no parameters"
/// and meant "POST", and the agent had no POST route to answer it.
function providerCalls(source) {
    const calls = [];
    const pattern = /app\(\s*'([^']+)'\s*(,\s*([^)]*))?\)/g;
    for (const m of source.matchAll(pattern)) {
        const body = (m[3] ?? '').trim();
        calls.push({ path: m[1], verb: body === '' ? 'GET' : 'POST' });
    }
    return calls;
}

/// The routes the agent actually serves, as `("VERB", "/path")`.
function agentRoutes(source) {
    const routes = new Set();
    for (const m of source.matchAll(/\("(GET|POST)",\s*"([^"]+)"\)/g)) {
        routes.add(`${m[1]} ${m[2]}`);
    }
    return routes;
}

test('every route the provider calls is one the agent serves, with that verb', () => {
    const routes = agentRoutes(agent);
    assert.ok(routes.size > 20, 'the route table should have parsed');

    const missing = [];
    for (const { path, verb } of providerCalls(background)) {
        // Only /api routes are the agent's; anything else is not its business.
        if (!path.startsWith('/api/')) continue;
        if (!routes.has(`${verb} ${path}`)) {
            const other = verb === 'GET' ? 'POST' : 'GET';
            missing.push(
                routes.has(`${other} ${path}`)
                    ? `${verb} ${path} — the agent serves it as ${other}`
                    : `${verb} ${path} — the agent serves no such route`,
            );
        }
    }
    assert.deepEqual(missing, [], `the provider calls routes the agent will refuse:\n  ${missing.join('\n  ')}`);
});

test('the deposit address is fetched, not posted', () => {
    // The regression by name: a POST here returns {"error":"no such call"},
    // which the page reported as the wallet failing when the wallet was fine.
    assert.match(background, /zyn_depositAddress:.*app\('\/api\/deposit-address'\)/);
    assert.doesNotMatch(background, /app\('\/api\/deposit-address',/);
});

test('the shipped build is the source', async () => {
    // dist/ is what Chrome loads. A fix that lands only in src/ is not a fix.
    const dist = await readFile(new URL('../dist/background.js', import.meta.url), 'utf8');
    assert.equal(dist, background, 'dist/background.js has drifted from src/background.js');
});
