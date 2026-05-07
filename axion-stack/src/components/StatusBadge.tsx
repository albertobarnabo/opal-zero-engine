type BadgeStatus = "info" | "success" | "warning" | "error";

interface StatusBadgeProps {
  label: string;
  status: BadgeStatus;
  description?: string;
}

const STYLES: Record<
  BadgeStatus,
  { bg: string; border: string; dot: string; text: string }
> = {
  info: {
    bg: "bg-blue-950/50",
    border: "border-blue-700/60",
    dot: "bg-blue-400",
    text: "text-blue-300",
  },
  success: {
    bg: "bg-emerald-950/50",
    border: "border-emerald-700/60",
    dot: "bg-emerald-400",
    text: "text-emerald-300",
  },
  warning: {
    bg: "bg-amber-950/50",
    border: "border-amber-700/60",
    dot: "bg-amber-400",
    text: "text-amber-300",
  },
  error: {
    bg: "bg-red-950/50",
    border: "border-red-700/60",
    dot: "bg-red-400",
    text: "text-red-300",
  },
};

export function StatusBadge({ label, status, description }: StatusBadgeProps) {
  const s = STYLES[status] ?? STYLES.info;
  return (
    <div className={`${s.bg} border ${s.border} rounded-xl px-5 py-4`}>
      <div className="flex items-center gap-2.5">
        <span className={`w-2 h-2 rounded-full shrink-0 ${s.dot}`} />
        <span className={`font-semibold text-sm ${s.text}`}>{label}</span>
      </div>
      {description && (
        <p className="text-xs text-gray-400 mt-2 pl-[18px]">{description}</p>
      )}
    </div>
  );
}
