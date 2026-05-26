/* global caches, fetch, self, URL */

const CACHE_PREFIX = 'hail-shell';
const CACHE_VERSION = 'v1';
const SHELL_CACHE = `${CACHE_PREFIX}-${CACHE_VERSION}`;

const STATIC_SHELL_ASSETS = [
  '/manifest.webmanifest',
  '/favicon.ico',
  '/favicon-32x32.png',
  '/apple-touch-icon.png',
  '/icon-192.png',
  '/icon-512.png',
  '/logo-icon-transparent.png',
];

self.addEventListener('install', (event) => {
  event.waitUntil(cacheShell().then(() => self.skipWaiting()));
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((cacheNames) =>
        Promise.all(
          cacheNames
            .filter((cacheName) => cacheName.startsWith(CACHE_PREFIX) && cacheName !== SHELL_CACHE)
            .map((cacheName) => caches.delete(cacheName)),
        ),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener('fetch', (event) => {
  const { request } = event;

  if (request.method !== 'GET') {
    return;
  }

  const url = new URL(request.url);

  if (
    url.origin !== self.location.origin ||
    url.pathname.startsWith('/api/') ||
    url.pathname === '/healthz' ||
    url.pathname === '/readyz'
  ) {
    return;
  }

  if (request.mode === 'navigate') {
    event.respondWith(networkFirstShell(request));
    return;
  }

  event.respondWith(cacheFirstSameOriginAsset(request));
});

async function cacheShell() {
  const cache = await caches.open(SHELL_CACHE);
  await cache.addAll(STATIC_SHELL_ASSETS);

  const indexResponse = await fetch('/index.html', { cache: 'no-store' });
  if (!indexResponse.ok || !isHtmlResponse(indexResponse)) {
    return;
  }

  const indexHtml = await indexResponse.clone().text();
  await cache.put('/', indexResponse.clone());
  await cache.put('/index.html', indexResponse);

  const builtAssets = findSameOriginShellAssets(indexHtml);
  if (builtAssets.length > 0) {
    await cache.addAll(builtAssets);
  }
}

async function networkFirstShell(request) {
  const cache = await caches.open(SHELL_CACHE);

  try {
    const response = await fetch(request);
    if (response.ok && isHtmlResponse(response)) {
      await cache.put('/index.html', response.clone());
    }
    return response;
  } catch (error) {
    const cachedShell = await cache.match('/index.html');
    if (cachedShell) {
      return cachedShell;
    }
    throw error;
  }
}

async function cacheFirstSameOriginAsset(request) {
  const cached = await caches.match(request);
  if (cached) {
    return cached;
  }

  const response = await fetch(request);
  if (!response.ok) {
    return response;
  }

  const cache = await caches.open(SHELL_CACHE);
  await cache.put(request, response.clone());
  return response;
}

function findSameOriginShellAssets(html) {
  const assets = new Set();
  const assetReferencePattern = /\b(?:href|src)="([^"]+)"/g;
  let match;

  while ((match = assetReferencePattern.exec(html)) !== null) {
    const [, rawUrl] = match;
    const url = new URL(rawUrl, self.location.origin);
    if (url.origin === self.location.origin && !url.pathname.startsWith('/api/')) {
      assets.add(`${url.pathname}${url.search}`);
    }
  }

  return [...assets];
}

function isHtmlResponse(response) {
  return response.headers.get('content-type')?.includes('text/html') ?? false;
}
