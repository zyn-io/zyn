// The X sign-in popup's last page (DECISIONS §187.8): tell /mint how it went,
// on this site's own channel (no window opener needed), and close.
const r = new URLSearchParams(location.search).get('r') || 'failed';
const SAY = {
  linked: 'Linked. This wallet gets the whitelist price once it is listed on Zyn.',
  already: 'This wallet is already linked to your X account.',
  'not-approved': 'This X account is not on the whitelist. Apply on the whitelist page first.',
  'other-wallet': 'Your X account is already linked to another wallet.',
  'wallet-used': 'This wallet is already linked to another X account.',
  'no-wallet': 'Connect Nap on the mint page first.',
  cancelled: 'Not signed in. Nothing was linked.',
  expired: 'That took too long. Try again from the mint page.',
  unavailable: 'X sign-in is not set up yet. Use your code instead.',
  failed: 'X did not answer. Try again in a moment.',
};
document.getElementById('x-say').textContent = SAY[r] || SAY.failed;
try { new BroadcastChannel('cave-x-signin').postMessage({ r }); } catch { /* the mint page reads again on focus */ }
setTimeout(() => window.close(), 1500);
