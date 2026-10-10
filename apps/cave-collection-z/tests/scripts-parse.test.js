// Every inline script on the site's pages parses. A script that does not is
// refused whole by the browser, silently: the whitelist page declared `dig`
// twice from 27 Sep to 10 Oct 2026, so its proof of work never ran and the
// register turned every application away.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';

const root = new URL('../', import.meta.url);

test('every inline script on every page parses', async () => {
  const pages = (await readdir(root)).filter((f) => f.endsWith('.html'));
  assert.ok(pages.includes('whitelist.html'));
  for (const page of pages) {
    const html = await readFile(new URL(page, root), 'utf8');
    for (const m of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/g)) {
      if (!m[2].trim()) continue;
      const isModule = /type=["']module["']/.test(m[1]);
      // A module may use import and top-level await; checked as a module body.
      const body = isModule ? m[2].replace(/^\s*import\s[^;]+;/gm, '') : m[2];
      assert.doesNotThrow(() => new Function(isModule ? `return (async () => {${body}\n})` : body), `${page}: a script does not parse`);
    }
  }
});
