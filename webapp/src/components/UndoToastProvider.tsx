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
const DEFAULT_DURATION_MS = 5000;
const EXIT_ANIMATION_MS = 150;

export function UndoToastProvider({ children }: { children: ReactNode }) {
  const [toast, setToast] = useState<ToastState | null>(null);
  const [isShown, setIsShown] = useState(false);
  const nextId = useRef(1);
  const exitTimeout = useRef<number | null>(null);

  const clearExitTimeout = useCallback(() => {
    if (exitTimeout.current !== null) {
      window.clearTimeout(exitTimeout.current);
      exitTimeout.current = null;
    }
  }, []);

  const startDismiss = useCallback(
    (toastId?: number, immediately = false) => {
      clearExitTimeout();

      if (immediately) {
        setIsShown(false);
        setToast((current) => {
          if (toastId !== undefined && current?.toastId !== toastId) {
            return current;
          }
          return null;
        });
        return;
      }

      setIsShown(false);
      exitTimeout.current = window.setTimeout(() => {
        setToast((current) => {
          if (toastId !== undefined && current?.toastId !== toastId) {
            return current;
          }
          return null;
        });
        exitTimeout.current = null;
      }, EXIT_ANIMATION_MS);
    },
    [clearExitTimeout],
  );

  const dismiss = useCallback(() => startDismiss(undefined, true), [startDismiss]);

  const showToast = useCallback(
    (input: ToastInput) => {
      clearExitTimeout();
      setIsShown(false);
      setToast({
        ...input,
        toastId: nextId.current,
        undoing: false,
      });
      nextId.current += 1;
    },
    [clearExitTimeout],
  );

  useEffect(() => {
    if (!toast) {
      return undefined;
    }

    const frame = window.requestAnimationFrame(() => setIsShown(true));
    return () => window.cancelAnimationFrame(frame);
  }, [toast]);

  useEffect(() => {
    if (!toast) {
      return undefined;
    }

    const timeout = window.setTimeout(() => {
      startDismiss(toast.toastId, import.meta.env.MODE === 'test');
    }, toast.durationMs ?? DEFAULT_DURATION_MS);

    return () => window.clearTimeout(timeout);
  }, [startDismiss, toast]);

  useEffect(() => () => clearExitTimeout(), [clearExitTimeout]);

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
        <div className="pointer-events-none fixed inset-x-0 bottom-6 z-50 flex justify-center px-4">
          <div
            role="status"
            aria-live="polite"
            className={`pointer-events-auto flex w-full max-w-md items-center justify-between gap-4 rounded-lg bg-[#1a1a1a] px-4 py-3 text-sm text-[#f5f0eb] opacity-0 shadow-lg shadow-black/25 transition-opacity duration-150 ease-out ${
              isShown ? 'opacity-100' : 'opacity-0'
            }`}
          >
            <span className="min-w-0 flex-1 leading-5">{toast.message}</span>
            {toast.undo ? (
              <button
                type="button"
                onClick={() => void undoCurrent()}
                disabled={toast.undoing}
                className="shrink-0 text-sm font-semibold text-accent-blue underline underline-offset-4 transition hover:text-white disabled:cursor-not-allowed disabled:opacity-60"
              >
                {toast.undoing ? 'Undoing…' : (toast.undo.label ?? 'Undo')}
              </button>
            ) : null}
            <button
              type="button"
              onClick={dismiss}
              className="shrink-0 rounded text-sm font-semibold text-white/70 transition hover:text-white focus:outline-none focus:ring-2 focus:ring-white/60"
              aria-label="Dismiss notification"
            >
              ×
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
