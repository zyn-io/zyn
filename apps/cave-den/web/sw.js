// The den's service worker: only so the page can be installed, and so a push
// can wake a closed page. It caches nothing and sees no messages: a push says
// only the den's name and which room, sealed for this browser by the den.
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));

self.addEventListener('push', (e) => {
  let d = {};
  try { d = e.data ? e.data.json() : {}; } catch { d = {}; }
  const text = (v, n) => (typeof v === 'string' ? v.slice(0, n) : '');
  e.waitUntil(self.registration.showNotification(text(d.title, 60) || 'Epeaky', {
    body: text(d.body, 120) || 'New message',
    tag: text(d.tag, 80) || 'den',
    icon: 'icons/icon-192.png',
    data: { room: text(d.tag, 80) },
  }));
});

self.addEventListener('notificationclick', (e) => {
  e.notification.close();
  const room = e.notification.data?.room || '';
  e.waitUntil((async () => {
    const wins = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
    const w = wins.find((c) => new URL(c.url).origin === self.location.origin);
    if (w) { await w.focus(); if (room) w.postMessage({ den: 'open', room }); return; }
    await self.clients.openWindow(room ? `./#room=${encodeURIComponent(room)}` : './');
  })());
});
