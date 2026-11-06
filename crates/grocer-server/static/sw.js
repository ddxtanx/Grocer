// Offline shell: pre-cache the entire on-device pipeline (app + WASM parser +
// Tesseract core + English model ~20 MB) so scanning works in an airplane
// grocery aisle with zero signal. API calls always go to the network.
const CACHE = 'grocer-v1';
const PRECACHE = [
  '/', '/app.js', '/style.css', '/manifest.json',
  '/grocer-parser.wasm',
  '/vendor/tesseract.min.js', '/vendor/worker.min.js',
  '/vendor/tesseract-core-simd.wasm.js', '/vendor/tesseract-core-simd.wasm',
  '/vendor/tessdata/eng.traineddata.gz',
];

self.addEventListener('install', e => {
  e.waitUntil(caches.open(CACHE).then(c => c.addAll(PRECACHE)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', e => {
  e.waitUntil(
    caches.keys()
      .then(keys => Promise.all(keys.filter(k => k !== CACHE).map(k => caches.delete(k))))
      .then(() => self.clients.claim())
  );
});

self.addEventListener('fetch', e => {
  const url = new URL(e.request.url);
  if (url.pathname.startsWith('/api/')) return; // never cache authenticated data
  e.respondWith(
    caches.match(e.request).then(hit => {
      const fresh = fetch(e.request)
        .then(res => {
          if (res.ok && url.origin === self.location.origin) {
            const copy = res.clone();
            caches.open(CACHE).then(c => c.put(e.request, copy));
          }
          return res;
        })
        .catch(() => hit);
      return hit || fresh;
    })
  );
});
