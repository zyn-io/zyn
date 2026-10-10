// A burn of REGI on Arc (or a test token on another EVM chain) for Collection
// Z sale credit (DECISIONS §182).
//
// The token's own `transfer(0x…dEaD, amount)` with one more 32-byte word
// appended: the commitment from the holder's Nap. The token ignores the
// extra word; the operator's watcher reads it from the transaction, which
// is the whole proof. Built here, signed by the holder's EVM wallet
// (EIP-1193), so no SDK and no contract of ours is involved.

export const DEAD = '000000000000000000000000000000000000dead';
const TRANSFER = 'a9059cbb';
const BALANCE_OF = '70a08231';
const EXTSLOAD = '1e2eaeaf';

const word = (hex) => hex.replace(/^0x/, '').toLowerCase().padStart(64, '0');

/// The calldata: transfer to 0x…dEaD, then the commitment.
export function burnData(amount, commitment) {
  if (!/^[0-9a-f]{64}$/.test(commitment)) throw new Error('the commitment must be the one Nap gave');
  const a = BigInt(amount);
  if (a <= 0n || a >= 2n ** 256n) throw new Error('burn more than nothing');
  return `0x${TRANSFER}${word(DEAD)}${a.toString(16).padStart(64, '0')}${commitment}`;
}

async function rpc(url, method, params) {
  const r = await fetch(url, {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }), credentials: 'omit', referrerPolicy: 'no-referrer',
  });
  const j = await r.json().catch(() => ({}));
  if (!r.ok || j.error || !('result' in j)) throw new Error('The chain did not answer; try again shortly.');
  return j.result;
}

/// The token's balance of `owner`, in base units.
export async function balanceOf(cfg, owner) {
  const r = await rpc(cfg.rpc, 'eth_call', [{ to: cfg.token, data: `0x${BALANCE_OF}${word(owner)}` }, 'latest']);
  return BigInt(r || '0x0');
}

/// The token's USD price from its Uniswap v4 pool: `slot0`'s sqrtPriceX96,
/// read from the PoolManager's storage at the pool's state slot (fixed by the
/// pool id, so the config carries it).
export async function v4Usd(cfg) {
  const r = await rpc(cfg.rpc, 'eth_call', [{ to: cfg.v4.manager, data: `0x${EXTSLOAD}${cfg.v4.slot}` }, 'latest']);
  const sqrt = BigInt(r || '0x0') & ((1n << 160n) - 1n);
  if (sqrt === 0n) throw new Error("The token's pool has no price.");
  const ratio = Number(sqrt) / 2 ** 96;
  const raw = ratio * ratio; // currency1 base units per currency0 base unit
  const tokenIs0 = cfg.token.toLowerCase() < cfg.v4.quote.toLowerCase();
  const scale = 10 ** (cfg.decimals - cfg.v4.quote_decimals);
  return tokenIs0 ? raw * scale : scale / raw;
}

/// The holder's wallet, on the token's chain: connect, switch (adding the
/// chain if the wallet does not know it), and return the account.
export async function connect(cfg) {
  const eth = globalThis.ethereum;
  if (!eth?.request) throw new Error('No EVM wallet found in this browser. Install one (MetaMask, Rabby) and reload.');
  const [account] = await eth.request({ method: 'eth_requestAccounts' });
  if (!account) throw new Error('The wallet shared no account.');
  const want = `0x${Number(cfg.chain_id).toString(16)}`;
  if ((await eth.request({ method: 'eth_chainId' })) !== want) {
    try {
      await eth.request({ method: 'wallet_switchEthereumChain', params: [{ chainId: want }] });
    } catch (e) {
      if (e?.code !== 4902 || !cfg.add_chain) throw e;
      await eth.request({ method: 'wallet_addEthereumChain', params: [{ chainId: want, ...cfg.add_chain }] });
    }
  }
  return account;
}

/// Send the burn from `account`; the transaction hash comes back.
export async function send(cfg, account, amount, commitment) {
  return globalThis.ethereum.request({ method: 'eth_sendTransaction', params: [{ from: account, to: cfg.token, data: burnData(amount, commitment) }] });
}
