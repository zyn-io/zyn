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

// What a site may ask for, and what each costs.
const METHODS = {
  zyn_account:  { read: true,  run: async () => { const o = await app('/api/overview'); return { account: o.zyn.account, address: o.address, network: o.network, chain: o.zyn.chain }; } },
  zyn_balances: { read: true,  run: async () => (await app('/api/overview')).zyn.record },
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
  zyn_depositAddress: { read: true, run: () => app('/api/deposit-address') },
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
  zyn_receivingAccount: { read: true, run: (p) => app('/api/receiving/collection', p) },
  zyn_receivingNext:    { read: true, run: () => app('/api/receiving/next') },
  // A signed read the page relays so a service can verify a payment it is not
  // allowed to look up. Approval-gated: it discloses one account's balance to
  // whoever the page hands it to, which is a disclosure and not a lookup.
  // Signs the entry payment without sending it — the sale service relays it,
  // so a payment cannot leave the wallet without reaching the sale.
  zyn_receivingPay: { read: false, run: (p) => app('/api/receiving/pay', p), describe: (p) => `Pay ${p.amount} ZEC.zy for your place in this collection. Signed now, sent by the sale.` },
  zyn_receivingRead: { read: false, run: (p) => app('/api/receiving/read', p), describe: () => 'Prove your payment for this collection. The site learns this collection account\'s balance and nothing else.' },
  zyn_zyn:      { read: true,  run: async () => (await app('/api/overview')).zyn },
  // The object ledger's NFT cycle, for the Cave store. Reads are public or
  // the holder's own; every write is signed in the desktop app after approval.
  zyn_nftItems:  { read: true,  run: () => app('/api/object/items') },
  zyn_nftOffers: { read: true,  run: () => app('/api/object/offers') },
  zyn_owed:      { read: true,  run: () => app('/api/object/owed') },
  zyn_nftList:   { read: false, run: (p) => app('/api/object/list', p),   describe: (p) => `List this item for ${p.price} ZEC.zy. Buyers pay the price plus the collection fee; your proceeds wait for you to collect.` },
  zyn_nftBuy:    { read: false, run: (p) => app('/api/object/buy', p),    describe: (p) => `Buy this item for ${p.pay || 'the listed price plus fee'} ZEC.zy. It arrives in your wallet.` },
  zyn_nftCancel: { read: false, run: (p) => app('/api/object/cancel', p), describe: () => 'Cancel your listing. The item is yours to move again.' },
  zyn_nftRedeem: { read: false, run: (p) => app('/api/object/redeem', p), describe: () => 'Burn this item for its share of the collection floor. This cannot be undone.' },
  zyn_collect:   { read: false, run: () => app('/api/object/collect', {}), describe: () => 'Collect everything the market owes you: sale proceeds, refunds, floor shares.' },
  zyn_transfer: { read: false, run: (p) => app('/api/transfer', p), describe: (p) => `Transfer ${p.amount} of asset ${p.asset} to ${String(p.to).slice(0, 12)}…` },
  zyn_swap:     { read: false, run: (p) => app('/api/swap', p),     describe: (p) => `Swap ${p.amount} of asset ${p.asset_in} for asset ${p.asset_out} (1% below quote at worst)` },
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
      if (!m.read) {
        const ok = await ask(msg.origin, msg.method, msg.params, m.describe(msg.params || {}));
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
