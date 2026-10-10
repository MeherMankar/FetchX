import { useState } from "react";

import type { DownloadSchedule } from "../api/types";
import { errorMessage, formatDatetime, msToDatetimeLocal } from "../lib/format";

interface ScheduleEditorProps {
  downloadId: string;
  schedule: DownloadSchedule | null;
  busy: boolean;
  onSet: (scheduledAtMs: number) => Promise<void>;
  onClear: () => Promise<void>;
  onError: (message: string) => void;
}

export function ScheduleEditor({
  downloadId,
  schedule,
  busy,
  onSet,
  onClear,
  onError,
}: ScheduleEditorProps) {
  const [editing, setEditing] = useState(false);
  const defaultValue = msToDatetimeLocal(Math.ceil(Date.now() / 60000) * 60000 + 3_600_000);
  const [inputValue, setInputValue] = useState(
    schedule ? msToDatetimeLocal(schedule.scheduled_at_ms) : defaultValue,
  );

  const apply = async () => {
    const ms = new Date(inputValue).getTime();
    if (!Number.isFinite(ms) || ms <= Date.now()) {
      onError("Please choose a time in the future.");
      return;
    }
    try {
      await onSet(ms);
      setEditing(false);
    } catch (cause) {
      onError(errorMessage(cause));
    }
  };

  const clear = async () => {
    try {
      await onClear();
      setEditing(false);
      setInputValue(defaultValue);
    } catch (cause) {
      onError(errorMessage(cause));
    }
  };

  if (!editing) {
    return (
      <div className="schedule-row">
        {schedule?.enabled ? (
          <>
            <span className="schedule-label">
              Starts {formatDatetime(schedule.scheduled_at_ms)}
            </span>
            <button
              className="button button-secondary button-xs"
              disabled={busy}
              onClick={() => {
                setInputValue(msToDatetimeLocal(schedule.scheduled_at_ms));
                setEditing(true);
              }}
            >
              Edit
            </button>
            <button
              className="button button-secondary button-xs"
              disabled={busy}
              onClick={() => void clear()}
            >
              Clear
            </button>
          </>
        ) : (
          <button
            className="button button-secondary button-xs"
            disabled={busy}
            onClick={() => setEditing(true)}
          >
            Schedule start
          </button>
        )}
      </div>
    );
  }

  return (
    <div className="schedule-row">
      <label className="visually-hidden" htmlFor={`sched-${downloadId}`}>
        Schedule start time
      </label>
      <input
        id={`sched-${downloadId}`}
        className="schedule-input"
        type="datetime-local"
        value={inputValue}
        onChange={(e) => setInputValue(e.target.value)}
        disabled={busy}
      />
      <button
        className="button button-primary button-xs"
        disabled={busy || !inputValue}
        onClick={() => void apply()}
      >
        Set
      </button>
      <button
        className="button button-secondary button-xs"
        disabled={busy}
        onClick={() => setEditing(false)}
      >
        Cancel
      </button>
    </div>
  );
}
