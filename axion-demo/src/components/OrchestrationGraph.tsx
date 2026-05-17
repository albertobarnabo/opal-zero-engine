"use client";

import { useRef, useEffect, useState } from "react";

// ── Types ──────────────────────────────────────────────────────────────────────
// Mirror of page.tsx StreamCard — kept in sync by comment (not imported to avoid
// a circular dependency through the app-level component).

interface StreamCard {
  slug:    string;
  role:    string;
  intent:  string;
  status:  "running" | "completed" | "failed";
  result?: string;
}

export interface OrchestrationGraphProps {
  cards:       Record<string, StreamCard>;
  cardOrder:   string[];
  activeAgent: { role: string; intent: string } | null | undefined;
  intent:      string;
  /** slug → durationMs, used to show timing badges in trace mode */
  timings?:    Record<string, number>;
  /** When true, renders a compact completed-state trace (no animations, smaller) */
  traceMode?:  boolean;
}

// ── Static lookup tables ───────────────────────────────────────────────────────

const ROLE_ICON: Record<string, string> = {
  WebSearcher: "🔍",
  Analyst:     "📊",
  Coder:       "💻",
  Planner:     "🧠",
};

function roleIcon(role: string): string {
  return ROLE_ICON[role] ?? "⚙️";
}

function shortRole(role: string): string {
  if (role === "WebSearcher") return "Searcher";
  return role;
}

// ── Graph node ────────────────────────────────────────────────────────────────

interface GraphNode {
  key:    string;   // role name for grouped nodes, slug for individual nodes
  role:   string;
  status: "running" | "completed" | "failed" | "pending";
  count:  number;   // 1 = individual, >1 = grouped
  slugs:  string[];
}

/** Build display nodes from cards, grouping roles that appear 3+ times. */
function buildNodes(
  cards:     Record<string, StreamCard>,
  cardOrder: string[],
): GraphNode[] {
  // Count per-role occurrences
  const roleCounts: Record<string, number> = {};
  for (const slug of cardOrder) {
    const role = cards[slug]?.role;
    if (role) roleCounts[role] = (roleCounts[role] ?? 0) + 1;
  }
  const groupedRoles = new Set(
    Object.keys(roleCounts).filter((r) => roleCounts[r] >= 3)
  );

  const nodes: GraphNode[] = [];
  const groupMap = new Map<string, GraphNode>();

  for (const slug of cardOrder) {
    const card = cards[slug];
    if (!card) continue;
    const { role, status } = card;

    if (groupedRoles.has(role)) {
      const existing = groupMap.get(role);
      if (existing) {
        existing.slugs.push(slug);
        existing.count++;
        // running > failed > completed priority
        if (status === "running") {
          existing.status = "running";
        } else if (status === "failed" && existing.status !== "running") {
          existing.status = "failed";
        }
      } else {
        const gn: GraphNode = { key: role, role, status, count: 1, slugs: [slug] };
        groupMap.set(role, gn);
        nodes.push(gn);
      }
    } else {
      nodes.push({ key: slug, role, status, count: 1, slugs: [slug] });
    }
  }

  return nodes;
}

/** Compute (cx, cy) for every node given SVG width and device size. */
function computePositions(
  count:    number,
  svgWidth: number,
  isSmall:  boolean,
): Array<{ cx: number; cy: number }> {
  const maxPerRow = isSmall ? 2 : 7;
  const rowY0     = isSmall ? 190 : 255;
  const rowStep   = isSmall ? 150 : 160;
  const pad       = isSmall ?  60 :  80;
  const positions: Array<{ cx: number; cy: number }> = [];

  for (let i = 0; i < count; i++) {
    const row      = Math.floor(i / maxPerRow);
    const col      = i % maxPerRow;
    const rowCount = Math.min(count - row * maxPerRow, maxPerRow);
    const cx =
      rowCount === 1
        ? svgWidth / 2
        : pad + col * ((svgWidth - 2 * pad) / (rowCount - 1));
    const cy = rowY0 + row * rowStep;
    positions.push({ cx, cy });
  }

  return positions;
}

// ── Component ──────────────────────────────────────────────────────────────────

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

