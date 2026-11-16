import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { QueueStatsEvent } from "../api/types";
import { formatSpeed } from "../lib/format";

const HISTORY_SIZE = 60;

function formatEtaShort(seconds: number): string {
  if (seconds < 60) return seconds + "s";
  if (seconds < 3600) return Math.round(seconds / 60) + "m";
  return Math.round(seconds / 3600) + "h";
}

export function SpeedChart() {
  const [history, setHistory] = useState<number[]>([]);
  const [currentSpeed, setCurrentSpeed] = useState(0);
  const [activeCount, setActiveCount] = useState(0);
  const [queuedCount, setQueuedCount] = useState(0);
  const [eta, setEta] = useState<number | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    let off: (() => void) | undefined;
    void listen<QueueStatsEvent>("queue_stats_updated", (event) => {
      const { speed_bps, active_count, queued_count, estimated_completion_seconds } = event.payload;
      setCurrentSpeed(speed_bps);
      setActiveCount(active_count);
      setQueuedCount(queued_count);
      setEta(estimated_completion_seconds ?? null);
      setHistory((prev) => {
        const next = [...prev, speed_bps];
        return next.length > HISTORY_SIZE ? next.slice(next.length - HISTORY_SIZE) : next;
      });
    }).then((fn) => { off = fn; });
    void listen("queue_idle", () => {
      setCurrentSpeed(0); setActiveCount(0); setQueuedCount(0); setEta(null);
    });
    return () => off?.();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || history.length < 2) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const { width, height } = canvas;
    ctx.clearRect(0, 0, width, height);
    const max = Math.max(...history, 1);
    const step = width / (HISTORY_SIZE - 1);
    ctx.beginPath();
    ctx.strokeStyle = "var(--accent, #4a9eff)";
    ctx.lineWidth = 1.5;
    history.forEach((val, i) => {
      const x = i * step;
      const y = height - (val / max) * (height - 4) - 2;
      if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
    });
    ctx.stroke();
    ctx.lineTo((history.length - 1) * step, height);
    ctx.lineTo(0, height);
    ctx.closePath();
    ctx.fillStyle = "rgba(74,158,255,0.12)";
    ctx.fill();
  }, [history]);

  if (activeCount === 0 && history.length === 0) return null;

  return (
    <div className="speed-chart" aria-label="Queue speed chart">
      <div className="speed-chart-stats">
        <span className="speed-chart-speed">{formatSpeed(currentSpeed)}</span>
        <span className="speed-chart-meta">
          {activeCount > 0 && <span>{activeCount} active</span>}
          {queuedCount > 0 && <span> {queuedCount} queued</span>}
          {eta !== null && eta > 0 && <span> ETA {formatEtaShort(eta)}</span>}
        </span>
      </div>
      <canvas ref={canvasRef} className="speed-chart-canvas" width={280} height={40} aria-hidden="true" />
    </div>
  );
}
