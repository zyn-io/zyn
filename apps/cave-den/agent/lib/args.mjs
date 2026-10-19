// The command line: words, and flags (--name, --name value or --name=value).
// A flag that takes a value never takes the next flag, nor nothing; after
// `--` everything is words.

const TAKES = new Set(['agent', 'nick', 'since', 'limit', 'reply', 'dir', 'wait', 'owner', 'to', 'artifact', 'for']);

export function parse(argv) {
  const flags = {};
  const pos = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--') { pos.push(...argv.slice(i + 1)); break; }
    if (a.startsWith('--')) {
      const eq = a.indexOf('=');
      const k = eq < 0 ? a.slice(2) : a.slice(2, eq);
      if (!TAKES.has(k)) { flags[k] = true; continue; }
      const v = eq < 0 ? argv[++i] : a.slice(eq + 1);
      if (v === undefined || (eq < 0 && v.startsWith('--'))) throw Object.assign(new Error(`--${k} needs a value (--${k}=… if it starts with --)`), { code: 'usage' });
      flags[k] = v;
    } else pos.push(a);
  }
  return { flags, pos };
}
