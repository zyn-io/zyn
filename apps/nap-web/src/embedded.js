// A Zyn account inside a site, opened with a wallet the holder already has
// (Phantom, MetaMask, Backpack, Rabby…) instead of Nap.
//
// The holder signs one fixed message naming the domain it is for. The
// signature is hashed into a 24-word phrase, and the account comes from that
// phrase exactly as Nap derives one — so the same wallet always opens the
// same account, on any device, and the phrase can be shown and restored in
// Nap. Signing the same message is opening the account: the message says so.
//
// Several wallets can open one account (a Solana wallet and an Ethereum one,
// say): the account's words are sealed for each linked wallet under a key
// only that wallet's signature makes, and kept by the Zyn app's service
// under an id the same signature makes (the keyring) — so nobody can look a
// wallet up, and the service holds ciphertext only.
//
// The provider answers the same methods Nap Connect does, from the same
// operations (`connect-ops.js`); writes go through the site's own approval,
// passed in as `approve(plan)`. Nothing is stored: the account lives in the
// page until it is closed.

import { sha256 } from '@noble/hashes/sha2.js';
import { x25519 } from '@noble/curves/ed25519.js';
import { xchacha20poly1305 } from '@noble/ciphers/chacha.js';
import * as ed from '@noble/ed25519';
import { entropyToMnemonic, mnemonicToEntropy } from '@scure/bip39';
import { wordlist } from '@scure/bip39/wordlists/english.js';
import { deriveIdentity, forgetIdentity } from './core.js';
import { ERR, METHODS, ProviderError, session } from './connect-ops.js';
import { objectCall, registerIntent } from './object.js';
import { submitIntent, call } from './ledger.js';
import { loadPocket, payOnZyn, resolveRecipient } from './pocket.js';
import CONFIG from '../public/config.json';

const te = new TextEncoder();

/// The message a wallet signs to open its Zyn account, for `domain` and its
/// subdomains (the Zyn app and Zyn Lend open the same account).
export function signText(domain) {
  return [
    'Open my Zyn account',
    '',
    `This signature is the key to your Zyn account on ${domain}.`,
    `Sign this exact message only on ${domain} and its subdomains. Anyone holding the signature can open the account.`,
    '',
    'Version: 1',
  ].join('\n');
}

const label = (l, sig) => sha256(new Uint8Array([...te.encode(l), 0, ...sig]));
const hexOf = (b) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
const bytesOf = (h) => Uint8Array.from(String(h).match(/../g) || [], (x) => parseInt(x, 16));

/// Where a wallet's sealed key is kept, and the key that opens it: both
/// from its signature, so only that wallet finds and opens its entry.
export const keyringId = (signature) => hexOf(label('zyn-keyring.id.v1', signature));
const wrapKey = (signature) => label('zyn-keyring.wrap.v1', signature);

/// The words this signature opens: the account it is linked to, if any,
/// else its own (`linked` says which).
export async function wordsFor(config, signature, domain) {
  // "Not linked" opens the wallet's own account; "cannot tell" must not, or
  // a linked wallet would quietly open an empty account of its own. A node
  // without a keyring yet is the first.
  const entry = await call(config, 'zyn_keyringGet', { key: keyringId(signature) }).catch((e) => {
    if (/does not serve zyn_keyring/.test(e.message)) return null;
    throw new Error(`The Zyn keyring did not answer (${e.message}); try again shortly`);
  });
  if (entry?.blob) {
    const blob = bytesOf(entry.blob);
    const entropy = xchacha20poly1305(wrapKey(signature), blob.slice(0, 24)).decrypt(blob.slice(24));
    return { words: entropyToMnemonic(entropy, wordlist), linked: true };
  }
  return { words: wordsFromSignature(signature, domain), linked: false };
}

/// Seal `words` (the open account's) for the wallet that made `signature`,
/// so it opens this account from now on. Signed by the account.
export async function link(config, identity, words, signature) {
  const nonce = crypto.getRandomValues(new Uint8Array(24));
  const sealed = xchacha20poly1305(wrapKey(signature), nonce).encrypt(mnemonicToEntropy(words, wordlist));
  const blob = new Uint8Array([...nonce, ...sealed]);
  const id = keyringId(signature);
  const digest = sha256(new Uint8Array([...te.encode('zyn-keyring.put.v1'), ...bytesOf(id), ...blob]));
  await call(config, 'zyn_keyringPut', { key: id, blob: hexOf(blob), account: hexOf(identity.publicKey), signature: hexOf(ed.sign(digest, identity.secretKey)) });
}

/// Undo `link` for the wallet that made `signature`.
export async function unlink(config, identity, signature) {
  const id = keyringId(signature);
  const digest = sha256(new Uint8Array([...te.encode('zyn-keyring.delete.v1'), ...bytesOf(id)]));
  await call(config, 'zyn_keyringDelete', { key: id, account: hexOf(identity.publicKey), signature: hexOf(ed.sign(digest, identity.secretKey)) });
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
  return {
    provider,
    session: await session(ctx),
    close: () => forgetIdentity(identity),
    /// Everything this account holds, named: coins, items, what swaps owe.
    pocket: () => loadPocket(config, identity),
    /// Pay `amount` (WAD) of `asset` to another Zyn account (64 hex).
    async send(asset, amount, to) {
      const { to: account } = await resolveRecipient(config, identity, to, { isZec: false, isItem: false });
      const { held } = await loadPocket(config, identity);
      return payOnZyn(config, identity, held, asset, BigInt(amount), account);
    },
  };
}

/// What `identity` holds, read without opening (or registering) it.
export const peek = (config, identity) => loadPocket(config, identity);

/// The account `words` open.
export const identityFromWords = (words) => deriveIdentity(words);

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
