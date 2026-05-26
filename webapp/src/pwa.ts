export function registerServiceWorker() {
  if (import.meta.env.DEV || !('serviceWorker' in navigator)) {
    return;
  }

  window.addEventListener('load', () => {
    navigator.serviceWorker.register('/service-worker.js').catch((error: unknown) => {
      console.warn('hail service worker registration failed', error);
    });
  });
}
