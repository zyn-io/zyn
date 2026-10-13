// The way in: a window on the host's own site (config.issuer) checks whatever
// the host requires and hands this page a pass code with postMessage. This
// page never runs the host's wallet, sign-in or extension scripts, so none of
// them can reach the keys it keeps.

const PASS = /^[0-9a-f]{64}$/;
const quiet = (m) => Object.assign(new Error(m), { quiet: true });

/// Opens the issuer, waits for it, and enters the den. By default this device
/// makes its own pass code and gives the issuer only its hash (after "#", so
/// it reaches no server log): the issuer registers the hash with the den and
/// answers { den: 'pass', registered: true }, and never learns the code. A
/// host whose check makes the code itself (config.issuerCode) answers
/// { den: 'pass', code } instead.
export async function enter({ den, say, config }) {
  if (!config.issuer) throw new Error('This den has no way in set up yet.');
  const own = config.issuerCode ? null : await den.constructor.newPass();
  const issuer = new URL(config.issuer, location.href);
  if (own) issuer.hash = `den-pass=${own.pass}`;
  const win = window.open(issuer.href, 'den-issuer', 'popup,width=480,height=680');
  if (!win) throw new Error('The browser blocked the window. Allow pop-ups for this page and try again.');
  say(config.issuerSay || 'Finish in the window that opened.');
  const code = await new Promise((resolve, reject) => {
    const done = (f) => { clearInterval(watch); removeEventListener('message', on); f(); };
    const on = (e) => {
      if (e.origin !== issuer.origin || e.source !== win || e.data?.den !== 'pass') return;
      if (e.data.error) return done(() => reject(e.data.cancelled ? quiet('Not entered. Nothing was registered.') : new Error('The other site could not let this device in.')));
      if (own) return done(() => (e.data.registered === true ? resolve(own.code) : reject(new Error('The other site did not register this device.'))));
      if (!PASS.test(e.data.code || '')) return done(() => reject(new Error('The window did not give a pass code.')));
      done(() => resolve(e.data.code));
    };
    addEventListener('message', on);
    const watch = setInterval(() => { if (win.closed) done(() => reject(quiet('Not entered. Nothing was registered.'))); }, 500);
  });
  try { win.close(); } catch { /* gone */ }
  say('The door is checking your pass…');
  return den.fromPass(code);
}
