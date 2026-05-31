import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { ThreadAttachment } from '../api/client';
import { AttachmentList } from './AttachmentList';

function attachment(index: number): ThreadAttachment {
  return {
    filename: `file-${index}.pdf`,
    size: index * 1024,
    mime_type: 'application/pdf',
    blob_id: `blob-${index}`,
    download_url: `/api/attachments/blob-${index}/download`,
    inline: false,
  };
}

describe('AttachmentList', () => {
  afterEach(() => {
    cleanup();
  });

  it('renders non-inline attachment rows with download links', () => {
    render(
      <AttachmentList
        items={[
          attachment(1),
          {
            ...attachment(2),
            filename: 'inline-logo.png',
            inline: true,
          },
        ]}
      />,
    );

    expect(screen.getByText('file-1.pdf')).toBeInTheDocument();
    expect(screen.getByText(/1 KB.*application\/pdf/)).toBeInTheDocument();
    expect(screen.queryByText('inline-logo.png')).not.toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Download' })).toHaveAttribute(
      'href',
      '/api/attachments/blob-1/download',
    );
    expect(screen.getByRole('link', { name: 'Download' })).toHaveAttribute(
      'download',
      'file-1.pdf',
    );
  });

  it('collapses more than five attachments until expanded', () => {
    render(<AttachmentList items={[1, 2, 3, 4, 5, 6, 7].map(attachment)} />);

    expect(screen.getByText('file-1.pdf')).toBeInTheDocument();
    expect(screen.getByText('file-5.pdf')).toBeInTheDocument();
    expect(screen.queryByText('file-6.pdf')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Show 2 more' }));

    expect(screen.getByText('file-6.pdf')).toBeInTheDocument();
    expect(screen.getByText('file-7.pdf')).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Show 2 more' }),
    ).not.toBeInTheDocument();
  });
});
