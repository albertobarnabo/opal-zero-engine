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
      className="border rounded-xl flex flex-col gap-1"
      style={{
        background: "var(--axion-glass-bg, rgb(31 41 55 / 0.8))",
        borderColor: "var(--axion-glass-border, rgb(55 65 81))",
        backdropFilter: "blur(var(--axion-blur, 0px))",
        WebkitBackdropFilter: "blur(var(--axion-blur, 0px))",
        padding: "var(--axion-pad, 20px)",
      }}
    >
      <p className="text-[11px] font-semibold uppercase tracking-widest text-gray-500">
        {title}
      </p>
      <div className="flex items-baseline gap-1.5 mt-1">
        <span
          className="text-3xl font-bold tabular-nums"
          style={{ color: "var(--axion-accent, white)" }}
        >
          {value}
        </span>
        {unit && <span className="text-sm text-gray-400">{unit}</span>}
        {trendIcon && (
          <span className={`text-sm font-bold ml-1 ${trendColor}`}>{trendIcon}</span>
        )}
      </div>
      {subtitle && <p className="text-xs text-gray-400 mt-0.5">{subtitle}</p>}
    </div>
  );
}
