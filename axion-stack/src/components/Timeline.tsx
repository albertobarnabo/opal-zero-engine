type StepStatus = "completed" | "current" | "upcoming";

interface TimelineStep {
  label: string;
  description?: string;
  time?: string;
  status?: StepStatus;
}

interface TimelineProps {
  title?: string;
  steps: TimelineStep[];
}

export function Timeline({ title, steps }: TimelineProps) {
  return (
    <div
      style={{
        background: "var(--axion-glass-bg, rgba(255,255,255,0.04))",
        border: "0.5px solid var(--axion-glass-border, rgba(255,255,255,0.10))",
        borderRadius: "var(--axion-radius, 24px)",
        backdropFilter: "blur(var(--axion-blur, 80px))",
        WebkitBackdropFilter: "blur(var(--axion-blur, 80px))",
        boxShadow: "var(--axion-glass-inset, inset 0 1px 0 rgba(255,255,255,0.15), inset 0 0 0 0.5px rgba(255,255,255,0.06))",
        padding: "var(--axion-pad, 20px)",
      }}
    >
      {title && (
        <p
          className="text-[11px] font-semibold uppercase tracking-widest mb-6"
          style={{ color: "rgba(255,255,255,0.55)" }}
        >
          {title}
        </p>
      )}
      <div>
        {steps.map((step, i) => {
          const s = step.status ?? "upcoming";
          const isLast = i === steps.length - 1;

          const dotStyle: React.CSSProperties =
            s === "completed"
              ? { background: "#34d399", borderColor: "#34d399" }
              : s === "current"
              ? {
                  background: "var(--axion-accent, #8b9cf4)",
                  borderColor: "var(--axion-accent, #8b9cf4)",
                  boxShadow: "0 0 12px var(--axion-glow, rgba(139,156,244,0.5))",
                }
              : { background: "transparent", borderColor: "rgba(255,255,255,0.18)" };

          const labelStyle: React.CSSProperties = {
            color:
              s === "completed"
                ? "rgba(255,255,255,0.80)"
                : s === "current"
                ? "rgba(255,255,255,0.95)"
                : "rgba(255,255,255,0.45)",
            fontWeight: s === "current" ? 600 : 400,
          };

          return (
            <div key={i} className="flex gap-4">
              <div className="flex flex-col items-center">
                <span
                  style={{
                    width: 12,
                    height: 12,
                    borderRadius: "50%",
                    border: "2px solid",
                    flexShrink: 0,
                    marginTop: 3,
                    ...dotStyle,
                  }}
                />
                {!isLast && (
                  <span
                    style={{
                      width: 1,
                      flex: 1,
                      margin: "5px 0",
                      minHeight: 18,
                      background:
                        s === "completed"
                          ? "rgba(52,211,153,0.35)"
                          : "rgba(255,255,255,0.10)",
                    }}
                  />
                )}
              </div>
              <div className={`${isLast ? "pb-0" : "pb-5"} min-w-0`}>
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="text-sm" style={labelStyle}>
                    {step.label}
                  </span>
                  {step.time && (
                    <span
                      className="text-[11px] px-2 py-0.5 rounded-md font-mono"
                      style={{
                        color: "rgba(255,255,255,0.45)",
                        background: "rgba(255,255,255,0.07)",
                      }}
                    >
                      {step.time}
                    </span>
                  )}
                </div>
                {step.description && (
                  <p
                    className="text-xs mt-1.5 leading-relaxed"
                    style={{ color: "rgba(255,255,255,0.55)" }}
                  >
                    {step.description}
                  </p>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
