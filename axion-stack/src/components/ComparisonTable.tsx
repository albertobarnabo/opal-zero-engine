"use client";

import { useState } from "react";

interface ComparisonTableProps {
  title?: string;
  headers: string[];
  rows: (string | number)[][];
}

const cellBorder = "1px solid var(--axion-glass-border, rgba(255,255,255,0.08))";

export function ComparisonTable({ title, headers, rows }: ComparisonTableProps) {
  const [sortCol, setSortCol] = useState<number | null>(null);
  const [sortDir, setSortDir] = useState<"asc" | "desc">("asc");

  function handleHeaderClick(colIndex: number) {
    if (sortCol === colIndex) {
      setSortDir((d) => (d === "asc" ? "desc" : "asc"));
    } else {
      setSortCol(colIndex);
      setSortDir("asc");
    }
  }

  const sortedRows =
    sortCol === null
      ? rows
      : [...rows].sort((a, b) => {
          const av = a[sortCol];
          const bv = b[sortCol];
          const an = parseFloat(String(av).replace(/[^0-9.-]/g, ""));
          const bn = parseFloat(String(bv).replace(/[^0-9.-]/g, ""));
          const cmp =
            !isNaN(an) && !isNaN(bn)
              ? an - bn
              : String(av).localeCompare(String(bv));
          return sortDir === "asc" ? cmp : -cmp;
        });

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
              {headers.map((h, i) => {
                const isActive = sortCol === i;
                const indicator = isActive
                  ? sortDir === "asc"
                    ? "↑"
                    : "↓"
                  : "↕";
                const indicatorColor = isActive
                  ? "var(--axion-accent, #a7cadc)"
                  : "rgba(255,255,255,0.20)";

                return (
                  <th
                    key={i}
                    onClick={() => handleHeaderClick(i)}
                    className="text-left text-[11px] font-semibold uppercase tracking-wide px-6 py-4 whitespace-nowrap select-none"
                    style={{
                      color: isActive
                        ? "var(--axion-accent, #a7cadc)"
                        : i === 0
                        ? "rgba(255,255,255,0.90)"
                        : "rgba(255,255,255,0.55)",
                      cursor: "pointer",
                      transition: "background 0.15s, color 0.15s",
                      userSelect: "none",
                    }}
                    onMouseEnter={(e) => {
                      (e.currentTarget as HTMLTableCellElement).style.background =
                        "rgba(255,255,255,0.04)";
                    }}
                    onMouseLeave={(e) => {
                      (e.currentTarget as HTMLTableCellElement).style.background =
                        "transparent";
                    }}
                  >
                    <span style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
                      {h}
                      <span
                        style={{
                          color: indicatorColor,
                          fontSize: 10,
                          lineHeight: 1,
                          transition: "color 0.15s",
                        }}
                      >
                        {indicator}
                      </span>
                    </span>
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {sortedRows.map((row, ri) => (
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
