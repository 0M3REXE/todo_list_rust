// Service worker — offline shell for the Todo List PWA.
// Caches static assets + page shells so the app loads offline.

var CACHE_NAME = 'todo-v2';
var STATIC_ASSETS = [
  '/',
  '/app',
  '/login',
  '/signup',
  '/static/vendor/htmx.min.js',
  '/static/app.css',
  '/static/theme-toggle.js',
  '/static/manifest.json',
  '/static/icons/icon-192x192.png',
  '/static/icons/icon-512x512.png'
];

// Install: cache the shell
self.addEventListener('install', function (event) {
  event.waitUntil(
    caches.open(CACHE_NAME).then(function (cache) {
      return cache.addAll(STATIC_ASSETS);
    })
  );
  self.skipWaiting();
});

// Activate: clean old caches
self.addEventListener('activate', function (event) {
  event.waitUntil(
    caches.keys().then(function (keys) {
      return Promise.all(
        keys.filter(function (key) { return key !== CACHE_NAME; })
          .map(function (key) { return caches.delete(key); })
      );
    })
  );
  self.clients.claim();
});

// Fetch: network-first for pages, cache-first for static assets
self.addEventListener('fetch', function (event) {
  var url = new URL(event.request.url);

  // Static assets: cache first
  if (url.origin === location.origin && url.pathname.startsWith('/static/')) {
    event.respondWith(
      caches.match(event.request).then(function (cached) {
        return cached || fetch(event.request);
      })
    );
    return;
  }

  // Page shells: network first, cache fallback
  if (event.request.mode === 'navigate') {
    event.respondWith(
      fetch(event.request).then(function (response) {
        // Cache a copy of the fresh page
        var copy = response.clone();
        caches.open(CACHE_NAME).then(function (cache) {
          cache.put(event.request, copy);
        });
        return response;
      }).catch(function () {
        return caches.match(event.request);
      })
    );
    return;
  }

  // API / everything else: try network, no offline fallback
  event.respondWith(fetch(event.request));
});
