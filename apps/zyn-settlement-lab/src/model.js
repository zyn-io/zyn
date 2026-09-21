export const ZAT = 100_000_000;
export const PRICE = 600_000;
export const CLAIM = 500_000;
export const DEPOSIT = 1_000_000;
export const FEE_BPS = 100;
export const SEED = 3_000_000;
export const ITEMS = [
  { id: 'shielded-byte-001', name: 'Shielded Byte', token: '001', collection: 'Zyn Privacy Relics', light: 'Laser', image: '/assets/shielded-byte.png' },
];
export const STEPS = [
  { name: 'Fund', layer: 'Zcash', title: 'Start with shielded ZEC.', description: 'Move 0.01000000 ZEC from Wallet A into the vault. Its Zyn balance is credited on confirmation.', action: 'Simulate deposit', amount: '0.01000000', note: 'Mainnet step in the final recording' },
  { name: 'Claim', layer: 'Zcash', title: 'One piece. A new owner.', description: 'A separate 0.00500000 ZEC claim payment assigns this piece to Wallet A. Sale proceeds stay separate from the seeded reserve.', action: 'Simulate paid claim', amount: '0.00500000', note: 'Creator-authorized mint after payment' },
  { name: 'Trade', layer: 'Zyn', title: 'Trade at the speed of Zyn.', description: 'Wallet B buys from A for 0.00600000 ZEC.zy. Each side pays 1%; half of the combined fees goes to the reserve.', action: 'Sell A → B', amount: '0.00600000', note: 'No new Zcash transaction for this trade' },
  { name: 'Force', layer: 'Zcash + Zyn', title: 'What if the sequencer says no?', description: 'Try the return sale. The demo sequencer refuses it. Then submit the same operation through a ZYF memo.', action: 'Attempt return sale', amount: '0.00600000', note: 'Deliberate censorship simulation' },
  { name: 'Redeem', layer: 'Zyn', title: 'The reserve becomes spendable.', description: 'Burn the piece and credit Wallet A with its current reserve share. Watch the pool and outstanding supply update together.', action: 'Redeem this piece', note: 'Redemption is credited in ZEC.zy' },
  { name: 'Anchor', layer: 'Zcash', title: 'Three transitions. One anchor.', description: 'Commit both trades and the redemption in one simulated ZYA anchor. Inspect exactly which transitions it covers.', action: 'Create demo anchor', note: 'A commitment is not a Zcash validation of each trade' },
  { name: 'Withdraw', layer: 'Zcash', title: 'Complete the round trip.', description: 'Withdraw the redeemed amount from Wallet A’s Zyn balance to its shielded Zcash balance.', action: 'Simulate shielded withdrawal', note: 'Network fees excluded from this rehearsal' },
];
export function createState(selected = ITEMS[0].id) {
  if (!ITEMS.some(i => i.id === selected)) throw new Error('Unknown item');
  return { version: 1, selected, step: 0, owner: null, a: 0, b: 2_000_000, externalA: 2_000_000,
    pool: SEED, outstanding: 5, creator: 0, reserveFees: 0, redeemed: 0, blocked: false,
    forced: false, transitions: [], anchor: null, chainTxs: 0, events: [], floors: [], counter: 0 };
}
export function floor(s) { return s.outstanding ? Math.floor(s.pool / s.outstanding) : 0; }
export function format(zat, digits = 8) { return (zat / ZAT).toFixed(digits); }
function event(s, title, layer, details) {
  s.counter += 1;
  const record = { id: `SIM-${String(s.counter).padStart(3, '0')}`, title, layer, details };
  s.events.push(record);
  return record.id;
}
function trade(s, from, to, forced = false) {
  const each = Math.floor(PRICE * FEE_BPS / 10_000);
  if (s.owner !== from || s[to.toLowerCase()] < PRICE + each) throw new Error('Trade cannot be funded');
  s[to.toLowerCase()] -= PRICE + each;
  s[from.toLowerCase()] += PRICE - each;
  s.pool += each;
  s.creator += each;
  s.reserveFees += each;
  s.owner = to;
  const id = event(s, `${from} → ${to} · ${forced ? 'forced sale accepted' : 'sale complete'}`, forced ? 'Zcash + Zyn' : 'Zyn',
    `Price ${format(PRICE)} ZEC.zy. Buyer and seller fee: ${format(each)} each. Reserve +${format(each)}; creator +${format(each)}.${forced ? ' Same operation: SIM-RETURN-001, accepted once.' : ''}`);
  s.transitions.push(id);
  s.floors.push(floor(s));
}
export function advance(current) {
  const s = structuredClone(current);
  if (s.step >= STEPS.length) throw new Error('Rehearsal is complete');
  switch (s.step) {
    case 0:
      s.externalA -= DEPOSIT; s.a += DEPOSIT; s.chainTxs++;
      event(s, 'Wallet A funded', 'Zcash', 'Simulated vault deposit: 0.01000000 ZEC credited as ZEC.zy. No transaction broadcast.'); break;
    case 1:
      s.externalA -= CLAIM; s.creator += CLAIM; s.outstanding++; s.owner = 'A'; s.chainTxs++;
      s.floors.push(floor(s));
      event(s, 'Claim confirmed · A owns the piece', 'Zcash', 'Simulated primary payment: 0.00500000 ZEC to sale proceeds. Reserve was seeded with 0.03000000 ZEC; five other pieces were already minted.'); break;
    case 2: trade(s, 'A', 'B'); break;
    case 3:
      if (!s.blocked) {
        s.blocked = true;
        event(s, 'Return sale refused', 'Sequencer', 'Fault injection for rehearsal. SIM-RETURN-001 remains pending; no balances or ownership changed.');
        return s;
      }
      s.chainTxs++; s.forced = true;
      trade(s, 'B', 'A', true); break;
    case 4:
      if (s.owner !== 'A') throw new Error('Only the holder can redeem');
      s.redeemed = floor(s); s.a += s.redeemed; s.pool -= s.redeemed; s.outstanding--; s.owner = 'burned';
      s.transitions.push(event(s, 'Piece redeemed · reserve paid', 'Zyn', `${format(s.redeemed)} ZEC.zy credited to Wallet A. One piece burned; ${s.outstanding} remain outstanding.`)); break;
    case 5:
      s.chainTxs++;
      s.anchor = { id: 'SIM-ZYA-001', epoch: 30, transitions: [...s.transitions] };
      event(s, 'Batch anchored · 3 transitions', 'Zcash', 'Simulated ZYA commitment for epoch 30. Covers the two sales and redemption. This receipt is not a mainnet txid or cryptographic proof.'); break;
    case 6:
      if (s.a < s.redeemed) throw new Error('Insufficient balance');
      s.a -= s.redeemed; s.externalA += s.redeemed; s.chainTxs++;
      event(s, 'Shielded withdrawal received', 'Zcash', `${format(s.redeemed)} ZEC returned to Wallet A’s shielded balance in rehearsal. Network fees are excluded.`); break;
  }
  s.step++;
  return s;
}
export function totalValue(s) { return s.a + s.b + s.externalA + s.pool + s.creator; }
