"use client";

import { useRef, useEffect } from "react";

interface ClarifyQuestion {
  key:         string;
  question:    string;
  required:    boolean;
  placeholder: string;
}

interface ClarifyModalProps {
  intent:          string;
  questions:       ClarifyQuestion[];
  answers:         Record<string, string>;
  onAnswerChange:  (key: string, value: string) => void;
  onSubmit:        () => void;
  onSkip:          () => void;
}

export function ClarifyModal({
  intent,
  questions,
  answers,
  onAnswerChange,
  onSubmit,
  onSkip,
}: ClarifyModalProps) {
  const firstInputRef = useRef<HTMLInputElement>(null);

  // Focus the first input when the modal mounts.
  useEffect(() => {
    const t = setTimeout(() => firstInputRef.current?.focus(), 60);
    return () => clearTimeout(t);
  }, []);

  const canSubmit = questions
    .filter((q) => q.required)
    .every((q) => answers[q.key]?.trim());

  function handleKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && canSubmit) {
      onSubmit();
    }
    if (e.key === "Escape") {
      onSkip();
    }
  }

  return (
    <div className="clarify-overlay" onKeyDown={handleKeyDown}>
      <div className="clarify-card" role="dialog" aria-modal="true" aria-label="Clarify intent">

        {/* Header */}
        <p style={{
          fontSize: 11,
          fontWeight: 700,
          textTransform: "uppercase",
          letterSpacing: "0.10em",
          color: "rgba(255,255,255,0.35)",
          marginBottom: 10,
        }}>
          A few quick questions
        </p>

        {/* Intent preview */}
        <div className="clarify-intent-preview">
          &ldquo;{intent}&rdquo;
        </div>

        {/* Questions */}
        <div style={{ display: "flex", flexDirection: "column", gap: 18, marginBottom: 24 }}>
          {questions.map((q, i) => (
            <div key={q.key} className="clarify-question">
              <label htmlFor={`clarify-${q.key}`}>
                {q.question}
                {q.required && <span className="clarify-required-star" aria-hidden="true">*</span>}
              </label>
              <input
                id={`clarify-${q.key}`}
                ref={i === 0 ? firstInputRef : undefined}
                type="text"
                value={answers[q.key] ?? ""}
                onChange={(e) => onAnswerChange(q.key, e.target.value)}
                placeholder={q.placeholder}
                autoComplete="off"
              />
            </div>
          ))}
        </div>

        {/* Footer */}
        <div style={{ display: "flex", gap: 10, justifyContent: "flex-end", alignItems: "center" }}>
          <span style={{ fontSize: 11, color: "rgba(255,255,255,0.22)", marginRight: "auto" }}>
            ⌘↵ to continue
          </span>
          <button
            onClick={onSkip}
            style={{
              padding: "8px 16px",
              borderRadius: 8,
              background: "transparent",
              border: "1px solid rgba(255,255,255,0.12)",
              color: "rgba(255,255,255,0.45)",
              fontSize: 13,
              fontWeight: 500,
              cursor: "pointer",
              transition: "border-color 0.15s, color 0.15s",
            }}
            onMouseEnter={(e) => {
              (e.currentTarget as HTMLButtonElement).style.borderColor = "rgba(255,255,255,0.28)";
              (e.currentTarget as HTMLButtonElement).style.color       = "rgba(255,255,255,0.70)";
            }}
            onMouseLeave={(e) => {
              (e.currentTarget as HTMLButtonElement).style.borderColor = "rgba(255,255,255,0.12)";
              (e.currentTarget as HTMLButtonElement).style.color       = "rgba(255,255,255,0.45)";
            }}
          >
            Skip, just run it
          </button>
          <button
            onClick={onSubmit}
            disabled={!canSubmit}
            style={{
              padding: "8px 20px",
              borderRadius: 8,
              background: canSubmit
                ? "var(--opalzero-accent, #a7cadc)"
                : "rgba(255,255,255,0.07)",
              border: "none",
              color: canSubmit
                ? "var(--opalzero-accent-fg, #07090c)"
                : "rgba(255,255,255,0.25)",
              fontSize: 13,
              fontWeight: 700,
              cursor: canSubmit ? "pointer" : "not-allowed",
              transition: "background 0.15s, color 0.15s",
              boxShadow: canSubmit
                ? "0 0 16px rgba(167,202,220,0.25)"
                : "none",
            }}
          >
            Continue →
          </button>
        </div>
      </div>
    </div>
  );
}
