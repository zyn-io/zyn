export const LIVE_STEPS = [
  { name: 'Deposit', layer: 'Zcash', title: 'Confirm Wallet A is funded.', description: 'Use the daemon-issued shielded address, then verify at least 0.01000000 ZEC.zy is spendable.', amount: '0.01000000', action: 'Check funded balance' },
  { name: 'Claim', layer: 'Zyn + creator', title: 'Fund the reserve, then mint.', description: 'Wallet A funds the collection. The creator daemon independently mints Shielded Byte to A.', amount: '0.00500000', action: 'Claim live item' },
  { name: 'Sell', layer: 'Zyn', title: 'Sell from A to B.', description: 'Wallet A creates a one-item offer. Wallet B accepts it as a separate signed intent.', amount: '0.00600000', action: 'Offer and take' },
  { name: 'Force return', layer: 'Zcash → Zyn', title: 'Return through forced inclusion.', description: 'Wallet B sends the item back to A through the daemon’s ZYF forced-transfer path.', amount: '1', action: 'Force item transfer' },
  { name: 'Redeem', layer: 'Zyn', title: 'Redeem against the live reserve.', description: 'Wallet A burns the returned item and receives its current on-chain redemption value.', amount: 'Live floor', action: 'Redeem live item' },
  { name: 'Withdraw', layer: 'Zyn → Zcash', title: 'Exit to shielded ZEC.', description: 'Bind Wallet A to its deposit address if needed, then withdraw the exact redemption amount.', amount: 'Redeemed value', action: 'Prepare withdrawal' },
  { name: 'Anchor', layer: 'Zcash', title: 'Wait for scheduled settlement.', description: 'Read the real anchor list until an anchor covers the latest epoch in this run.', amount: '', action: 'Check for anchor' },
];

export const LIVE_DEFAULTS = Object.freeze({
  deposit: '0.01',
  claim: '0.005',
  price: '0.006',
  itemSymbol: 'BYTE',
  content: 'cb08f65eec9fd64d89c240ab2a0f8ec0f0db77f0ede32b4196c28d47158c17dd',
});

export class LiveApiError extends Error {
  constructor(message, { status = 0, uncertain = false, data = null } = {}) {
    super(message);
    this.name = 'LiveApiError';
    this.status = status;
    this.uncertain = uncertain;
    this.data = data;
  }
}

export class LiveApi {
  async request(role, path, { method = 'GET', body } = {}) {
    try {
      const response = await fetch(`/daemon/${role}${path}`, {
        method,
        headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: AbortSignal.timeout(method === 'GET' ? 9000 : 16000),
      });
      const text = await response.text();
      let data = {};
      try { data = text ? JSON.parse(text) : {}; }
      catch { throw new LiveApiError('The daemon returned an unreadable response.', { status: response.status, uncertain: method === 'POST' }); }
      if (!response.ok) throw new LiveApiError(data.error || `${method} ${path} failed.`, { status: response.status, uncertain: method === 'POST' && response.status >= 500, data });
      return data;
    } catch (error) {
      if (error instanceof LiveApiError) throw error;
      throw new LiveApiError(error?.name === 'TimeoutError' ? 'The daemon response timed out.' : 'The daemon connection was interrupted.', { uncertain: method === 'POST' });
    }
  }
  get(role, path) { return this.request(role, path); }
  post(role, path, body = {}) { return this.request(role, path, { method: 'POST', body }); }
}

export function decimalToUnits(value, places = 8) {
  const match = String(value ?? '0').trim().match(/^(\d+)(?:\.(\d+))?$/);
  if (!match) throw new Error(`Invalid decimal amount: ${value}`);
  const fraction = (match[2] || '').padEnd(places, '0');
  if (fraction.length > places) throw new Error(`Amount has more than ${places} decimal places.`);
  return BigInt(match[1]) * 10n ** BigInt(places) + BigInt(fraction || '0');
}

export function unitsToDecimal(value, places = 8) {
  const units = typeof value === 'bigint' ? value : BigInt(value || 0);
  const base = 10n ** BigInt(places);
  return `${units / base}.${String(units % base).padStart(places, '0')}`;
}

export function xzecBalance(overview) {
  return overview?.record?.spendable?.find(entry => Number(entry.asset) === 1)?.amount || '0';
}

export function findCollection(snapshot, collectionId) {
  if (collectionId === null || collectionId === undefined || collectionId === '') return null;
  const id = Number(collectionId);
  for (const role of ['creator', 'a', 'b']) {
    const found = snapshot?.wallets?.[role]?.collections?.find(collection => Number(collection.id) === id);
    if (found) return found;
  }
  return null;
}

