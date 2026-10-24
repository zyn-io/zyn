// What a connected site may read beyond its connection: the holder's leave,
// one scope at a time (audit, 9 Oct 2026).
//
// Connecting shows a site the Zyn account and a Zcash address, and lets it
// ask for balances and items. A few reads say far more: the private
// channel's notes carry commitments the other chain shows beside the wallet
// that deposited them, and lending positions are public ids whose fate anyone
// can follow. Those were read without a prompt by any connected site. Now a
// method that names a scope runs only once the holder has allowed that scope
// for that site, on a card naming exactly what goes; until then the call
// shows the card.
//
// The leave is kept with the connection record (origin → network, account,
// scopes) in localStorage, so it holds per site, network and wallet. It goes
// with the connection: when the site disconnects, or the holder connects it
// again on another network or from another wallet. No DOM here: the connect
// window, the phone app's browser and the tests share it.

/// The scopes, as their cards say them. `what` names it, `shows` is what the
/// site gets, `risk` what it can learn from that — shown before Allow.
export const SCOPES = Object.freeze({
  notes: Object.freeze({
    what: 'your private-channel notes',
    shows: "Each note's channel, size and state, and the commitment of each note not yet spent",
    risk: 'A commitment is public on the chain the note was deposited on: with it the site can match the note to the deposit that made it, and so to the wallet that sent it.',
  }),
  positions: Object.freeze({
    what: 'your lending positions',
    shows: 'Each Zyn Lend position: its id, collateral, debt and health',
    risk: 'Position ids are public on Zyn: with them the site can follow each position — repayments, top-ups and liquidation — as yours.',
  }),
  payments: Object.freeze({
    what: 'the payments you set up with it',
    shows: 'Each scheduled payment this site asked you to approve: payee, amount, schedule, what is set aside, next date and state. Never a schedule another site set up',
    risk: 'Anyone holding a schedule id can read that schedule: what is set aside, its state and its next date.',
  }),
});

/// Whether `origin` is connected on `net` to the wallet `account`.
export function connectedTo(list, origin, net, account) {
  const c = list?.[origin];
  return !!account && c?.network === net && c.account === account;
}

/// Whether the holder of `account` has allowed `scope` to `origin` on `net`.
export function allowed(list, origin, net, account, scope) {
  const c = list?.[origin];
  return !!account && c?.network === net && c.account === account && Array.isArray(c.scopes) && c.scopes.includes(scope);
}

/// The list with `origin` connected afresh: a new connection starts with no
/// scopes, whatever an earlier one on another network or wallet had.
export function withConnection(list, origin, net, account, at = Date.now()) {
  // The schedules a site set up stay with its record while the same wallet
  // connects it again (on this network or another, each kept apart); the
  // payments scope that reads them is asked again like any other.
  const pay = list?.[origin]?.account === account ? payMap(list[origin]) : {};
  return { ...list, [origin]: { network: net, account, at, scopes: [], ...(Object.keys(pay).length ? { pay } : {}) } };
}

/// The list with `scope` allowed to `origin`'s connection on `net`. A record
/// from before the account was kept (the phone app's) takes the account now;
/// a record for another network or wallet is left as it is.
export function withScope(list, origin, net, account, scope) {
  const c = list?.[origin];
  if (!c || !SCOPES[scope] || c.network !== net || (c.account && c.account !== account)) return list;
  const scopes = Array.isArray(c.scopes) ? c.scopes.filter((s) => SCOPES[s]) : [];
  return { ...list, [origin]: { ...c, account, scopes: scopes.includes(scope) ? scopes : [...scopes, scope] } };
}

/// The list without `origin`: its connection and every scope with it.
export function without(list, origin) {
  const { [origin]: _gone, ...rest } = list || {};
  return rest;
}

/// The scopes `origin` holds on `net` for `account`, as their cards name them.
export function scopesOf(list, origin, net, account) {
  const c = list?.[origin];
  if (!connectedTo(list, origin, net, account) || !Array.isArray(c.scopes)) return [];
  return c.scopes.filter((s) => SCOPES[s]);
}

// Zyn Pay (§188): a site sees only the schedules it asked for. Each schedule
// id a site's request created is kept in that site's connection record, per
// network, and `zyn_paySchedules` answers from that list alone. Schedule ids
// are access to the record (§188.11): they are kept here and nowhere else.

/// Most schedule ids kept for one site on one network; the oldest go first.
export const MAX_PAY_IDS = 1000;
const isId = (v) => typeof v === 'string' && /^[0-9a-f]{64}$/.test(v);
const payMap = (c) => {
  const out = {};
  if (c?.pay && typeof c.pay === 'object' && !Array.isArray(c.pay)) {
    for (const [net, ids] of Object.entries(c.pay)) if (Array.isArray(ids)) out[net] = ids.filter(isId);
  }
  return out;
};

/// The list with schedule `id` kept for `origin` on `net`. Like `withScope`:
/// a record from before the account was kept takes the account now; a record
/// for another network or wallet, or none, is left as it is.
export function withPaySchedule(list, origin, net, account, id) {
  const c = list?.[origin];
  if (!c || !isId(id) || !account || c.network !== net || (c.account && c.account !== account)) return list;
  const pay = payMap(c);
  const ids = pay[net] || [];
  if (ids.includes(id)) return list;
  return { ...list, [origin]: { ...c, account, pay: { ...pay, [net]: [...ids, id].slice(-MAX_PAY_IDS) } } };
}

/// The schedule ids `origin` set up on `net` from the wallet `account`.
export function paySchedulesOf(list, origin, net, account) {
  if (!connectedTo(list, origin, net, account)) return [];
  return payMap(list[origin])[net] || [];
}
