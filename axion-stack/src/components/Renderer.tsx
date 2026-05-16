"use client";

import { motion, AnimatePresence } from "framer-motion";
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

    // "Bento-Wide" kept for legacy; "DataHeavy" is the canonical manifest value
    case "Bento-Wide":
    case "DataHeavy":
      if (t === "ComparisonTable") return { col: 3, row: 1 };
      if (t === "ChartCard")       return { col: 2, row: 1 };
      if (t === "Timeline")        return { col: 2, row: 1 };
      if (t === "ImageCard")       return { col: 1, row: 1 };
      return { col: 1, row: 1 };

    // "Magazine-Flow" kept for legacy; "Narrative" is the canonical manifest value
    case "Magazine-Flow":
    case "Narrative":
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

interface RendererProps {
  blueprint: UIBlueprint;
  pinnedCards?: Set<number>;
  dismissedCards?: Set<number>;
  /** Indices of cards that were added/updated by a refinement pass. */
  refinedIndices?: Set<number>;
  onPin?: (index: number) => void;
  onDismiss?: (index: number) => void;
}

export function Renderer({
  blueprint,
  pinnedCards = new Set(),
  dismissedCards = new Set(),
  refinedIndices = new Set(),
  onPin,
  onDismiss,
}: RendererProps) {
  if (!blueprint.components.length) return null;

  const strategy = blueprint.layout_strategy ?? "Overview";
  const hasImageCards = blueprint.components.some(c => c.component_type === "ImageCard");
  const cols = gridColumns(strategy);
  const total = blueprint.components.length;

  // Sort so pinned cards appear first, then normal, with dismissed excluded
  const indices = Array.from({ length: blueprint.components.length }, (_, i) => i);
  const sortedIndices = [
    ...indices.filter((i) => pinnedCards.has(i) && !dismissedCards.has(i)),
    ...indices.filter((i) => !pinnedCards.has(i) && !dismissedCards.has(i)),
  ];
  const dismissed = indices.filter((i) => dismissedCards.has(i));

  return (
    <>
    <div
      style={{
        display: "grid",
        gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))`,
        gridAutoFlow: "dense",
        gap: "var(--axion-gap, 28px)",
      }}
    >
      <AnimatePresence mode="popLayout">
      {sortedIndices.map((i) => {
        const c = blueprint.components[i];
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
                  description={String(
                    p.image_url ?? p.photo_url ?? p.url ?? p.src ?? p.description ?? p.value ?? ""
                  )}
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
        const isPinned   = pinnedCards.has(i);
        const isRefined  = refinedIndices.has(i);

        return (
          <motion.div
            key={i}
            layout
            initial={{ opacity: 0, y: 20, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, scale: 0.93, y: -12, transition: { duration: 0.22 } }}
            transition={{ type: "spring", stiffness: 240, damping: 22, delay: i * 0.055 }}
            className={`axion-grain${isRefined ? " axion-refined-card" : ""}`}
            style={{
              gridColumn: `span ${colSpan}`,
              gridRow: `span ${rowSpan}`,
              position: "relative",
              transformStyle: "preserve-3d",
              willChange: "transform",
              transition: "box-shadow 0.2s",
              // Pinned cards get a subtle accent ring
              outline: isPinned ? "1.5px solid var(--axion-accent, rgba(167,202,220,0.60))" : "none",
              outlineOffset: 2,
            }}
            onMouseMove={(e: React.MouseEvent<HTMLDivElement>) => {
              const r = e.currentTarget.getBoundingClientRect();
              const x = (e.clientX - r.left) / r.width  - 0.5;
              const y = (e.clientY - r.top)  / r.height - 0.5;
              e.currentTarget.style.transform =
                `perspective(700px) rotateY(${x * 7}deg) rotateX(${-y * 7}deg) translateZ(6px)`;
              e.currentTarget.style.boxShadow =
                "0 24px 60px rgba(0,0,0,0.55), 0 0 44px var(--axion-glow, rgba(167,202,220,0.12))";
              e.currentTarget.style.zIndex = "10";
            }}
            onMouseLeave={(e: React.MouseEvent<HTMLDivElement>) => {
              e.currentTarget.style.transform = "";
              e.currentTarget.style.boxShadow = "";
              e.currentTarget.style.zIndex = "";
            }}
          >
            {inner}

            {/* ── Refined badge ─────────────────────────────────────────────── */}
            {isRefined && (
              <span className="axion-refined-badge">updated</span>
            )}

            {/* ── Bento-Snap controls (pin + dismiss) — revealed on hover ─── */}
            {(onPin || onDismiss) && (
              <div
                className="bento-snap-controls"
                style={{
                  position: "absolute",
                  top: 8,
                  left: 10,
                  zIndex: 20,
                  display: "flex",
                  gap: 4,
                  opacity: 0,
                  transition: "opacity 0.18s",
                  pointerEvents: "none",
                }}
              >
                {/* Pin button */}
                {onPin && (
                  <button
                    onClick={(e) => { e.stopPropagation(); onPin(i); }}
                    title={isPinned ? "Unpin card" : "Pin card to top"}
                    style={{
                      width: 22, height: 22, borderRadius: 6,
                      display: "flex", alignItems: "center", justifyContent: "center",
                      background: isPinned ? "rgba(var(--axion-accent-rgb,167,202,220),0.20)" : "rgba(255,255,255,0.08)",
                      border: isPinned ? "1px solid rgba(var(--axion-accent-rgb,167,202,220),0.45)" : "0.5px solid rgba(255,255,255,0.15)",
                      color: isPinned ? "var(--axion-accent,#a7cadc)" : "rgba(255,255,255,0.50)",
                      fontSize: 10, fontWeight: 800,
                      cursor: "pointer",
                      backdropFilter: "blur(8px)",
                    }}
                  >
                    {isPinned ? "◈" : "◉"}
                  </button>
                )}
                {/* Dismiss button */}
                {onDismiss && (
                  <button
                    onClick={(e) => { e.stopPropagation(); onDismiss(i); }}
                    title="Dismiss card"
                    style={{
                      width: 22, height: 22, borderRadius: 6,
                      display: "flex", alignItems: "center", justifyContent: "center",
                      background: "rgba(255,255,255,0.08)",
                      border: "0.5px solid rgba(255,255,255,0.15)",
                      color: "rgba(255,255,255,0.40)",
                      fontSize: 10, fontWeight: 800,
                      cursor: "pointer",
                      backdropFilter: "blur(8px)",
                    }}
                  >
                    ✕
                  </button>
                )}
              </div>
            )}

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
      </AnimatePresence>
    </div>

    {/* Dismissed cards tray — shows count of hidden cards with restore option */}
    {dismissed.length > 0 && (
      <motion.div
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        style={{
          marginTop: 12,
          padding: "8px 14px",
          borderRadius: 10,
          background: "rgba(255,255,255,0.04)",
          border: "0.5px solid rgba(255,255,255,0.08)",
          display: "flex",
          alignItems: "center",
          gap: 10,
        }}
      >
        <span style={{ fontSize: 10, color: "rgba(255,255,255,0.28)", flex: 1 }}>
          {dismissed.length} card{dismissed.length !== 1 ? "s" : ""} hidden
        </span>
        <button
          onClick={() => {
            // Signal parent to clear dismissed — handled via onDismiss with a sentinel
            // For now we emit onPin on all dismissed IDs to bubble the restore intent
            if (onDismiss) dismissed.forEach((idx) => onDismiss(-idx - 1)); // negative idx = restore signal
          }}
          style={{
            fontSize: 10, color: "rgba(255,255,255,0.45)", cursor: "pointer",
            padding: "4px 10px", borderRadius: 6,
            background: "rgba(255,255,255,0.06)",
            border: "0.5px solid rgba(255,255,255,0.10)",
            transition: "color 0.15s, background 0.15s",
          }}
        >
          Restore all
        </button>
      </motion.div>
    )}
    </>
  );
}
