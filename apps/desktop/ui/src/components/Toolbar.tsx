import { useEffect, useRef, useState, type RefObject } from "react";

import type { SortKey } from "../App";

interface ToolbarProps {
  title: string;
  count: number;
  search: string;
  sort: SortKey;
  searchRef: RefObject<HTMLInputElement | null>;
  onSearchChange: (value: string) => void;
  onSortChange: (sort: SortKey) => void;
  onRefresh: () => void;
  dispatchManuallyPaused: boolean;
  dispatchSchedulePaused: boolean;
  onToggleQueueDispatch: () => void;
  onAddClick: () => void;
  onBatchClick: () => void;
  activeCount: number;
  pausedCount: number;
  onPauseAll: () => void;
  onResumeAll: () => void;
  appVersion: string | undefined;
  theme: string;
  onToggleTheme: () => void;
}

export function Toolbar({
  title,
  count,
  search,
  sort,
  searchRef,
  onSearchChange,
  onSortChange,
  onRefresh,
  dispatchManuallyPaused,
  dispatchSchedulePaused,
  onToggleQueueDispatch,
  onAddClick,
  onBatchClick,
  activeCount,
  pausedCount,
  onPauseAll,
  onResumeAll,
  appVersion,
  theme,
  onToggleTheme,
}: ToolbarProps) {
  const [actionsOpen, setActionsOpen] = useState(false);
  const actionsRef = useRef<HTMLDivElement>(null);
  const actionsButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!actionsOpen) return;

    const handlePointerDown = (event: PointerEvent) => {
      if (event.target instanceof Node && !actionsRef.current?.contains(event.target)) {
        setActionsOpen(false);
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setActionsOpen(false);
        actionsButtonRef.current?.focus();
      }
    };

    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [actionsOpen]);

  const runAction = (action: () => void) => {
    setActionsOpen(false);
    actionsButtonRef.current?.focus();
    action();
  };

  return (
    <header className="toolbar">
      <div className="toolbar-left">
        <h1 className="toolbar-title">{title}</h1>
        <span className="toolbar-count" aria-label={`${count} items`}>
          {count} {count === 1 ? "item" : "items"}
        </span>
      </div>

      <div className="toolbar-controls">
        {dispatchSchedulePaused && (
          <span className="queue-schedule-status" role="status">
            Outside schedule
          </span>
        )}
        <label className="visually-hidden" htmlFor="search-input">Search downloads</label>
        <input
          id="search-input"
          ref={searchRef}
          className="search-input"
          type="search"
          placeholder="Search"
          value={search}
          onChange={(event) => onSearchChange(event.target.value)}
          aria-label="Search downloads"
        />

        <button
          className="button button-primary"
          type="button"
          onClick={onAddClick}
          aria-label="Add new download"
        >
          Add download
        </button>

        <div className="toolbar-menu" ref={actionsRef}>
          <button
            ref={actionsButtonRef}
            className="button button-secondary toolbar-menu-trigger"
            type="button"
            aria-label="More actions"
            aria-expanded={actionsOpen}
            aria-controls="toolbar-actions-menu"
            title="More actions"
            onClick={() => setActionsOpen((open) => !open)}
          >
            <svg viewBox="0 0 16 16" aria-hidden="true">
              <circle cx="3" cy="8" r="1" />
              <circle cx="8" cy="8" r="1" />
              <circle cx="13" cy="8" r="1" />
            </svg>
          </button>

          <div
            className="toolbar-menu-popover"
            id="toolbar-actions-menu"
            role="group"
            aria-label="More download actions"
            hidden={!actionsOpen}
          >
            <label className="toolbar-menu-label" htmlFor="sort-select">Sort by</label>
            <select
              id="sort-select"
              className="sort-select toolbar-menu-sort"
              value={sort}
              onChange={(event) => {
                onSortChange(event.target.value as SortKey);
                setActionsOpen(false);
                actionsButtonRef.current?.focus();
              }}
              aria-label="Sort downloads"
            >
              <option value="newest">Newest first</option>
              <option value="oldest">Oldest first</option>
              <option value="filename">Filename A–Z</option>
              <option value="progress">Progress</option>
              <option value="size">Size</option>
              <option value="speed">Speed</option>
              <option value="priority">Priority</option>
              <option value="queue">Queue position</option>
            </select>

            <div className="toolbar-menu-divider" />
            <button className="toolbar-menu-item" type="button" onClick={() => runAction(onBatchClick)}>
              Batch add downloads
            </button>
            {activeCount > 0 && (
              <button className="toolbar-menu-item" type="button" onClick={() => runAction(onPauseAll)}>
                Pause all active downloads
              </button>
            )}
            {pausedCount > 0 && (
              <button className="toolbar-menu-item" type="button" onClick={() => runAction(onResumeAll)}>
                Resume all paused downloads
              </button>
            )}
            <button
              className="toolbar-menu-item"
              type="button"
              onClick={() => runAction(onToggleQueueDispatch)}
            >
              {dispatchManuallyPaused ? "Resume queued downloads" : "Hold queued downloads"}
            </button>
            <button className="toolbar-menu-item" type="button" onClick={() => runAction(onRefresh)}>
              Refresh downloads list
            </button>
            <button className="toolbar-menu-item" type="button" onClick={() => runAction(onToggleTheme)}>
              {theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
            </button>
            {appVersion && (
              <span className="toolbar-version" aria-label={`App version ${appVersion}`}>
                {appVersion.startsWith("v") || appVersion.startsWith("alpha") || appVersion.startsWith("beta")
                  ? appVersion
                  : `v${appVersion}`}
              </span>
            )}
          </div>
        </div>
      </div>
    </header>
  );
}
