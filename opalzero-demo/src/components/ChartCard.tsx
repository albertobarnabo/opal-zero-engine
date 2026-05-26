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
  // Accept both real numbers and string-encoded numbers (LLMs often serialize as strings)
  const numericKeys = Object.keys(data[0]).filter((k) => {
    if (k === xKey) return false;
    const v = data[0][k];
    if (typeof v === "number") return true;
    if (typeof v === "string" && v.trim() !== "" && !isNaN(Number(v))) return true;
    return false;
  });
  if (numericKeys.length <= 1) return numericKeys;
  // Return only the key with the highest max value — avoids mixed-unit multi-series
  // (e.g. price: 230 and rating: 4.5 must not share the same Y axis)
  return [numericKeys.reduce((best, k) => {
    const maxK    = Math.max(...data.map((row) => Number(row[k])    || 0));
    const maxBest = Math.max(...data.map((row) => Number(row[best]) || 0));
    return maxK > maxBest ? k : best;
  })];
}

const SERIES_COLORS = [
  "var(--opalzero-accent, #a7cadc)",
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
        background: "var(--opalzero-card, rgba(10,14,20,0.97))",
        border: "1px solid rgba(255,255,255,0.10)",
        borderRadius: "10px",
        padding: "8px 12px",
        fontSize: 12,
        color: "rgba(255,255,255,0.85)",
        backdropFilter: "blur(20px)",
        WebkitBackdropFilter: "blur(20px)",
      }}
    >
      <p style={{ fontSize: 11, fontWeight: 600, marginBottom: 4, color: "rgba(255,255,255,0.50)" }}>
        {label}
      </p>
      {payload.map((entry: { name: string; value: number; color: string }, i: number) => (
        <p key={i} style={{ fontFamily: "var(--opalzero-font-mono, monospace)", fontSize: 12, color: entry.color }}>
          {entry.name}:{" "}
          <span style={{ color: "rgba(255,255,255,0.85)" }}>
            {typeof entry.value === "number" ? entry.value.toLocaleString() : entry.value}
          </span>
        </p>
      ))}
    </div>
  );
}

export function ChartCard({ title, data, xKey, dataKeys, chartType }: ChartCardProps) {
  if (!data || data.length === 0) return null;

  const resolvedXKey = xKey ?? detectXKey(data);
  const resolvedDataKeys = dataKeys ?? detectDataKeys(data, resolvedXKey);

  // Coerce string-encoded numbers to real numbers so Recharts renders them correctly
  const normalizedData = data.map((row) => {
    const out: Record<string, string | number> = { ...row };
    for (const k of resolvedDataKeys) {
      if (typeof out[k] === "string") out[k] = Number(out[k]);
    }
    return out;
  });
  const isArea = chartType === "area" || (!chartType && resolvedDataKeys.length === 1);

  const gradientId = `chart-grad-${title?.replace(/\s+/g, "") ?? "default"}`;

  return (
    <div
      style={{
        background: "var(--opalzero-glass-bg)",
        border: "1px solid var(--opalzero-glass-border)",
        borderRadius: "var(--opalzero-radius)",
        backdropFilter: "blur(var(--opalzero-blur)) saturate(130%)",
        WebkitBackdropFilter: "blur(var(--opalzero-blur)) saturate(130%)",
        boxShadow: "var(--shadow-glass)",
        padding: "var(--opalzero-pad, 20px)",
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
          <AreaChart data={normalizedData} margin={{ top: 4, right: 4, bottom: 0, left: 0 }}>
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
            <Tooltip
              content={<CustomTooltip />}
              cursor={{ stroke: "rgba(255,255,255,0.06)" }}
              wrapperStyle={{ boxShadow: "none", outline: "none" }}
            />
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
          <BarChart data={normalizedData} margin={{ top: 4, right: 4, bottom: 0, left: 0 }}>
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
            <Tooltip
              content={<CustomTooltip />}
              cursor={{ fill: "rgba(255,255,255,0.04)" }}
              wrapperStyle={{ boxShadow: "none", outline: "none" }}
            />
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
