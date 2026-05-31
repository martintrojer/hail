import { HailApiError } from '../api/client';

function errorCode(error: HailApiError): string {
  const body = error.body;
  return body && typeof body === 'object' && 'error' in body && typeof body.error === 'string'
    ? body.error
    : '';
}

export function viewErrorMessage(error: Error, context?: string): string {
  if (error instanceof HailApiError) {
    if (context === 'Search' && (error.status === 400 || error.status === 422)) {
      return 'Search terms must be at least 2 characters.';
    }
    if (error.status === 401) {
      switch (context) {
        case 'Mail view':
          return 'Your session expired. Sign in again to refresh this view.';
        case 'Trash':
          return 'Your session expired. Sign in again to refresh Trash.';
        case 'Archive':
          return 'Your session expired. Sign in again to refresh Archive.';
        case 'Screener':
          return 'Your session expired. Sign in again to refresh the Screener.';
        case 'Drafts':
          return 'Your session expired. Sign in again to refresh drafts.';
        case 'Search':
          return 'Your session expired. Sign in again to search.';
        default:
          return 'Your session expired. Sign in again.';
      }
    }
    return `${context || 'Request'} failed with HTTP ${error.status}.`;
  }

  switch (context) {
    case 'Mail view':
      return 'Mail view failed to load. Refresh and try again.';
    case 'Trash':
      return 'Trash failed to load. Refresh and try again.';
    case 'Archive':
      return 'Archive failed to load. Refresh and try again.';
    case 'Screener':
      return 'Screener failed to load. Refresh and try again.';
    case 'Drafts':
      return 'Drafts failed to load. Refresh and try again.';
    case 'Search':
      return 'Search failed. Refresh and try again.';
    default:
      return `${context || 'Request'} failed. Try again.`;
  }
}

export function actionErrorMessage(error: Error, context = 'Action'): string {
  if (error instanceof HailApiError) {
    if (context === 'Decision') {
      if (error.status === 400 || error.status === 422) {
        return 'The server rejected this decision. Refresh and try again.';
      }
      if (error.status === 401) {
        return 'Your session expired. Sign in again before deciding.';
      }
    }
    return `${context} failed with HTTP ${error.status}.`;
  }
  return `${context} failed. Try again.`;
}

export function formErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof HailApiError) {
    const detail = apiErrorDetail(error);
    if (detail) {
      return detail;
    }
    if (error.status === 401) {
      return 'Email or password was not accepted.';
    }
    if (error.status === 409) {
      return 'Setup is no longer active. Try signing in instead.';
    }
    if (error.status === 422 || error.status === 400) {
      return 'Check the form values and try again.';
    }
  }

  return fallback;
}

function apiErrorDetail(error: HailApiError): string | null {
  const body = error.body;
  return body && typeof body === 'object' && 'detail' in body && typeof body.detail === 'string'
    ? body.detail
    : null;
}

export function composeErrorMessage(error: unknown, fallback: string): string {
  if (!(error instanceof HailApiError)) return fallback;
  if (error.status === 401) return 'Your session expired. Sign in again before sending.';
  if (error.status !== 400) return `${fallback} HTTP ${error.status}.`;

  const code = errorCode(error);
  if (code === 'attachments_not_supported') {
    return 'Attachments are selected, but this server does not support sending attachments yet. Remove them and try again.';
  }
  if (code === 'invalid_send_at') return 'Choose a future send-later time.';
  if (code.includes('recipient') || code.includes('to')) return 'Check recipient addresses and try again.';
  if (code.includes('subject')) return 'Check the subject and try again.';
  if (code.includes('body')) return 'Write a message body and try again.';
  return 'Check the compose fields and try again.';
}

export function threadErrorMessage(error: Error): string {
  if (error instanceof HailApiError) {
    if (error.status === 401) {
      return 'Your session expired. Sign in again to open this thread.';
    }
    if (error.status === 404) {
      return 'This thread was not found. It may have moved or been deleted.';
    }
    if (error.status === 400 || error.status === 422) {
      return 'This thread link is invalid.';
    }
    return `Thread failed with HTTP ${error.status}.`;
  }

  return 'Thread failed to load. Refresh and try again.';
}

export function contactErrorMessage(error: Error): string {
  if (error instanceof HailApiError) {
    if (error.status === 401) {
      return 'Your session expired. Sign in again to view this note.';
    }
    if (error.status === 404) {
      return 'This contact was not found yet.';
    }
    if (error.status === 400 || error.status === 422) {
      return 'Contact note validation failed. Refresh and try again.';
    }
    return `Contact note failed with HTTP ${error.status}.`;
  }

  return 'Contact note failed to load. Refresh and try again.';
}

export function contactNoteMutationErrorMessage(error: Error): string {
  if (error instanceof HailApiError) {
    if (error.status === 400 || error.status === 422) {
      return 'The server rejected this note. Check the markdown and try again.';
    }
    if (error.status === 401) {
      return 'Your session expired. Sign in again before changing this note.';
    }
    return `Contact note update failed with HTTP ${error.status}.`;
  }

  return 'Contact note update failed. Try again.';
}

export function adminErrorMessage(error: Error, action: string): string {
  if (error instanceof HailApiError) {
    if (error.status === 400 || error.status === 422) {
      return `Check the ${action} values and try again.`;
    }
    if (error.status === 401) {
      return 'Your session expired. Sign in again.';
    }
    if (error.status === 403) {
      return 'Admin access is required.';
    }
    if (error.status === 404) {
      return 'That item no longer exists. Refresh and try again.';
    }
    if (error.status === 501) {
      return 'Stalwart management is not configured for this instance.';
    }
    if (error.status === 502) {
      return 'Stalwart management failed. Try again or check the server logs.';
    }
    return `${action} failed with HTTP ${error.status}.`;
  }

  return `${action} failed. Try again.`;
}
