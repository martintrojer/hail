import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { defaultApiClient } from '../api/query';
import { queryClient } from '../lib/queryClient';

interface UndoToastAction {
  id: string;
  label?: string;
}

interface ToastInput {
  message: string;
  undo?: UndoToastAction | null;
  undoSuccessMessage?: string;
  undoFailureMessage?: string;
  durationMs?: number;
}

interface ToastState extends ToastInput {
  toastId: number;
  undoing: boolean;
}

interface UndoToastContextValue {
  showToast: (toast: ToastInput) => void;
}

const UndoToastContext = createContext<UndoToastContextValue | null>(null);
const DEFAULT_DURATION_MS = 6000;

export function UndoToastProvider({ children }: { children: ReactNode }) {
  const [toast, setToast] = useState<ToastState | null>(null);
  const nextId = useRef(1);

  const dismiss = useCallback(() => setToast(null), []);

  const showToast = useCallback((input: ToastInput) => {
    setToast({
      ...input,
      toastId: nextId.current,
      undoing: false,
    });
    nextId.current += 1;
  }, []);

  useEffect(() => {
    if (!toast) {
      return undefined;
    }

    const timeout = window.setTimeout(() => {
      setToast((current) =>
        current?.toastId === toast.toastId ? null : current,
      );
    }, toast.durationMs ?? DEFAULT_DURATION_MS);

    return () => window.clearTimeout(timeout);
  }, [toast]);

  const value = useMemo(() => ({ showToast }), [showToast]);

  async function undoCurrent() {
    if (!toast?.undo || toast.undoing) {
      return;
    }

    const toastId = toast.toastId;
    setToast((current) =>
      current?.toastId === toastId ? { ...current, undoing: true } : current,
    );

    try {
      await defaultApiClient.undo(toast.undo.id);
      await queryClient.invalidateQueries({ queryKey: ['hail'] });
      setToast((current) =>
        current?.toastId === toastId
          ? {
              message: toast.undoSuccessMessage ?? 'Undone.',
              toastId,
              undoing: false,
              durationMs: 3000,
            }
          : current,
      );
    } catch {
      setToast((current) =>
        current?.toastId === toastId
          ? {
              message:
                toast.undoFailureMessage ??
                'Undo failed. Refresh and try again.',
              toastId,
              undoing: false,
              durationMs: 5000,
            }
          : current,
      );
    }
  }

  return (
    <UndoToastContext.Provider value={value}>
      {children}
      {toast ? (
        <div className="fixed inset-x-0 bottom-4 z-50 flex justify-center px-4 pointer-events-none">
          <div
            role="status"
            aria-live="polite"
            className="pointer-events-auto flex w-full max-w-md items-center justify-between gap-3 rounded-2xl border border-slate-700 bg-slate-900/95 px-4 py-3 text-sm text-slate-100 shadow-2xl shadow-slate-950/70 backdrop-blur"
          >
            <span className="min-w-0 flex-1">{toast.message}</span>
            {toast.undo ? (
              <button
                type="button"
                onClick={() => void undoCurrent()}
                disabled={toast.undoing}
                className="shrink-0 rounded-lg bg-sky-400 px-3 py-1.5 text-xs font-semibold text-slate-950 transition hover:bg-sky-300 disabled:cursor-not-allowed disabled:opacity-60"
              >
                {toast.undoing ? 'Undoing…' : (toast.undo.label ?? 'Undo')}
              </button>
            ) : null}
            <button
              type="button"
              onClick={dismiss}
              className="shrink-0 rounded-lg border border-slate-700 px-2 py-1 text-xs font-semibold text-slate-300 transition hover:border-slate-500 hover:text-slate-100"
              aria-label="Dismiss notification"
            >
              Close
            </button>
          </div>
        </div>
      ) : null}
    </UndoToastContext.Provider>
  );
}

export function useUndoToast() {
  const context = useContext(UndoToastContext);
  if (!context) {
    throw new Error('useUndoToast must be used inside UndoToastProvider');
  }
  return context;
}
