interface ComparisonTableProps {
  title?: string;
  headers: string[];
  rows: (string | number)[][];
}

export function ComparisonTable({ title, headers, rows }: ComparisonTableProps) {
  return (
    <div
      className="border rounded-xl overflow-hidden"
      style={{
        background: "var(--axion-glass-bg, rgb(31 41 55 / 0.8))",
        borderColor: "var(--axion-glass-border, rgb(55 65 81))",
        backdropFilter: "blur(var(--axion-blur, 0px))",
        WebkitBackdropFilter: "blur(var(--axion-blur, 0px))",
      }}
    >
      {title && (
        <div
          className="px-5 py-3"
          style={{ borderBottom: "1px solid var(--axion-glass-border, rgb(55 65 81 / 0.8))" }}
        >
          <p className="text-[11px] font-semibold uppercase tracking-widest text-gray-500">
            {title}
          </p>
        </div>
      )}
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr style={{ background: "var(--axion-glass-bg, rgb(17 24 39 / 0.5))" }}>
              {headers.map((h, i) => (
                <th
                  key={i}
                  className="text-left text-[11px] font-semibold uppercase tracking-wide
                             text-gray-400 px-5 py-3 whitespace-nowrap"
                  style={i === 0 ? { color: "var(--axion-accent, rgb(156 163 175))" } : undefined}
                >
                  {h}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row, ri) => (
              <tr
                key={ri}
                className="transition-colors"
                style={{ borderTop: "1px solid var(--axion-glass-border, rgb(55 65 81 / 0.5))" }}
              >
                {row.map((cell, ci) => (
                  <td
                    key={ci}
                    className={`px-5 py-3 ${ci === 0 ? "font-medium text-gray-200" : "text-gray-400"}`}
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