export function OrchestrationGraph({
  cards,
  cardOrder,
  timings,
  traceMode = false,
}: OrchestrationGraphProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [svgWidth, setSvgWidth] = useState(760);

  // Observe container width for responsive layout
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (w && w > 0) setSvgWidth(w);
    });
    ro.observe(el);
    setSvgWidth(el.getBoundingClientRect().width || 760);
    return () => ro.disconnect();
  }, []);

  // ── Derived geometry ─────────────────────────────────────────────────────────
  // traceMode uses smaller nodes to fit compactly above the result grid
  const isSmall = svgWidth < 640 || traceMode;

  const nodeR     = traceMode ? 18 : isSmall ? 20 : 32;
  const nodeInner = traceMode ? 12 : isSmall ? 14 : 24;
  const hubR      = traceMode ? 22 : isSmall ? 26 : 36;
  const hubInner  = traceMode ? 15 : isSmall ? 18 : 28;
  const dotR      = traceMode ?  3 : isSmall ?  4 :  5;
  const labelFS   = traceMode ?  8 : isSmall ?  9 : 10;
  const iconFS    = traceMode ? 11 : isSmall ? 13 : 18;
  const hubIconFS = traceMode ? 14 : isSmall ? 16 : 22;
  const hubCY     = traceMode ? 46 : isSmall ? 58 : 78;
  const hubCX     = svgWidth / 2;

  const nodes     = buildNodes(cards, cardOrder);
  const positions = computePositions(nodes.length, svgWidth, isSmall);

  // Per-node timing: for grouped nodes, sum all slugs' durations
  function nodeTimingMs(node: GraphNode): number | undefined {
    if (!timings) return undefined;
    const total = node.slugs.reduce((acc, s) => acc + (timings[s] ?? 0), 0);
    return total > 0 ? total : undefined;
  }

  // SVG height
  const lastCY = nodes.length > 0
    ? positions[nodes.length - 1].cy
    : (isSmall ? 190 : 255);
  // Extra space for timing label below node label
  const timingLabelExtra = timings ? 14 : 0;
  const svgHeight = traceMode
    ? Math.max(220, lastCY + nodeR + 40 + timingLabelExtra)
    : Math.max(360, lastCY + nodeR + 52 + timingLabelExtra);

  // Activity log
  const recentActivity = [...cardOrder]
    .reverse()
    .slice(0, 5)
    .map((slug) => cards[slug])
    .filter((c): c is StreamCard => Boolean(c));

  // ── Render ───────────────────────────────────────────────────────────────────
  return (
    <div ref={containerRef} style={{ width: "100%", userSelect: "none" }}>

      {/* Keyframe animations injected once per mount */}
      <style>{`
        @keyframes pulse-ring {
          0%,100% { opacity: 1; }
          50%     { opacity: 0.35; }
        }
        @keyframes blink {
          0%,100% { opacity: 1; }
          50%     { opacity: 0.3; }
        }
        @keyframes march {
          to { stroke-dashoffset: -8; }
        }
        @keyframes hub-breathe {
          0%,100% { opacity: 0.55; }
          50%     { opacity: 1; }
        }
      `}</style>

      <svg
        width="100%"
        height={svgHeight}
        viewBox={`0 0 ${svgWidth} ${svgHeight}`}
        style={{ overflow: "visible", display: "block" }}
        aria-label="Live orchestration graph"
      >
        {/* ── Edges — rendered before nodes so nodes always sit on top ──────── */}
        {nodes.map((node, i) => {
          const { cx, cy } = positions[i];
          const isRunning  = node.status === "running";
          return (
            <line
              key={`edge-${node.key}`}
              x1={hubCX}
              y1={hubCY}
              x2={cx}
              y2={cy}
              stroke={isRunning ? "rgba(167,202,220,0.55)" : "rgba(167,202,220,0.18)"}
              strokeWidth={isRunning ? 1.5 : 1}
              strokeDasharray={isRunning && !traceMode ? "4 4" : undefined}
              style={isRunning && !traceMode ? { animation: "march 2s linear infinite" } : undefined}
            />
          );
        })}

        {/* ── Hub node ────────────────────────────────────────────────────────── */}
        <g>
          {/* Outer ring — pulses gently while planning (no tasks yet) */}
          <circle
            cx={hubCX}
            cy={hubCY}
            r={hubR}
            fill="rgba(167,202,220,0.07)"
            stroke="#a7cadc"
            strokeWidth={1.5}
            style={
              nodes.length === 0 && !traceMode
                ? { animation: "hub-breathe 1.8s ease-in-out infinite" }
                : undefined
            }
          />
          {/* Inner fill */}
          <circle cx={hubCX} cy={hubCY} r={hubInner} fill="rgba(167,202,220,0.08)" />
          {/* 🧠 icon */}
          <text
            x={hubCX}
            y={hubCY}
            fontSize={hubIconFS}
            textAnchor="middle"
            dominantBaseline="middle"
          >
            🧠
          </text>
          {/* Label */}
          <text
            x={hubCX}
            y={hubCY + hubR + 14}
            fontSize={11}
            fill="rgba(235,239,242,0.72)"
            textAnchor="middle"
            fontFamily="var(--axion-font-mono, monospace)"
          >
            {nodes.length === 0 ? "Planning…" : "Axion"}
          </text>
        </g>

        {/* ── Agent nodes ─────────────────────────────────────────────────────── */}
        {nodes.map((node, i) => {
          const { cx, cy } = positions[i];
          const s = node.status;

          const stroke =
            s === "running"   ? "#a7cadc" :
            s === "completed" ? "#6ee7b7" :
            s === "failed"    ? "#f87171" :
                                "rgba(255,255,255,0.12)";

          const fill =
            s === "running"   ? "rgba(167,202,220,0.12)" :
            s === "completed" ? "rgba(110,231,183,0.10)"  :
            s === "failed"    ? "rgba(248,113,113,0.10)"  :
                                "rgba(255,255,255,0.04)";

          const dotColor =
            s === "running"   ? "#a7cadc" :
            s === "completed" ? "#6ee7b7" :
            s === "failed"    ? "#f87171" :
                                "transparent";

          // Status dot sits in the top-right quadrant of the outer ring
          const dotCX = cx + nodeR - dotR - 1;
          const dotCY = cy - nodeR + dotR + 1;

          return (
            <g key={`node-${node.key}`}>
              {/* Outer ring */}
              <circle
                cx={cx}
                cy={cy}
                r={nodeR}
                fill="none"
                stroke={stroke}
                strokeWidth={s === "running" ? 2 : 1.5}
                style={
                  s === "running" && !traceMode
                    ? { animation: "pulse-ring 1.4s ease-in-out infinite" }
                    : undefined
                }
              />
              {/* Inner coloured fill */}
              <circle cx={cx} cy={cy} r={nodeInner} fill={fill} />
              {/* Role emoji */}
              <text
                x={cx}
                y={cy}
                fontSize={iconFS}
                textAnchor="middle"
                dominantBaseline="middle"
              >
                {roleIcon(node.role)}
              </text>
              {/* Label (role name + optional count badge) */}
              <text
                x={cx}
                y={cy + nodeR + 13}
                fontSize={labelFS}
                fill="rgba(147,153,160,0.78)"
                textAnchor="middle"
                fontFamily="var(--axion-font-mono, monospace)"
              >
                {node.count > 1
                  ? `${shortRole(node.role)} ×${node.count}`
                  : shortRole(node.role)}
              </text>
              {/* Timing badge — shown in trace mode when duration is known */}
              {(() => {
                const ms = nodeTimingMs(node);
                if (ms === undefined) return null;
                return (
                  <text
                    x={cx}
                    y={cy + nodeR + 13 + labelFS + 3}
                    fontSize={labelFS - 1}
                    fill="rgba(110,231,183,0.60)"
                    textAnchor="middle"
                    fontFamily="var(--axion-font-mono, monospace)"
                  >
                    {formatDuration(ms)}
                  </text>
                );
              })()}
              {/* Status dot — hidden for "pending" nodes */}
              {s !== "pending" && (
                <circle
                  cx={dotCX}
                  cy={dotCY}
                  r={dotR}
                  fill={dotColor}
                  style={
                    s === "running" && !traceMode
                      ? { animation: "blink 0.9s ease-in-out infinite" }
                      : undefined
                  }
                />
              )}
            </g>
          );
        })}
      </svg>

      {/* ── Compact activity log — hidden in trace mode (results already visible) */}
      {recentActivity.length > 0 && !traceMode && (
        <div
          style={{
            marginTop: 10,
            padding: "9px 14px",
            background: "rgba(255,255,255,0.022)",
            border: "0.5px solid rgba(255,255,255,0.07)",
            borderRadius: 10,
            display: "flex",
            flexDirection: "column",
            gap: 5,
            overflow: "hidden",
          }}
        >
          {recentActivity.map((card) => {
            const dot =
              card.status === "running"   ? "#a7cadc" :
              card.status === "completed" ? "#6ee7b7" :
                                            "#f87171";
            const raw   = `${roleIcon(card.role)} ${card.intent}`;
            const label = raw.length > 60 ? `${raw.slice(0, 60)}…` : raw;
            return (
              <div
                key={card.slug}
                style={{
                  display:    "flex",
                  alignItems: "center",
                  gap:        8,
                  overflow:   "hidden",
                }}
              >
                {/* Status colour dot */}
                <span
                  style={{
                    width:      6,
                    height:     6,
                    borderRadius: "50%",
                    background: dot,
                    flexShrink: 0,
                    opacity:    card.status === "running" ? 1 : 0.65,
                  }}
                />
                {/* Monospace one-liner */}
                <span
                  style={{
                    fontFamily:   "var(--axion-font-mono, monospace)",
                    fontSize:     11,
                    color:        "rgba(147,153,160,0.72)",
                    overflow:     "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace:   "nowrap",
                  }}
                >
                  {label}
                </span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
