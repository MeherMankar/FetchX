import { useEffect, useRef } from "react";
import type { Download } from "../api/types";
import { canCancel, canPause, canResume, canRetry } from "../lib/status";

interface ContextMenuProps {
  x: number;
  y: number;
  download: Download;
  onClose: () => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onCancel: (id: string) => void;
  onDelete: (id: string) => void;
  onRetry: (id: string) => void;
  onCopyUrl: (download: Download) => void;
  onOpenFolder: (id: string) => void;
}

export function ContextMenu({
  x,
  y,
  download,
  onClose,
  onPause,
  onResume,
  onCancel,
  onDelete,
  onRetry,
  onCopyUrl,
  onOpenFolder,
}: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const isLegacyAria2 = download.engine === "aria2";

  // Close on click outside or Escape
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const handleMouseDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    document.addEventListener("mousedown", handleMouseDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      document.removeEventListener("mousedown", handleMouseDown);
    };
  }, [onClose]);

  // Position to stay in viewport (flip up if too close to bottom)
  const MENU_HEIGHT = 280;
  const MENU_WIDTH = 200;
  const adjustedY = y + MENU_HEIGHT > window.innerHeight ? y - MENU_HEIGHT : y;
  const adjustedX = x + MENU_WIDTH > window.innerWidth ? x - MENU_WIDTH : x;

  const handleItem = (action: () => void) => {
    action();
    onClose();
  };

  return (
    <div
      ref={menuRef}
      className="context-menu"
      style={{ left: adjustedX, top: adjustedY }}
      role="menu"
      aria-label={`Actions for ${download.filename}`}
    >
      {!isLegacyAria2 && canPause(download.status) && (
        <button
          className="context-menu-item"
          role="menuitem"
          onClick={() => handleItem(() => onPause(download.id))}
        >
          Pause
        </button>
      )}
      {!isLegacyAria2 && canResume(download.status) && (
        <button
          className="context-menu-item"
          role="menuitem"
          onClick={() => handleItem(() => onResume(download.id))}
        >
          Resume
        </button>
      )}
      {!isLegacyAria2 && canRetry(download.status) && (
        <button
          className="context-menu-item"
          role="menuitem"
          onClick={() => handleItem(() => onRetry(download.id))}
        >
          Retry
        </button>
      )}
      {!isLegacyAria2 && canCancel(download.status) && (
        <button
          className="context-menu-item"
          role="menuitem"
          onClick={() => handleItem(() => onCancel(download.id))}
        >
          Cancel
        </button>
      )}

      {(canPause(download.status) || canResume(download.status) || canRetry(download.status) || canCancel(download.status)) && (
        <div className="context-menu-separator" role="separator" />
      )}

      <button
        className="context-menu-item"
        role="menuitem"
        onClick={() => handleItem(() => onCopyUrl(download))}
      >
        Copy URL
      </button>

      {download.status === "completed" && (
        <button
          className="context-menu-item"
          role="menuitem"
          onClick={() => handleItem(() => onOpenFolder(download.id))}
        >
          Open folder
        </button>
      )}

      <div className="context-menu-separator" role="separator" />

      <button
        className="context-menu-item context-menu-item--danger"
        role="menuitem"
        onClick={() => handleItem(() => onDelete(download.id))}
      >
        Delete
      </button>
    </div>
  );
}
