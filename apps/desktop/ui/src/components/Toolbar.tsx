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
  onAddClick: () => void;
  appVersion: string | undefined;
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
  onAddClick,
  appVersion,
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
        <label className="visually-hidden" htmlFor="search-input">Search downloads</label>
        <input
          id="search-input"
          ref={searchRef}
          className="search-input"
          type="search"
          placeholder="Search…"
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
        </select>

        <button
          className="button button-secondary"
          onClick={onRefresh}
          aria-label="Refresh downloads list"
          title="Refresh"
        >
          ↺
        </button>

        <button
          className="button button-primary"
          onClick={onAddClick}
          aria-label="Add new download"
        >
          + Add
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
