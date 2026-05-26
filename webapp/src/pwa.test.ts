import { afterEach, describe, expect, it, vi } from 'vitest';
import { registerServiceWorker } from './pwa';

describe('registerServiceWorker', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
  });

  it('registers the app service worker after window load in production', () => {
    vi.stubEnv('DEV', false);
    const register = vi.fn().mockResolvedValue(undefined);
    const addEventListener = vi.spyOn(window, 'addEventListener');
    vi.stubGlobal('navigator', {
      serviceWorker: { register },
    });

    registerServiceWorker();
    const loadHandler = addEventListener.mock.calls.find(
      ([eventName]) => eventName === 'load',
    )?.[1] as EventListener | undefined;
    loadHandler?.(new Event('load'));

    expect(register).toHaveBeenCalledWith('/service-worker.js');
  });

  it('skips registration during Vite dev', () => {
    vi.stubEnv('DEV', true);
    const register = vi.fn().mockResolvedValue(undefined);
    const addEventListener = vi.spyOn(window, 'addEventListener');
    vi.stubGlobal('navigator', {
      serviceWorker: { register },
    });

    registerServiceWorker();

    expect(addEventListener).not.toHaveBeenCalledWith('load', expect.any(Function));
    expect(register).not.toHaveBeenCalled();
  });
});
