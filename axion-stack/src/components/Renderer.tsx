"use client";

import { motion } from "framer-motion";
import { MetricCard } from "./MetricCard";
import { ComparisonTable } from "./ComparisonTable";
import { StatusBadge } from "./StatusBadge";
import { Timeline } from "./Timeline";
import { ChartCard } from "./ChartCard";
import { ImageCard } from "./ImageCard";

export interface UIComponent {
  component_type: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  props: Record<string, any>;
  /** Optional bento-grid span set by the ApplicationMapper. */
  span?: { col: number; row: number };
}

export interface UIBlueprint {
  components: UIComponent[];
  /** Layout strategy hint from the Kernel's DesignTokens. */
  layout_strategy?: string;
}

// ── Span computation ──────────────────────────────────────────────────────────
// Returns the CSS grid span for a component given its type, explicit span
// override, layout strategy, and its index in the component list.

function resolveSpan(
  c: UIComponent,
  index: number,
  strategy: string,
  totalComponents: number,
  hasImageCards: boolean,
): { col: number; row: number } {
  // Honour an explicit span from the ApplicationMapper first.
  if (c.span) return c.span;

  const t = c.component_type;

  switch (strategy) {

    case "Bento-Wide":
      // Comparison-heavy layout: tables full-width, charts wide, metrics small.
      if (t === "ComparisonTable") return { col: 3, row: 1 };
      if (t === "ChartCard")       return { col: 2, row: 1 };
      if (t === "Timeline")        return { col: 2, row: 1 };
      if (t === "ImageCard")       return { col: 1, row: 1 };
      return { col: 1, row: 1 };

    case "Magazine-Flow":
      // Narrative-heavy: first component is a hero, rest flow in 2-col.
      if (index === 0)             return { col: 3, row: 1 };
      if (t === "ComparisonTable") return { col: 2, row: 1 };
      if (t === "ImageCard")       return { col: 2, row: 1 };
      if (t === "ChartCard")       return { col: 3, row: 1 };
      return { col: 1, row: 1 };

    case "FocusOnCharts":
      if (t === "ChartCard")       return { col: 2, row: 1 };
      if (t === "ComparisonTable") return { col: 3, row: 1 };
      return { col: 1, row: 1 };

    default:
      // "Overview" / unset — spatial-aware defaults.
      // Hotel/product pattern: ImageCards + Tables coexist → side-by-side.
      if (hasImageCards && t === "ComparisonTable") return { col: 2, row: 1 };
      if (t === "ComparisonTable") return { col: 3, row: 1 };
      if (t === "ChartCard")       return { col: 3, row: 1 };
      if (t === "Timeline")        return { col: 2, row: 1 };
      if (t === "ImageCard")       return { col: 1, row: 1 };
      if (
        t === "MetricCard" &&
        (String(c.props.value ?? "").length > 15 || c.props.subtitle)
      )                            return { col: 2, row: 1 };
      return { col: 1, row: 1 };
  }
}

// ── Column count ─────────────────────────────────────────────────────────────

function gridColumns(strategy: string): number {
  if (strategy === "Magazine-Flow") return 3; // still 3-col but hero spans it
  return 3;
}

// ── Renderer ──────────────────────────────────────────────────────────────────

