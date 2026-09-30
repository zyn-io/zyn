const cards = [...document.querySelectorAll('[data-card]')];
const prefersReducedMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;
let topLayer = 10;

const hero = document.querySelector('.hero');
const torchEmitter = document.querySelector('.torch-emitter');
const torchState = { x: 0, y: 0, targetX: 0, targetY: 0, initialized: false };

function resetTorchTarget() {
  const bounds = hero.getBoundingClientRect();
  torchState.targetX = bounds.width * 0.7;
  torchState.targetY = bounds.height * 0.42;
  if (!torchState.initialized) {
    torchState.x = torchState.targetX;
    torchState.y = torchState.targetY;
    torchState.initialized = true;
  }
}

function aimTorch(event) {
  const bounds = hero.getBoundingClientRect();
  torchState.targetX = Math.max(bounds.width * .07, Math.min(bounds.width * .94, event.clientX - bounds.left));
  torchState.targetY = Math.max(bounds.height * .08, Math.min(bounds.height * .86, event.clientY - bounds.top));
}

hero.addEventListener('pointermove', aimTorch);
window.addEventListener('resize', resetTorchTarget);
resetTorchTarget();

function renderTorch() {
  const speed = prefersReducedMotion ? 1 : .095;
  torchState.x += (torchState.targetX - torchState.x) * speed;
  torchState.y += (torchState.targetY - torchState.y) * speed;
  const bounds = hero.getBoundingClientRect();
  const emitterBounds = torchEmitter.getBoundingClientRect();
  const sourceViewportX = emitterBounds.left + emitterBounds.width / 2;
  const sourceViewportY = emitterBounds.top + emitterBounds.height / 2;
  const sourceX = sourceViewportX - bounds.left;
  const sourceY = sourceViewportY - bounds.top;
  const dx = torchState.x - sourceX;
  const dy = torchState.y - sourceY;
  const angle = Math.atan2(dx, -dy) * 180 / Math.PI;
  const distance = Math.max(180, Math.hypot(dx, dy));
  hero.style.setProperty('--light-x', `${torchState.x.toFixed(1)}px`);
  hero.style.setProperty('--light-y', `${torchState.y.toFixed(1)}px`);
  hero.style.setProperty('--beam-angle', `${angle.toFixed(2)}deg`);
  hero.style.setProperty('--beam-length', `${distance.toFixed(1)}px`);
  hero.style.setProperty('--beam-source-x', `${sourceViewportX.toFixed(1)}px`);
  hero.style.setProperty('--beam-source-y', `${sourceViewportY.toFixed(1)}px`);
  hero.style.setProperty('--torch-lean', `${Math.max(-34, Math.min(34, angle * .82)).toFixed(2)}deg`);
  requestAnimationFrame(renderTorch);
}

renderTorch();

