/** Human-readable formatting utilities. No side effects, no IPC. */

export function formatBytes(value: number | null): string {
  if (value === null) return "—";
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = value;
  let unit = -1;
  do {
    size /= 1024;
    unit += 1;
  } while (size >= 1024 && unit < units.length - 1);
  return `${size.toFixed(1)} ${units[unit]}`;
}

export function formatSpeed(bytesPerSecond: number | null): string {
  if (bytesPerSecond === null) return "—";
  return `${formatBytes(bytesPerSecond)}/s`;
}

export function formatEta(seconds: number | null): string {
  if (seconds === null) return "—";
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return m > 0 ? `${h}h ${m}m` : `${h}h`;
}

export function formatPercent(basisPoints: number | null): string {
  if (basisPoints === null) return "—";
  return `${(basisPoints / 100).toFixed(1)}%`;
}

/** "Oct 10, 2026, 14:30" */
export function formatDatetime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** "YYYY-MM-DDTHH:MM" for <input type="datetime-local"> */
export function msToDatetimeLocal(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}` +
    `T${pad(d.getHours())}:${pad(d.getMinutes())}`
  );
}

export function errorMessage(error: unknown): string {
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof (error as Record<string, unknown>).message === "string"
  ) {
    return (error as { message: string }).message;
  }
  if (typeof error === "string") return error;
  return "The request could not be completed.";
}
