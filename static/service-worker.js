// Cache public assets and a data-free offline screen. Never persist account pages
// or todo responses: they can contain private data on a shared device.
var CACHE_NAME = 'todo-v5';
var STATIC_ASSETS = [
  '/offline',
  '/static/vendor/htmx.min.js',
  '/static/app.css',
  '/static/theme-toggle.js',
  '/static/manifest.json',
  '/static/brand-mark.svg',
  '/static/icons/icon-180x180.png',
  '/static/icons/icon-192x192.png',
  '/static/icons/icon-512x512.png',
  '/static/icons/icon-512-maskable.png'
];

self.addEventListener('install', function (event) {
  event.waitUntil(
    caches.open(CACHE_NAME).then(function (cache) {
      return cache.addAll(STATIC_ASSETS);
    })
  );
  self.skipWaiting();
});

self.addEventListener('activate', function (event) {
  event.waitUntil(
    caches.keys().then(function (keys) {
      return Promise.all(
        keys.filter(function (key) { return key !== CACHE_NAME; })
          .map(function (key) { return caches.delete(key); })
      );
    }).then(function () { return self.clients.claim(); })
  );
});

self.addEventListener('fetch', function (event) {
  var request = event.request;
  var url = new URL(request.url);
  if (request.method !== 'GET' || url.origin !== self.location.origin) return;

  if (url.pathname.startsWith('/static/')) {
    event.respondWith(
      caches.match(request).then(function (cached) {
        return cached || fetch(request);
      })
    );
    return;
  }

  if (request.mode === 'navigate') {
    event.respondWith(
      fetch(request).catch(function () {
        return caches.match('/offline');
      })
    );
  }
});
