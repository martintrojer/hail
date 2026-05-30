import { useCallback, useEffect, useMemo, useRef } from 'react';
import { cn } from '../lib/utils';

const EMAIL_FRAME_SANDBOX = 'allow-same-origin allow-popups allow-popups-to-escape-sandbox';

const EMAIL_BASE_CSS = `
  html {
    background: transparent;
    color-scheme: light dark;
  }

  body {
    margin: 0;
    background: transparent;
    color: #111827;
    font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    font-size: 16px;
    line-height: 1.55;
    overflow-wrap: anywhere;
  }

  img,
  video,
  canvas,
  svg {
    max-width: 100%;
    height: auto;
  }

  table {
    max-width: 100%;
  }

  pre {
    white-space: pre-wrap;
  }

  a {
    color: #2563eb;
  }

  @media (prefers-color-scheme: dark) {
    body {
      color: #e5e7eb;
    }

    a {
      color: #93c5fd;
    }
  }
`;

function escapeStyleClosingTag(css: string) {
  return css.replaceAll('</style', '<\\/style');
}

function emailDocument(html: string) {
  return `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><base target="_blank"><style>${escapeStyleClosingTag(EMAIL_BASE_CSS)}</style></head><body>${html}</body></html>`;
}

function isHttpUrl(value: string) {
  try {
    const url = new URL(value, window.location.href);
    return url.protocol === 'http:' || url.protocol === 'https:' || url.protocol === 'mailto:';
  } catch {
    return false;
  }
}

export interface EmailFrameProps {
  html: string;
  title?: string;
  className?: string;
  onHeightChange?: (height: number) => void;
}

/**
 * Renders sanitized email HTML in an iframe so the message keeps its own CSS
 * context instead of inheriting Tailwind resets from the app shell.
 *
 * The sandbox deliberately allows same-origin but not scripts. Same-origin lets
 * the parent measure the srcdoc document and install click/form guards without
 * running any code inside the email document. Scripts, forms, and top-level
 * navigation remain disabled by the sandbox.
 */
export function EmailFrame({
  html,
  title = 'Email body',
  className,
  onHeightChange,
}: EmailFrameProps) {
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const srcDoc = useMemo(() => emailDocument(html), [html]);

  const configureFrame = useCallback(() => {
    const iframe = iframeRef.current;
    const doc = iframe?.contentDocument;
    if (!iframe || !doc) {
      return undefined;
    }

    const frame = iframe;
    const frameDocument = doc;

    // jsdom does not reliably hydrate srcdoc documents, so mirror the body
    // fragment for tests without reopening the iframe document (doc.open/doc.write
    // can fire recursive load events in jsdom). Browsers still render srcDoc.
    if (frameDocument.body && frameDocument.body.innerHTML !== html) {
      frameDocument.body.innerHTML = html;
    }

    const anchors = Array.from(frameDocument.querySelectorAll<HTMLAnchorElement>('a[href]'));
    for (const anchor of anchors) {
      anchor.target = '_blank';
      anchor.rel = 'noopener noreferrer';
    }

    function measure() {
      const root = frameDocument.documentElement;
      const body = frameDocument.body;
      const height = Math.max(
        root?.scrollHeight ?? 0,
        body?.scrollHeight ?? 0,
        root?.offsetHeight ?? 0,
        body?.offsetHeight ?? 0,
      );
      const nextHeight = Math.max(48, Math.ceil(height));
      frame.style.height = `${nextHeight}px`;
      onHeightChange?.(nextHeight);
    }

    function handleClick(event: MouseEvent) {
      const target = event.target;
      if (!(target instanceof frameDocument.defaultView!.Element)) {
        return;
      }
      const anchor = target.closest<HTMLAnchorElement>('a[href]');
      if (!anchor) {
        return;
      }

      event.preventDefault();
      const href = anchor.href;
      if (isHttpUrl(href)) {
        window.open(href, '_blank', 'noopener,noreferrer');
      }
    }

    function handleSubmit(event: SubmitEvent) {
      event.preventDefault();
    }

    frameDocument.addEventListener('click', handleClick);
    frameDocument.addEventListener('submit', handleSubmit);
    measure();

    let resizeObserver: ResizeObserver | null = null;
    if (typeof ResizeObserver !== 'undefined') {
      resizeObserver = new ResizeObserver(measure);
      if (frameDocument.documentElement) {
        resizeObserver.observe(frameDocument.documentElement);
      }
      if (frameDocument.body) {
        resizeObserver.observe(frameDocument.body);
      }
    }

    return () => {
      frameDocument.removeEventListener('click', handleClick);
      frameDocument.removeEventListener('submit', handleSubmit);
      resizeObserver?.disconnect();
    };
  }, [html, onHeightChange]);

  useEffect(() => {
    const iframe = iframeRef.current;
    if (!iframe) {
      return undefined;
    }

    let cleanup = configureFrame();
    function handleLoad() {
      cleanup?.();
      cleanup = configureFrame();
    }

    iframe.addEventListener('load', handleLoad);
    return () => {
      iframe.removeEventListener('load', handleLoad);
      cleanup?.();
    };
  }, [configureFrame]);

  return (
    <iframe
      ref={iframeRef}
      title={title}
      className={cn('block w-full overflow-hidden border-0 bg-transparent', className)}
      sandbox={EMAIL_FRAME_SANDBOX}
      srcDoc={srcDoc}
      scrolling="no"
    />
  );
}
