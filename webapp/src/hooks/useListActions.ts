import { useApiClient } from '../api/ApiClientProvider';
import type { HailApiClient, MailClassification, UndoableResponse } from '../api/client';
import {
  useArchiveThreadMutation,
  useClassifyThreadMutation,
  useDestroyThreadMutation,
  useReplyLaterThreadMutation,
  useRestoreThreadMutation,
  useSetAsideThreadMutation,
  useTrashThreadMutation,
  useDeleteDraftMutation,
} from '../api/query';
import { useUndoToast } from '../components/UndoToastProvider';

export type ListAction =
  | 'archive'
  | 'trash'
  | 'set-aside'
  | 'reply-later'
  | 'classify'
  | 'restore'
  | 'delete'
  | 'delete-forever';

export interface ListActionConfig {
  client?: HailApiClient;
  availableActions: ListAction[];
  /** Target for the generic classify action. Defaults to Imbox. */
  classifyTo?: Extract<MailClassification, 'imbox' | 'feed' | 'papertrail'>;
  /** Archive restores by classifying to Imbox; Trash restores through the restore endpoint. */
  restoreMode?: 'classify-imbox' | 'restore-endpoint';
}

export interface ListActionHandlerOptions {
  toast?: boolean;
}

const singularMessages: Record<ListAction, { message: string; undoSuccessMessage?: string }> = {
  archive: { message: 'Thread archived.', undoSuccessMessage: 'Archive undone.' },
  trash: { message: 'Thread moved to trash.', undoSuccessMessage: 'Trash undone.' },
  'set-aside': { message: 'Thread added to Set Aside.', undoSuccessMessage: 'Set Aside undone.' },
  'reply-later': { message: 'Thread added to Reply Later.', undoSuccessMessage: 'Reply Later undone.' },
  classify: { message: 'Thread moved to Imbox.', undoSuccessMessage: 'Thread classification undone.' },
  restore: { message: 'Thread restored to Imbox.', undoSuccessMessage: 'Restore undone.' },
  delete: { message: 'Draft deleted.' },
  'delete-forever': { message: 'Thread deleted forever.' },
};

const batchMessages: Record<ListAction, (count: number) => string> = {
  archive: (count) => `${count} thread${count === 1 ? '' : 's'} archived.`,
  trash: (count) => `${count} thread${count === 1 ? '' : 's'} moved to trash.`,
  'set-aside': (count) => `${count} thread${count === 1 ? '' : 's'} added to Set Aside.`,
  'reply-later': (count) => `${count} thread${count === 1 ? '' : 's'} added to Reply Later.`,
  classify: (count) => `${count} thread${count === 1 ? '' : 's'} moved to Imbox.`,
  restore: (count) => `${count} thread${count === 1 ? '' : 's'} restored to Imbox.`,
  delete: (count) => `${count} draft${count === 1 ? '' : 's'} deleted.`,
  'delete-forever': (count) => `${count} thread${count === 1 ? '' : 's'} deleted forever.`,
};

function undoFrom(data: unknown) {
  const undoable = data as UndoableResponse | undefined;
  return undoable?.undo ? { id: undoable.undo.id } : null;
}

export function useListActions(config: ListActionConfig) {
  const contextClient = useApiClient();
  const client = config.client ?? contextClient;
  const undoToast = useUndoToast();
  const archiveMutation = useArchiveThreadMutation(client);
  const trashMutation = useTrashThreadMutation(client);
  const setAsideMutation = useSetAsideThreadMutation(client);
  const replyLaterMutation = useReplyLaterThreadMutation(client);
  const classifyMutation = useClassifyThreadMutation(client);
  const restoreMutation = useRestoreThreadMutation(client);
  const destroyMutation = useDestroyThreadMutation(client);
  const deleteDraftMutation = useDeleteDraftMutation(client);

  const classifyTo = config.classifyTo ?? 'imbox';
  const restoreMode = config.restoreMode ?? 'classify-imbox';

  async function run(action: ListAction, threadId: string, options: ListActionHandlerOptions = {}) {
    const shouldToast = options.toast ?? true;
    let data: unknown;

    if (action === 'archive') {
      data = await archiveMutation.mutateAsync({ threadId });
    } else if (action === 'trash') {
      data = await trashMutation.mutateAsync({ threadId });
    } else if (action === 'set-aside') {
      data = await setAsideMutation.mutateAsync({ threadId });
    } else if (action === 'reply-later') {
      data = await replyLaterMutation.mutateAsync({ threadId });
    } else if (action === 'classify') {
      data = await classifyMutation.mutateAsync({ threadId, to: classifyTo });
    } else if (action === 'restore') {
      data = restoreMode === 'restore-endpoint'
        ? await restoreMutation.mutateAsync({ threadId })
        : await classifyMutation.mutateAsync({ threadId, to: 'imbox' });
    } else if (action === 'delete') {
      data = await deleteDraftMutation.mutateAsync(threadId);
    } else {
      data = await destroyMutation.mutateAsync({ threadId });
    }

    if (shouldToast) {
      const message = singularMessages[action];
      undoToast.showToast({
        message: message.message,
        undo: undoFrom(data),
        undoSuccessMessage: message.undoSuccessMessage,
      });
    }

    return data;
  }

  async function runBatch(action: ListAction, threadIds: string[]) {
    await Promise.all(threadIds.map((threadId) => run(action, threadId, { toast: false })));
    undoToast.showToast({ message: batchMessages[action](threadIds.length), undo: null });
  }

  return {
    availableActions: config.availableActions,
    archive: (threadId: string, options?: ListActionHandlerOptions) => run('archive', threadId, options),
    trash: (threadId: string, options?: ListActionHandlerOptions) => run('trash', threadId, options),
    setAside: (threadId: string, options?: ListActionHandlerOptions) => run('set-aside', threadId, options),
    replyLater: (threadId: string, options?: ListActionHandlerOptions) => run('reply-later', threadId, options),
    classify: (threadId: string, options?: ListActionHandlerOptions) => run('classify', threadId, options),
    restore: (threadId: string, options?: ListActionHandlerOptions) => run('restore', threadId, options),
    delete: (threadId: string, options?: ListActionHandlerOptions) => run('delete', threadId, options),
    deleteForever: (threadId: string, options?: ListActionHandlerOptions) => run('delete-forever', threadId, options),
    run,
    runBatch,
    isBusy:
      archiveMutation.isPending ||
      trashMutation.isPending ||
      setAsideMutation.isPending ||
      replyLaterMutation.isPending ||
      classifyMutation.isPending ||
      restoreMutation.isPending ||
      destroyMutation.isPending ||
      deleteDraftMutation.isPending,
    error:
      archiveMutation.error ??
      trashMutation.error ??
      setAsideMutation.error ??
      replyLaterMutation.error ??
      classifyMutation.error ??
      restoreMutation.error ??
      destroyMutation.error ??
      deleteDraftMutation.error,
  };
}
