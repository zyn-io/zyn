// Headings that come apart into cubes as their section leaves, and assemble
// from cubes as the next one arrives. Scroll-linked, so scrolling back plays
// it backwards. Each heading is rasterised once into a grid of cubes that
// carry its real colours (the brushed-jade words included); a fixed canvas
// draws them only while a heading is in transition, and the real, crisp text
// takes over the moment it has formed.
(() => {
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  const els = [...document.querySelectorAll('[data-cubes]')];
  if (!els.length) return;

  const canvas = document.createElement('canvas');
  canvas.className = 'cubes';
  canvas.setAttribute('aria-hidden', 'true');
  document.body.appendChild(canvas);
  const ctx = canvas.getContext('2d');
  let W = 0; let H = 0; let dpr = 1;

  const BRUSH = ['#2fc497', '#6ff0c8', '#c6fff0', '#45dcac', '#20b387', '#5ee9bd'];
  const clamp = (x) => (x < 0 ? 0 : x > 1 ? 1 : x);
  const ease = (x) => x * x * (3 - 2 * x);
  // a small deterministic hash, so a heading comes apart the same way each time
  const hash = (a, b) => { let h = (a * 374761393 + b * 668265263) | 0; h = (h ^ (h >>> 13)) * 1274126177; return ((h ^ (h >>> 16)) >>> 0) / 4294967295; };

  function stretchKeyword(pct) {
    const v = parseFloat(pct) || 100;
    return v >= 150 ? 'extra-expanded' : v >= 124 ? 'expanded' : v >= 110 ? 'semi-expanded' : 'normal';
  }

  // Draw an element's words where the browser laid them out, then sample a grid.
  function rasterise(el) {
    const box = el.getBoundingClientRect();
    const w = Math.ceil(box.width); const h = Math.ceil(box.height);
    if (w < 2 || h < 2) return [];
    const off = document.createElement('canvas');
    off.width = w; off.height = h;
    const o = off.getContext('2d', { willReadFrequently: true });
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    let fontPx = 16;
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const cs = getComputedStyle(node.parentElement);
      fontPx = parseFloat(cs.fontSize);
      o.font = `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
      if ('fontStretch' in o) o.fontStretch = stretchKeyword(cs.fontStretch);
      o.textBaseline = 'middle';
      const clear = cs.color === 'rgba(0, 0, 0, 0)' || cs.webkitTextFillColor === 'rgba(0, 0, 0, 0)';
      if (clear) {
        const g = o.createLinearGradient(0, 0, w, h * 0.2);
        BRUSH.forEach((c, i) => g.addColorStop(i / (BRUSH.length - 1), c));
        o.fillStyle = g;
      } else {
        o.fillStyle = cs.color;
      }
      const text = node.textContent;
      const re = /\S+/g;
      for (let m = re.exec(text); m; m = re.exec(text)) {
        const range = document.createRange();
        range.setStart(node, m.index);
        range.setEnd(node, m.index + m[0].length);
        const r = range.getClientRects()[0];
        if (!r) continue;
        const measured = o.measureText(m[0]).width || 1;
        o.save();
        o.translate(r.left - box.left, r.top - box.top + r.height / 2);
        o.scale(r.width / measured, 1);
        o.fillText(m[0], 0, 0);
        o.restore();
      }
    }
    const step = Math.max(3, Math.min(14, Math.round(fontPx / 13)));
    const data = o.getImageData(0, 0, w, h).data;
    const cubes = [];
    for (let y = Math.floor(step / 2); y < h; y += step) {
      for (let x = Math.floor(step / 2); x < w; x += step) {
        const i = (y * w + x) * 4;
        if (data[i + 3] < 110) continue;
        const n1 = hash(x, y); const n2 = hash(y + 7, x + 3); const n3 = hash(x + 11, y + 5);
        cubes.push({
          x: x - step / 2, y: y - step / 2, s: step,
          c: `rgb(${data[i]},${data[i + 1]},${data[i + 2]})`,
          // peel from the left edge with some noise, so it reads as coming apart, not fading
          delay: (x / w) * 0.55 + n1 * 0.45,
          dx: (n2 - 0.5) * w * 0.35,
          dy: -(40 + n3 * h * 1.6),
          spin: (n1 - 0.5) * 1.6,
        });
      }
    }
    return cubes;
  }

  const items = els.map((el) => ({ el, mode: el.dataset.cubes, section: el.closest('section, footer') || el, cubes: null, key: '' }));

  function size() {
    dpr = Math.min(devicePixelRatio || 1, 2);
    W = innerWidth; H = innerHeight;
    canvas.width = Math.round(W * dpr); canvas.height = Math.round(H * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }

  // How far each heading is from being whole: 0 formed, 1 fully scattered.
  // The sign says which way: arriving from below (+) or leaving (-). A heading
  // leaves only once its section has begun to scroll away above the screen,
  // so a section settled at its edge always shows its heading whole.
  const LEAVE_AT = 0.04;
  function progress(item, r) {
    if (item.mode === 'giant') {
      const toBottom = document.documentElement.scrollHeight - innerHeight - scrollY;
      return clamp(toBottom / (H * 0.55));
    }
    const s = item.section.getBoundingClientRect().top;
    const leave = clamp((-s - H * LEAVE_AT) / (H * 0.42));
    if (leave > 0) return -leave;
    if (item.mode === 'hero') return 0;
    return clamp((r.top - H * 0.64) / (H * 0.28));
  }

  let queued = false;
  function frame() {
    queued = false;
    ctx.clearRect(0, 0, W, H);
    for (const item of items) {
      const r = item.el.getBoundingClientRect();
      if (item.mode !== 'giant' && (item.section.getBoundingClientRect().bottom < 0 || r.top > H * 1.4)) { item.el.style.opacity = ''; continue; }
      const p = progress(item, r);
      const t = Math.abs(p);
      if (t < 0.002) { item.el.style.opacity = ''; continue; }
      const key = `${Math.round(r.width)}x${Math.round(r.height)}`;
      if (!item.cubes || item.key !== key) { item.cubes = rasterise(item.el); item.key = key; }
      // a short crossfade where the pixels hand over to the crisp text
      const hand = clamp(t / 0.06);
      item.el.style.opacity = String(1 - hand);
      const up = p < 0;
      // Leaving, the cubes break off where the heading stood when its section
      // began to go, and crumble there while the page moves on beneath them.
      const baseY = up ? r.top - item.section.getBoundingClientRect().top - H * LEAVE_AT : r.top;
      for (const c of item.cubes) {
        const lt = ease(clamp(t * 1.55 - c.delay * 0.55));
        const a = (1 - lt * lt) * hand;
        if (a <= 0.01) continue;
        const dy = up ? c.dy : -c.dy * 0.6;
        const x = r.left + c.x + c.dx * lt;
        const y = baseY + c.y + dy * lt;
        const s = c.s * (1 - 0.35 * lt) - 0.6;
        ctx.globalAlpha = a;
        if (lt > 0.02) {
          ctx.save();
          ctx.translate(x + s / 2, y + s / 2);
          ctx.rotate(c.spin * lt);
          ctx.fillStyle = c.c; ctx.fillRect(-s / 2, -s / 2, s, s);
          ctx.fillStyle = 'rgba(0,0,0,0.28)'; ctx.fillRect(-s / 2, s / 2 - s * 0.28, s, s * 0.28);
          ctx.restore();
        } else {
          ctx.fillStyle = c.c; ctx.fillRect(x, y, s, s);
        }
      }
    }
    ctx.globalAlpha = 1;
  }
  const request = () => { if (!queued) { queued = true; requestAnimationFrame(frame); } };

  size();
  addEventListener('scroll', request, { passive: true });
  addEventListener('resize', () => { size(); items.forEach((i) => { i.cubes = null; }); request(); });
  // fonts change every glyph's shape: rasterise again once they are in
  (document.fonts ? document.fonts.ready : Promise.resolve()).then(() => { items.forEach((i) => { i.cubes = null; }); request(); });
  request();
})();
