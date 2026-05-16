"use client";

import { useState } from "react";
import { motion } from "framer-motion";
import { TEMPLATES } from "../data/templates";

// ── Category filter ───────────────────────────────────────────────────────────

const CATEGORY_TABS = [
  "All",
  "Research",
  "Analysis",
  "Technical",
  "Creative",
] as const;

type CategoryTab = (typeof CATEGORY_TABS)[number];

// ── Props ─────────────────────────────────────────────────────────────────────

interface TemplateGalleryProps {
  /** Called with the selected template's intent string. Do NOT auto-submit. */
  onSelect: (intent: string) => void;
}

// ── Component ─────────────────────────────────────────────────────────────────

export function TemplateGallery({ onSelect }: TemplateGalleryProps) {
  const [activeCategory, setActiveCategory] = useState<CategoryTab>("All");
  const [hoveredId, setHoveredId] = useState<string | null>(null);

  const filtered =
    activeCategory === "All"
      ? TEMPLATES
      : TEMPLATES.filter((t) => t.category === activeCategory);

  return (
    <motion.div
      initial={{ opacity: 0, y: 16 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.35, delay: 0.08 }}
      style={{
        paddingTop: "3rem",
        paddingBottom: "2.5rem",
        paddingLeft: 24,
        paddingRight: 24,
      }}
    >
      <div style={{ maxWidth: 1040, margin: "0 auto", width: "100%" }}>

        {/* Section header */}
        <p
          style={{
            fontSize: 11,
            textTransform: "uppercase",
            letterSpacing: "0.1em",
            color: "rgba(255,255,255,0.28)",
            marginBottom: 14,
            fontWeight: 600,
          }}
        >
          Start with a template
        </p>

        {/* Category filter tabs */}
        <div
          style={{
            display: "flex",
            gap: 6,
            flexWrap: "wrap",
            marginBottom: 18,
          }}
        >
          {CATEGORY_TABS.map((cat) => {
            const active = activeCategory === cat;
            return (
              <button
                key={cat}
                onClick={() => setActiveCategory(cat)}
                style={{
                  fontSize: 11,
                  fontWeight: active ? 600 : 500,
                  padding: "5px 14px",
                  borderRadius: 999,
                  cursor: "pointer",
                  background: active
                    ? "var(--axion-accent, #a7cadc)"
                    : "rgba(255,255,255,0.07)",
                  border: active
                    ? "0.5px solid var(--axion-accent, #a7cadc)"
                    : "0.5px solid rgba(255,255,255,0.12)",
                  color: active ? "var(--axion-accent-fg, #07090c)" : "rgba(255,255,255,0.50)",
                  transition: "background 0.15s, color 0.15s",
                }}
              >
                {cat}
              </button>
            );
          })}
        </div>

        {/* Template cards grid */}
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
            gap: 14,
          }}
        >
          {filtered.map((tpl) => (
            <button
              key={tpl.id}
              onClick={() => onSelect(tpl.intent)}
              onMouseEnter={() => setHoveredId(tpl.id)}
              onMouseLeave={() => setHoveredId(null)}
              style={{
                textAlign: "left",
                borderRadius: 14,
                padding: "20px 22px",
                background: hoveredId === tpl.id
                  ? "linear-gradient(rgba(255,255,255,0.07), rgba(255,255,255,0.03))"
                  : "linear-gradient(rgba(255,255,255,0.04), rgba(255,255,255,0.016))",
                border: "1px solid rgba(255,255,255,0.08)",
                backdropFilter: "blur(28px) saturate(130%)",
                WebkitBackdropFilter: "blur(28px) saturate(130%)",
                boxShadow: "inset 0 1px 0 0 rgba(255,255,255,0.06), 0 0 0 1px rgba(255,255,255,0.04), 0 20px 40px -15px rgba(0,0,0,0.50)",
                cursor: "pointer",
                transition: "background 0.18s",
              }}
            >
              {/* Top row: icon + category label */}
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "space-between",
                }}
              >
                <span style={{ fontSize: 22 }}>{tpl.icon}</span>
                <span
                  style={{
                    fontSize: 10,
                    textTransform: "uppercase",
                    letterSpacing: "0.08em",
                    color: tpl.accent,
                    opacity: 0.75,
                    fontWeight: 600,
                  }}
                >
                  {tpl.category}
                </span>
              </div>

              {/* Title */}
              <p
                style={{
                  fontSize: 14,
                  fontWeight: 700,
                  color: "rgba(255,255,255,0.9)",
                  marginTop: 10,
                  lineHeight: 1.3,
                }}
              >
                {tpl.title}
              </p>

              {/* Description */}
              <p
                style={{
                  fontSize: 12,
                  color: "rgba(255,255,255,0.45)",
                  marginTop: 4,
                  lineHeight: 1.5,
                }}
              >
                {tpl.description}
              </p>

              {/* CTA */}
              <p
                style={{
                  fontSize: 11,
                  color: "var(--axion-accent, #a7cadc)",
                  marginTop: 12,
                  fontWeight: 500,
                }}
              >
                Use template →
              </p>
            </button>
          ))}
        </div>

      </div>
    </motion.div>
  );
}
