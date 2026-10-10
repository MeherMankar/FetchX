interface BulkActionBarProps {
  count: number;
  canPauseAny: boolean;
  canCancelAny: boolean;
  onPauseAll: () => void;
  onCancelAll: () => void;
  onDeleteAll: () => void;
  onClearSelection: () => void;
}

/**
 * Toolbar that appears when one or more downloads are selected.
 * Destructive actions (cancel, delete) are handled by the parent
 * which shows a ConfirmDialog before executing.
 */
export function BulkActionBar({
  count,
  canPauseAny,
  canCancelAny,
  onPauseAll,
  onCancelAll,
  onDeleteAll,
  onClearSelection,
}: BulkActionBarProps) {
  return (
    <div className="bulk-bar" role="toolbar" aria-label="Bulk actions">
      <span className="bulk-count">
        {count} {count === 1 ? "download" : "downloads"} selected
      </span>
      <div className="bulk-actions">
        {canPauseAny && (
          <button className="button button-secondary" onClick={onPauseAll}>
            Pause
          </button>
        )}
        {canCancelAny && (
          <button className="button button-secondary" onClick={onCancelAll}>
            Cancel
          </button>
        )}
        <button className="button button-danger" onClick={onDeleteAll}>
          Delete
        </button>
        <button className="button button-secondary" onClick={onClearSelection}>
          Clear selection
        </button>
      </div>
    </div>
  );
}
