import { useEffect, useRef, useState } from "react";
import type { Download, DownloadPriority } from "../api/types";
import {
  canCancel,
  canChangePriority,
} from "../lib/status";

interface ContextMenuProps {
  x: number;
  y: number;
  download: Download;
  onClose: () => void;
  onCancel: (id: string) => void;
  onDelete: (id: string) => void;
  onCopyUrl: (download: Download) => void;
  onShareDownload?: (download: Download) => void;
  onOpenFolder: (id: string) => void;
  onCopyAsCurl?: (download: Download) => void;
  onPriorityChange: (id: string, priority: DownloadPriority) => void;
  onQueueMove: (id: string, direction: "up" | "down") => void;
  onSetBandwidthLimit: (id: string) => void;
  onUpdateUrl: (id: string, newUrl: string) => Promise<void>;
  onError: (message: string) => void;
}

export function ContextMenu({
  x,
  y,
  download,
  onClose,
  onCancel,
  onDelete,
  onCopyUrl,
  onShareDownload,
  onOpenFolder,
  onCopyAsCurl,
  onPriorityChange,
  onQueueMove,
  onSetBandwidthLimit,
  onUpdateUrl,
  onError,
}: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const isLegacyAria2 = download.engine === "aria2";
  const [urlEditing, setUrlEditing] = useState(false);
  const [urlDraft, setUrlDraft] = useState(download.url);
  const [urlSaving, setUrlSaving] = useState(false);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    const handleMouseDown = (event: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
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

  useEffect(() => {
    if (urlEditing) {
      menuRef.current
        ?.querySelector<HTMLInputElement>(".context-menu-url-editor input")
        ?.focus();
      return;
    }

    menuRef.current
      ?.querySelector<HTMLElement>('[role="menuitem"], [role="menuitemradio"]')
      ?.focus();
  }, [urlEditing]);

  const MENU_HEIGHT = 440;
  const MENU_WIDTH = 240;
  const adjustedY =
    y + MENU_HEIGHT > window.innerHeight
      ? Math.max(8, window.innerHeight - MENU_HEIGHT - 8)
      : y;
  const adjustedX =
    x + MENU_WIDTH > window.innerWidth
      ? Math.max(8, window.innerWidth - MENU_WIDTH - 8)
      : x;

  const handleItem = (action: () => void) => {
    action();
    onClose();
  };

  const submitUrl = async () => {
    const newUrl = urlDraft.trim();
    if (!newUrl) {
      onError("Paste a fresh download URL.");
      return;
    }

    setUrlSaving(true);
    try {
      await onUpdateUrl(download.id, newUrl);
      onClose();
    } catch (cause) {
      onError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setUrlSaving(false);
    }
  };

  const canUpdateUrl =
    !isLegacyAria2 &&
    ["failed", "cancelled", "paused", "queued"].includes(download.status);
  const canSetBandwidthLimit =
    !isLegacyAria2 &&
    ["downloading", "queued", "paused"].includes(download.status);
  const hasTaskAction = canCancel(download.status);

  return (
    <div
      ref={menuRef}
      className="context-menu"
      style={{ left: adjustedX, top: adjustedY }}
      role="menu"
      aria-label={`Actions for ${download.filename}`}
      onKeyDown={(event) => {
        if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
        if (
          !(event.target instanceof HTMLElement) ||
          !event.target.closest('[role="menuitem"], [role="menuitemradio"]')
        ) {
          return;
        }
        const items = Array.from(
          menuRef.current?.querySelectorAll<HTMLElement>(
            '[role="menuitem"], [role="menuitemradio"]',
          ) ?? [],
        ).filter((item) => !item.hasAttribute("disabled"));
        if (items.length === 0) return;

        event.preventDefault();
        const currentIndex = items.indexOf(document.activeElement as HTMLElement);
        const nextIndex =
          event.key === "Home"
            ? 0
            : event.key === "End"
              ? items.length - 1
              : (currentIndex + (event.key === "ArrowDown" ? 1 : -1) + items.length) %
                items.length;
        items[nextIndex]?.focus();
      }}
    >
      {!isLegacyAria2 && download.status === "queued" && (
        <>
          <button
            className="context-menu-item"
            role="menuitem"
            onClick={() => handleItem(() => onQueueMove(download.id, "up"))}
          >
            Move earlier in queue
          </button>
          <button
            className="context-menu-item"
            role="menuitem"
            onClick={() => handleItem(() => onQueueMove(download.id, "down"))}
          >
            Move later in queue
          </button>
          <div className="context-menu-separator" role="separator" />
        </>
      )}

      {!isLegacyAria2 && canChangePriority(download.status) && (
        <div
          className="context-menu-priority"
          role="group"
          aria-label="Download priority"
        >
          <span className="context-menu-heading">Priority</span>
          {(["low", "normal", "high"] as const).map(
            (priority: DownloadPriority) => (
              <button
                key={priority}
                className="context-menu-item"
                role="menuitemradio"
                aria-checked={download.priority === priority}
                onClick={() =>
                  handleItem(() => onPriorityChange(download.id, priority))
                }
              >
                <span>{priority[0].toUpperCase() + priority.slice(1)}</span>
                {download.priority === priority && (
                  <span aria-hidden="true">✓</span>
                )}
              </button>
            ),
          )}
          <div className="context-menu-separator" role="separator" />
        </div>
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

      {hasTaskAction && <div className="context-menu-separator" role="separator" />}

      {canUpdateUrl &&
        (urlEditing ? (
          <div className="context-menu-url-editor">
            <input
              type="text"
              value={urlDraft}
              aria-label="Refreshed download URL"
              placeholder="Paste refreshed URL"
              disabled={urlSaving}
              onChange={(event) => setUrlDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") void submitUrl();
                if (event.key === "Escape") setUrlEditing(false);
              }}
            />
            <p>
              Saved bytes are reused only when the server confirms the same file.
            </p>
            <button
              className="context-menu-item"
              role="menuitem"
              disabled={urlSaving}
              onClick={() => void submitUrl()}
            >
              {urlSaving ? "Updating..." : "Replace URL and retry"}
            </button>
            <button
              className="context-menu-item"
              role="menuitem"
              disabled={urlSaving}
              onClick={() => setUrlEditing(false)}
            >
              Back
            </button>
          </div>
        ) : (
          <button
            className="context-menu-item"
            role="menuitem"
            onClick={() => setUrlEditing(true)}
          >
            {download.status === "failed" || download.status === "cancelled"
              ? "Refresh link"
              : "Update URL"}
          </button>
        ))}

      {canSetBandwidthLimit && (
        <button
          className="context-menu-item"
          role="menuitem"
          onClick={() => handleItem(() => onSetBandwidthLimit(download.id))}
        >
          {download.bandwidth_limit
            ? `Set speed limit · ${Math.round(download.bandwidth_limit / 1024)} KiB/s`
            : "Set speed limit"}
        </button>
      )}

      <button
        className="context-menu-item"
        role="menuitem"
        onClick={() => handleItem(() => onCopyUrl(download))}
      >
        Copy URL
      </button>

      {onCopyAsCurl && (
        <button
          className="context-menu-item"
          role="menuitem"
          onClick={() => handleItem(() => onCopyAsCurl(download))}
        >
          Copy as cURL
        </button>
      )}

      {onShareDownload && (
        <button
          className="context-menu-item"
          role="menuitem"
          onClick={() => handleItem(() => onShareDownload(download))}
        >
          Share link
        </button>
      )}

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