export function findHeldItem(snapshot, { asset, collectionId, content = LIVE_DEFAULTS.content } = {}) {
  const wantedContent = String(content).toLowerCase().replace(/^0x/, '');
  for (const role of ['a', 'b', 'creator']) {
    const items = snapshot?.wallets?.[role]?.items || [];
    const found = items.find(item => asset != null
      ? Number(item.asset) === Number(asset)
      : String(item.content || '').toLowerCase().replace(/^0x/, '') === wantedContent && (collectionId == null || Number(item.collection) === Number(collectionId)));
    if (found) return { role, item: found };
  }
  return null;
}

export async function readLiveSnapshot(api = new LiveApi()) {
  const [creator, a, b, offers, anchors, deposit] = await Promise.all([
    api.get('creator', '/api/overview'),
    api.get('a', '/api/overview'),
    api.get('b', '/api/overview'),
    api.get('a', '/api/offers'),
    api.get('a', '/api/anchors'),
    api.get('a', '/api/deposit-address'),
  ]);
  return {
    wallets: { creator, a, b },
    offers: offers.offers || a.offers || [],
    anchors: anchors.anchors || [],
    deposit,
    readAt: new Date().toISOString(),
  };
}

export function createLiveState(config = {}) {
  const rawCollection = config.collectionId ?? import.meta.env?.VITE_ZYN_COLLECTION_ID ?? '';
  return {
    connected: false,
    step: 0,
    snapshot: null,
    collectionId: rawCollection === '' ? null : Number(rawCollection),
    content: config.content || import.meta.env?.VITE_ZYN_CONTENT_HASH || LIVE_DEFAULTS.content,
    itemAsset: null,
    owner: null,
    events: [],
    targetEpoch: null,
    claimFunded: false,
    claimMinted: false,
    saleOfferId: null,
    forcedSubmitted: false,
    redeemAmount: null,
    boundRequested: false,
    withdrawn: false,
    anchor: null,
    unknown: null,
    error: '',
  };
}

function sync(state, snapshot) {
  const held = findHeldItem(snapshot, { asset: state.itemAsset, collectionId: state.collectionId, content: state.content });
  return {
    ...state,
    connected: true,
    snapshot,
    itemAsset: state.itemAsset ?? held?.item?.asset ?? null,
    owner: held?.role || (state.claimMinted ? 'burned' : null),
  };
}

export async function connectLive(state, api = new LiveApi()) {
  return sync({ ...state, error: '' }, await readLiveSnapshot(api));
}

function addReceipt(state, title, layer, role, path, result, details) {
  const receipt = {
    id: result.txid || (result.seq != null ? `seq-${result.seq}` : `${role}-${Date.now()}`),
    title,
    layer,
    role,
    path,
    seq: result.seq ?? null,
    epoch: result.epoch ?? null,
    txid: result.txid ?? null,
    details,
    at: new Date().toISOString(),
  };
  const epoch = result.epoch == null ? state.targetEpoch : Math.max(state.targetEpoch ?? 0, Number(result.epoch));
  return { ...state, events: [...state.events, receipt], targetEpoch: epoch };
}

function findSaleOffer(state) {
  const account = state.snapshot?.wallets?.a?.account;
  return state.snapshot?.offers?.find(offer =>
    String(offer.maker).toLowerCase() === String(account).toLowerCase()
    && Number(offer.offer_asset) === Number(state.itemAsset)
    && decimalToUnits(offer.want_amount) === decimalToUnits(LIVE_DEFAULTS.price));
}

function collectionPool(state) {
  return findCollection(state.snapshot, state.collectionId)?.pool || '0';
}

function includeObservedEpoch(state) {
  const epochs = Object.values(state.snapshot?.wallets || {}).map(view => Number(view?.status?.epoch)).filter(Number.isFinite);
  if (!epochs.length) return state;
  return { ...state, targetEpoch: Math.max(state.targetEpoch ?? 0, ...epochs) };
}

async function refresh(state, api) {
  return sync(state, await readLiveSnapshot(api));
}

