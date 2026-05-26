"use client";

import { useState } from "react";

// ── Types ──────────────────────────────────────────────────────────────────────

export interface ModeSelectorProps {
  onSelect: (intent: string) => void;
}

interface Mode {
  id: string;
  label: string;
  icon: string;
  accent: string;
  description: string;
  queries: string[];
}

// ── Mode definitions ───────────────────────────────────────────────────────────

const MODES: Mode[] = [
  {
    id: "due-diligence",
    label: "Due Diligence Brief",
    icon: "🏦",
    accent: "#6ee7b7",
    description: "Rapid multi-source brief on any company or investment target.",
    queries: [
      "Due diligence brief on [Company Name] — business model, financials, moat, red flags.",
      "Investor brief on [Startup Name]: team, traction, market size, valuation.",
      "Risk assessment for acquiring [Company Name] in the [Industry] sector.",
    ],
  },
  {
    id: "market-intel",
    label: "Market Intelligence",
    icon: "📡",
    accent: "#818cf8",
    description: "Live market landscape, sizing, and trend synthesis.",
    queries: [
      "Market intelligence report on [Industry]: size, growth, key players, trends.",
      "Emerging trends in [Sector] for [Year] — signals, drivers, risks.",
      "TAM/SAM/SOM analysis for a [Product Type] targeting [Customer Segment].",
    ],
  },
  {
    id: "competitive",
    label: "Competitive Analysis",
    icon: "⚔️",
    accent: "#fbbf24",
    description: "Map the competitive field around any product or company.",
    queries: [
      "Competitive landscape: [Your Company] vs top 3 rivals — positioning, pricing, moat.",
      "Feature comparison: [Product A] vs [Product B] vs [Product C].",
      "Competitive threats to [Company] from adjacent markets in [Timeframe].",
    ],
  },
  {
    id: "deep-dive",
    label: "Deep-Dive Research",
    icon: "🔬",
    accent: "#f472b6",
    description: "Comprehensive synthesis on any topic, technology, or domain.",
    queries: [
      "Deep dive on [Technology/Topic] — history, state of the art, open problems.",
      "Research synthesis on [Subject]: key findings, debates, future outlook.",
      "Expert-level overview of [Field] for a first-principles understanding.",
    ],
  },
];

// ── Component ──────────────────────────────────────────────────────────────────

export function ModeSelector({ onSelect }: ModeSelectorProps) {
  const [hoveredCard, setHoveredCard] = useState<string | null>(null);
  const [hoveredPill, setHoveredPill] = useState<string | null>(null);

  return (
    <>
      {/* Responsive grid — 2 cols on ≥640 px, 1 col below */}
      <style>{`
        .opalzero-mode-grid {
          display: grid;
          grid-template-columns: repeat(2, 1fr);
          gap: 16px;
          width: 100%;
        }
        @media (max-width: 639px) {
          .opalzero-mode-grid {
            grid-template-columns: 1fr;
          }
        }
      `}</style>

      <div className="opalzero-mode-grid">
        {MODES.map((mode) => {
          const cardHovered = hoveredCard === mode.id;

          return (
            <div
              key={mode.id}
              onMouseEnter={() => setHoveredCard(mode.id)}
              onMouseLeave={() => setHoveredCard(null)}
              style={{
                // Glass base
                background: cardHovered
                  ? "rgba(255,255,255,0.072)"
                  : "rgba(255,255,255,0.042)",
                // Full border — overridden on left below
                border: `0.5px solid ${cardHovered ? "rgba(255,255,255,0.18)" : "rgba(255,255,255,0.10)"}`,
                // Accent left bar (overrides the left side of border above)
                borderLeft: `3px solid ${mode.accent}`,
                borderRadius: 16,
                backdropFilter: "blur(24px)",
                WebkitBackdropFilter: "blur(24px)",
                padding: "22px 20px",
                cursor: "default",
                transition: "background 0.18s, box-shadow 0.18s, border-color 0.18s",
                boxShadow: cardHovered ? `0 0 36px ${mode.accent}18` : "none",
                textAlign: "left",
              }}
            >
              {/* Icon */}
              <div style={{ fontSize: 24, lineHeight: 1, marginBottom: 10 }}>
                {mode.icon}
              </div>

              {/* Mode label */}
              <div
                style={{
                  fontFamily: "var(--opalzero-font-display)",
                  fontSize: 17,
                  fontWeight: 500,
                  color: "rgba(235,239,242,0.95)",
                  letterSpacing: "-0.01em",
                  marginBottom: 6,
                }}
              >
                {mode.label}
              </div>

              {/* Description */}
              <div
                style={{
                  fontSize: 13,
                  color: "rgba(147,153,160,0.78)",
                  lineHeight: 1.45,
                  marginBottom: 14,
                }}
              >
                {mode.description}
              </div>

              {/* Example query pills */}
              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                {mode.queries.map((query, qi) => {
                  const pillKey = `${mode.id}-${qi}`;
                  const pillHovered = hoveredPill === pillKey;

                  return (
                    <button
                      key={pillKey}
                      onMouseEnter={() => setHoveredPill(pillKey)}
                      onMouseLeave={() => setHoveredPill(null)}
                      onClick={() => onSelect(query)}
                      style={{
                        display: "block",
                        width: "100%",
                        textAlign: "left",
                        padding: "5px 11px",
                        borderRadius: 999,
                        border: `0.5px solid ${pillHovered ? `${mode.accent}60` : `${mode.accent}30`}`,
                        background: pillHovered
                          ? `${mode.accent}1a`
                          : `${mode.accent}0e`,
                        color: pillHovered ? mode.accent : `${mode.accent}cc`,
                        fontSize: 12,
                        fontWeight: 400,
                        lineHeight: 1.45,
                        cursor: "pointer",
                        transition: "background 0.15s, color 0.15s, border-color 0.15s",
                        // No text truncation — let queries wrap naturally
                      }}
                    >
                      {query}
                    </button>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>
    </>
  );
}
