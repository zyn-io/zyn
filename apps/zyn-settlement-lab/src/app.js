import './styles.css';
import QRCode from 'qrcode';
import { ITEMS, STEPS, createState, advance, floor, format, PRICE, CLAIM, SEED } from './model.js';
import { LIVE_DEFAULTS, LIVE_STEPS, LiveApi, advanceLive, connectLive, createLiveState, decimalToUnits, findCollection, liveDepositUri, unitsToDecimal, xzecBalance } from './live.js';

const $ = (q, root = document) => root.querySelector(q);
const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const KEY = 'zyn-settlement-lab-v1';
let state = createState();
let busy = false;
let auto = null;
let modalTrigger = null;
let daemon = null;
let nodeLoading = false;
let nodeError = '';
let modalGeneration = 0;
let selectedTab = 'all';
let runMode = 'rehearsal';
let liveState = createLiveState();
const liveApi = new LiveApi();
try {
  const saved = JSON.parse(localStorage.getItem(KEY));
  if (saved && ITEMS.some(i => i.id === saved.selected) && Number.isInteger(saved.actions) && saved.actions >= 0 && saved.actions <= 8) {
    state = createState(saved.selected);
    for (let n = 0; n < saved.actions; n++) state = advance(state);
  }
} catch { /* Storage is optional; a fresh rehearsal is always usable. */ }
const icon = (name, cls = '') => {
  const paths = {
    arrow: '<path d="M4 12h16m-6-6 6 6-6 6"/>',
    check: '<path d="m5 12 4 4L19 6"/>',
    shield: '<path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6z"/><path d="m8 12 3 3 5-6"/>',
    link: '<path d="m10 13 4-4m-6 6-2 2a4 4 0 0 1-5-5l5-5a4 4 0 0 1 5 0m2 2 2-2a4 4 0 0 1 5 5l-5 5a4 4 0 0 1-5 0"/>',
    reset: '<path d="M3 10a9 9 0 1 1 1 9M3 4v6h6"/>',
    play: '<path d="m8 4 12 8-12 8z"/>',
    pause: '<path d="M8 5v14M16 5v14"/>',
    cube: '<path d="m12 2 9 5v10l-9 5-9-5V7zM3 7l9 5 9-5M12 12v10"/>',
    expand: '<path d="M14 3h7v7m0-7L10 14M8 3H3v18h18v-5"/>',
  };
  return `<svg class="icon ${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] || paths.cube}</svg>`;
};
function persist() {
  try { localStorage.setItem(KEY, JSON.stringify({ selected: state.selected, actions: state.step + (state.blocked ? 1 : 0) })); } catch { /* Private browsing still works. */ }
}
function announce(message) { $('#announcement').textContent = message; }
function currentItem() { return ITEMS.find(i => i.id === state.selected); }
function stopAuto() { clearInterval(auto); auto = null; }
function ownerName() { return state.owner === 'burned' ? 'Redeemed' : state.owner ? `Wallet ${state.owner}` : 'Unclaimed'; }
function wallet(name) {
  const owns = state.owner === name;
  const label = name === 'A' ? 'Collector' : 'Counterparty';
  const value = name === 'A' ? state.a : state.b;
  return `<section class="wallet ${owns ? 'owns' : ''}" aria-label="Wallet ${name}">
    <div class="wallet-avatar ${name === 'B' ? 'blue' : ''}">${name}</div>
    <div class="wallet-person"><h2>Wallet ${name} <span>${label}</span></h2><p>${name === 'A' ? `Shielded <b>${format(state.externalA, 5)}</b> ZEC` : 'Seeded with 0.02000000 ZEC.zy'}</p></div>
    <div class="wallet-balance"><strong>${format(value)}</strong><span>ZEC.zy available</span></div>
    <div class="wallet-item ${owns ? 'is-owner' : ''}">${owns ? `<img src="${currentItem().image}" alt=""/> <span>Owns #${currentItem().token}</span>` : `${icon('cube')} <span>No active piece</span>`}</div>
  </section>`;
}
function floorGraph() {
  const values = state.floors.length ? state.floors : [500000];
  const coords = values.map((v,i) => `${24+i*112},${78-(v-500000)/2000*48}`);
  const end = coords[coords.length-1].split(',');
  return `<svg class="floor-chart" viewBox="0 0 272 106" role="img" aria-label="Reserve floor after claim and each trade">
    <path d="M24 30H248M24 54H248M24 78H248" stroke="#dfe5dd" stroke-dasharray="3 5"/>
    <path d="M24 78 L${coords.join(' L')} L${end[0]} 88 L24 88Z" fill="#ddedcb"/>
    <path d="M${coords.join(' L')}" fill="none" stroke="#4c7657" stroke-width="2.5"/>
    ${coords.map((p,i)=>`<circle cx="${p.split(',')[0]}" cy="${p.split(',')[1]}" r="4" fill="${i===values.length-1?'#42684c':'#acbdac'}" stroke="white" stroke-width="2"/>`).join('')}
    <text x="24" y="103">CLAIM</text><text x="119" y="103">TRADE 01</text><text x="214" y="103">TRADE 02</text>
  </svg>`;
}
function appHeader() {
  const live = runMode === 'live';
  return `<header class="header"><a class="brand" href="#lab" aria-label="Zyn Settlement Lab"><span class="brand-lockup"><span class="brand-symbol"><img src="/assets/zyn-symbol.png" alt=""/></span><span class="brand-wordmark"><img src="/assets/zyn-wordmark.png" alt="Zyn"/></span></span><span class="brand-title">Settlement Lab</span></a><div class="header-actions"><button class="mode ${live?'is-live':''}" data-action="mode"><i></i> ${live?'Live testnet':'Rehearsal'} <span>· ${live?'real writes':'no funds move'}</span></button><button class="button ghost" data-action="guide">Demo guide</button><button class="button icon-button" data-action="node" aria-label="${live?'Inspect live daemons':'Connect daemon telemetry'}">${icon('link')}</button></div></header>`;
}
function render() {
  document.title = 'Zyn — Settlement Lab';
  if (runMode === 'live') return renderLive();
  const item = currentItem();
  const done = state.step === 7;
  const step = STEPS[Math.min(state.step, 6)];
  const forcedPending = state.step === 3 && state.blocked;
  const amount = state.step === 4 || state.step === 6 ? format(state.redeemed || floor(state)) : step.amount;
  const events = state.events.filter(e => selectedTab === 'all' || (selectedTab === 'zyn' ? e.layer.includes('Zyn') : e.layer.includes('Zcash')));
  $('#app').innerHTML = `
    ${appHeader()}
    <main>
      <section class="intro"><div><div class="eyebrow">ZCASH × ZYN <span> / </span> SHIELDED BYTE</div><h1>One object. <span>Every layer.</span></h1><p>Fund, trade, force, redeem. Follow the value all the way home.</p></div>
        <div class="run-controls"><button class="button" data-action="reset">${icon('reset')} Reset</button><button class="button dark" data-action="autoplay" ${done ? 'disabled' : ''}>${icon(auto?'pause':'play')} ${auto?'Pause':'Play walkthrough'}</button></div></section>
      <div class="wallet-rail">${wallet('A')}<div class="wallet-connector">⇄</div>${wallet('B')}</div>
      <nav class="steps" aria-label="Demo progress">${STEPS.map((s,i)=>`<button class="step ${i===state.step?'active':''} ${i<state.step?'complete':''}" data-action="step-info" data-index="${i}" ${i===state.step?'aria-current="step"':''}><span class="step-number">${i<state.step?icon('check'):String(i+1).padStart(2,'0')}</span><span>${s.name}<small>${s.layer}</small></span>${i<6?'<span class="step-arrow">›</span>':''}</button>`).join('')}</nav>
      <div class="workspace">
        <section class="stage panel" aria-labelledby="stage-heading"><div class="panel-heading"><h2 id="stage-heading">${item.name} <span class="muted">#${item.token}</span></h2><button class="text-button" data-action="art">View artwork ${icon('expand')}</button></div>
          <div class="art-stage ${state.owner==='burned'?'redeemed':''}" data-light="${item.light.toLowerCase()}"><div class="stage-grid"></div><span class="edition-tag">${item.collection.toUpperCase()} / DEMO CARD</span><img class="stage-art" src="${item.image}" alt="${item.name} #${item.token}, a pixel-art guardian holding a glowing Zcash coin"/><span class="art-glint"></span><span class="owner-pill"><i></i> ${ownerName()}</span><span class="light-label">Privacy is power.</span></div>
          <div class="action-area ${forcedPending?'censored':''}"><div class="action-meta"><span class="eyebrow">${done?'ROUND TRIP COMPLETE':`STEP ${String(state.step+1).padStart(2,'0')} / 07`}</span><span class="layer-tag">${done?'Rehearsal':step.layer}</span></div>
            <h3>${done?'Back in your shielded wallet.':forcedPending?'The direct path is blocked.':step.title}</h3><p>${done?`${format(state.redeemed)} ZEC returned. Two trades and one redemption covered by a single demo anchor.`:forcedPending?'SIM-RETURN-001 was refused. Send this exact operation through a simulated encrypted Zcash memo to complete the sale.':step.description}</p>
            ${amount&&!done?`<div class="action-amount"><span>${state.step===4?'Redemption value':state.step===6?'Withdrawal amount':'Transaction amount'}</span><strong>${amount} <small>${state.step===2||state.step===3||state.step===4?'ZEC.zy':'ZEC'}</small></strong></div>`:''}
            <button class="button primary next-action" data-action="${done?'reset':'advance'}" ${busy?'disabled':''}>${busy?'Updating rehearsal…':done?'Start a new run':forcedPending?'Force via Zcash memo':step.action}${icon(done?'reset':'arrow')}</button>
            <div class="action-note">${icon('shield')} ${done?'All values and receipts in this run are simulated.':step.note}</div>
          </div>
        </section>
        <aside class="reserve panel"><div class="panel-heading"><h2>Redemption reserve</h2><button class="help" data-action="economics" aria-label="Explain reserve economics">?</button></div><div class="reserve-value"><span class="eyebrow">${state.step<2?'FLOOR AFTER CLAIM':'CURRENT FLOOR / PIECE'}</span><strong>${format(state.step<2?500000:floor(state))}</strong><span>ZEC.zy <b class="growth">${state.reserveFees?`+${format(state.reserveFees/6)} from fees`:'Opening target'}</b></span></div>
          ${floorGraph()}
          <dl class="reserve-details"><div><dt>Reserve pool</dt><dd>${format(state.pool)}</dd></div><div><dt>Outstanding</dt><dd>${state.outstanding} pieces</dd></div><div><dt>Fees to reserve</dt><dd class="green">+${format(state.reserveFees)}</dd></div><div><dt>Fee per side</dt><dd>1.00%</dd></div></dl>
          <div class="reserve-seed">${icon('shield')}<p>Seeded with <b>0.03000000 ZEC.</b><br/>Trading fees increase the reserve. Redemption is not trading profit.</p></div>
          <div class="fee-split"><span>Every trade’s fees</span><div><i></i><i></i></div><p><span>50% reserve</span><span>50% creator</span></p></div>
        </aside>
        <section class="trail panel"><div class="panel-heading"><h2>Action trail <span class="count">${state.events.length}</span></h2><div class="tabs" role="group" aria-label="Filter action trail">${[['all','All'],['zyn','Zyn'],['zcash','Zcash']].map(([key,label])=>`<button data-action="filter" data-filter="${key}" aria-pressed="${selectedTab===key}">${label}</button>`).join('')}</div></div>
          <div class="events">${!events.length?`<div class="empty-trail">${icon('link')}<div><strong>${state.events.length?'No matching actions yet.':'Every move leaves a receipt.'}</strong><p>${state.events.length?'Try the All filter to see the full sequence.':'Begin with a deposit. Ownership, fees and settlement will appear here.'}</p></div></div>`:[...events].reverse().map(e=>`<button class="event" data-action="receipt" data-id="${e.id}"><span class="event-icon ${e.layer==='Sequencer'?'warning':''}">${icon(e.layer==='Sequencer'?'shield':'check')}</span><span><strong>${e.title}</strong><small>${e.id} · simulated</small></span><span class="event-layer">${e.layer}</span>${icon('expand')}</button>`).join('')}</div>
        </section>
      </div>
      <section class="settlement ${state.anchor?'anchored':''}"><div class="settlement-icon">${icon('link')}</div><div class="settlement-copy"><span class="eyebrow">BATCH SETTLEMENT</span><h2>${state.anchor?'3 transitions. 1 demo anchor.':state.transitions.length?`${state.transitions.length} transitions awaiting an anchor.`:'Many moves. One commitment.'}</h2><p>${state.anchor?'SIM-ZYA-001 · epoch 30 · rehearsal receipt':`${state.chainTxs} simulated Zcash transactions across the full journey`}</p></div><div class="settlement-count"><b>${String(state.transitions.length).padStart(2,'0')}</b><span>Zyn transitions</span><em>→</em><b>${state.anchor?'01':'00'}</b><span>Zcash anchors</span></div><button class="button" data-action="anchor" ${!state.anchor?'disabled':''}>Inspect batch ${icon('expand')}</button></section>
      <footer><span><b>ZYN</b> Settlement you can follow.</span><span>Rehearsal · local simulated state · network fees excluded</span><button class="text-button" data-action="guide">About this demo ↗</button></footer>
    </main>`;
  const progress = $('.steps');
  const activeStep = progress.children[Math.min(state.step, 6)];
  if (progress.scrollWidth > progress.clientWidth) progress.scrollLeft = activeStep.offsetLeft - progress.offsetLeft - (progress.clientWidth - activeStep.clientWidth) / 2;
}

function displayLiveAmount(value) {
  try { return unitsToDecimal(decimalToUnits(value)); }
  catch { return value == null ? '—' : String(value); }
}
function short(value, size = 8) {
  const text = String(value || '');
  return text.length > size * 2 + 1 ? `${text.slice(0, size)}…${text.slice(-size)}` : text || '—';
}
function liveWallet(role) {
  const overview = liveState.snapshot?.wallets?.[role];
  const name = role.toUpperCase();
  const owns = liveState.owner === role;
  return `<section class="wallet ${owns?'owns':''}" aria-label="Wallet ${name}">
    <div class="wallet-avatar ${role==='b'?'blue':''}">${name}</div>
    <div class="wallet-person"><h2>Wallet ${name} <span>${role==='a'?'Collector':'Counterparty'}</span></h2><p>${overview?`${short(overview.account)} · epoch ${escape(overview.status?.epoch ?? '—')}`:'Daemon not read yet'}</p></div>
    <div class="wallet-balance"><strong>${overview?displayLiveAmount(xzecBalance(overview)):'—'}</strong><span>ZEC.zy spendable</span></div>
    <div class="wallet-item ${owns?'is-owner':''}">${owns?`<img src="/assets/shielded-byte.png" alt=""/> <span>Owns asset ${escape(liveState.itemAsset)}</span>`:`${icon('cube')} <span>No demo item</span>`}</div>
  </section>`;
}
function liveActionLabel() {
  if (!liveState.connected) return 'Connect three daemons';
  if (liveState.unknown) return 'Read back unknown write';
  if (liveState.step === 3 && liveState.forcedSubmitted) return 'Check forced inclusion';
  if (liveState.step === 5 && liveState.boundRequested && !liveState.snapshot?.wallets?.a?.record?.bound_here) return 'Check address binding';
  return LIVE_STEPS[Math.min(liveState.step, 6)]?.action || 'Live run complete';
}
function liveOwnerName() {
  if (liveState.owner === 'a') return 'Wallet A';
  if (liveState.owner === 'b') return 'Wallet B';
  if (liveState.owner === 'creator') return 'Creator';
  if (liveState.claimMinted) return 'Redeemed';
  return 'Unclaimed';
}
function renderLive() {
  const done = liveState.step === 7;
  const step = LIVE_STEPS[Math.min(liveState.step, 6)];
  const collection = findCollection(liveState.snapshot, liveState.collectionId);
  const latestAnchor = liveState.snapshot?.anchors?.at(-1) || liveState.snapshot?.wallets?.a?.settlement?.last;
  const network = liveState.snapshot?.wallets?.a?.chain != null ? `chain ${liveState.snapshot.wallets.a.chain}` : 'testnet chain 212';
  const receiptEvents = [...liveState.events].reverse();
  const actionAmount = liveState.step === 4 ? (collection?.redeem_price || 'Live floor') : liveState.step === 5 ? (liveState.redeemAmount || 'Pending redemption') : step?.amount;
  $('#app').innerHTML = `${appHeader()}<main class="live-main">
    <section class="intro"><div><div class="eyebrow">ZCASH × ZYN <span> / </span> LIVE SETTLEMENT</div><h1>Real intents. <span>Visible receipts.</span></h1><p>Three single-key daemons, one item, and no simulated success in the live path.</p></div><div class="run-controls"><span class="live-chain"><i></i>${escape(network)}</span><button class="button" data-action="live-refresh" ${busy?'disabled':''}>${icon('reset')} Refresh reads</button></div></section>
    <div class="wallet-rail">${liveWallet('a')}<div class="wallet-connector">⇄</div>${liveWallet('b')}</div>
    <nav class="steps live-steps" aria-label="Live demo progress">${LIVE_STEPS.map((item,index)=>`<button class="step ${index===liveState.step?'active':''} ${index<liveState.step?'complete':''}" data-action="live-step-info" data-index="${index}" ${index===liveState.step?'aria-current="step"':''}><span class="step-number">${index<liveState.step?icon('check'):String(index+1).padStart(2,'0')}</span><span>${item.name}<small>${item.layer}</small></span>${index<6?'<span class="step-arrow">›</span>':''}</button>`).join('')}</nav>
    <div class="workspace live-workspace">
      <section class="stage panel" aria-labelledby="live-stage-heading"><div class="panel-heading"><h2 id="live-stage-heading">Shielded Byte <span class="muted">${liveState.itemAsset==null?'unminted':`asset ${escape(liveState.itemAsset)}`}</span></h2><button class="text-button" data-action="art">View artwork ${icon('expand')}</button></div>
        <div class="art-stage ${liveState.claimMinted&&!liveState.owner?'redeemed':''}" data-light="live"><div class="stage-grid"></div><span class="edition-tag">LIVE TESTNET / ${escape(network.toUpperCase())}</span><img class="stage-art" src="/assets/shielded-byte.png" alt="Shielded Byte collectible card"/><span class="art-glint"></span><span class="owner-pill"><i></i>${liveOwnerName()}</span><span class="light-label">CONTENT ${short(liveState.content,5)}</span></div>
        <div class="action-area ${liveState.unknown?'unknown-action':''}"><div class="action-meta"><span class="eyebrow">${done?'LIVE RUN COMPLETE':`BEAT ${String(liveState.step+1).padStart(2,'0')} / 07`}</span><span class="layer-tag">${done?'Observed':escape(step.layer)}</span></div>
          <h3>${done?'The live round trip is observable.':liveState.unknown?'A write has an unknown outcome.':step.title}</h3><p>${done?'Every row below came from a daemon response or a scheduled anchor read.':liveState.unknown?'No retry will be sent. Read all three wallets back and verify whether the prior intent landed.':step.description}</p>
          ${actionAmount&&!done?`<div class="action-amount"><span>${liveState.step===3?'Transfer amount':'Transaction amount'}</span><strong>${escape(displayLiveAmount(actionAmount))} <small>${liveState.step===3?'ITEM':liveState.step===6?'':'ZEC.zy'}</small></strong></div>`:''}
          ${liveState.error?`<div class="live-message ${liveState.unknown?'unknown':''}" role="status">${escape(liveState.error)}</div>`:''}
          ${liveState.step===0&&liveState.connected?`<button class="button deposit-link" data-action="live-deposit">${icon('link')} View real deposit request</button>`:''}
          <button class="button primary next-action" data-action="live-advance" ${busy||done?'disabled':''}>${busy?'Waiting for daemon…':liveActionLabel()}${icon('arrow')}</button>
          <div class="action-note">${icon('shield')} ${done?'Live mode never stores private keys in the browser.':!liveState.connected||liveState.step===0||liveState.step===6?'This beat performs reads only; no transaction is sent.':'This button may submit real testnet intents. It never retries a write automatically.'}</div>
        </div>
      </section>
      <aside class="reserve panel"><div class="panel-heading"><h2>Live collection</h2><button class="help" data-action="live-status" aria-label="Inspect live configuration">?</button></div>
        <div class="reserve-value"><span class="eyebrow">REDEMPTION FLOOR / PIECE</span><strong>${collection?displayLiveAmount(collection.redeem_price):'—'}</strong><span>ZEC.zy <b class="growth">${collection?escape(collection.phase_name || collection.phase || 'live'):'Not configured'}</b></span></div>
        <dl class="reserve-details"><div><dt>Collection</dt><dd>${liveState.collectionId==null?'Set env':`#${liveState.collectionId}`}</dd></div><div><dt>Reserve pool</dt><dd>${collection?displayLiveAmount(collection.pool):'—'}</dd></div><div><dt>Outstanding</dt><dd>${collection?.outstanding ?? '—'}</dd></div><div><dt>Fee per side</dt><dd>${collection?.fee_bps != null?`${collection.fee_bps} bps`:'—'}</dd></div></dl>
        <div class="daemon-stack"><span class="eyebrow">DAEMON READS</span>${['creator','a','b'].map(role=>{const view=liveState.snapshot?.wallets?.[role];return `<div><i class="${view?'up':''}"></i><b>${role==='creator'?'Creator':`Wallet ${role.toUpperCase()}`}</b><span>${view?`seq ${escape(view.status?.seq ?? '—')} · epoch ${escape(view.status?.epoch ?? '—')}`:'offline'}</span></div>`}).join('')}</div>
        <div class="reserve-seed">${icon('shield')}<p><b>Forced return is a transfer.</b><br/>The current daemon cannot force an offer or take, so this live beat does not claim that a price leg occurred.</p></div>
      </aside>
      <section class="trail panel"><div class="panel-heading"><h2>Live receipts <span class="count">${liveState.events.length}</span></h2><span class="receipt-key">SEQ · EPOCH · TXID</span></div><div class="events">${!receiptEvents.length?`<div class="empty-trail">${icon('link')}<div><strong>${liveState.connected?'Connected. No writes yet.':'Waiting for three daemons.'}</strong><p>${liveState.connected?'The first live receipt appears after the collection fund call.':'Creator, A, and B are read independently before any controls are enabled.'}</p></div></div>`:receiptEvents.map((item,index)=>`<button class="event" data-action="live-receipt" data-index="${liveState.events.length-1-index}"><span class="event-icon ${item.txid?'warning':''}">${icon(item.txid?'shield':'check')}</span><span><strong>${escape(item.title)}</strong><small>${item.seq!=null?`seq ${escape(item.seq)}`:short(item.txid,8)}${item.epoch!=null?` · epoch ${escape(item.epoch)}`:''}</small></span><span class="event-layer">${escape(item.role)}</span>${icon('expand')}</button>`).join('')}</div></section>
    </div>
    <section class="settlement ${liveState.anchor?'anchored':''}"><div class="settlement-icon">${icon('link')}</div><div class="settlement-copy"><span class="eyebrow">SCHEDULED BATCH SETTLEMENT</span><h2>${liveState.anchor?`Epoch ${escape(liveState.anchor.epoch)} anchored on Zcash.`:liveState.targetEpoch!=null?`Waiting for an anchor covering epoch ${escape(liveState.targetEpoch)}.`:'Receipts will resolve into a scheduled anchor.'}</h2><p>${liveState.anchor?short(liveState.anchor.txid,12):latestAnchor?`Latest observed: epoch ${escape(latestAnchor.epoch)} · ${short(latestAnchor.txid,8)}`:'No anchor observed yet · nothing is fabricated'}</p></div><div class="settlement-count"><b>${String(liveState.events.filter(item=>item.seq!=null).length).padStart(2,'0')}</b><span>real receipts</span><em>→</em><b>${liveState.anchor?'01':'00'}</b><span>matched anchors</span></div><button class="button" data-action="live-anchor" ${!liveState.anchor?'disabled':''}>Inspect anchor ${icon('expand')}</button></section>
    <footer><span><b>ZYN</b> Settlement you can follow.</span><span>Live testnet · browser holds no signing key · writes use local daemons</span><button class="text-button" data-action="guide">About this demo ↗</button></footer>
  </main>`;
  const progress = $('.steps');
  const activeStep = progress?.children[Math.min(liveState.step, 6)];
  if (progress && activeStep && progress.scrollWidth > progress.clientWidth) progress.scrollLeft = activeStep.offsetLeft - progress.offsetLeft - (progress.clientWidth - activeStep.clientWidth) / 2;
}
async function runLiveStep() {
  if (busy || liveState.step === 7 || $('#modal').open) return;
  busy = true; liveState = { ...liveState, error: '' }; render();
  try {
    liveState = await advanceLive(liveState, liveApi);
    announce(liveState.error || liveState.events.at(-1)?.title || (liveState.connected ? 'Live daemon reads connected' : 'Live state updated'));
  } catch (error) {
    liveState = { ...(error.liveState || liveState), error: error.message };
    announce(error.message);
  }
  busy = false; render();
  $('[data-action="live-advance"]')?.focus({ preventScroll: true });
}
async function refreshLive() {
  if (busy) return;
  busy = true; render();
  try { liveState = await connectLive(liveState, liveApi); announce('Live daemon reads refreshed'); }
  catch (error) { liveState = { ...liveState, error: error.message }; announce(error.message); }
  busy = false; render();
}
function liveStatus() {
  const rows = ['creator','a','b'].map(role=>{const view=liveState.snapshot?.wallets?.[role];return `<div><dt>${role==='creator'?'Creator':`Wallet ${role.toUpperCase()}`}</dt><dd>${view?`${short(view.account)} · seq ${escape(view.status?.seq ?? '—')}`:'Not connected'}</dd></div>`}).join('');
  modal('Live testnet configuration.', `<p>The frontend can reach exactly three daemon targets through an explicit same-origin allowlist. It never receives their signing keys.</p><dl class="modal-ledger">${rows}<div><dt>Collection</dt><dd>${liveState.collectionId==null?'VITE_ZYN_COLLECTION_ID is missing':`#${liveState.collectionId}`}</dd></div><div><dt>Content hash</dt><dd>${short(liveState.content,10)}</dd></div></dl><details class="setup" open><summary>Default local ports</summary><pre>ZYN_DAEMON_CREATOR=http://127.0.0.1:8977
ZYN_DAEMON_A=http://127.0.0.1:8978
ZYN_DAEMON_B=http://127.0.0.1:8979
VITE_ZYN_COLLECTION_ID=&lt;id&gt;</pre></details><div class="callout">Switching back to rehearsal discards this browser’s live presentation state. It does not undo any on-chain action.</div>`);
}
async function liveDeposit() {
  const uri = liveDepositUri(liveState);
  if (!uri) return modal('Deposit request unavailable.', '<p>Wallet A did not return a daemon-issued deposit URI. No fallback address is invented.</p>');
  modal('Fund Wallet A with real testnet ZEC.', `<p>This request comes from Wallet A’s daemon and preserves its account memo when the vault is shared.</p><div id="live-deposit-qr" class="qr-request"></div><code class="address">${escape(uri)}</code><button class="button" data-action="copy-live-uri">Copy payment URI</button><div class="callout">After sending, return to the lab and use “Check funded balance.” The beat advances only after 0.01000000 ZEC.zy is spendable.</div>`);
  const generation = modalGeneration;
  try {
    const data = await QRCode.toDataURL(uri, { width: 224, margin: 2, color: { dark: '#172622', light: '#ffffff' } });
    if (generation === modalGeneration && $('#live-deposit-qr')) $('#live-deposit-qr').innerHTML = `<img src="${data}" alt="Live testnet ZIP-321 payment QR"/><p><b>0.01000000 testnet ZEC</b><br/>Daemon-issued payment URI</p>`;
  } catch { if ($('#live-deposit-qr')) $('#live-deposit-qr').textContent = 'QR generation failed; use the payment URI below.'; }
}
function liveReceipt(index) {
  const item = liveState.events[Number(index)];
  if (!item) return;
  modal(item.title, `<span class="layer-tag">${escape(item.layer)}</span><p>${escape(item.details)}</p><dl class="modal-ledger"><div><dt>Daemon</dt><dd>${escape(item.role)}</dd></div><div><dt>Route</dt><dd>${escape(item.path)}</dd></div><div><dt>Sequence</dt><dd>${escape(item.seq ?? 'not returned')}</dd></div><div><dt>Epoch</dt><dd>${escape(item.epoch ?? 'not returned')}</dd></div><div><dt>Zcash txid</dt><dd>${escape(item.txid ? short(item.txid,12) : 'not returned')}</dd></div></dl><pre>${escape(JSON.stringify(item,null,2))}</pre><div class="callout">This receipt is copied from the daemon response. It is not a rehearsal SIM id.</div>`);
}
function liveAnchor() {
  if (!liveState.anchor) return;
  modal('Scheduled anchor observed.', `<p>This is the epoch-to-Zcash mapping returned by <code>GET /api/anchors</code>.</p><dl class="modal-ledger"><div><dt>Epoch</dt><dd>${escape(liveState.anchor.epoch)}</dd></div><div><dt>Height</dt><dd>${escape(liveState.anchor.height ?? '—')}</dd></div><div><dt>Anchor id</dt><dd>${escape(short(liveState.anchor.anchor_id,12))}</dd></div><div><dt>Transaction</dt><dd>${escape(short(liveState.anchor.txid,12))}</dd></div><div><dt>Root</dt><dd>${escape(short(liveState.anchor.root,12))}</dd></div></dl><pre>${escape(JSON.stringify(liveState.anchor,null,2))}</pre>`);
}
async function runStep() {
  if (busy || state.step === 7 || $('#modal').open) return;
  busy = true; render();
  await new Promise(resolve => setTimeout(resolve, 450));
  try { state = advance(state); persist(); announce(state.events.at(-1).title); }
  catch (e) { stopAuto(); announce(e.message); }
  busy = false;
  if (state.step === 7) stopAuto();
  render();
  $('[data-action="advance"], .next-action')?.focus({ preventScroll: true });
}
function modal(title, content) {
  const trigger = document.activeElement?.closest('[data-action]');
  const triggerData = trigger && { ...trigger.dataset };
  stopAuto(); render();
  modalGeneration++;
  if (!$('#modal').open) modalTrigger = triggerData && [...document.querySelectorAll('[data-action]')].find(element => Object.entries(triggerData).every(([key, value]) => element.dataset[key] === value));
  $('#modal').innerHTML = `<button class="modal-close" data-action="close" aria-label="Close dialog">×</button><p class="eyebrow">ZYN / SETTLEMENT LAB</p><h2 id="modal-title">${title}</h2>${content}`;
  if (!$('#modal').open) $('#modal').showModal();
}
function guide() {
  if (runMode === 'live') return modal('A real round trip through testnet.', `<p>Wallet A funds a collection and receives one creator-minted item. Wallet B buys it, then returns it through the daemon’s forced-transfer path. A redeems, withdraws, and waits for a scheduled Zcash anchor.</p><ol class="guide-steps">${LIVE_STEPS.map(s=>`<li><b>${s.name}</b><span>${s.layer}</span></li>`).join('')}</ol><div class="callout"><b>Live means live.</b><p>Gold controls may sign and submit testnet intents through the three local daemons. There is no automatic write retry, and a timeout is displayed as unknown until state is read back.</p></div><p>The current <code>/api/force</code> accepts only a transfer payload. The live demo therefore says “forced return transfer”; it does not claim the return includes a sale price.</p><button class="button dark full" data-action="close">Return to live run ${icon('arrow')}</button>`);
  modal('A round trip through Zyn.', `<p>One collectible makes the journey visible: fund a vault, claim, trade, recover from censorship, redeem, anchor and withdraw.</p><ol class="guide-steps">${STEPS.map(s=>`<li><b>${s.name}</b><span>${s.layer}</span></li>`).join('')}</ol><div class="callout"><b>This is an interactive rehearsal.</b><p>Balances, confirmations and SIM receipts are local illustrations. No keys, funds or real transactions are used. Use the mode control in the header only when three keyed testnet daemons are ready.</p></div><p>Wallet B begins with 0.02000000 ZEC.zy. The reserve begins with 0.03000000 ZEC and five other minted pieces. Reset restores this exact seed.</p><button class="button dark full" data-action="close">Let’s walk through it ${icon('arrow')}</button>`);
}
function economics() {
  modal('A reserve you can account for.', `<p>The floor is the reserve divided by outstanding pieces. Every number here uses integer zatoshis.</p><dl class="modal-ledger"><div><dt>Seeded reserve</dt><dd>${format(SEED)} ZEC</dd></div><div><dt>Trade price</dt><dd>${format(PRICE)} ZEC.zy</dd></div><div><dt>Fee per participant</dt><dd>0.00006000 ZEC.zy</dd></div><div><dt>Per trade → reserve</dt><dd>0.00006000 ZEC.zy</dd></div><div><dt>Per trade → creator</dt><dd>0.00006000 ZEC.zy</dd></div><div><dt>After two trades / 6 pieces</dt><dd>0.00502000 ZEC.zy</dd></div></dl><p>The 0.00500000 ZEC claim payment goes to primary-sale proceeds, not the seeded reserve. Redeeming burns one piece and removes its share from the pool.</p><div class="callout">The floor rises by 0.00002000 ZEC.zy in this run. That increase comes from trading fees, not newly created value. Mainnet transaction fees are excluded.</div>`);
}
function receipt(id) {
  const event = state.events.find(e => e.id === id);
  if (!event) return;
  modal(event.title, `<span class="layer-tag">${escape(event.layer)}</span><p>${escape(event.details)}</p><pre>${escape(JSON.stringify({receipt:event.id,mode:'rehearsal',...event},null,2))}</pre><div class="callout">This is a simulated receipt, not a mainnet transaction or a proof. It has no block-explorer link.</div>`);
}
function anchor() {
  if (!state.anchor) return;
  modal('Inside this batch.', `<p>One simulated anchor covers these three completed Zyn transitions. The forced-inclusion memo is a separate Zcash transaction.</p><ol class="batch-list">${state.anchor.transitions.map(id=>`<li><code>${id}</code><b>${escape(state.events.find(e=>e.id===id).title)}</b></li>`).join('')}</ol><dl class="modal-ledger"><div><dt>Anchor reference</dt><dd>SIM-ZYA-001</dd></div><div><dt>Epoch</dt><dd>30 (illustrative)</dd></div><div><dt>Real state root</dt><dd>Not connected</dd></div></dl><div class="callout">A Zcash txid establishes that an anchor transaction exists. Verifying its coverage additionally requires the actual commitment, included transitions and verification data from the daemon.</div>`);
}
function nodePanel() {
  const view = daemon?.overview;
  modal('Connect Wallet A telemetry.', `<p>Read network telemetry from the configured Wallet A daemon. Your rehearsal remains a separate set of simulated balances.</p><div class="node-state ${view?'connected':''}">${icon('link')}<span>${nodeLoading?'Connecting…':view?`chain ${escape(view.chain)} · connected`:'No daemon connected'}</span></div>
    ${nodeError?`<p class="error" role="alert">${escape(nodeError)}</p>`:''}
    ${view?`<dl class="modal-ledger"><div><dt>Chain</dt><dd>${escape(view.chain)}</dd></div><div><dt>Sequence</dt><dd>${escape(view.status?.seq ?? '—')}</dd></div><div><dt>Epoch</dt><dd>${escape(view.status?.epoch ?? '—')}</dd></div><div><dt>Zyn available</dt><dd>${view.available?'Yes':'No'}</dd></div></dl><div id="deposit-request"></div>`:''}
    <button class="button dark full" data-action="connect" ${nodeLoading?'disabled':''}>${nodeLoading?'Reading daemon…':view?'Refresh telemetry':'Check daemon connection'}</button><details class="setup"><summary>Local connection setup</summary><p>Wallet A defaults to port 8978. Override it when needed:</p><pre>ZYN_DAEMON_A=http://127.0.0.1:&lt;port&gt; npm run dev</pre><p>This rehearsal panel only performs allowlisted reads. Switch modes from the header to use guarded live actions.</p></details>`);
  if (view && daemon.deposit?.address && daemon.deposit.memo === false) renderDeposit(daemon.deposit.address, modalGeneration).catch(() => {
    const request = $('#deposit-request');
    if (request) request.textContent = 'The payment QR could not be generated. No payment request is shown.';
  });
  else if(view) $('#deposit-request').innerHTML='<div class="callout">An account-specific deposit address is unavailable. The shared-vault fallback requires an account memo; no payment request is generated.</div>';
}
async function renderDeposit(address, generation) {
  // Only display a ZIP-321 QR for an actual daemon-issued per-account address.
  const uri = `zcash:${address}?amount=0.01`;
  const data = await QRCode.toDataURL(uri, { width: 192, margin: 2, color: {dark:'#172622',light:'#ffffff'} });
  if (generation !== modalGeneration || !$('#deposit-request')) return;
  $('#deposit-request').innerHTML=`<details class="setup"><summary>Actual deposit request · 0.01 ZEC</summary><div class="qr-request"><img src="${data}" alt="ZIP-321 deposit QR for the daemon’s actual address"/><p>This QR requests a real payment for <b>chain ${escape(daemon.overview.chain)}</b>. Scanning it is separate from the simulated walkthrough.</p></div><code class="address">${escape(uri)}</code><button class="button" data-action="copy-uri">Copy payment URI</button></details>`;
}
async function connect() {
  if (nodeLoading) return;
  nodeLoading = true; nodeError = ''; nodePanel();
  try {
    const response = await fetch('/daemon/a/api/overview', {signal:AbortSignal.timeout(9000)});
    let data;
    try { data = await response.json(); } catch { throw new Error('No daemon bridge is available. Use npm run dev or npm run preview with ZYN_DAEMON_A.'); }
    if(!response.ok) throw new Error(data.error || 'Daemon connection failed');
    if(data.chain == null || !data.status) throw new Error('Unexpected daemon overview format');
    daemon = {overview:data,deposit:null};
    const deposit = await fetch('/daemon/a/api/deposit-address', {signal:AbortSignal.timeout(9000)});
    if(deposit.ok) daemon.deposit = await deposit.json();
  } catch(e) { nodeError = e.message; }
  nodeLoading = false;
  if($('#modal').open && $('#modal-title').textContent==='Connect a daemon.') nodePanel();
}
document.addEventListener('click', async event => {
  const button = event.target.closest('[data-action]');
  if(!button || button.disabled) return;
  const action = button.dataset.action;
  if(action==='mode') return runMode==='live'
    ? modal('Leave the live view?', '<p>This clears only the browser’s live presentation state. It cannot reverse transactions already accepted by the daemons.</p><button class="button dark full" data-action="confirm-rehearsal">Return to rehearsal</button><button class="button full" data-action="close">Stay live</button>')
    : modal('Enter live testnet mode?', '<p>The connection itself performs reads only. Once connected, gold action buttons can submit real testnet intents through the three local daemons.</p><div class="callout"><b>Required before writing</b><p>Creator on 8977, Wallet A on 8978, Wallet B on 8979, and a valid <code>VITE_ZYN_COLLECTION_ID</code>.</p></div><button class="button dark full" data-action="confirm-live">Connect live reads</button><button class="button full" data-action="close">Keep rehearsing</button>');
  if(action==='confirm-live') { $('#modal').close(); runMode='live'; liveState=createLiveState(); render(); return refreshLive(); }
  if(action==='confirm-rehearsal') { $('#modal').close(); runMode='rehearsal'; liveState=createLiveState(); render(); announce('Rehearsal mode'); return; }
  if(action==='live-advance') return runLiveStep();
  if(action==='live-refresh') return refreshLive();
  if(action==='live-deposit') return liveDeposit();
  if(action==='live-status') return liveStatus();
  if(action==='live-receipt') return liveReceipt(button.dataset.index);
  if(action==='live-anchor') return liveAnchor();
  if(action==='copy-live-uri') {
    try { await navigator.clipboard.writeText(liveDepositUri(liveState)); button.textContent='Copied'; }
    catch { button.textContent='Copy the URI below'; }
    return;
  }
  if(action==='live-step-info') {
    const step=LIVE_STEPS[Number(button.dataset.index)];
    return modal(step.name, `<span class="layer-tag">${escape(step.layer)}</span><p>${escape(step.description)}</p><div class="callout">${Number(button.dataset.index)===3?'This is the daemon’s real forced transfer route. No sale or payment leg is implied.':'Live steps advance only when their daemon state can be observed.'}</div>`);
  }
  if(action==='advance') return runStep();
  if(action==='close') return $('#modal').close();
  if(action==='guide') return guide();
  if(action==='economics') return economics();
  if(action==='receipt') return receipt(button.dataset.id);
  if(action==='anchor') return anchor();
  if(action==='node') return runMode==='live' ? liveStatus() : nodePanel();
  if(action==='connect') return connect();
  if(action==='copy-uri') {
    try { await navigator.clipboard.writeText(`zcash:${daemon.deposit.address}?amount=0.01`); button.textContent='Copied'; }
    catch { button.textContent='Copy the URI above'; }
    return;
  }
  if(action==='step-info') {
    const step = STEPS[Number(button.dataset.index)];
    return modal(step.name, `<span class="layer-tag">${step.layer}</span><p>${step.description}</p><div class="callout">Use the main action button to advance in order. ${step.note}.</div>`);
  }
  if(action==='art') {
    const item=currentItem();
    return modal(`${item.name} #${item.token}`, `<img class="modal-art" src="${item.image}" alt="Shielded Byte collectible card"/><p>${item.collection} · Zero Trace. Original card concept from the Zcash pixel NFT example. Fan-made; not affiliated with the Zcash Foundation or Electric Coin Co.</p>`);
  }
  if(action==='filter') { selectedTab=button.dataset.filter; return render(); }
  if(action==='autoplay') {
    if(auto) stopAuto(); else { runStep(); auto=setInterval(runStep,3200); }
    render(); return;
  }
  if(action==='reset') return modal('Start a fresh rehearsal?', '<p>This restores the two simulated wallets, reserve and collection. The local action trail will be cleared.</p><button class="button dark full" data-action="confirm-reset">Reset rehearsal</button><button class="button full" data-action="close">Keep this run</button>');
  if(action==='confirm-reset' && !busy) { stopAuto(); state=createState(); selectedTab='all'; persist(); $('#modal').close(); render(); announce('Rehearsal reset'); }
});
$('#modal').addEventListener('click', event => { if(event.target===$('#modal')) {const r=$('#modal').getBoundingClientRect(); if(event.clientX<r.left||event.clientX>r.right||event.clientY<r.top||event.clientY>r.bottom) $('#modal').close();} });
$('#modal').addEventListener('close', ()=>{modalGeneration++; modalTrigger?.isConnected ? modalTrigger.focus() : $('.next-action')?.focus({preventScroll:true});});
document.addEventListener('visibilitychange',()=>{if(document.hidden){stopAuto();render();}});
render();
