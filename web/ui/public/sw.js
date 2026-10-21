// bearclaw service worker: push notifications only (no fetch handler, nothing is cached).
// A push carries { title, body, tag, url }; tapping it opens the app on `url` (a hash route).

self.addEventListener('install', () => self.skipWaiting())
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()))

self.addEventListener('push', (event) => {
  let data = {}
  try {
    data = event.data ? event.data.json() : {}
  } catch {
    data = { body: event.data ? event.data.text() : '' }
  }
  event.waitUntil(
    self.registration.showNotification(data.title || 'bearclaw', {
      body: data.body || '',
      tag: data.tag || undefined,
      icon: '/icon-192.png',
      badge: '/icon-192.png',
      data: { url: data.url || '/#/' },
    }),
  )
})

self.addEventListener('notificationclick', (event) => {
  event.notification.close()
  const url = new URL(event.notification.data?.url || '/#/', self.location.origin).href
  event.waitUntil(
    (async () => {
      const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true })
      const open = windows[0]
      if (open) {
        // Same document: the app switches its hash route (a navigate() would reload it).
        open.postMessage({ type: 'bearclaw:open', url })
        await open.focus()
        return
      }
      await self.clients.openWindow(url)
    })(),
  )
})
