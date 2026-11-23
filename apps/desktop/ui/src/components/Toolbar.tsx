import { type RefObject } from "react";

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
          onChange={(e) => onSearchChange(e.target.value)}
          aria-label="Search downloads"
        />

        <label className="visually-hidden" htmlFor="sort-select">Sort by</label>
        <select
          id="sort-select"
          className="sort-select"
          value={sort}
          onChange={(e) => onSortChange(e.target.value as SortKey)}
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

        <button
          className="button button-secondary"
          onClick={onToggleQueueDispatch}
          aria-label={dispatchManuallyPaused ? "Resume queued downloads" : "Hold queued downloads"}
          title={dispatchManuallyPaused ? "Resume queue" : "Hold queue"}
        >
          {dispatchManuallyPaused ? "Resume queue" : "Hold queue"}
        </button>

        {activeCount > 0 && (
          <button
            className="button button-secondary"
            onClick={onPauseAll}
            aria-label="Pause all active downloads"
          >
            Pause all
          </button>
        )}
        {pausedCount > 0 && (
          <button
            className="button button-secondary"
            onClick={onResumeAll}
            aria-label="Resume all paused downloads"
          >
            Resume all
          </button>
        )}

        <button
          className="button button-secondary"
          onClick={onRefresh}
          aria-label="Refresh downloads list"
          title="Refresh"
        >
          Refresh
        </button>

        <button
          className="button button-secondary"
          onClick={onBatchClick}
          aria-label="Batch add downloads"
        >
          Batch add
        </button>

        <button
          className="button button-primary"
          onClick={onAddClick}
          aria-label="Add new download"
        >
          Add download
        </button>

        <button
          className="button button-secondary theme-toggle-button"
          onClick={onToggleTheme}
          aria-label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
          title={theme === "dark" ? "Light mode" : "Dark mode"}
        >
          <svg
            className="theme-toggle-icon"
            viewBox="0 0 16 16"
            aria-hidden="true"
          >
            {theme === "dark" ? (
              <>
                <circle cx="8" cy="8" r="3" />
                <path d="M8 1.5v1.25M8 13.25v1.25M1.5 8h1.25M13.25 8h1.25M3.4 3.4l.9.9m7.4 7.4.9.9m0-9.2-.9.9m-7.4 7.4-.9.9" />
              </>
            ) : (
              <path d="M13.25 9.5A5.5 5.5 0 0 1 6.5 2.75 5.5 5.5 0 1 0 13.25 9.5Z" />
            )}
          </svg>
        </button>
      </div>

      {appVersion && (
        <span className="toolbar-version" aria-label={`App version ${appVersion}`}>
          {appVersion.startsWith("v") || appVersion.startsWith("alpha") || appVersion.startsWith("beta")
            ? appVersion
            : `v${appVersion}`}
        </span>
      )}
    </header>
  );
}
