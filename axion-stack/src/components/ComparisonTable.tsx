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
        background: "var(--axion-glass-bg, rgba(255,255,255,0.04))",
        border: "0.5px solid var(--axion-glass-border, rgba(255,255,255,0.10))",
        borderRadius: "var(--axion-radius, 24px)",
        backdropFilter: "blur(var(--axion-blur, 80px))",
        WebkitBackdropFilter: "blur(var(--axion-blur, 80px))",
        boxShadow: "var(--axion-glass-inset, inset 0 1px 0 rgba(255,255,255,0.15), inset 0 0 0 0.5px rgba(255,255,255,0.06))",
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
