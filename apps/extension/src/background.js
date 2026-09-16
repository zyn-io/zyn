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
  zyn_depositAddress: { read: true, run: () => app('/api/deposit-address', {}) },
  zyn_zyn:      { read: true,  run: async () => (await app('/api/overview')).zyn },
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