async function reconcileUnknown(state, api) {
  let next = await refresh({ ...state, error: '' }, api);
  const unknown = state.unknown;
  let resolved = false;
  if (unknown.path === '/api/fund') {
    resolved = decimalToUnits(collectionPool(next)) > decimalToUnits(unknown.before || '0');
    if (resolved) next.claimFunded = true;
  } else if (unknown.path === '/api/mint') {
    resolved = next.owner === 'a';
    if (resolved) { next.claimMinted = true; next.step = 2; }
  } else if (unknown.path === '/api/offer') {
    const offer = findSaleOffer(next);
    resolved = Boolean(offer);
    if (offer) next.saleOfferId = offer.id;
  } else if (unknown.path === '/api/take') {
    resolved = next.owner === 'b';
    if (resolved) next.step = 3;
  } else if (unknown.path === '/api/force') {
    resolved = next.owner === 'a';
    if (resolved) next.step = 4;
  } else if (unknown.path === '/api/redeem') {
    resolved = next.owner !== 'a';
    if (resolved) next.step = 5;
  } else if (unknown.path === '/api/bind') {
    resolved = Boolean(next.snapshot.wallets.a.record?.bound_here);
  } else if (unknown.path === '/api/withdraw') {
    const exiting = next.snapshot.wallets.a.record?.exiting || [];
    resolved = exiting.length > Number(unknown.before || 0) || decimalToUnits(xzecBalance(next.snapshot.wallets.a)) < decimalToUnits(unknown.balance || '0');
    if (resolved) { next.withdrawn = true; next.step = 6; }
  }
  if (resolved && ['/api/force', '/api/redeem', '/api/withdraw'].includes(unknown.path)) next = includeObservedEpoch(next);
  return { ...next, unknown: resolved ? null : unknown, error: resolved ? 'The read-back confirms the write landed.' : 'Still unknown. No retry was sent; inspect the live wallets before clearing this state.' };
}

