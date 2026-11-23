import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { DownloadProgress, QueueStatsEvent } from "../api/types";
import { formatEta, formatSpeed } from "../lib/format";

const HISTORY_SIZE = 60;

interface SpeedChartProps {
  singleActiveProgress: Pick<
    DownloadProgress,
    "speed_bytes_per_second" | "estimated_remaining_seconds"
  > | null;
  singleActiveSpeedHistory: number[] | null;
}

export function SpeedChart({
  singleActiveProgress,
  singleActiveSpeedHistory,
}: SpeedChartProps) {
  const [history, setHistory] = useState<number[]>([]);
  const [currentSpeed, setCurrentSpeed] = useState(0);
  const [activeCount, setActiveCount] = useState(0);
  const [queuedCount, setQueuedCount] = useState(0);
  const [eta, setEta] = useState<number | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const chartHistory =
    singleActiveProgress && singleActiveSpeedHistory
      ? singleActiveSpeedHistory
      : history;
  const displayedSpeed = singleActiveProgress
    ? singleActiveProgress.speed_bytes_per_second
    : currentSpeed;
  const displayedEta = singleActiveProgress
    ? singleActiveProgress.estimated_remaining_seconds
    : eta;

  useEffect(() => {
    let offStats: (() => void) | undefined;
    let offIdle: (() => void) | undefined;

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
    }).then((fn) => { offStats = fn; });

    void listen("queue_idle", () => {
      setCurrentSpeed(0);
      setActiveCount(0);
      setQueuedCount(0);
      setEta(null);
      // Keep history visible for a moment then fade to zero
      setHistory((prev) => (prev.length > 0 ? [...prev, 0] : prev));
    }).then((fn) => { offIdle = fn; });

    return () => {
      offStats?.();
      offIdle?.();
    };
  }, []);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container || chartHistory.length < 2) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    // Keep canvas pixel dimensions in sync with container width
    const dpr = window.devicePixelRatio || 1;
    const cssWidth = container.clientWidth;
    const cssHeight = 44;
    if (canvas.width !== Math.round(cssWidth * dpr) || canvas.height !== Math.round(cssHeight * dpr)) {
      canvas.width = Math.round(cssWidth * dpr);
      canvas.height = Math.round(cssHeight * dpr);
      ctx.scale(dpr, dpr);
    }
    const w = cssWidth;
    const h = cssHeight;

    ctx.clearRect(0, 0, w, h);

    const max = Math.max(...chartHistory, 1);
    const pad = 4;
    const step = w / (chartHistory.length - 1);

    // Smooth curve using quadratic bezier
    ctx.beginPath();
    chartHistory.forEach((val, i) => {
      const x = i * step;
      const y = h - pad - (val / max) * (h - pad * 2);
      if (i === 0) {
        ctx.moveTo(x, y);
      } else {
        const prevX = (i - 1) * step;
        const prevY = h - pad - (chartHistory[i - 1] / max) * (h - pad * 2);
        const cpX = (prevX + x) / 2;
        ctx.quadraticCurveTo(prevX, prevY, cpX, (prevY + y) / 2);
        // Final segment
        if (i === chartHistory.length - 1) ctx.lineTo(x, y);
      }
    });

    // Gradient fill
    const gradient = ctx.createLinearGradient(0, 0, 0, h);
    gradient.addColorStop(0, "rgba(126,200,227,0.18)");
    gradient.addColorStop(1, "rgba(126,200,227,0.02)");

    // Close path for fill
    const lastX = (chartHistory.length - 1) * step;
    ctx.lineTo(lastX, h);
    ctx.lineTo(0, h);
    ctx.closePath();
    ctx.fillStyle = gradient;
    ctx.fill();

    // Draw the line on top, clean
    ctx.beginPath();
    chartHistory.forEach((val, i) => {
      const x = i * step;
      const y = h - pad - (val / max) * (h - pad * 2);
      if (i === 0) {
        ctx.moveTo(x, y);
      } else {
        const prevX = (i - 1) * step;
        const prevY = h - pad - (chartHistory[i - 1] / max) * (h - pad * 2);
        const cpX = (prevX + x) / 2;
        ctx.quadraticCurveTo(prevX, prevY, cpX, (prevY + y) / 2);
        if (i === chartHistory.length - 1) ctx.lineTo(x, y);
      }
    });
    ctx.strokeStyle = "rgba(126,200,227,0.7)";
    ctx.lineWidth = 1.5;
    ctx.lineJoin = "round";
    ctx.lineCap = "round";
    ctx.stroke();
  }, [chartHistory]);

  // Redraw when history changes
  useEffect(() => {
    draw();
  }, [draw]);

  // Redraw on resize
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const ro = new ResizeObserver(() => draw());
    ro.observe(container);
    return () => ro.disconnect();
  }, [draw]);

  if (activeCount === 0 && chartHistory.every((v) => v === 0)) return null;

  const showQueued = queuedCount > 0;
  const showEta = singleActiveProgress !== null || displayedEta !== null;

  return (
    <div className="speed-chart" ref={containerRef} aria-label="Queue speed chart">
      <div className="speed-chart-stats">
        <span className="speed-chart-speed">{formatSpeed(displayedSpeed)}</span>
        <span className="speed-chart-meta">
          {activeCount > 0 && (
            <span>{activeCount} {activeCount === 1 ? "active" : "active"}</span>
          )}
          {showQueued && <span> · {queuedCount} queued</span>}
          {showEta && <span> · ETA {formatEta(displayedEta)}</span>}
        </span>
      </div>
      <canvas
        ref={canvasRef}
        className="speed-chart-canvas"
        aria-hidden="true"
        style={{ width: "100%", height: "44px", display: "block" }}
      />
    </div>
  );
}
