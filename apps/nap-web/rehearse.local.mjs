// Testnet rehearsal of the bridge, through the wallet's own code. Not shipped.
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { x25519 } from '@noble/curves/ed25519.js';
import { deriveIdentity, newMnemonic, checkedEpoch } from './src/core.js';
import { scan, objectCall, registerIntent, signedFrame, submitSigned, withdrawIntent, hex, unhex } from './src/object.js';
import { fund } from './src/market.js';

const RPC = 'https://zyn-rpc.cavebro.win/object', CHAIN = 212;
const ZEC = '0e212262cb35bf6e88bc7d0a52b60a5cc70c2859b65966a2557229d2ea469201';
const [,, dir, cmd, ...args] = process.argv;
const pf = `${dir}/rehearsal.phrase`;
const words = existsSync(pf) ? readFileSync(pf, 'utf8').trim() : newMnemonic();
writeFileSync(pf, words);
const id = deriveIdentity(words);

async function submit(intent) {
  const epoch = checkedEpoch(await objectCall(RPC, 'zyn_status'), CHAIN);
  const r = await submitSigned(RPC, await signedFrame(id.secretKey, CHAIN, epoch + 100n, intent));
  if (/Refused/.test(r.receipts)) throw new Error(r.receipts);
  return r.receipts;
}
async function held() {
  return scan(await objectCall(RPC, 'zyn_published'), { secret: id.viewing, nkey: id.nullifier, owner: id.account });
}

if (cmd === 'init') {
  let r = await objectCall(RPC, 'zyn_depositAddress', { account: id.accountHex });
  console.log('account', id.accountHex, 'registered', r.registered);
  if (!r.registered) {
    console.log('register:', (await submit(registerIntent(id.account, x25519.getPublicKey(id.viewing)))).slice(0, 60));
    r = await objectCall(RPC, 'zyn_depositAddress', { account: id.accountHex });
  }
  console.log('registered', r.registered, '\naddress', r.address);
} else if (cmd === 'balance') {
  const { balances } = await held();
  console.log('ZEC.zy', String(balances.get(ZEC) || 0n));
} else if (cmd === 'withdraw') {
  const [address, zec] = args;
  const raw = BigInt(Math.round(Number(zec) * 1e8)) * 10_000_000_000n;
  const salt = hex(crypto.getRandomValues(new Uint8Array(32)));
  const { destination } = await objectCall(RPC, 'zyn_exitCommitment', { address, salt });
  const { held: h } = await held();
  const { inputs, change } = fund(id, h, unhex(ZEC), raw);
  console.log('withdraw:', (await submit(withdrawIntent({ holder: id.account, inputs, asset: unhex(ZEC), amount: raw, destination: unhex(destination), outputs: change }))).slice(0, 80));
  console.log('reveal:', JSON.stringify(await objectCall(RPC, 'zyn_exitReveal', { address, salt })));
}
