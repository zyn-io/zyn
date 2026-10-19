// The settings the functions share (wrangler.toml vars, Pages secrets).
import { decode } from '../public/shared.mjs';

export const SOL_MINT = 'So11111111111111111111111111111111111111112';
const JUP = 'https://lite-api.jup.ag/swap/v1';

export const amounts = (env) => String(env.AMOUNTS || '0.1,0.25,0.5').split(',').map((s) => s.trim());
export const lamports = (sol) => Math.round(Number(sol) * 1e9);
export const minLamports = (env) => Math.min(...amounts(env).map(lamports));

export const json = (body, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', 'cache-control': 'no-store' } });

export function isWallet(s) {
  try { return typeof s === 'string' && decode(s).length === 32; } catch { return false; }
}

/// SOLANA_RPC is a comma-separated list, tried in order: public nodes refuse
/// some requests from Cloudflare, so one that fails hands over to the next.
export async function rpc(env, method, params) {
  let last = new Error('rpc');
  for (const url of String(env.SOLANA_RPC || 'https://solana-rpc.publicnode.com,https://api.mainnet-beta.solana.com').split(',')) {
    try {
      const r = await fetch(url.trim(), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
      });
      const a = await r.json();
      if (!a.error) return a.result;
      last = new Error(a.error.message || 'rpc');
    } catch (e) { last = e; }
  }
  throw last;
}

/// Jupiter's quote for one of the fixed amounts; anything else is refused.
export async function quote(env, sol) {
  if (!amounts(env).includes(sol)) return null;
  const u = new URL(`${JUP}/quote`);
  u.search = new URLSearchParams({ inputMint: SOL_MINT, outputMint: env.MINT, amount: String(lamports(sol)), slippageBps: '300' });
  const r = await fetch(u);
  if (!r.ok) throw new Error('quote');
  return r.json();
}

/// What Jupiter gives now for `lamports` of SOL, in the mint's base units:
/// the market price a claimed buy is held to. (MARKET_QUOTE: another quote
/// URL of the same form, for tests only.)
export async function marketOut(env, lamports) {
  const u = new URL(env.MARKET_QUOTE || `${JUP}/quote`);
  u.search = new URLSearchParams({ inputMint: SOL_MINT, outputMint: env.MINT, amount: String(lamports), slippageBps: '300' });
  const r = await fetch(u);
  if (!r.ok) throw new Error('quote');
  const out = (await r.json()).outAmount;
  if (!/^\d+$/.test(String(out))) throw new Error('quote');
  return out;
}

/// A transaction signature: 64 bytes of base58. Checked before asking a node.
export function isSignature(s) {
  try { return typeof s === 'string' && s.length <= 100 && decode(s).length === 64; } catch { return false; }
}

export async function swap(q, wallet) {
  const r = await fetch(`${JUP}/swap`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ quoteResponse: q, userPublicKey: wallet, dynamicComputeUnitLimit: true, prioritizationFeeLamports: 'auto' }),
  });
  if (!r.ok) throw new Error('swap');
  return (await r.json()).swapTransaction;
}