for (const card of cards) {
  const state = {
    dragging: false,
    pointerId: null,
    originX: 0,
    originY: 0,
    startX: 0,
    startY: 0,
    x: 0,
    y: 0,
    targetX: 0,
    targetY: 0,
    rx: 0,
    ry: 0,
    targetRx: 0,
    targetRy: 0,
  };

  const updateTargetFromPointer = (event) => {
    const rect = card.getBoundingClientRect();
    const localX = Math.max(0, Math.min(rect.width, event.clientX - rect.left));
    const localY = Math.max(0, Math.min(rect.height, event.clientY - rect.top));
    const nx = localX / rect.width;
    const ny = localY / rect.height;
    state.targetRy = (nx - 0.5) * 18;
    state.targetRx = (0.5 - ny) * 18;
    card.style.setProperty('--glare-x', `${nx * 100}%`);
    card.style.setProperty('--glare-y', `${ny * 100}%`);

    const response = {
      moonlight: { x: -3 + (nx - 0.5) * 3, y: -3 + (ny - 0.5) * 3 },
      laser: { x: (nx - 0.5) * 6, y: (ny - 0.5) * 3 },
      sunlight: { x: -4 + (nx - 0.5) * 5, y: -5 + (ny - 0.5) * 5 },
    }[card.dataset.effect];
    card.style.setProperty('--rim-x', `${response.x.toFixed(2)}px`);
    card.style.setProperty('--rim-y', `${response.y.toFixed(2)}px`);
    card.style.setProperty('--rim-far-x', `${(response.x * 2.35).toFixed(2)}px`);
    card.style.setProperty('--rim-far-y', `${(response.y * 2.35).toFixed(2)}px`);
  };

  card.addEventListener('pointerdown', (event) => {
    if (event.button !== 0) return;
    state.dragging = true;
    state.pointerId = event.pointerId;
    state.originX = event.clientX;
    state.originY = event.clientY;
    state.startX = state.targetX;
    state.startY = state.targetY;
    card.classList.add('is-dragging');
    card.style.zIndex = ++topLayer;
    card.setPointerCapture(event.pointerId);
    updateTargetFromPointer(event);
  });

  card.addEventListener('pointermove', (event) => {
    updateTargetFromPointer(event);
    if (!state.dragging || event.pointerId !== state.pointerId) return;
    const bounds = card.closest('.specimens').getBoundingClientRect();
    state.targetX = Math.max(-bounds.width * 0.36, Math.min(bounds.width * 0.36, state.startX + event.clientX - state.originX));
    state.targetY = Math.max(-bounds.height * 0.26, Math.min(bounds.height * 0.26, state.startY + event.clientY - state.originY));
  });

  const release = (event) => {
    if (!state.dragging || (event.pointerId !== undefined && event.pointerId !== state.pointerId)) return;
    state.dragging = false;
    card.classList.remove('is-dragging');
    state.targetRx *= 0.38;
    state.targetRy *= 0.38;
  };

  card.addEventListener('pointerup', release);
  card.addEventListener('pointercancel', release);
  card.addEventListener('lostpointercapture', release);
  card.addEventListener('pointerleave', () => {
    if (!state.dragging) {
      state.targetRx = 0;
      state.targetRy = 0;
      card.style.setProperty('--glare-x', '50%');
      card.style.setProperty('--glare-y', '50%');
    }
  });

  card.addEventListener('keydown', (event) => {
    const movement = { ArrowLeft: [-12, 0], ArrowRight: [12, 0], ArrowUp: [0, -12], ArrowDown: [0, 12] }[event.key];
    if (!movement) return;
    event.preventDefault();
    // The same bounds as dragging: a card nudged by keys stays in the cave.
    const bounds = (card.closest('.specimens') || card.parentElement).getBoundingClientRect();
    state.targetX = Math.max(-bounds.width * 0.36, Math.min(bounds.width * 0.36, state.targetX + movement[0]));
    state.targetY = Math.max(-bounds.height * 0.26, Math.min(bounds.height * 0.26, state.targetY + movement[1]));
    card.style.zIndex = ++topLayer;
  });

  const render = () => {
    const speed = prefersReducedMotion ? 1 : 0.14;
    state.x += (state.targetX - state.x) * speed;
    state.y += (state.targetY - state.y) * speed;
    state.rx += (state.targetRx - state.rx) * speed;
    state.ry += (state.targetRy - state.ry) * speed;
    card.style.setProperty('--tx', `${state.x.toFixed(2)}px`);
    card.style.setProperty('--ty', `${state.y.toFixed(2)}px`);
    card.style.setProperty('--rx', `${state.rx.toFixed(2)}deg`);
    card.style.setProperty('--ry', `${state.ry.toFixed(2)}deg`);
    requestAnimationFrame(render);
  };
  render();
}

const dialog = document.querySelector('#manifest');
document.querySelectorAll('[data-dialog-open]').forEach((button) => {
  button.addEventListener('click', () => dialog.showModal());
});
document.querySelectorAll('[data-dialog-close]').forEach((button) => {
  button.addEventListener('click', () => dialog.close());
});
dialog.addEventListener('click', (event) => {
  const bounds = dialog.getBoundingClientRect();
  const inside = event.clientX >= bounds.left && event.clientX <= bounds.right && event.clientY >= bounds.top && event.clientY <= bounds.bottom;
  if (!inside) dialog.close();
});

document.documentElement.dataset.caveReady = 'true';
window.__cave = { ready: true, cards: cards.length, supply: 4444 };

// The door's real state, from the chain: the sale on mainnet, if any.
const door = document.querySelector('#door');
if (door) {
  fetch('https://zyn-rpc.cavebro.win/', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'zyn_market', params: [] }),
    credentials: 'omit', referrerPolicy: 'no-referrer', signal: AbortSignal.timeout(10000),
  }).then((r) => (r.ok ? r.json() : null)).then((b) => {
    const sale = (b?.result?.sales || []).find((x) => x.collection === '024277b607ce4edbf78fe2855aba562096d00264729168bd0ddea71bdf5b5f1e');
    const label = { open: 'open', closed: 'drawing', drawn: 'claims open', openmint: 'open mint' }[sale?.phase];
    if (label) door.textContent = label;
  }).catch(() => { /* the static text stands */ });
}
