"use client";

import {
  ResponsiveContainer,
  AreaChart,
  BarChart,
  Area,
  Bar,
  XAxis,
  YAxis,
  Tooltip,
  CartesianGrid,
} from "recharts";

interface ChartCardProps {
  title?: string;
  data: Record<string, string | number>[];
  xKey?: string;
  dataKeys?: string[];
  chartType?: "area" | "bar";
}

const TEMPORAL_KEYS = ["date", "time", "month", "year", "week", "day", "quarter", "period", "timestamp"];

function detectXKey(data: Record<string, string | number>[]): string {
  if (!data.length) return "";
  const keys = Object.keys(data[0]);
  return (
    keys.find((k) => TEMPORAL_KEYS.some((t) => k.toLowerCase().includes(t))) ??
    keys[0]
  );
}

function detectDataKeys(data: Record<string, string | number>[], xKey: string): string[] {
  if (!data.length) return [];
  return Object.keys(data[0]).filter(
    (k) => k !== xKey && typeof data[0][k] === "number"
  );
}

const SERIES_COLORS = [
  "var(--axion-accent, #8b9cf4)",
  "#6ee7b7",
  "#fbbf24",
  "#c084fc",
  "#f87171",
];

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function CustomTooltip({ active, payload, label }: any) {
  if (!active || !payload?.length) return null;
  return (
    <div
      style={{
        background: "rgba(8,12,28,0.88)",
        border: "0.5px solid rgba(255,255,255,0.12)",
        borderRadius: "10px",
        padding: "8px 12px",
        backdropFilter: "blur(20px)",
      }}
    >
      <p className="text-[11px] font-semibold mb-1" style={{ color: "rgba(255,255,255,0.5)" }}>
        {label}
      </p>
      {payload.map((entry: { name: string; value: number; color: string }, i: number) => (
        <p key={i} className="text-xs font-mono" style={{ color: entry.color }}>
          {entry.name}: {typeof entry.value === "number" ? entry.value.toLocaleString() : entry.value}
        </p>
      ))}
    </div>
  );
}

export function ChartCard({ title, data, xKey, dataKeys, chartType }: ChartCardProps) {
  if (!data || data.length === 0) return null;

  const resolvedXKey = xKey ?? detectXKey(data);
  const resolvedDataKeys = dataKeys ?? detectDataKeys(data, resolvedXKey);
  const isArea = chartType === "area" || (!chartType && resolvedDataKeys.length === 1);

  const gradientId = `chart-grad-${title?.replace(/\s+/g, "") ?? "default"}`;

  return (
    <div
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
      {title && (
        <p
          className="text-[11px] font-semibold uppercase tracking-widest mb-4"
          style={{ color: "rgba(255,255,255,0.55)" }}
        >
          {title}
        </p>
      )}
      <ResponsiveContainer width="100%" height={180}>
        {isArea ? (
          <AreaChart data={data} margin={{ top: 4, right: 4, bottom: 0, left: 0 }}>
            <defs>
              {resolvedDataKeys.map((key, i) => (
                <linearGradient key={key} id={`${gradientId}-${i}`} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="5%"  stopColor={SERIES_COLORS[i % SERIES_COLORS.length]} stopOpacity={0.25} />
                  <stop offset="95%" stopColor={SERIES_COLORS[i % SERIES_COLORS.length]} stopOpacity={0} />
                </linearGradient>
              ))}
            </defs>
            <CartesianGrid strokeDasharray="3 3" stroke="rgba(255,255,255,0.05)" vertical={false} />
            <XAxis
              dataKey={resolvedXKey}
              tick={{ fill: "rgba(255,255,255,0.30)", fontSize: 10 }}
              axisLine={false}
              tickLine={false}
            />
            <YAxis
              tick={{ fill: "rgba(255,255,255,0.25)", fontSize: 10 }}
              axisLine={false}
              tickLine={false}
              width={36}
            />
            <Tooltip content={<CustomTooltip />} cursor={{ stroke: "rgba(255,255,255,0.06)" }} />
            {resolvedDataKeys.map((key, i) => (
              <Area
                key={key}
                type="monotone"
                dataKey={key}
                stroke={SERIES_COLORS[i % SERIES_COLORS.length]}
                strokeWidth={1.5}
                fill={`url(#${gradientId}-${i})`}
                dot={false}
                activeDot={{ r: 4, strokeWidth: 0, fill: SERIES_COLORS[i % SERIES_COLORS.length] }}
              />
            ))}
          </AreaChart>
        ) : (
          <BarChart data={data} margin={{ top: 4, right: 4, bottom: 0, left: 0 }}>
            <CartesianGrid strokeDasharray="3 3" stroke="rgba(255,255,255,0.05)" vertical={false} />
            <XAxis
              dataKey={resolvedXKey}
              tick={{ fill: "rgba(255,255,255,0.30)", fontSize: 10 }}
              axisLine={false}
              tickLine={false}
            />
            <YAxis
              tick={{ fill: "rgba(255,255,255,0.25)", fontSize: 10 }}
              axisLine={false}
              tickLine={false}
              width={36}
            />
            <Tooltip content={<CustomTooltip />} cursor={{ fill: "rgba(255,255,255,0.03)" }} />
            {resolvedDataKeys.map((key, i) => (
              <Bar
                key={key}
                dataKey={key}
                fill={SERIES_COLORS[i % SERIES_COLORS.length]}
                fillOpacity={0.75}
                radius={[4, 4, 0, 0]}
              />
            ))}
          </BarChart>
        )}
      </ResponsiveContainer>
    </div>
  );
}