export async function advanceLive(initial, api = new LiveApi()) {
  let state = { ...initial, error: '' };
  if (!state.connected) return connectLive(state, api);
  if (state.unknown) return reconcileUnknown(state, api);
  const write = async (role, path, body, before = null) => {
    state = { ...state, currentOperation: { role, path, before } };
    const result = await api.post(role, path, body);
    state = { ...state, currentOperation: null };
    return result;
  };
  try {
    if (state.step === 0) {
      state = await refresh(state, api);
      if (decimalToUnits(xzecBalance(state.snapshot.wallets.a)) < decimalToUnits(LIVE_DEFAULTS.deposit)) throw new LiveApiError('Wallet A needs at least 0.01000000 spendable ZEC.zy before the live claim.');
      return { ...state, step: 1 };
    }
    if (state.step === 1) {
      if (!Number.isInteger(state.collectionId) || !findCollection(state.snapshot, state.collectionId)) throw new LiveApiError('Set VITE_ZYN_COLLECTION_ID to a live collection visible to these daemons.');
      if (!state.claimFunded) {
        const before = collectionPool(state);
        const result = await write('a', '/api/fund', { collection: state.collectionId, amount: LIVE_DEFAULTS.claim }, before);
        state = addReceipt({ ...state, claimFunded: true }, 'Collection funded', 'Zyn', 'A', '/api/fund', result, `${LIVE_DEFAULTS.claim} ZEC.zy moved from Wallet A into collection ${state.collectionId}.`);
      }
      if (!state.claimMinted) {
        const result = await write('creator', '/api/mint', { collection: state.collectionId, serial: 0, to: state.snapshot.wallets.a.account, content: state.content, symbol: LIVE_DEFAULTS.itemSymbol });
        state = addReceipt({ ...state, claimMinted: true }, 'Shielded Byte minted', 'Zyn', 'Creator', '/api/mint', result, 'The creator signed an independent mint directly to Wallet A.');
      }
      state = await refresh(state, api);
      if (state.owner !== 'a') return { ...state, error: 'Mint accepted; waiting for Wallet A’s overview to observe the item.' };
      return { ...state, step: 2 };
    }
    if (state.step === 2) {
      if (state.owner === 'b') return { ...state, step: 3 };
      if (state.owner !== 'a' || state.itemAsset == null) throw new LiveApiError('Wallet A does not currently hold the configured demo item.');
      if (state.saleOfferId == null) {
        const result = await write('a', '/api/offer', { asset: Number(state.itemAsset), price: LIVE_DEFAULTS.price });
        state = addReceipt(state, 'Offer placed', 'Zyn', 'A', '/api/offer', result, `Wallet A offered asset ${state.itemAsset} for ${LIVE_DEFAULTS.price} ZEC.zy.`);
        state = await refresh(state, api);
        const offer = findSaleOffer(state);
        if (!offer) return { ...state, error: 'Offer accepted by the chain; waiting for it to appear in the order book.' };
        state = { ...state, saleOfferId: offer.id };
      }
      const result = await write('b', '/api/take', { offer: Number(state.saleOfferId) });
      state = addReceipt(state, 'Offer taken', 'Zyn', 'B', '/api/take', result, `Wallet B accepted offer ${state.saleOfferId}.`);
      state = await refresh(state, api);
      return state.owner === 'b' ? { ...state, step: 3 } : { ...state, error: 'Take accepted; waiting for Wallet B to observe ownership.' };
    }
    if (state.step === 3) {
      if (state.owner === 'a') return { ...state, step: 4 };
      if (!state.forcedSubmitted) {
        if (state.owner !== 'b' || state.itemAsset == null) throw new LiveApiError('Wallet B must hold the item before forcing its return.');
        const result = await write('b', '/api/force', { to: state.snapshot.wallets.a.account, asset: Number(state.itemAsset), amount: '1' });
        state = addReceipt({ ...state, forcedSubmitted: true }, 'Forced return submitted', 'Zcash → Zyn', 'B', '/api/force', result, 'A ZYF transfer memo was submitted. This is a transfer, not a paid sale.');
      }
      state = await refresh(state, api);
      state = includeObservedEpoch(state);
      return state.owner === 'a' ? { ...state, step: 4 } : { ...state, error: 'Forced transfer submitted. Waiting for the item to appear in Wallet A.' };
    }
    if (state.step === 4) {
      if (state.owner !== 'a' || state.itemAsset == null) throw new LiveApiError('Wallet A must hold the item before redemption.');
      const collection = findCollection(state.snapshot, state.collectionId);
      const held = findHeldItem(state.snapshot, { asset: state.itemAsset });
      const redeemAmount = held?.item?.redeem_price || collection?.redeem_price;
      if (!redeemAmount) throw new LiveApiError('The live overview does not expose a redemption price for this item.');
      const result = await write('a', '/api/redeem', { asset: Number(state.itemAsset) });
      state = addReceipt({ ...state, redeemAmount }, 'Item redeemed', 'Zyn', 'A', '/api/redeem', result, `Asset ${state.itemAsset} burned for ${redeemAmount} ZEC.zy.`);
      state = await refresh(state, api);
      state = includeObservedEpoch(state);
      return state.owner !== 'a' ? { ...state, step: 5 } : { ...state, error: 'Redeem accepted; waiting for the item to leave Wallet A.' };
    }
    if (state.step === 5) {
      if (!state.redeemAmount) throw new LiveApiError('No confirmed redemption amount is available to withdraw.');
      if (!state.snapshot.wallets.a.record?.bound_here) {
        const result = await write('a', '/api/bind', {});
        state = addReceipt({ ...state, boundRequested: true }, 'Withdrawal address bound', 'Zcash', 'A', '/api/bind', result, 'Wallet A requested binding to its daemon-managed shielded address.');
        state = await refresh(state, api);
        if (!state.snapshot.wallets.a.record?.bound_here) return { ...state, error: 'Binding submitted. Wait for the daemon to report bound_here before withdrawing.' };
      }
      const before = state.snapshot.wallets.a.record?.exiting?.length || 0;
      const balance = xzecBalance(state.snapshot.wallets.a);
      const result = await write('a', '/api/withdraw', { amount: state.redeemAmount }, { count: before, balance });
      state = addReceipt({ ...state, withdrawn: true }, 'Shielded withdrawal requested', 'Zyn → Zcash', 'A', '/api/withdraw', result, `${state.redeemAmount} ZEC.zy entered the shielded withdrawal path.`);
      state = await refresh(state, api);
      state = includeObservedEpoch(state);
      return { ...state, step: 6 };
    }
    if (state.step === 6) {
      state = await refresh(state, api);
      if (state.targetEpoch == null) throw new LiveApiError('No receipt epoch is available to match against an anchor.');
      const anchor = [...state.snapshot.anchors].sort((a, b) => Number(a.epoch) - Number(b.epoch)).find(entry => Number(entry.epoch) >= Number(state.targetEpoch));
      if (!anchor) return { ...state, error: `Waiting for a scheduled anchor covering epoch ${state.targetEpoch}. No anchor was fabricated.` };
      const anchored = addReceipt({ ...state, anchor }, 'Batch anchor observed', 'Zcash', 'Observer', '/api/anchors', anchor, `Anchor epoch ${anchor.epoch} covers the latest receipt epoch ${state.targetEpoch}.`);
      return { ...anchored, step: 7 };
    }
    return state;
  } catch (error) {
    if (error?.uncertain && state.currentOperation) {
      const before = state.currentOperation.before;
      const unknown = {
        ...state.currentOperation,
        before: typeof before === 'object' ? before?.count : before,
        balance: typeof before === 'object' ? before?.balance : undefined,
      };
      return { ...state, currentOperation: null, unknown, error: `${error.message} The write result is unknown; the next action will only read back state.` };
    }
    error.liveState = { ...state, currentOperation: null };
    throw error;
  }
}

export function liveDepositUri(state) {
  const fromOverview = state.snapshot?.wallets?.a?.deposit_uri;
  const address = state.snapshot?.deposit?.address;
  const base = fromOverview || (address ? `zcash:${address}` : '');
  if (!base) return '';
  return `${base}${base.includes('?') ? '&' : '?'}amount=${LIVE_DEFAULTS.deposit}`;
}
