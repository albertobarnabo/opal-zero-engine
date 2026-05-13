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

  return (
    <div
      className="flex flex-col gap-1 h-full"
      style={{
        background: "var(--axion-glass-bg, rgba(255,255,255,0.04))",
        border: "0.5px solid var(--axion-glass-border, rgba(255,255,255,0.10))",
        borderRadius: "var(--axion-radius, 24px)",
        backdropFilter: "blur(var(--axion-blur, 80px))",
        WebkitBackdropFilter: "blur(var(--axion-blur, 80px))",
        boxShadow: "var(--axion-glass-inset, inset 0 1px 0 rgba(255,255,255,0.15), inset 0 0 0 0.5px rgba(255,255,255,0.06))",
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
            background: "linear-gradient(135deg, #f1f5f9 0%, #94a3b8 40%, #e2e8f0 75%, #cbd5e1 100%)",
            WebkitBackgroundClip: "text",
            WebkitTextFillColor: "transparent",
            backgroundClip: "text",
          }}
        >
          {value}
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
