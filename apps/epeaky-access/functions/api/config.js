// What the page needs: the den it answers, the amounts, the token's transfer fee.
import { amounts, json, rpc } from '../../lib/env.mjs';

export async function onRequestGet({ env }) {
  let fee = null;
  try {
    const mint = await rpc(env, 'getAccountInfo', [env.MINT, { encoding: 'jsonParsed' }]);
    const ext = mint?.value?.data?.parsed?.info?.extensions?.find((e) => e.extension === 'transferFeeConfig');
    if (ext) fee = ext.state.newerTransferFee.transferFeeBasisPoints;
  } catch { /* shown as unknown */ }
  return json({ den: new URL(env.DEN_URL).origin, amounts: amounts(env), mint: env.MINT, feeBps: fee });
}
