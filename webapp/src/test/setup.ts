import '@testing-library/jest-dom/vitest';
import { vi } from 'vitest';

Object.defineProperty(window, 'scrollTo', {
  value: vi.fn(),
  writable: true,
});

Object.defineProperty(window.HTMLElement.prototype, 'scrollIntoView', {
  value: vi.fn(),
  writable: true,
});

Object.defineProperty(window, 'matchMedia', {
  value: vi.fn().mockImplementation((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })),
  writable: true,
});

class TestResizeObserver {
  observe() {
    // jsdom has no layout; tests only need components to mount.
  }

  unobserve() {
    // jsdom has no layout; tests only need components to mount.
  }

  disconnect() {
    // jsdom has no layout; tests only need components to mount.
  }
}

Object.defineProperty(window, 'ResizeObserver', {
  value: TestResizeObserver,
  writable: true,
});

Object.defineProperty(globalThis, 'ResizeObserver', {
  value: TestResizeObserver,
  writable: true,
});
