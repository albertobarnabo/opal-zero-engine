import { MetricCard } from "./MetricCard";
import { ComparisonTable } from "./ComparisonTable";
import { StatusBadge } from "./StatusBadge";
import { Timeline } from "./Timeline";

export interface UIComponent {
  component_type: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  props: Record<string, any>;
}

export interface UIBlueprint {
  components: UIComponent[];
}

export function Renderer({ blueprint }: { blueprint: UIBlueprint }) {
  if (!blueprint.components.length) return null;

  return (
    <div className="space-y-3">
      {blueprint.components.map((c, i) => {
        const p = c.props;
        switch (c.component_type) {
          case "MetricCard":
            return (
              <MetricCard
                key={i}
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
                key={i}
                title={p.title as string | undefined}
                headers={p.headers as string[]}
                rows={p.rows as (string | number)[][]}
              />
            );
          case "StatusBadge":
            return (
              <StatusBadge
                key={i}
                label={String(p.label ?? "")}
                status={p.status as "info" | "success" | "warning" | "error"}
                description={p.description as string | undefined}
              />
            );
          case "Timeline":
            return (
              <Timeline
                key={i}
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
          default:
            // Debug card — renders when the LLM produces a component_type that
            // doesn't exist in the library.  Helps diagnose model hallucinations
            // or mismatched casing without a silent no-op.
            return (
              <div
                key={i}
                className="bg-amber-950/30 border border-amber-700/50 rounded-xl p-4"
              >
                <p className="text-[11px] font-semibold uppercase tracking-widest text-amber-500 mb-2">
                  ⚠ Unknown component type:{" "}
                  <code className="font-mono normal-case">{c.component_type}</code>
                </p>
                <pre className="text-xs text-gray-400 font-mono overflow-x-auto whitespace-pre-wrap break-words">
                  {JSON.stringify(c.props, null, 2)}
                </pre>
              </div>
            );
        }
      })}
    </div>
  );
}
