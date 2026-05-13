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

const STEP: Record<StepStatus, { dotClass: string; lineClass: string; labelClass: string; useAccent?: boolean }> = {
  completed: {
    dotClass: "bg-emerald-500 border-emerald-500",
    lineClass: "bg-emerald-700/50",
    labelClass: "text-gray-200",
  },
  current: {
    dotClass: "border-2",
    lineClass: "bg-gray-600",
    labelClass: "text-white font-semibold",
    useAccent: true,
  },
  upcoming: {
    dotClass: "bg-transparent border-gray-600",
    lineClass: "bg-gray-700",
    labelClass: "text-gray-400",
  },
};

export function Timeline({ title, steps }: TimelineProps) {
  return (
    <div
      className="border rounded-xl"
      style={{
        background: "var(--axion-glass-bg, rgb(31 41 55 / 0.8))",
        borderColor: "var(--axion-glass-border, rgb(55 65 81))",
        backdropFilter: "blur(var(--axion-blur, 0px))",
        WebkitBackdropFilter: "blur(var(--axion-blur, 0px))",
        padding: "var(--axion-pad, 20px)",
      }}
    >
      {title && (
        <p className="text-[11px] font-semibold uppercase tracking-widest text-gray-500 mb-5">
          {title}
        </p>
      )}
      <div>
        {steps.map((step, i) => {
          const s = STEP[step.status ?? "upcoming"] ?? STEP.upcoming;
          const isLast = i === steps.length - 1;
          return (
            <div key={i} className="flex gap-4">
              {/* spine */}
              <div className="flex flex-col items-center">
                <span
                  className={`w-3 h-3 rounded-full shrink-0 mt-0.5 ${s.dotClass}`}
                  style={
                    s.useAccent
                      ? {
                          backgroundColor: "var(--axion-accent, #6366f1)",
                          borderColor: "var(--axion-accent, #6366f1)",
                          boxShadow: "0 0 8px var(--axion-glow, rgba(99,102,241,0.4))",
                        }
                      : undefined
                  }
                />
                {!isLast && (
                  <span className={`w-0.5 flex-1 my-1 min-h-[16px] ${s.lineClass}`} />
                )}
              </div>
              {/* content */}
              <div className={`${isLast ? "pb-0" : "pb-4"} min-w-0`}>
                <div className="flex items-center gap-2 flex-wrap">
                  <span className={`text-sm ${s.labelClass}`}>{step.label}</span>
                  {step.time && (
                    <span className="text-[11px] text-gray-500 bg-gray-900/60 px-1.5 py-0.5 rounded font-mono">
                      {step.time}
                    </span>
                  )}
                </div>
                {step.description && (
                  <p className="text-xs text-gray-400 mt-1 leading-relaxed">
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
