// Bringing a coin onto Zyn from where it is (DECISIONS §178), so a launch or
// a buy needs no trip to a bridge page first. SOL comes from a Solana wallet,
// ETH and BOLD from an Ethereum wallet, and ZEC from Nap's own Zcash balance
// or any Zcash wallet.
//
// Nap makes the one-time deposit code; the other wallet sends to the
// bridge's vault, which this file pins per network and checks against what
// Nap names, so a wrong answer sends nothing; the bridge credits it once its
// chain has finalized it; collecting moves it into the account. The page
// then carries on with what was asked, approved in Nap as always.

import { depositMessage, signAndSend, solanaWallets, unsigned } from './solana.js';

/// The coins a bridge brings, by the symbol Jade shows. `wait` is what the
/// holder is told while the bridge waits for the chain to finalize it.
export const SOURCES = Object.freeze({
  ZEC: { key: 'zec', decimals: 8, chain: 'Zcash', wait: 'a few Zcash blocks' },
  SOL: { key: 'sol', decimals: 9, chain: 'Solana', wallet: 'Solana wallet', wait: 'about a minute' },
  ETH: { key: 'eth', decimals: 18, chain: 'Ethereum', wallet: 'Ethereum wallet', wait: 'about 15 minutes, until Ethereum finalizes it' },
  BOLD: { key: 'bold', decimals: 18, chain: 'Ethereum', wallet: 'Ethereum wallet', wait: 'about 15 minutes, until Ethereum finalizes it' },
});

/// The vaults, fixed here as in Zyn Lend's bridge page: Nap's answer must
/// name the same one, or nothing is sent.
const ETH_VAULT = '0x235730d7738CC16d3b94d0e3D7097F9ddE22A459';
const SEPOLIA_VAULT = '0xe17bb5943b34133f6a8a5D221BE4eD9493B3ef27';
export const VAULTS = Object.freeze({
  testnet: {
    sol: { vault: '7KacSYXuVhKSf8dpXcLY3qFZQYiZg2m6vKh4m1HxKZmA', rpc: 'https://api.devnet.solana.com', cluster: 'devnet', tx: 'https://explorer.solana.com/tx/{tx}?cluster=devnet' },
    eth: { vault: SEPOLIA_VAULT, chainId: 11155111, assetId: 1, chain: 'Sepolia', tx: 'https://sepolia.etherscan.io/tx/{tx}' },
    bold: { vault: SEPOLIA_VAULT, token: '0xD58C9e3EeBE83fda55D59C186E86b37aa5d0f79a', chainId: 11155111, assetId: 2, chain: 'Sepolia', tx: 'https://sepolia.etherscan.io/tx/{tx}' },
  },
  mainnet: {
    sol: { vault: '23bHgTpb8kcbBXcKP66MdjrRfM4UgAtjDDFkidWYLwnU', rpc: 'https://solana-rpc.publicnode.com', cluster: 'mainnet', tx: 'https://explorer.solana.com/tx/{tx}' },
    eth: { vault: ETH_VAULT, chainId: 1, assetId: 1, chain: 'Ethereum', tx: 'https://etherscan.io/tx/{tx}' },
    bold: { vault: ETH_VAULT, token: '0x6440f144b7e50D6a8439336510312d2F54beB01D', chainId: 1, assetId: 2, chain: 'Ethereum', tx: 'https://etherscan.io/tx/{tx}' },
  },
});

/// `depositNative(bytes32,uint32)`; ERC-20 `approve(address,uint256)`,
/// `allowance(address,address)`, `balanceOf(address)`; the vault's
/// `deposit(bytes32,uint32,uint256)`.
const SEL = Object.freeze({ depositNative: '52d29193', approve: '095ea7b3', allowance: 'dd62ed3e', balanceOf: '70a08231', deposit: 'dedd8ed5' });
const word = (v) => BigInt(v).toString(16).padStart(64, '0');

/// `wad` (Zyn counts 18 places) in the chain's own units, rounded up so
/// what arrives covers it.
export function chainUnits(wad, decimals) {
  const scale = 10n ** BigInt(18 - decimals);
  return (wad + scale - 1n) / scale;
}

/// What to send so that `short` arrives: the bridge keeps `bps` of every
/// deposit (objectvm: the credit is the deposit less fee_of(deposit), the
/// fee rounded down), so the deposit is grossed up, in whole chain units.
export function grossUnits(short, bps, decimals) {
  const scale = 10n ** BigInt(18 - decimals);
  const B = 10_000n;
  const net = (u) => { const g = u * scale; return g - (g * BigInt(bps)) / B; };
  let units = chainUnits((short * B + (B - BigInt(bps)) - 1n) / (B - BigInt(bps)), decimals);
  while (net(units) < short) units += 1n;
  while (units > 1n && net(units - 1n) >= short) units -= 1n;
  return units;
}

/// Chain units as people read them: "0.55".
export function unitsText(units, decimals) {
  const d = 10n ** BigInt(decimals);
  const frac = (units % d).toString().padStart(decimals, '0').replace(/0+$/, '');
  return `${units / d}${frac ? `.${frac}` : ''}`;
}

