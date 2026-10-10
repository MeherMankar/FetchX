import type { QueueSnapshot } from "../api/types";

interface SnapshotBarProps {
  snapshot: QueueSnapshot | null;
}

/**
 * Compact live queue statistics shown in the sidebar footer.
 * Null snapshot means not yet loaded — renders nothing.
 */
export function SnapshotBar({ snapshot }: SnapshotBarProps) {
  if (!snapshot) return null;

  return (
    <div className="snapshot-bar" aria-label="Queue statistics">
      <span className="snapshot-item">
        <span className="snapshot-label">Active</span>
        <span className="snapshot-value">{snapshot.active_count}</span>
      </span>
      <span className="snapshot-sep" aria-hidden="true">·</span>
      <span className="snapshot-item">
        <span className="snapshot-label">Queued</span>
        <span className="snapshot-value">{snapshot.queued_count}</span>
      </span>
      <span className="snapshot-sep" aria-hidden="true">·</span>
      <span className="snapshot-item">
        <span className="snapshot-label">Conn</span>
        <span className="snapshot-value">
          {snapshot.used_connections}/{snapshot.total_connections}
        </span>
      </span>
    </div>
  );
}
