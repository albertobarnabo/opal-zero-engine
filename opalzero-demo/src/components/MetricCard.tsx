"use client";

import { useRef, useState, useEffect } from "react";

function formatValue(raw: unknown): string {
  if (typeof raw === "number") {
    const abs = Math.abs(raw);
    if (abs >= 1_000_000_000) return (raw / 1_000_000_000).toFixed(1).replace(/\.0$/, "") + "B";
    if (abs >= 1_000_000)     return (raw / 1_000_000).toFixed(1).replace(/\.0$/, "") + "M";
    if (abs >= 10_000)        return (raw / 1_000).toFixed(1).replace(/\.0$/, "") + "K";
    if (Number.isInteger(raw)) return raw.toLocaleString();
    return raw.toFixed(2);
  }
  return String(raw ?? "—");
}

interface MetricCardProps {
  title: string;
  value: string | number;
  subtitle?: string;
  unit?: string;
  trend?: "up" | "down" | "neutral";
}

export function MetricCard({ title, value, subtitle, unit, trend }: MetricCardProps) {
  const trendIcon = trend === "up" ? "↑" : trend === "down" ? "↓" : null;
  const trendColor =
    trend === "up" ? "text-emerald-400" : trend === "down" ? "text-red-400" : "";

  // ── Count-up animation ──────────────────────────────────────────────────────
  const ref = useRef<HTMLDivElement>(null);
  const [displayed, setDisplayed] = useState<number>(0);
  const [hasAnimated, setHasAnimated] = useState(false);

  // Normalise raw numbers to formatted strings before animation extraction.
  // LLMs sometimes serialize large numbers as JSON strings — handle both.
  const numericValue = typeof value === "string" && value.trim() !== "" && !isNaN(Number(value))
    ? Number(value)
    : value;
  const rawStr = typeof numericValue === "number" ? formatValue(numericValue) : String(numericValue);
  const prefix = rawStr.match(/^[^0-9]*/)?.[0] ?? "";
  // Capture everything after the last digit as suffix (handles "B", "M", "K", "%")
  const suffix = rawStr.replace(/^.*[0-9]/, "");
  const target = parseFloat(rawStr.replace(/[^0-9.-]/g, ""));
  const isNumeric = !isNaN(target);

  useEffect(() => {
    const el = ref.current;
    if (!el || !isNumeric) return;

    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting && !hasAnimated) {
          setHasAnimated(true);
          observer.disconnect();

          const duration = 900; // ms
          const start = performance.now();

          const tick = (now: number) => {
            const progress = Math.min((now - start) / duration, 1);
            // ease-out cubic
            const eased = 1 - Math.pow(1 - progress, 3);
            setDisplayed(Math.round(eased * target));
            if (progress < 1) {
              requestAnimationFrame(tick);
            } else {
              setDisplayed(target);
            }
          };

          requestAnimationFrame(tick);
        }
      },
      { threshold: 0.3 }
    );

    observer.observe(el);
    return () => observer.disconnect();
    // Re-run only if the target number or animation gate changes
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target, hasAnimated]);

  // What to show in the big number slot
  const displayValue = !isNumeric
    ? formatValue(value)                             // non-numeric: still run through formatter
    : !hasAnimated
    ? `${prefix}0${suffix}`                          // not yet animated: start at 0
    : displayed === target
    ? typeof numericValue === "number" ? formatValue(numericValue) : String(numericValue)  // animation done: formatted
    : `${prefix}${displayed}${suffix}`;              // animating: show counted value

  return (
    <div
      ref={ref}
      className="flex flex-col gap-1 h-full"
      style={{
        background: "var(--axion-glass-bg)",
        border: "1px solid var(--axion-glass-border)",
        borderRadius: "var(--axion-radius)",
        backdropFilter: "blur(var(--axion-blur)) saturate(130%)",
        WebkitBackdropFilter: "blur(var(--axion-blur)) saturate(130%)",
        boxShadow: "var(--shadow-glass)",
        padding: "var(--axion-pad, 20px)",
      }}
    >
      <p
        className="text-[11px] font-semibold uppercase tracking-widest"
        style={{ color: "rgba(255,255,255,0.55)" }}
      >
        {title}
      </p>
      <div className="flex items-baseline gap-2 mt-2">
        <span
          className="text-6xl font-black tabular-nums leading-none tracking-tighter"
          style={{
            fontFamily: "var(--axion-font-main)",
            background: "linear-gradient(135deg, oklch(0.97 0.004 250), oklch(0.82 0.012 250))",
            WebkitBackgroundClip: "text",
            WebkitTextFillColor: "transparent",
            backgroundClip: "text",
          }}
        >
          {displayValue}
        </span>
        {unit && (
          <span className="text-base" style={{ color: "rgba(255,255,255,0.55)" }}>
            {unit}
          </span>
        )}
        {trendIcon && (
          <span className={`text-base font-bold ml-1 ${trendColor}`}>{trendIcon}</span>
        )}
      </div>
      {subtitle && (
        <p className="text-sm mt-1.5" style={{ color: "rgba(255,255,255,0.55)" }}>
          {subtitle}
        </p>
      )}
    </div>
  );
}
