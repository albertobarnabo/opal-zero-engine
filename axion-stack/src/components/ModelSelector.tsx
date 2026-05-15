"use client";

import { useState, useRef, useEffect } from "react";
import { MODEL_CATALOG, type ModelId } from "../data/models";

interface ModelSelectorProps {
  value: ModelId;
  onChange: (id: ModelId) => void;
  disabled?: boolean;
}

export function ModelSelector({ value, onChange, disabled = false }: ModelSelectorProps) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  const selected = MODEL_CATALOG.find((m) => m.id === value) ?? MODEL_CATALOG[0];

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    function handleClick(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [open]);

  return (
    <div ref={containerRef} style={{ position: "relative", display: "inline-block" }}>
      {/* Trigger button */}
      <button
        className="model-selector-trigger"
        onClick={() => { if (!disabled) setOpen((v) => !v); }}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        style={{ opacity: disabled ? 0.45 : 1, cursor: disabled ? "not-allowed" : "pointer" }}
      >
        <span style={{ fontWeight: 600, fontSize: 12 }}>{selected.label}</span>
        {selected.badge && (
          <span className="model-badge">{selected.badge}</span>
        )}
        <svg
          width="10"
          height="10"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.5"
          style={{
            marginLeft: 2,
            opacity: 0.45,
            transform: open ? "rotate(180deg)" : "none",
            transition: "transform 0.15s",
            flexShrink: 0,
          }}
        >
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>

      {/* Dropdown */}
      {open && (
        <div className="model-selector-dropdown" role="listbox">
          {MODEL_CATALOG.map((m) => (
            <div
              key={m.id}
              className="model-option"
              role="option"
              aria-selected={m.id === value}
              data-selected={m.id === value ? "true" : "false"}
              onMouseDown={(e) => {
                e.preventDefault(); // prevent textarea blur
                onChange(m.id as ModelId);
                setOpen(false);
              }}
            >
              {/* Row 1: name + badge */}
              <div style={{ display: "flex", alignItems: "center", gap: 7, marginBottom: 2 }}>
                <span style={{ fontWeight: 600, fontSize: 13, color: "rgba(255,255,255,0.90)" }}>
                  {m.label}
                </span>
                {m.badge && <span className="model-badge">{m.badge}</span>}
                {m.id === value && (
                  <span style={{
                    marginLeft: "auto",
                    fontSize: 10,
                    color: "var(--axion-accent, #a7cadc)",
                    fontWeight: 700,
                  }}>
                    ✓
                  </span>
                )}
              </div>
              {/* Row 2: description */}
              <div style={{ fontSize: 11, color: "rgba(255,255,255,0.45)", marginBottom: 3 }}>
                {m.description}
              </div>
              {/* Row 3: cost */}
              <div className="model-option-cost">
                ${m.inputPer1M.toFixed(2)} in&nbsp;/&nbsp;${m.outputPer1M.toFixed(2)} out per 1M tokens
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
