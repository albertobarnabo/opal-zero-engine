interface ComparisonTableProps {
  title?: string;
  headers: string[];
  rows: (string | number)[][];
}

export function ComparisonTable({ title, headers, rows }: ComparisonTableProps) {
  return (
    <div className="bg-gray-800/80 border border-gray-700 rounded-xl overflow-hidden">
      {title && (
        <div className="px-5 py-3 border-b border-gray-700/80">
          <p className="text-[11px] font-semibold uppercase tracking-widest text-gray-500">
            {title}
          </p>
        </div>
      )}
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="bg-gray-900/50">
              {headers.map((h, i) => (
                <th
                  key={i}
                  className="text-left text-[11px] font-semibold uppercase tracking-wide
                             text-gray-400 px-5 py-3 whitespace-nowrap"
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
                className="border-t border-gray-700/50 hover:bg-gray-700/20 transition-colors"
              >
                {row.map((cell, ci) => (
                  <td
                    key={ci}
                    className={`px-5 py-3 ${
                      ci === 0 ? "font-medium text-gray-200" : "text-gray-400"
                    }`}
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
