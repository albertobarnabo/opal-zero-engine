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

const STEP: Record<StepStatus, { dot: string; line: string; label: string }> = {
  completed: {
    dot: "bg-emerald-500 border-emerald-500",
    line: "bg-emerald-700/50",
    label: "text-gray-200",
  },
  current: {
    dot: "bg-indigo-500 border-indigo-400 ring-2 ring-indigo-500/30",
    line: "bg-gray-600",
    label: "text-white font-semibold",
  },
  upcoming: {
    dot: "bg-transparent border-gray-600",
    line: "bg-gray-700",
    label: "text-gray-400",
  },
};

export function Timeline({ title, steps }: TimelineProps) {
  return (
    <div className="bg-gray-800/80 border border-gray-700 rounded-xl p-5">
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
                  className={`w-3 h-3 rounded-full border-2 shrink-0 mt-0.5 ${s.dot}`}
                />
                {!isLast && (
                  <span className={`w-0.5 flex-1 my-1 min-h-[16px] ${s.line}`} />
                )}
              </div>
              {/* content */}
              <div className={`${isLast ? "pb-0" : "pb-4"} min-w-0`}>
                <div className="flex items-center gap-2 flex-wrap">
                  <span className={`text-sm ${s.label}`}>{step.label}</span>
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
