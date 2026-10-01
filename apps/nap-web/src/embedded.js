// A Zyn account inside a site, opened with a wallet the holder already has
// (Phantom, MetaMask, Backpack, Rabby…) instead of Nap.
//
// The holder signs one fixed message bound to the site's host. The signature
// is hashed into a 24-word phrase, and the account comes from that phrase
// exactly as Nap derives one — so the same wallet always opens the same
// account, on any device, and the phrase can be shown and restored in Nap.
// Signing the same message is opening the account: the message says so, and
// names the one host it is for.
//
// The provider answers the same methods Nap Connect does, from the same
// operations (`connect-ops.js`); writes go through the site's own approval,
// passed in as `approve(plan)`. Nothing is stored: the account lives in the
// page until it is closed.

import { sha256 } from '@noble/hashes/sha2.js';
import { x25519 } from '@noble/curves/ed25519.js';
import { entropyToMnemonic } from '@scure/bip39';
import { wordlist } from '@scure/bip39/wordlists/english.js';
import { deriveIdentity, forgetIdentity } from './core.js';
import { ERR, METHODS, ProviderError, session } from './connect-ops.js';
import { objectCall, registerIntent } from './object.js';
import { submitIntent } from './ledger.js';
import CONFIG from '../public/config.json';

const te = new TextEncoder();

/// The message a wallet signs to open its Zyn account on `host`.
export function signText(host) {
  return [
    'Open my Zyn account',
    '',
    `This signature is the key to your Zyn account on ${host}.`,
    `Sign this exact message only on ${host}. Anyone holding the signature can open the account.`,
    '',
    'Version: 1',
  ].join('\n');
}

/// The 24 words a signature opens: the same signature, the same words.
export function wordsFromSignature(signature, host) {
  if (!(signature instanceof Uint8Array) || signature.length < 64) throw new Error('The wallet returned no signature');
  const entropy = sha256(new Uint8Array([...te.encode('zyn.embedded.v1\0'), ...te.encode(host), 0, ...signature]));
  return entropyToMnemonic(entropy, wordlist);
}

/// A provider for `identity` on `config`, answering as Nap Connect does.
/// `approve(plan)` shows a write's title, rows and note and resolves true or
/// false; `progress(text)` reports a long step (proving, syncing).
export async function open({ config, identity, approve, progress = () => {} }) {
  const ctx = { config, identity, zec: null, zecReady: null };
  // A new account publishes where its notes are sealed before anything can
  // arrive for it: deposits, claims and withdrawals all wait on it.
  const r = await objectCall(config.rpcUrl, 'zyn_depositAddress', { account: identity.accountHex }).catch(() => null);
  if (r?.registered === false && identity.viewing) {
    await submitIntent(config, identity, registerIntent(identity.account, x25519.getPublicKey(identity.viewing)));
  }
  const provider = {
    async request({ method, params = {} }) {
      if (method === 'nap_connect' || method === 'nap_session') return session(ctx);
      const m = METHODS[method];
      if (!m) throw new ProviderError(ERR.UNSUPPORTED, `This account does not do ${method}`);
      if (m.kind === 'public' || m.kind === 'read' || m.kind === 'auto') return m.run(ctx, params);
      const plan = await m.prepare(ctx, params);
      if (!(await approve(plan))) throw new ProviderError(ERR.REJECTED, 'You declined');
      return plan.run((s) => {
        if (s?.proving) progress('Proving the transaction — about half a minute…');
        else if (s?.to) progress(`Syncing ${Number(s.at).toLocaleString('en-US')} of ${Number(s.to).toLocaleString('en-US')}…`);
      });
    },
  };
  return { provider, session: await session(ctx), close: () => forgetIdentity(identity) };
}

/// The account a signature opens, ready for `open`.
export function identityFromSignature(signature, host) {
  return deriveIdentity(wordsFromSignature(signature, host));
}

/// Nap's settings for network `id`, with the ledger reached at `rpcUrl` (a
/// site calls the node directly, not through Nap's own paths).
export function networkConfig(id, rpcUrl) {
  const n = CONFIG.networks.find((x) => x.id === id);
  if (!n) throw new Error(`No network ${id}`);
  return { ...n, rpcUrl, lightdUrl: null };
}
