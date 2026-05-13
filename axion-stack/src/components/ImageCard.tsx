interface ImageCardProps {
  title?: string;
  description: string;
}

// Deterministic hue from a string for varied gradient anchors
function stringToHue(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) & 0xffffffff;
  return Math.abs(h) % 360;
}

export function ImageCard({ title, description }: ImageCardProps) {
  const hue = stringToHue(description);
  const hue2 = (hue + 40) % 360;

  return (
    <div
      style={{
        background: "var(--axion-glass-bg, rgba(255,255,255,0.04))",
        border: "0.5px solid var(--axion-glass-border, rgba(255,255,255,0.10))",
        borderRadius: "var(--axion-radius, 24px)",
        backdropFilter: "blur(var(--axion-blur, 80px))",
        WebkitBackdropFilter: "blur(var(--axion-blur, 80px))",
        boxShadow: "var(--axion-glass-inset, inset 0 1px 0 rgba(255,255,255,0.15), inset 0 0 0 0.5px rgba(255,255,255,0.06))",
        overflow: "hidden",
        position: "relative",
      }}
    >
      {/* Gradient visual placeholder */}
      <div
        style={{
          height: "160px",
          background: `radial-gradient(ellipse 80% 70% at 30% 40%,
            hsla(${hue},40%,60%,0.25) 0%,
            hsla(${hue2},35%,40%,0.14) 55%,
            transparent 100%)`,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        <span style={{ fontSize: 44, opacity: 0.16 }}>◈</span>
      </div>

      {/* Caption */}
      <div style={{ padding: "var(--axion-pad, 20px)", paddingTop: "16px" }}>
        {title && (
          <p
            className="text-[11px] font-semibold uppercase tracking-widest mb-1.5"
            style={{ color: "rgba(255,255,255,0.55)" }}
          >
            {title}
          </p>
        )}
        <p
          className="text-sm leading-relaxed"
          style={{ color: "rgba(255,255,255,0.60)" }}
        >
          {description}
        </p>
      </div>
    </div>
  );
}
