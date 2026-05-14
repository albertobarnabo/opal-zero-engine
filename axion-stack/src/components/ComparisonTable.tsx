interface ComparisonTableProps {
  title?: string;
  headers: string[];
  rows: (string | number)[][];
}

const cellBorder = "1px solid var(--axion-glass-border, rgba(255,255,255,0.08))";

export function ComparisonTable({ title, headers, rows }: ComparisonTableProps) {
  return (
    <div
      className="overflow-hidden"
      style={{
        background: "linear-gradient(rgba(255,255,255,0.04), rgba(255,255,255,0.016))",
        border: "1px solid var(--axion-glass-border, rgba(255,255,255,0.08))",
        borderRadius: "var(--axion-radius, 14px)",
        backdropFilter: "blur(var(--axion-blur, 28px)) saturate(130%)",
        WebkitBackdropFilter: "blur(var(--axion-blur, 28px)) saturate(130%)",
        boxShadow: "inset 0 1px 0 0 rgba(255,255,255,0.06), 0 0 0 1px rgba(255,255,255,0.04), 0 40px 80px -30px rgba(0,0,0,0.65)",
      }}
    >
      {title && (
        <div style={{ padding: "14px 24px 12px", borderBottom: cellBorder }}>
          <p
            className="text-[11px] font-semibold uppercase tracking-widest"
            style={{ color: "rgba(255,255,255,0.55)" }}
          >
            {title}
          </p>
        </div>
      )}
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr style={{ background: "rgba(255,255,255,0.025)" }}>
              {headers.map((h, i) => (
                <th
                  key={i}
                  className="text-left text-[11px] font-semibold uppercase tracking-wide px-6 py-4 whitespace-nowrap"
                  style={{
                    color:
                      i === 0
                        ? "var(--axion-accent, rgba(255,255,255,0.90))"
                        : "rgba(255,255,255,0.55)",
                  }}
                >
                  {h}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row, ri) => (
              <tr key={ri} style={{ borderTop: cellBorder }}>
                {row.map((cell, ci) => (
                  <td
                    key={ci}
                    className="px-6 py-4"
                    style={{
                      color:
                        ci === 0
                          ? "rgba(255,255,255,0.90)"
                          : "rgba(255,255,255,0.60)",
                      fontWeight: ci === 0 ? 600 : 400,
                    }}
                  >
                    {cell}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
