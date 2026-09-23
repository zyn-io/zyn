// --- zip321.js (from apps/nap-web) ---
// ZIP-321 payment requests: `zcash:<address>?amount=…&memo=…`.
//
// The one way Zcash wallets agree on for a site, a shop or a QR code to ask
// for a payment. Nap reads it in two places: pasted into Send, and handed to
// the connect window by a site (`zec_pay {uri}`). Either way it only fills in
// a payment the holder then reviews; nothing here sends anything.
//
// Nap pays one recipient per request. A multi-payment request (`address.1`,
// `amount.1`, …) is refused whole rather than paid in part, and so is any
// `req-` parameter this parser does not know — ZIP-321 says a wallet must not
// pay a request whose required parameters it does not understand.

const ZAT = 100_000_000n;

function base64urlBytes(text) {
  if (!/^[A-Za-z0-9_-]*$/.test(text)) throw new Error('The memo is not base64url');
  const b64 = text.replace(/-/g, '+').replace(/_/g, '/').padEnd(Math.ceil(text.length / 4) * 4, '=');
  const bin = atob(b64);
  return Uint8Array.from(bin, (c) => c.charCodeAt(0));
}

/// ZEC as written in a request ("1.5", "0.0001") to zatoshi.
function zatoshiOf(text) {
  if (!/^\d+(\.\d{1,8})?$/.test(text)) throw new Error('The amount must be ZEC with at most eight decimals');
  const [whole, frac = ''] = text.split('.');
  const zat = BigInt(whole) * ZAT + BigInt(frac.padEnd(8, '0'));
  if (zat > 21_000_000n * ZAT) throw new Error('That is more ZEC than exists');
  return Number(zat);
}

/// `{address, zatoshi|null, memo, label, message}`, or an error saying why
/// the request cannot be paid.
function parsePaymentRequest(uri) {
  const text = String(uri || '').trim();
  if (!/^zcash:/i.test(text)) throw new Error('A payment request starts with zcash:');
  const rest = text.slice(6);
  const q = rest.indexOf('?');
  const path = decodeURIComponent(q < 0 ? rest : rest.slice(0, q));
  const out = { address: path || null, zatoshi: null, memo: '', label: '', message: '' };
  const seen = new Set();
  if (q >= 0) {
    for (const part of rest.slice(q + 1).split('&')) {
      if (!part) continue;
      const eq = part.indexOf('=');
      const rawKey = eq < 0 ? part : part.slice(0, eq);
      const value = decodeURIComponent((eq < 0 ? '' : part.slice(eq + 1)).replace(/\+/g, '%20'));
      const dot = rawKey.indexOf('.');
      const key = dot < 0 ? rawKey : rawKey.slice(0, dot);
      const index = dot < 0 ? '' : rawKey.slice(dot + 1);
      if (index !== '' && index !== '0') throw new Error('This request pays more than one recipient; Nap pays one per request');
      if (seen.has(key)) throw new Error(`The request names ${key} twice`);
      seen.add(key);
      switch (key) {
        case 'address': out.address = value; break;
        case 'amount': out.zatoshi = zatoshiOf(value); break;
        case 'memo': {
          const bytes = base64urlBytes(value);
          if (bytes.length > 512) throw new Error('The memo is longer than 512 bytes');
          try { out.memo = new TextDecoder('utf-8', { fatal: true }).decode(bytes); }
          catch { throw new Error('The memo is not text; Nap sends text memos only'); }
          break;
        }
        case 'label': out.label = value; break;
        case 'message': out.message = value; break;
        default:
          if (key.startsWith('req-')) throw new Error(`The request requires ${key}, which Nap does not understand`);
      }
    }
  }
  if (!out.address) throw new Error('The request names no address');
  if (out.memo && /^t/i.test(out.address)) throw new Error('A memo only reaches a shielded address');
  return out;
}

/// The inverse, for a page that wants to show a request of its own.
function paymentRequest({ address, zatoshi, memo = '', message = '' }) {
  const params = [];
  if (zatoshi != null) {
    const v = BigInt(zatoshi);
    const frac = (v % ZAT).toString().padStart(8, '0').replace(/0+$/, '');
    params.push(`amount=${v / ZAT}${frac ? `.${frac}` : ''}`);
  }
  if (memo) {
    const bytes = new TextEncoder().encode(memo);
    params.push(`memo=${btoa(String.fromCharCode(...bytes)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')}`);
  }
  if (message) params.push(`message=${encodeURIComponent(message)}`);
  return `zcash:${address}${params.length ? `?${params.join('&')}` : ''}`;
}

// --- background.js ---
// The extension's brain. Reads go straight to the local app; writes wait
// for the person to approve them in a window the site cannot touch.
const APP = 'http://127.0.0.1:8977';

