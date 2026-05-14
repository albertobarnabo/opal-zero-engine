"use client";

import { useRef, useState, useEffect } from "react";

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

  // Extract prefix (e.g. "$"), numeric target, and suffix (e.g. "%", "k")
  const rawStr = String(value);
  const prefix = rawStr.match(/^[^0-9]*/)?.[0] ?? "";
  const suffix = rawStr.replace(/^[0-9.,\s-]+/, "").replace(/^[^0-9]*/, "");
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
    ? value                                          // non-numeric: show as-is
    : !hasAnimated
    ? `${prefix}0${suffix}`                          // not yet animated: start at 0
    : displayed === target
    ? value                                          // animation done: restore exact original
    : `${prefix}${displayed}${suffix}`;              // animating: show counted value

  return (
    <div
      ref={ref}
      className="flex flex-col gap-1 h-full"
      style={{
        background: "linear-gradient(rgba(255,255,255,0.04), rgba(255,255,255,0.016))",
        border: "1px solid var(--axion-glass-border, rgba(255,255,255,0.08))",
        borderRadius: "var(--axion-radius, 14px)",
        backdropFilter: "blur(var(--axion-blur, 28px)) saturate(130%)",
        WebkitBackdropFilter: "blur(var(--axion-blur, 28px)) saturate(130%)",
        boxShadow: "inset 0 1px 0 0 rgba(255,255,255,0.06), 0 0 0 1px rgba(255,255,255,0.04), 0 40px 80px -30px rgba(0,0,0,0.65)",
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
            background: "linear-gradient(135deg, #ebeff2 0%, #9399a0 40%, #ebeff2 75%, #c8d4da 100%)",
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
