type BadgeStatus = "info" | "success" | "warning" | "error";

interface StatusBadgeProps {
  label: string;
  status: BadgeStatus;
  description?: string;
}

const DOT_COLOR: Record<BadgeStatus, string> = {
  info:    "#60a5fa",
  success: "#34d399",
  warning: "#fbbf24",
  error:   "#f87171",
};

const LABEL_COLOR: Record<BadgeStatus, string> = {
  info:    "rgba(147,197,253,0.95)",
  success: "rgba(110,231,183,0.95)",
  warning: "rgba(252,211,77,0.95)",
  error:   "rgba(252,165,165,0.95)",
};

export function StatusBadge({ label, status, description }: StatusBadgeProps) {
  return (
    <div
      style={{
        background: "var(--axion-glass-bg)",
        border: "1px solid var(--axion-glass-border)",
        borderRadius: "var(--axion-radius)",
        backdropFilter: "blur(var(--axion-blur)) saturate(130%)",
        WebkitBackdropFilter: "blur(var(--axion-blur)) saturate(130%)",
        boxShadow: "var(--shadow-glass)",
        padding: "var(--axion-pad, 20px)",
      }}
    >
      <div className="flex items-center gap-3">
        <span
          style={{
            width: 10,
            height: 10,
            borderRadius: "50%",
            flexShrink: 0,
            background: DOT_COLOR[status],
            boxShadow: `0 0 10px ${DOT_COLOR[status]}99`,
          }}
        />
        <span
          className="font-semibold text-base"
          style={{ color: LABEL_COLOR[status] }}
        >
          {label}
        </span>
      </div>
      {description && (
        <p
          className="text-sm mt-2.5 pl-[22px] leading-relaxed"
          style={{ color: "rgba(255,255,255,0.60)" }}
        >
          {description}
        </p>
      )}
    </div>
  );
}