export function Renderer({ blueprint }: { blueprint: UIBlueprint }) {
  if (!blueprint.components.length) return null;

  const strategy = blueprint.layout_strategy ?? "Overview";
  const hasImageCards = blueprint.components.some(c => c.component_type === "ImageCard");
  const cols = gridColumns(strategy);
  const total = blueprint.components.length;

  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
        gridAutoFlow: "dense",
        gap: "var(--axion-gap, 28px)",
      }}
    >
      {blueprint.components.map((c, i) => {
        const { col: colSpan, row: rowSpan } = resolveSpan(c, i, strategy, total, hasImageCards);
        const p = c.props;

        const inner = (() => {
          switch (c.component_type) {
            case "MetricCard":
              return (
                <MetricCard
                  title={String(p.title ?? "")}
                  value={p.value as string | number}
                  subtitle={p.subtitle as string | undefined}
                  unit={p.unit as string | undefined}
                  trend={p.trend as "up" | "down" | "neutral" | undefined}
                />
              );
            case "ComparisonTable":
              return (
                <ComparisonTable
                  title={p.title as string | undefined}
                  headers={p.headers as string[]}
                  rows={p.rows as (string | number)[][]}
                />
              );
            case "StatusBadge":
              return (
                <StatusBadge
                  label={String(p.label ?? "")}
                  status={p.status as "info" | "success" | "warning" | "error"}
                  description={p.description as string | undefined}
                />
              );
            case "Timeline":
              return (
                <Timeline
                  title={p.title as string | undefined}
                  steps={
                    p.steps as {
                      label: string;
                      description?: string;
                      time?: string;
                      status?: "completed" | "current" | "upcoming";
                    }[]
                  }
                />
              );
            case "ChartCard":
              return (
                <ChartCard
                  title={p.title as string | undefined}
                  data={p.data as Record<string, string | number>[]}
                  xKey={p.xKey as string | undefined}
                  dataKeys={p.dataKeys as string[] | undefined}
                  chartType={p.chartType as "area" | "bar" | undefined}
                />
              );
            case "ImageCard":
              return (
                <ImageCard
                  title={p.title as string | undefined}
                  description={String(p.description ?? p.value ?? "")}
                />
              );
            default:
              return (
                <div
                  style={{
                    background: "var(--axion-glass-bg, rgba(255,255,255,0.04))",
                    border: "0.5px solid rgba(255,255,255,0.10)",
                    borderRadius: "var(--axion-radius, 24px)",
                    padding: "var(--axion-pad, 20px)",
                    backdropFilter: "blur(var(--axion-blur, 80px))",
                  }}
                >
                  <p className="text-[11px] font-semibold uppercase tracking-widest text-amber-500/70 mb-2">
                    ⚠ Unknown: <code className="font-mono normal-case">{c.component_type}</code>
                  </p>
                  <pre className="text-xs text-gray-400 font-mono overflow-x-auto whitespace-pre-wrap break-words">
                    {JSON.stringify(c.props, null, 2)}
                  </pre>
                </div>
              );
          }
        })();

        const sourceUrl = p.source_url as string | undefined;

        return (
          <motion.div
            key={i}
            initial={{ opacity: 0, y: 20, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            transition={{
              type: "spring",
              stiffness: 240,
              damping: 22,
              delay: i * 0.055,
            }}
            whileHover={{
              scale: 1.018,
              y: -3,
              transition: { type: "spring", stiffness: 400, damping: 28 },
            }}
            style={{
              gridColumn: `span ${colSpan}`,
              gridRow: `span ${rowSpan}`,
              position: "relative",
            }}
            onMouseEnter={(e: React.MouseEvent<HTMLElement>) => {
              e.currentTarget.style.boxShadow =
                "0 20px 56px rgba(0,0,0,0.50), 0 0 44px var(--axion-glow, rgba(139,156,244,0.10))";
              e.currentTarget.style.zIndex = "10";
            }}
            onMouseLeave={(e: React.MouseEvent<HTMLElement>) => {
              e.currentTarget.style.boxShadow = "";
              e.currentTarget.style.zIndex = "";
            }}
          >
            {inner}

            {/* Source citation — floating link icon ──────────────────────── */}
            {sourceUrl && (
              <a
                href={sourceUrl}
                target="_blank"
                rel="noopener noreferrer"
                title={sourceUrl}
                onClick={(e) => e.stopPropagation()}
                style={{
                  position: "absolute",
                  top: 12,
                  right: 12,
                  zIndex: 20,
                  width: 22,
                  height: 22,
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  borderRadius: 6,
                  background: "rgba(255,255,255,0.07)",
                  border: "0.5px solid rgba(255,255,255,0.14)",
                  backdropFilter: "blur(8px)",
                  color: "rgba(255,255,255,0.45)",
                  fontSize: 11,
                  fontWeight: 600,
                  textDecoration: "none",
                  transition: "background 0.2s, color 0.2s",
                }}
                onMouseEnter={(e) => {
                  const el = e.currentTarget as HTMLAnchorElement;
                  el.style.background = "rgba(255,255,255,0.14)";
                  el.style.color = "rgba(255,255,255,0.85)";
                }}
                onMouseLeave={(e) => {
                  const el = e.currentTarget as HTMLAnchorElement;
                  el.style.background = "rgba(255,255,255,0.07)";
                  el.style.color = "rgba(255,255,255,0.45)";
                }}
              >
                ↗
              </a>
            )}
          </motion.div>
        );
      })}
    </div>
  );
}
