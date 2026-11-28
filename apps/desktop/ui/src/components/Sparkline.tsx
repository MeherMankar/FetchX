import { useCallback, useEffect, useRef } from "react";
import { getAccentColorWithAlpha } from "../lib/color";

interface SparklineProps {
  data: number[];
  width?: number;
  height?: number;
  color?: string;
}

export function Sparkline({ data, width = 80, height = 24, color }: SparklineProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const draw = useCallback(
    (cssWidth: number) => {
      const canvas = canvasRef.current;
      if (!canvas || data.length < 2) return;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;

      const dpr = window.devicePixelRatio || 1;
      const cssHeight = height;
      const targetW = Math.round(cssWidth * dpr);
      const targetH = Math.round(cssHeight * dpr);

      if (canvas.width !== targetW || canvas.height !== targetH) {
        canvas.width = targetW;
        canvas.height = targetH;
      }

      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, cssWidth, cssHeight);

      const max = Math.max(...data, 1);
      const pad = 2;
      const w = cssWidth;
      const h = cssHeight;
      const step = w / (data.length - 1);

      const strokeColor = color ?? getAccentColorWithAlpha(0.7);

      // Smooth curve using quadratic bezier
      ctx.beginPath();
      data.forEach((val, i) => {
        const x = i * step;
        const y = h - pad - (val / max) * (h - pad * 2);
        if (i === 0) {
          ctx.moveTo(x, y);
        } else {
          const prevX = (i - 1) * step;
          const prevY = h - pad - (data[i - 1] / max) * (h - pad * 2);
          const cpX = (prevX + x) / 2;
          ctx.quadraticCurveTo(prevX, prevY, cpX, (prevY + y) / 2);
          if (i === data.length - 1) ctx.lineTo(x, y);
        }
      });

      // Gradient fill
      const gradient = ctx.createLinearGradient(0, 0, 0, h);
      gradient.addColorStop(0, getAccentColorWithAlpha(0.18));
      gradient.addColorStop(1, getAccentColorWithAlpha(0.02));

      const lastX = (data.length - 1) * step;
      ctx.lineTo(lastX, h);
      ctx.lineTo(0, h);
      ctx.closePath();
      ctx.fillStyle = gradient;
      ctx.fill();

      // Draw the line on top
      ctx.beginPath();
      data.forEach((val, i) => {
        const x = i * step;
        const y = h - pad - (val / max) * (h - pad * 2);
        if (i === 0) {
          ctx.moveTo(x, y);
        } else {
          const prevX = (i - 1) * step;
          const prevY = h - pad - (data[i - 1] / max) * (h - pad * 2);
          const cpX = (prevX + x) / 2;
          ctx.quadraticCurveTo(prevX, prevY, cpX, (prevY + y) / 2);
          if (i === data.length - 1) ctx.lineTo(x, y);
        }
      });
      ctx.strokeStyle = strokeColor;
      ctx.lineWidth = 1.5;
      ctx.lineJoin = "round";
      ctx.lineCap = "round";
      ctx.stroke();
    },
    [data, height, color],
  );

  // Redraw when data changes (uses fixed width when width > 0)
  useEffect(() => {
    if (width > 0) {
      draw(width);
    } else {
      const container = containerRef.current;
      if (container) draw(container.clientWidth);
    }
  }, [draw, width]);

  useEffect(() => {
    const observer = new MutationObserver(() => {
      if (width > 0) {
        draw(width);
      } else if (containerRef.current) {
        draw(containerRef.current.clientWidth);
      }
    });
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-accent", "data-theme"],
    });
    return () => observer.disconnect();
  }, [draw, width]);

  // ResizeObserver when width is 0 (auto-fit container)
  useEffect(() => {
    if (width > 0) return;
    const container = containerRef.current;
    if (!container) return;
    const ro = new ResizeObserver(() => {
      draw(container.clientWidth);
    });
    ro.observe(container);
    return () => ro.disconnect();
  }, [draw, width]);

  if (width > 0) {
    return (
      <canvas
        ref={canvasRef}
        className="download-sparkline"
        aria-hidden="true"
        style={{ width: `${width}px`, height: `${height}px`, display: "inline-block" }}
      />
    );
  }

  return (
    <div ref={containerRef} style={{ display: "inline-block", width: "100%", height: `${height}px` }}>
      <canvas
        ref={canvasRef}
        className="download-sparkline"
        aria-hidden="true"
        style={{ width: "100%", height: `${height}px`, display: "block" }}
      />
    </div>
  );
}
