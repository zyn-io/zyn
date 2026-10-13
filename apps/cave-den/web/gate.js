// The way in: a window on the host's own site (config.issuer) checks whatever
// the host requires and hands this page a pass code with postMessage. This
// page never runs the host's wallet, sign-in or extension scripts, so none of
// them can reach the keys it keeps.

const PASS = /^[0-9a-f]{64}$/;
const quiet = (m) => Object.assign(new Error(m), { quiet: true });

/// Opens the issuer, waits for its pass code, and enters the den with it.
export async function enter({ den, say, config }) {
  if (!config.issuer) throw new Error('This den has no way in set up yet.');
  const issuer = new URL(config.issuer, location.href);
  const win = window.open(issuer.href, 'den-issuer', 'popup,width=480,height=680');
  if (!win) throw new Error('The browser blocked the window. Allow pop-ups for this page and try again.');
  say(config.issuerSay || 'Finish in the window that opened.');
  const code = await new Promise((resolve, reject) => {
    const done = (f) => { clearInterval(watch); removeEventListener('message', on); f(); };
    const on = (e) => {
      if (e.origin !== issuer.origin || e.source !== win || e.data?.den !== 'pass') return;
      if (e.data.error) return done(() => reject(e.data.cancelled ? quiet(e.data.error) : new Error(e.data.error)));
      if (!PASS.test(e.data.code || '')) return done(() => reject(new Error('The window did not give a pass code.')));
      done(() => resolve(e.data.code));
    };
    addEventListener('message', on);
    const watch = setInterval(() => { if (win.closed) done(() => reject(quiet('Not entered. Nothing was registered.'))); }, 500);
  });
  say('The door is checking your pass…');
  return den.fromPass(code);
}
