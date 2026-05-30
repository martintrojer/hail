import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { EmailFrame } from './EmailFrame';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('EmailFrame', () => {
  it('renders heavy sanitized email HTML inside a scriptless sandboxed iframe', async () => {
    const html = [
      '<table width="640" style="width:640px;background:#fff">',
      '<tbody><tr><td style="padding:24px;color:#123456">',
      '<h1 style="font-size:32px">Marketing layout</h1>',
      '<a href="https://example.com/deal" style="color:#ff00aa">Read deal</a>',
      '</td></tr></tbody></table>',
    ].join('');

    render(<EmailFrame html={html} />);

    const iframe = screen.getByTitle('Email body') as HTMLIFrameElement;
    expect(iframe).toHaveAttribute(
      'sandbox',
      'allow-same-origin allow-popups allow-popups-to-escape-sandbox',
    );
    expect(iframe).not.toHaveAttribute('sandbox', expect.stringContaining('allow-scripts'));

    await waitFor(() => {
      const body = iframe.contentDocument?.body;
      expect(body?.querySelector('table[width="640"]')).toBeInTheDocument();
      expect(body?.querySelector('h1')).toHaveTextContent('Marketing layout');
      expect(body?.querySelector('a')).toHaveAttribute('href', 'https://example.com/deal');
    });
  });

  it('keeps remote-image-blocked variants image-free until callers pass remote image html', async () => {
    const { rerender } = render(<EmailFrame html="<p>Logo</p>" />);
    const iframe = screen.getByTitle('Email body') as HTMLIFrameElement;

    await waitFor(() => {
      expect(iframe.contentDocument?.querySelector('img')).toBeNull();
    });

    rerender(
      <EmailFrame html='<p>Logo</p><img src="https://cdn.example/logo.png" alt="Logo">' />,
    );

    await waitFor(() => {
      expect(iframe.contentDocument?.querySelector('img')).toHaveAttribute(
        'src',
        'https://cdn.example/logo.png',
      );
    });
  });

  it('opens email links through the parent window with noopener semantics', async () => {
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    render(<EmailFrame html='<a href="https://example.com/path">Open link</a>' />);
    const iframe = screen.getByTitle('Email body') as HTMLIFrameElement;

    await waitFor(() => {
      expect(iframe.contentDocument?.querySelector('a')).toHaveAttribute('target', '_blank');
      expect(iframe.contentDocument?.querySelector('a')).toHaveAttribute('rel', 'noopener noreferrer');
    });

    fireEvent.click(iframe.contentDocument!.querySelector('a')!);

    expect(open).toHaveBeenCalledWith(
      'https://example.com/path',
      '_blank',
      'noopener,noreferrer',
    );
  });
});
