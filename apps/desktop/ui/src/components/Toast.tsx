import { useCallback, useEffect, useRef, useState } from "react";

export interface ToastMessage {
  id: number;
  text: string;
  kind: "success" | "info" | "error";
}

interface ToastProps {
  toasts: ToastMessage[];
  onDismiss: (id: number) => void;
}

const AUTO_DISMISS_MS = 4000;

/**
 * Renders a stack of transient toast notifications.
 * Each toast auto-dismisses after AUTO_DISMISS_MS.
 */
export function ToastStack({ toasts, onDismiss }: ToastProps) {
  return (
    <div className="toast-stack" aria-live="polite" aria-atomic="false">
      {toasts.map((toast) => (
        <ToastItem key={toast.id} toast={toast} onDismiss={onDismiss} />
      ))}
    </div>
  );
}

function ToastItem({ toast, onDismiss }: { toast: ToastMessage; onDismiss: (id: number) => void }) {
  const [visible, setVisible] = useState(true);

  useEffect(() => {
    const timer = setTimeout(() => {
      setVisible(false);
      setTimeout(() => onDismiss(toast.id), 300); // allow fade-out animation
    }, AUTO_DISMISS_MS);
    return () => clearTimeout(timer);
  }, [toast.id, onDismiss]);

  return (
    <div
      className={`toast toast-${toast.kind}${visible ? "" : " toast-hiding"}`}
      role="status"
      aria-label={toast.text}
    >
      <span>{toast.text}</span>
      <button
        className="icon-button toast-close"
        aria-label="Dismiss notification"
        onClick={() => onDismiss(toast.id)}
      >
        ×
      </button>
    </div>
  );
}

/** Hook to manage toast state. Returns toasts array + an `addToast` function. */
export function useToasts() {
  const [toasts, setToasts] = useState<ToastMessage[]>([]);
  const nextId = useRef(0);

  const addToast = useCallback((text: string, kind: ToastMessage["kind"] = "info") => {
    const id = ++nextId.current;
    setToasts((current) => {
      // Deduplicate: skip if the most-recent toast has the same text.
      if (current.length > 0 && current[current.length - 1].text === text) return current;
      // Cap at 5 toasts: drop the oldest when already at the limit.
      const capped = current.length >= 5 ? current.slice(1) : current;
      return [...capped, { id, text, kind }];
    });
  }, []);

  const dismiss = useCallback((id: number) => {
    setToasts((current) => current.filter((t) => t.id !== id));
  }, []);

  return { toasts, addToast, dismiss };
}
