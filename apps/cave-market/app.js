const prefersReducedMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;

const starlightField = document.querySelector('.starlight-field');
if (starlightField && !prefersReducedMotion) {
  const starlight = { x: 0, y: 0, targetX: 0, targetY: 0 };

  window.addEventListener('pointermove', (event) => {
    starlight.targetX = (event.clientX / window.innerWidth - .5) * -12;
    starlight.targetY = (event.clientY / window.innerHeight - .5) * -8;
  }, { passive: true });

  const driftStarlight = () => {
    starlight.x += (starlight.targetX - starlight.x) * .035;
    starlight.y += (starlight.targetY - starlight.y) * .035;
    starlightField.style.setProperty('--star-x', `${starlight.x.toFixed(2)}px`);
    starlightField.style.setProperty('--star-y', `${starlight.y.toFixed(2)}px`);
    requestAnimationFrame(driftStarlight);
  };

  driftStarlight();
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
window.__cave = { ready: true, supply: 4444, marketItems: 0 };

const home = document.querySelector('#top');
const market = document.querySelector('#market');
const launch = document.querySelector('#launch');
const portfolio = document.querySelector('#portfolio');
const routeLinks = [...document.querySelectorAll('[data-route]')];
const themeColor = document.querySelector('meta[name="theme-color"]');
const defaultView = document.documentElement.dataset.defaultView || 'home';

const walletDialog = document.querySelector('#wallet-dialog');
document.querySelectorAll('[data-wallet-dialog]').forEach((button) => {
  button.addEventListener('click', () => walletDialog.showModal());
});
document.querySelectorAll('[data-wallet-close]').forEach((button) => {
  button.addEventListener('click', () => walletDialog.close());
});
walletDialog.addEventListener('click', (event) => {
  const bounds = walletDialog.getBoundingClientRect();
  const inside = event.clientX >= bounds.left && event.clientX <= bounds.right && event.clientY >= bounds.top && event.clientY <= bounds.bottom;
  if (!inside) walletDialog.close();
});

function syncRoute({ focus = false } = {}) {
  const view = location.hash === '#market' ? 'market' : location.hash === '#launch' ? 'launch' : location.hash === '#portfolio' ? 'portfolio' : location.hash === '#collection' ? 'home' : defaultView;
  home.hidden = view !== 'home';
  market.hidden = view !== 'market';
  launch.hidden = view !== 'launch';
  portfolio.hidden = view !== 'portfolio';
  document.body.dataset.view = view;
  themeColor.content = '#070807';
  document.title = view === 'market' ? 'Market — Cave' : view === 'launch' ? 'NFT Studio — Cave' : view === 'portfolio' ? 'Portfolio — Cave' : 'Cave — Private Objects on Zcash';

  routeLinks.forEach((link) => {
    const current = view === 'home' ? link.getAttribute('href') === (location.hash || '#collection') : link.dataset.route === view;
    if (current) link.setAttribute('aria-current', 'page');
    else link.removeAttribute('aria-current');
  });

  if (focus && view !== 'home') document.querySelector(`#${view}-title`)?.focus({ preventScroll: true });
}

window.addEventListener('hashchange', () => {
  window.scrollTo(0, 0);
  syncRoute({ focus: true });
});
syncRoute();
if (location.hash) requestAnimationFrame(() => window.scrollTo(0, 0));

const launchForm = document.querySelector('#launch-policy-form');

if (launchForm) {
  const quantityPanels = [...document.querySelectorAll('[data-quantity-panel]')];
  const outcomes = document.querySelector('#weighted-outcomes');
  const formatter = new Intl.NumberFormat('en-US');

  const numberValue = (selector, fallback = 0) => {
    const value = Number(document.querySelector(selector)?.value);
    return Number.isFinite(value) ? value : fallback;
  };

  const outcomeValues = () => [...outcomes.querySelectorAll('.outcome-row')].map((row) => ({
    items: Math.max(1, Number(row.querySelector('.outcome-items')?.value) || 1),
    weight: Math.max(0, Number(row.querySelector('.outcome-weight')?.value) || 0),
  }));

  const getPolicy = () => {
    const quantityMode = launchForm.elements['quantity-mode'].value;
    const identityMode = launchForm.elements['identity-mode'].value;
    const weighted = outcomeValues();
    const totalWeight = weighted.reduce((sum, outcome) => sum + outcome.weight, 0);
    let expected = 0;
    let minimum = 1;
    let maximum = 1;

    if (quantityMode === 'weighted') {
      expected = totalWeight ? weighted.reduce((sum, outcome) => sum + outcome.items * outcome.weight, 0) / totalWeight : 0;
      minimum = Math.min(...weighted.map((outcome) => outcome.items));
      maximum = Math.max(...weighted.map((outcome) => outcome.items));
    } else if (quantityMode === 'fixed') {
      expected = Math.max(1, numberValue('#fixed-count', 1));
      minimum = expected;
      maximum = expected;
    } else {
      minimum = Math.max(1, numberValue('#buyer-min', 1));
      maximum = Math.max(minimum, numberValue('#buyer-max', minimum));
      expected = (minimum + maximum) / 2;
    }

    return {
      version: 'cave-allocation/v1',
      collection: document.querySelector('#launch-name').value.trim() || 'Untitled collection',
      supply: Math.max(1, numberValue('#launch-supply', 1)),
      entryPriceZec: Math.max(0, numberValue('#launch-price', 0)),
      quantity: { mode: quantityMode, outcomes: quantityMode === 'weighted' ? weighted : undefined, fixed: quantityMode === 'fixed' ? minimum : undefined, minimum, maximum },
      identity: { mode: identityMode },
      expected,
      totalWeight,
    };
  };

  const assignmentLabels = {
    'concealed-shuffle': 'Concealed public shuffle',
    'instant-random': 'Instant public random',
    sequential: 'Sequential indices',
  };

  const modeLabels = { fixed: 'Fixed receipt', weighted: 'Weighted draw', buyer: 'Buyer chooses' };

  const policySentence = (policy) => {
    if (policy.quantity.mode === 'fixed') return `Every entry receives exactly ${policy.quantity.fixed} ${policy.quantity.fixed === 1 ? 'object' : 'objects'}.`;
    if (policy.quantity.mode === 'buyer') return `Collectors choose between ${policy.quantity.minimum} and ${policy.quantity.maximum} objects. Cave does not draw a quantity.`;
    const amounts = policy.quantity.outcomes.map(({ items }) => items).join(', ').replace(/, ([^,]*)$/, ', or $1');
    return `Each entry receives ${amounts} concealed ${policy.quantity.maximum === 1 ? 'object' : 'objects'} through a weighted draw.`;
  };

  const renderPolicy = () => {
    const policy = getPolicy();
    const capacity = policy.expected > 0 ? Math.floor(policy.supply / policy.expected) : 0;
    const quantityUsesRandomness = policy.quantity.mode === 'weighted';
    const identityUsesRandomness = policy.identity.mode !== 'sequential';

    document.querySelector('#preview-name').textContent = policy.collection;
    document.querySelector('#preview-supply').textContent = formatter.format(policy.supply);
    document.querySelector('#preview-price').textContent = `${policy.entryPriceZec.toLocaleString(undefined, { maximumFractionDigits: 8 })} ZEC`;
    document.querySelector('#preview-mode').textContent = modeLabels[policy.quantity.mode];
    document.querySelector('#preview-expected').textContent = policy.quantity.mode === 'buyer' ? `${policy.quantity.minimum}–${policy.quantity.maximum}` : policy.expected.toFixed(2).replace(/\.00$/, '');
    document.querySelector('#preview-capacity').textContent = policy.quantity.mode === 'buyer' ? `${formatter.format(Math.floor(policy.supply / policy.quantity.maximum))}–${formatter.format(Math.floor(policy.supply / policy.quantity.minimum))}` : `≈ ${formatter.format(capacity)}`;
    document.querySelector('#preview-assignment').textContent = assignmentLabels[policy.identity.mode];
    document.querySelector('#preview-randomness').textContent = quantityUsesRandomness && identityUsesRandomness ? 'Quantity + identity randomness' : quantityUsesRandomness ? 'Quantity randomness only' : identityUsesRandomness ? 'Identity randomness only' : 'No randomness used';
    document.querySelector('#quantity-randomness-state').textContent = quantityUsesRandomness ? 'Randomness on' : 'Randomness off';
    document.querySelector('#policy-sentence').textContent = policySentence(policy);

    const totalLabel = document.querySelector('#weight-total');
    totalLabel.textContent = policy.totalWeight === 100 ? '100% allocated' : `${policy.totalWeight}% allocated — must equal 100%`;
    totalLabel.classList.toggle('invalid', policy.totalWeight !== 100);

    const colors = ['#f4b728', '#97d4e2', '#c7f04b', '#f28560', '#d2a7e7', '#f0eee6'];
    document.querySelector('#weight-visual').innerHTML = policy.quantity.mode === 'weighted' && policy.totalWeight
      ? policy.quantity.outcomes.map((outcome, index) => `<i style="--weight:${outcome.weight}; --tone:${colors[index % colors.length]}" title="${outcome.items} objects: ${outcome.weight}%"></i>`).join('')
      : `<i style="--weight:100; --tone:${policy.quantity.mode === 'fixed' ? '#f4b728' : '#97d4e2'}"></i>`;
  };

  const syncQuantityMode = () => {
    const mode = launchForm.elements['quantity-mode'].value;
    quantityPanels.forEach((panel) => { panel.hidden = panel.dataset.quantityPanel !== mode; });
    document.querySelector('#preview-draw').textContent = mode === 'weighted' ? 'Run 10,000-entry preview →' : 'Preview allocation →';
    document.querySelector('#simulation').innerHTML = '<div class="simulation-idle"><span aria-hidden="true">░▒▓</span><p>Run a preview to inspect the distribution before publishing.</p></div>';
    renderPolicy();
  };

  const bindOutcomeRow = (row) => {
    row.querySelectorAll('input').forEach((input) => input.addEventListener('input', renderPolicy));
    row.querySelector('.outcome-remove').addEventListener('click', () => {
      if (outcomes.children.length <= 1) return;
      row.remove();
      renderPolicy();
    });
  };

  [...outcomes.children].forEach(bindOutcomeRow);
  document.querySelector('#add-outcome').addEventListener('click', () => {
    if (outcomes.children.length >= 6) return;
    const row = document.createElement('div');
    row.className = 'outcome-row';
    row.innerHTML = '<span class="outcome-grip" aria-hidden="true">⋮⋮</span><label><span class="sr-only">Objects received</span><input class="outcome-items" type="number" min="1" max="100" value="4" inputmode="numeric" /></label><label><span class="sr-only">Chance percent</span><input class="outcome-weight" type="number" min="0" max="100" value="0" inputmode="decimal" /><b>%</b></label><button class="outcome-remove" type="button" aria-label="Remove outcome">×</button>';
    outcomes.append(row);
    bindOutcomeRow(row);
    renderPolicy();
    row.querySelector('.outcome-items').focus();
  });

  const randomUnit = () => {
    const values = new Uint32Array(1);
    crypto.getRandomValues(values);
    return values[0] / 4294967296;
  };

  document.querySelector('#preview-draw').addEventListener('click', () => {
    const policy = getPolicy();
    const counts = new Map();

    if (policy.quantity.mode === 'weighted' && policy.totalWeight !== 100) {
      document.querySelector('#simulation').innerHTML = '<div class="simulation-error"><strong>Distribution incomplete</strong><p>Chances must total exactly 100% before the preview can run.</p></div>';
      return;
    }

    const entries = policy.quantity.mode === 'weighted' ? 10000 : 1;
    for (let index = 0; index < entries; index += 1) {
      let result;
      if (policy.quantity.mode === 'weighted') {
        const target = randomUnit() * policy.totalWeight;
        let cursor = 0;
        result = policy.quantity.outcomes.find((outcome) => {
          cursor += outcome.weight;
          return target < cursor;
        })?.items ?? policy.quantity.outcomes.at(-1).items;
      } else if (policy.quantity.mode === 'fixed') result = policy.quantity.fixed;
      else result = `${policy.quantity.minimum}–${policy.quantity.maximum}`;
      counts.set(result, (counts.get(result) || 0) + 1);
    }

    const rows = [...counts.entries()].map(([items, count]) => ({ items, count, percent: (count / entries) * 100 }));
    document.querySelector('#simulation').innerHTML = `<div class="simulation-head"><span>${policy.quantity.mode === 'weighted' ? '10,000-entry sample' : 'Allocation preview'}</span><b>Browser simulation / not the draw</b></div><div class="simulation-bars">${rows.map((row) => `<div><span>${row.items} ${String(row.items).includes('–') ? 'objects selectable' : Number(row.items) === 1 ? 'object' : 'objects'}</span><i><b style="--bar:${Math.max(row.percent, 1)}%"></b></i><strong>${policy.quantity.mode === 'weighted' ? `${row.percent.toFixed(1)}%` : '100%'}</strong></div>`).join('')}</div>`;
  });

  launchForm.querySelectorAll('input').forEach((input) => {
    if (input.name === 'quantity-mode') input.addEventListener('change', syncQuantityMode);
    else input.addEventListener('input', renderPolicy);
    if (input.name === 'identity-mode') input.addEventListener('change', renderPolicy);
  });

  renderPolicy();
}