async function app(path, body) {
  const opts = body ? { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Zyn': '1' }, body: JSON.stringify(body) } : {};
  let r;
  try { r = await fetch(APP + path, opts); } catch { throw new Error('the Nap Wallet app is not running on this machine'); }
  const j = await r.json().catch(() => ({ error: 'bad reply' }));
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}

// Which sites the holder connected. Nothing about the holder — the account,
// an address, what they own — goes to a site that is not in here; asking for
// it is what prompts the connection, as `eth_requestAccounts` does.
async function connected() { return (await chrome.storage.local.get('connected')).connected || {}; }
async function remember(origin) { const c = await connected(); c[origin] = { at: Date.now() }; await chrome.storage.local.set({ connected: c }); }
async function forget(origin) { const c = await connected(); delete c[origin]; await chrome.storage.local.set({ connected: c }); }

/// What a connected site learns: the account and the Zcash address.
async function session() {
  const o = await app('/api/overview');
  return { account: o.zyn.account, network: o.network, chain: o.zyn.chain, zcash: o.address ? { network: o.network, address: o.address } : null };
}

const ZATS = 100000000n;
const zecText = (zat) => { const v = BigInt(zat); const f = (v % ZATS).toString().padStart(8, '0').replace(/0+$/, ''); return `${v / ZATS}${f ? `.${f}` : ''}`; };

/// A ZEC payment a site asks for, as a ZIP-321 URI or as fields; the same
/// parser the browser wallet uses (prepended to this file by the build).
function payment(p) {
  const req = p.uri
    ? parsePaymentRequest(p.uri)
    : { address: String(p.address || '').trim(), zatoshi: zatoshiOf(String(p.amount ?? '')), memo: String(p.memo || ''), message: '' };
  if (!req.address) throw new Error('address is required');
  if (req.zatoshi == null || req.zatoshi <= 0) throw new Error('The request names no amount');
  if (req.memo && /^t/i.test(req.address)) throw new Error('A memo only reaches a shielded address');
  return req;
}

// What a site may ask for, and what each costs. `mine` marks a read of the
// holder's own data: those need a connected site. Unmarked reads are the
// chain's public data. Every write needs a connected site and an approval.
const METHODS = {
  nap_connect:    { session: true },
  nap_session:    { session: true },
  nap_disconnect: { session: true },
  zec_getAddress: { read: true, mine: true, run: async () => (await session()).zcash },
  zec_pay: {
    read: false,
    run: (p) => { const r = payment(p); return app('/api/send', { to: r.address, amount: zecText(r.zatoshi), memo: r.memo }); },
    describe: (p) => { const r = payment(p); return `Pay ${zecText(r.zatoshi)} ZEC to ${r.address.slice(0, 16)}…${r.memo ? ` with the memo "${r.memo}"` : ''}${r.message ? `. The site says: ${r.message}` : ''}. A Zcash transaction from desktop Nap; the network fee is added.`; },
  },
  zyn_account:  { read: true, mine: true, run: async () => { const o = await app('/api/overview'); return { account: o.zyn.account, address: o.address, network: o.network, chain: o.zyn.chain }; } },
  zyn_balances: { read: true, mine: true, run: async () => (await app('/api/overview')).zyn.record },
  zyn_assets:   { read: true,  run: async () => (await app('/api/overview')).zyn.assets },
  zyn_pools:    { read: true,  run: async () => (await app('/api/overview')).zyn.pools },
  zyn_curves:   { read: true,  run: async () => (await app('/api/curves')).curves },
  zyn_curve:    { read: true,  run: async (p) => (await app('/api/curve', p)).curve },
  zyn_curveQuote: { read: true, run: (p) => app('/api/curve-quote', p) },
  zyn_quote:    { read: true,  run: (p) => app('/api/quote', p) },
  // Where to pay this account. `memo: true` means the node could not issue a
  // per-account address and this is the shared vault, which credits nobody
  // without the account memo attached — a page must say so rather than show
  // an address that silently loses money.
  // No body, deliberately: `app()` treats any body as a POST, and this route
  // is GET-only. Passing `{}` here sent a POST that the agent had no route
  // for, so the page reported a wallet failure for a wallet that was fine.
  zyn_depositAddress: { read: true, mine: true, run: () => app('/api/deposit-address') },
  // The account that holds this collection's items (§97.5). A checkout quotes
  // against this rather than the identity, so the money and the item land in an
  // account with no other history and nothing has to be moved afterwards.
  // The order book. Reading it is public — it is the same data `zyn_offers`
  // serves to anyone — but placing, taking and cancelling move an item or a
  // balance, so each is approval-gated and says what it does.
  //
  // `collection` is what routes these to the *receiving* account: an item
  // bought through a store is held there, not by the identity, and signing
  // with the identity would be a signature from an account holding nothing.
  zyn_offers:      { read: true,  run: () => app('/api/offers') },
  zyn_placeOffer:  { read: false, run: (p) => app('/api/offer', p),        describe: (p) => `List asset ${String(p.asset).slice(0, 12)}… for ${p.price} ZEC.zy. It is escrowed until taken or cancelled.` },
  zyn_takeOffer:   { read: false, run: (p) => app('/api/take', p),         describe: (p) => `Take offer #${p.offer}. You pay the asking price in ZEC.zy and receive the item.` },
  zyn_cancelOffer: { read: false, run: (p) => app('/api/cancel-offer', p), describe: (p) => `Cancel offer #${p.offer} and take the item back out of escrow.` },
  zyn_receivingAccount: { read: true, mine: true, run: (p) => app('/api/receiving/collection', p) },
  zyn_receivingNext:    { read: true, mine: true, run: () => app('/api/receiving/next') },
  // A signed read the page relays so a service can verify a payment it is not
  // allowed to look up. Approval-gated: it discloses one account's balance to
  // whoever the page hands it to, which is a disclosure and not a lookup.
  // Signs the entry payment without sending it — the sale service relays it,
  // so a payment cannot leave the wallet without reaching the sale.
  zyn_receivingPay: { read: false, run: (p) => app('/api/receiving/pay', p), describe: (p) => `Pay ${p.amount} ZEC.zy for your place in this collection. Signed now, sent by the sale.` },
  zyn_receivingRead: { read: false, run: (p) => app('/api/receiving/read', p), describe: () => 'Prove your payment for this collection. The site learns this collection account\'s balance and nothing else.' },
  zyn_zyn:      { read: true, mine: true,  run: async () => (await app('/api/overview')).zyn },
  // The object ledger's NFT cycle, for the Cave store. Reads are public or
  // the holder's own; every write is signed in the desktop app after approval.
  zyn_nftItems:  { read: true, mine: true,  run: () => app('/api/object/items') },
  zyn_nftOffers: { read: true,  run: () => app('/api/object/offers') },
  zyn_owed:      { read: true, mine: true,  run: () => app('/api/object/owed') },
  zyn_nftList:   { read: false, run: (p) => app('/api/object/list', p),   describe: (p) => `List this item for ${p.price} ZEC.zy. Buyers pay the price plus the collection fee; your proceeds wait for you to collect.` },
  zyn_nftBuy:    { read: false, run: (p) => app('/api/object/buy', p),    describe: (p) => `Buy this item for ${p.pay || 'the listed price plus fee'} ZEC.zy. It arrives in your wallet.` },
  zyn_nftCancel: { read: false, run: (p) => app('/api/object/cancel', p), describe: () => 'Cancel your listing. The item is yours to move again.' },
  zyn_nftRedeem: { read: false, run: (p) => app('/api/object/redeem', p), describe: () => 'Burn this item for its share of the collection floor. This cannot be undone.' },
  // Collecting only moves what the ledger already owes this wallet into it,
  // so a connected site may ask without a prompt (as Nap in the browser does).
  zyn_collect:   { read: true, mine: true, run: () => app('/api/object/collect', {}) },
  zyn_transfer: { read: false, run: (p) => app('/api/transfer', p), describe: (p) => `Transfer ${p.amount} of asset ${p.asset} to ${String(p.to).slice(0, 12)}…` },
  // A graduated launch's pool (or any pool) on the object ledger.
  zyn_swap:     { read: false, run: (p) => app('/api/object/swap', p), describe: (p) => `Swap ${p.amount} into pool ${String(p.pool).slice(0, 10)}… (at most ${(Number(p.slippageBps ?? 100) / 100)}% below the quote). Swaps in one epoch clear together at one price.` },
  // The launchpad (DECISIONS §141): one curve for every token.
  zyn_curves:        { read: true, run: () => app('/api/object/curves') },
  zyn_curveHoldings: { read: true, mine: true, run: () => app('/api/object/curve-holdings') },
  zyn_curveCreate:   { read: false, run: (p) => app('/api/object/curve/create', p), describe: (p) => `Create the token ${String(p.symbol).toUpperCase()} ("${p.name}"). All 1B tokens are sold on its curve or put in its locked pool; you get none unless you buy. A bond, if this network sets one, comes back at graduation.` },
  zyn_curveBuy:      { read: false, run: (p) => app('/api/object/curve/buy', p), describe: (p) => `Buy token ${String(p.token).slice(0, 10)}… with ${p.amount} ZEC.zy. Everyone buying this epoch pays one price; the fee is on top and anything unspent comes back.` },
  zyn_curveSell:     { read: false, run: (p) => app('/api/object/curve/sell', p), describe: (p) => `Sell ${p.tokens} of token ${String(p.token).slice(0, 10)}… back to its curve. Sellers this epoch share one price; the fee comes out of the proceeds.` },
  zyn_deposit:  { read: false, run: (p) => app('/api/deposit', p),  describe: (p) => `Deposit ${p.amount} from the Zcash wallet to the vault` },
  zyn_withdraw: { read: false, run: (p) => app('/api/withdraw', p), describe: (p) => `Withdraw ${p.amount} ZEC.zy to the bound wallet` },
  zyn_bind:     { read: false, run: () => app('/api/bind', {}),      describe: () => 'Bind Zyn exits to your Nap Wallet address' },
  zyn_bind_sol: { read: false, run: (p) => app('/api/bind-sol', p), describe: (p) => `Bind Zyn exits to the Solana address ${p.address}` },
  zyn_withdraw_sol: { read: false, run: (p) => app('/api/withdraw-sol', p), describe: (p) => `Withdraw ${p.amount} of asset ${p.asset} to the bound Solana address` },
  zyn_add_liquidity: { read: false, run: (p) => app('/api/liquidity/add', p), describe: (p) => `Add up to ${p.amount0} + ${p.amount1} to pool ${p.pool}` },
  zyn_remove_liquidity: { read: false, run: (p) => app('/api/liquidity/remove', p), describe: (p) => `Remove ${p.shares} shares from pool ${p.pool}` },
  zyn_launchCurve: { read: false, run: (p) => app('/api/launch-curve', p), describe: (p) => `Launch ${p.display_name} (${p.symbol}) with a ${Number(p.fee_bps) / 100}% trading fee. Pay 0.01 ZEC.zy launch fee${Number(p.dev_buy) > 0 ? ` and buy ${p.dev_buy} tokens for no more than ${p.max_zec} ZEC.zy` : ''}.` },
  zyn_buyCurve: { read: false, run: (p) => app('/api/buy-curve', p), describe: (p) => `Buy exactly ${p.tokens} tokens of ${p.asset} for no more than ${p.max_zec} ZEC.zy.` },
  zyn_sellCurve: { read: false, run: (p) => app('/api/sell-curve', p), describe: (p) => `Sell exactly ${p.tokens} tokens of ${p.asset} for at least ${p.min_zec} ZEC.zy.` },
};

const approvals = new Map(); // id -> {resolve, reject, req}
let nextId = 1;

function ask(origin, method, params, description) {
  return new Promise((resolve, reject) => {
    const id = nextId++;
    approvals.set(id, { resolve, reject, req: { id, origin, method, params, description } });
    chrome.windows.create({ url: chrome.runtime.getURL(`confirm.html?id=${id}`), type: 'popup', width: 440, height: 560 });
  });
}

chrome.runtime.onMessage.addListener((msg, sender, reply) => {
  (async () => {
    if (msg.kind === 'provider') {
      const m = METHODS[msg.method];
      if (!m) throw new Error(`unknown method ${msg.method}`);
      const origin = msg.origin;
      const isConnected = !!(await connected())[origin];
      if (msg.method === 'nap_session') return isConnected ? await session() : null;
      if (msg.method === 'nap_disconnect') { await forget(origin); return true; }
      // Anything about the holder, and anything that acts for them, needs a
      // connected site first. Connecting is its own approval.
      if ((m.session || m.mine || !m.read) && !isConnected) {
        const ok = await ask(origin, 'nap_connect', {}, `Connect ${origin} to Nap. It will see your Zyn account ID and your Zcash address, not your balances or keys. Nothing moves unless you approve it.`);
        if (!ok) throw new Error('rejected by the user');
        await remember(origin);
      }
      if (msg.method === 'nap_connect') return await session();
      if (!m.read) {
        const ok = await ask(origin, msg.method, msg.params, m.describe(msg.params || {}));
        if (!ok) throw new Error('rejected by the user');
      }
      return await m.run(msg.params || {});
    }
    if (msg.kind === 'confirm:get') {
      const a = approvals.get(msg.id);
      return a ? a.req : { error: 'no such request' };
    }
    if (msg.kind === 'confirm:answer') {
      const a = approvals.get(msg.id);
      if (!a) return { error: 'no such request' };
      approvals.delete(msg.id);
      a.resolve(!!msg.approved);
      return { ok: true };
    }
    throw new Error('unknown message');
  })().then(reply, (e) => reply({ error: e.message }));
  return true;
});
