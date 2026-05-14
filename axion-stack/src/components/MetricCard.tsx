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
