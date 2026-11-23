import { useEffect } from "react";

interface ShortcutModalProps {
  onClose: () => void;
}

const SHORTCUTS: { key: string; action: string }[] = [
  { key: "A",           action: "Add download" },
  { key: "/",           action: "Focus search" },
  { key: "↑ / ↓",      action: "Navigate downloads" },
  { key: "Enter",       action: "Open detail panel" },
  { key: "Del / ⌫",    action: "Delete selected" },
  { key: "Escape",      action: "Close panel / clear selection" },
  { key: "?",           action: "Show shortcuts" },
];

export function ShortcutModal({ onClose }: ShortcutModalProps) {
  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onClose]);

  return (
    <div
      className="dialog-backdrop"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-label="Keyboard shortcuts"
    >
      <div
        className="dialog shortcut-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="dialog-header">
          <h2>Keyboard shortcuts</h2>
          <button
            className="icon-button"
            aria-label="Close shortcuts"
            onClick={onClose}
          >
            ×
          </button>
        </div>
        <div className="dialog-body">
          <div className="shortcut-grid">
            {SHORTCUTS.map(({ key, action }) => (
              <div key={key} className="shortcut-row">
                <kbd className="shortcut-key">{key}</kbd>
                <span className="shortcut-desc">{action}</span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
