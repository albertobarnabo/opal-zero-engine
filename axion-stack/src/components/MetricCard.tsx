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
    <div className="bg-gray-800/80 border border-gray-700 rounded-xl p-5 flex flex-col gap-1">
      <p className="text-[11px] font-semibold uppercase tracking-widest text-gray-500">
        {title}
      </p>
      <div className="flex items-baseline gap-1.5 mt-1">
        <span className="text-3xl font-bold text-white tabular-nums">{value}</span>
        {unit && <span className="text-sm text-gray-400">{unit}</span>}
        {trendIcon && (
          <span className={`text-sm font-bold ml-1 ${trendColor}`}>{trendIcon}</span>
        )}
      </div>
      {subtitle && <p className="text-xs text-gray-400 mt-0.5">{subtitle}</p>}
    </div>
  );
}