/// What Nap answered for a deposit, checked against the pinned vault: the
/// calls to make, or a thrown error that says nothing was sent.
export function checkDeposit(network, key, r, units) {
  const v = VAULTS[network]?.[key];
  const refuse = () => { throw new Error('Nap named another vault; nothing was sent.'); };
  if (key === 'zec') {
    if (typeof r?.address !== 'string' || !/^(u1|utest1|t1|t3|tm|t2)[0-9a-zA-Z]+$/.test(r.address)) throw new Error('Nap has no Zcash deposit address for this account yet.');
    return { address: r.address };
  }
  if (!v) throw new Error(`No ${key.toUpperCase()} bridge on ${network}.`);
  if (key === 'sol') {
    if (r?.vault !== v.vault) refuse();
    if (!/^zyn:[0-9a-f]{64}$/.test(r?.memo || '')) throw new Error('No deposit code; nothing was sent.');
    return { memo: r.memo };
  }
  if (key === 'eth') {
    if (String(r?.to).toLowerCase() !== v.vault.toLowerCase() || r?.chainId !== v.chainId) refuse();
    if (!new RegExp(`^0x${SEL.depositNative}[0-9a-f]{64}${word(v.assetId)}$`).test(r?.data || '')) throw new Error('No deposit code; nothing was sent.');
    return { calls: [{ to: v.vault, data: r.data, value: `0x${units.toString(16)}` }] };
  }
  if (String(r?.to).toLowerCase() !== v.vault.toLowerCase() || String(r?.token).toLowerCase() !== v.token.toLowerCase() || r?.chainId !== v.chainId || r?.assetId !== v.assetId) refuse();
  if (!/^[0-9a-f]{64}$/.test(r?.code || '')) throw new Error('No deposit code; update Nap for BOLD deposits.');
  return {
    calls: [
      { to: v.token, data: `0x${SEL.approve}${word(BigInt(v.vault))}${word(units)}`, approve: true },
      { to: v.vault, data: `0x${SEL.deposit}${r.code}${word(v.assetId)}${word(units)}` },
    ],
  };
}

/// Whether the bridge takes `units` now: open on this network, and under
/// its cap if it has one. Nap's `zyn_bridges` says both.
export function checkOpen(state, key, units, decimals) {
  const b = (state?.bridges || []).find((x) => x.key === key);
  if (key === 'zec') return;
  if (!b?.live) throw new Error(b?.waiting === 'vault' ? `The vault does not take ${key.toUpperCase()} yet; nothing was sent.` : `${key.toUpperCase()} deposits are not open on this network yet; nothing was sent.`);
  if (b.capWei && b.issued != null) {
    const room = BigInt(b.capWei) - BigInt(b.issued);
    const wad = units * 10n ** BigInt(18 - decimals);
    if (wad > room) throw new Error(room > 0n ? `The bridge takes at most ${unitsText(room / 10n ** BigInt(18 - decimals), decimals)} ${key.toUpperCase()} more right now.` : 'The bridge is full for now: nothing more can come in.');
  }
}

let solFound = null;
/// The browser's Solana wallet, connected.
async function solWallet(say) {
  if (!solFound) solFound = solanaWallets();
  await new Promise((z) => setTimeout(z, 300));
  const w = solFound[0];
  if (!w) throw new Error('No Solana wallet in this browser. Install one (Phantom, Solflare, Backpack…) and reload.');
  say(`Connecting ${w.name}…`);
  const { accounts } = await w.features['standard:connect'].connect();
  if (!accounts?.[0]) throw new Error(`${w.name} shared no account.`);
  return { w, account: accounts[0] };
}

/// The browser's Ethereum wallet, on the vault's chain.
async function evmWallet(v, say) {
  const eth = globalThis.ethereum;
  if (!eth?.request) throw new Error('No Ethereum wallet in this browser. Install one (MetaMask, Rabby, Coinbase Wallet…) and reload.');
  const [from] = await eth.request({ method: 'eth_requestAccounts' });
  const chainId = `0x${v.chainId.toString(16)}`;
  if ((await eth.request({ method: 'eth_chainId' })) !== chainId) {
    say(`Switch your wallet to ${v.chain}…`);
    await eth.request({ method: 'wallet_switchEthereumChain', params: [{ chainId }] });
  }
  return { eth, from };
}

async function mined(eth, tx) {
  for (let i = 0; i < 100; i++) {
    const r = await eth.request({ method: 'eth_getTransactionReceipt', params: [tx] }).catch(() => null);
    if (r?.status === '0x1') return;
    if (r?.status === '0x0') throw new Error('The approval failed; nothing was deposited.');
    await new Promise((z) => setTimeout(z, 3000));
  }
  throw new Error('The approval has not confirmed yet. Try again once it has; it will not ask twice.');
}

