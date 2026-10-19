// The den's service worker: only so the page can be installed, and so a push
// can wake a closed page. It caches nothing and sees no messages: a push says
// only the den's name and which room, sealed for this browser by the den.
// The words shown are the page's own, so a den cannot put any of its own in
// a notification: only its name (one line) and a room id.
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));

const ROOM_ID = /^[a-z0-9][a-z0-9_-]{0,63}$/;

self.addEventListener('push', (e) => {
  let d = {};
  try { d = e.data ? e.data.json() : {}; } catch { d = {}; }
  const name = typeof d.title === 'string' ? d.title.replace(/[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]+/gu, ' ').replace(/\s+/g, ' ').trim().slice(0, 40) : '';
  const room = typeof d.tag === 'string' && ROOM_ID.test(d.tag) ? d.tag : '';
  const where = !room ? '' : room.startsWith('p-') ? ' in a private room' : ` in #${room}`;
  e.waitUntil(self.registration.showNotification(name || 'Epeaky', {
    body: `New message${where}`,
    tag: room || 'den',
    icon: 'icons/icon-192.png',
    data: { room },
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
