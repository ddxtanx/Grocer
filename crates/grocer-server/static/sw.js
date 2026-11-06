// Offline shell: pre-cache the entire on-device pipeline (app + WASM parser +
// Tesseract core + English model ~20 MB) so scanning works in an airplane
// grocery aisle with zero signal. API calls always go to the network.
//
// Handler rule (learned the hard way): a cache miss + network failure must
// never resolve to `undefined` — respondWith(undefined) makes Firefox report
// NS_ERROR_CORRUPTED_CONTENT and hides the real 404. Honest errors pass
// through; only precached/known assets get served stale.
const CACHE = 'grocer-v3';
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
  e.respondWith((async () => {
    const hit = await caches.match(e.request);
    if (hit) {
      // stale-while-revalidate, quietly
      fetch(e.request).then(res => {
        if (res.ok && url.origin === self.location.origin) {
          const copy = res.clone();
          caches.open(CACHE).then(c => c.put(e.request, copy)).catch(() => {});
        }
      }).catch(() => {});
      return hit;
    }
    const res = await fetch(e.request); // real 404/errors surface as themselves
    if (res.ok && url.origin === self.location.origin) {
      const copy = res.clone();
      caches.open(CACHE).then(c => c.put(e.request, copy)).catch(() => {});
    }
    return res;
  })());
});