/// Send `units` of `symbol` (chain units) to the bridge for the connected
/// account. `ask` is Nap; it is asked first, before anything is awaited,
/// because Nap in the browser may need to open its window and only a click
/// may. `from` picks ZEC's source: 'nap', or 'other' for any Zcash wallet.
/// Resolves to `{ tx, link }` once sent, or `{ address }` for a ZEC deposit
/// another wallet makes.
export async function send({ ask, network, symbol, units, bps = 0, from = 'nap', say = () => {} }) {
  const src = SOURCES[symbol];
  if (!src) throw new Error(`${symbol} has no bridge.`);
  const v = VAULTS[network][src.key] || {};
  const answer = ask('zyn_bridgeDeposit', { key: src.key });
  const plan = checkDeposit(network, src.key, await answer, units);
  const state = await ask('zyn_bridges', {});
  if (Number(state?.bps) > bps) throw new Error(`The bridge fee is ${Number(state.bps) / 100}% now; nothing was sent. Try again.`);
  checkOpen(state, src.key, units, src.decimals);
  const link = (tx) => (v.tx ? v.tx.replace('{tx}', encodeURIComponent(tx)) : null);
  if (src.key === 'zec') {
    if (from !== 'nap') return { address: plan.address };
    say(`Approve sending ${unitsText(units, 8)} ZEC in Nap. Proving it takes about half a minute; keep Nap's window open.`);
    const r = await ask('zec_pay', { address: plan.address, amount: unitsText(units, 8) });
    if (typeof r?.txid !== 'string') throw new Error('Nap did not say it sent the ZEC.');
    return { tx: r.txid, link: link(r.txid) };
  }
  if (src.key === 'sol') {
    const { w, account } = await solWallet(say);
    const r = await fetch(v.rpc, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'getLatestBlockhash', params: [{ commitment: 'finalized' }] }) });
    const blockhash = (await r.json().catch(() => ({})))?.result?.value?.blockhash;
    if (!blockhash) throw new Error('Solana did not answer; nothing was sent. Try again shortly.');
    const message = depositMessage({ owner: account.address, vault: v.vault, lamports: units, memo: plan.memo, blockhash });
    say(`Approve sending ${unitsText(units, 9)} SOL in ${w.name}.`);
    const tx = await signAndSend(w, account, unsigned(message), `solana:${v.cluster}`);
    return { tx, link: link(tx) };
  }
  const { eth, from: holder } = await evmWallet(v, say);
  const [first, last] = plan.calls.length === 2 ? plan.calls : [null, plan.calls[0]];
  if (first) {
    const owner = word(BigInt(holder));
    const held = BigInt(await eth.request({ method: 'eth_call', params: [{ to: v.token, data: `0x${SEL.balanceOf}${owner}` }, 'latest'] }) || '0x0');
    if (held < units) throw new Error(`This wallet holds ${unitsText(held, 18)} ${symbol}; nothing was sent.`);
    const allowed = BigInt(await eth.request({ method: 'eth_call', params: [{ to: v.token, data: `0x${SEL.allowance}${owner}${word(BigInt(v.vault))}` }, 'latest'] }) || '0x0');
    if (allowed < units) {
      say(`Step 1 of 2: approve the vault for ${unitsText(units, 18)} ${symbol} in your wallet.`);
      await mined(eth, await eth.request({ method: 'eth_sendTransaction', params: [{ from: holder, to: first.to, data: first.data }] }));
    }
    say(`Step 2 of 2: approve the deposit of ${unitsText(units, 18)} ${symbol} in your wallet.`);
  } else say(`Approve sending ${unitsText(units, 18)} ETH in your wallet.`);
  const tx = await eth.request({ method: 'eth_sendTransaction', params: [{ from: holder, to: last.to, data: last.data, ...(last.value ? { value: last.value } : {}) }] });
  return { tx, link: link(tx) };
}

/// Until the account holds what was asked: collect what has arrived, then
/// look. `collect` and `enough` are the page's; `canAsk` is false while
/// Nap's window is closed, and `say` is told so.
export async function arrive({ collect, enough, canAsk = () => true, say = () => {}, every = 15_000, limit = 50 * 60_000, now = () => Date.now(), sleep = (ms) => new Promise((z) => setTimeout(z, ms)) }) {
  const start = now();
  for (;;) {
    if (canAsk()) {
      try { await collect(); } catch { /* the next round */ }
      if (await enough().catch(() => false)) return;
    } else say("Open Nap's window to see it arrive.");
    if (now() - start > limit) throw new Error('Not arrived yet. It is safe: it reaches your account once the bridge credits it, and Jade picks it up next time.');
    await sleep(every);
  }
}

/// A deposit on its way, kept per network and account so a reload picks the
/// wait up again.
const KEY = (network, account) => `jade.fund.${network}.${account}`;
export function remember(network, account, pending) {
  try { localStorage.setItem(KEY(network, account), JSON.stringify(pending)); } catch { /* this tab only */ }
}
export function recall(network, account) {
  try {
    const p = JSON.parse(localStorage.getItem(KEY(network, account)) || 'null');
    return p && typeof p.asset === 'string' && /^\d+$/.test(p.target || '') && Number.isFinite(p.at) ? p : null;
  } catch { return null; }
}
export function forget(network, account) {
  try { localStorage.removeItem(KEY(network, account)); } catch { /* nothing kept */ }
}
