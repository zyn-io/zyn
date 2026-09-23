// The Zcash wallet's loop: sync and send, over any transport.
//
// `zec` is the wasm core (`../zec-pkg/nap_zec.js`); `post(frame)` carries one
// block-server request frame and resolves to the reply frame. The worker
// passes a fetch to `/zec/<network>`; the tests pass a relay to a real server.
// The core decides what to ask for and checks every answer; this file only
// sequences the asking.

/// Blocks per request. The server allows more, but a first scan makes it read
/// each block from Zebra, and a request has to finish inside the proxy's
/// timeout.
export const BATCH = 100;

/// How far behind the tip a block must be before it is scanned — the core's
/// figure, repeated here only to compute the target height.
const CONFIRMATIONS = 3;

export function engine(zec, post) {
  let wallet = null;
  let network = null;

  const call = async (frame) => zec.replyBody(await post(frame));
  const info = async () => {
    const i = JSON.parse(zec.decodeInfo(await call(zec.infoRequest())));
    // The server says which network it serves. A wallet that scanned the
    // other one would show a balance that is not there.
    if (i.network !== network) throw new Error(`The block server is on ${i.network}, not ${network}`);
    return i;
  };
  const need = () => { if (!wallet) throw new Error('No Zcash wallet is open'); return wallet; };

  function summary(tip = null) {
    const w = need();
    return {
      network,
      address: w.address(),
      hasState: w.hasState(),
      syncedTo: w.syncedTo(),
      birthday: w.birthday(),
      tip,
      balance: JSON.parse(w.balance()),
      notes: JSON.parse(w.notes()),
    };
  }

  return {
    /// Both networks' spending keys for a phrase: the words are chain
    /// independent, and switching network must not cost a restore.
    derive(phrase, passphrase, account) {
      return {
        mainnet: zec.spendingKeyFromPhrase(phrase, passphrase, 'mainnet', account),
        testnet: zec.spendingKeyFromPhrase(phrase, passphrase, 'testnet', account),
      };
    },

    /// Open the wallet for `sk` on `net`, resuming `state` when it reads.
    /// A state that does not read (another network, a damaged record) is
    /// dropped: it costs a rescan, never the money.
    open(sk, net, state) {
      wallet?.free();
      network = net;
      wallet = new zec.ZecWallet(sk, net);
      let resumed = false;
      if (state) {
        try { wallet.loadState(state); resumed = true; } catch { /* rescan */ }
      }
      return { ...summary(), resumed };
    },

    /// Fresh trees at `birthday` (or the tip, for a wallet made today).
    async seed(birthday) {
      const w = need();
      const i = await info();
      const at = Math.max(1, Math.min(Number.isInteger(birthday) ? birthday : i.tip, i.tip));
      const orchard = await call(zec.treeStateRequest(at - 1, 0));
      const ironwood = await call(zec.treeStateRequest(at - 1, 1));
      w.seed(at, orchard, ironwood);
      return { ...summary(i.tip), state: w.state() };
    },

    /// Scan to three blocks behind the tip. `onStep` hears each batch: the
    /// height reached, what happened, and the state to keep.
    async sync(onStep = () => {}) {
      const w = need();
      const i = await info();
      const to = i.tip - CONFIRMATIONS;
      while (w.nextHeight() <= to) {
        const from = w.nextHeight();
        const count = Math.min(BATCH, to - from + 1);
        const wanted = w.takeBlocks(await call(zec.blocksRequest(from, count)), to);
        for (let k = 0; k < wanted; k++) {
          w.giveTransaction(k, await call(w.transactionRequest(k)));
        }
        const events = JSON.parse(w.apply());
        if (w.nextHeight() === from) throw new Error(`The block server has no block ${from} yet`);
        onStep({ at: w.syncedTo(), to, events, state: w.state() });
      }
      return { ...summary(i.tip), state: w.state() };
    },

    /// Sync, then build, prove, sign and broadcast one payment. Proving is
    /// the slow part, and it happens here, on this thread.
    async send(to, zatoshi, memo, onStep = () => {}) {
      const w = need();
      await this.sync(onStep);
      const i = await info();
      onStep({ proving: true });
      const raw = w.buildPayment(to, zatoshi, memo || '', i.tip);
      const txid = zec.decodeSent(await call(zec.sendRequest(raw)));
      w.confirmSent();
      return { ...summary(i.tip), txid, state: w.state() };
    },

    summary: () => summary(),
  };
}
