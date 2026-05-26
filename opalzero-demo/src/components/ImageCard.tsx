interface ImageCardProps {
  title?: string;
  description: string;
}

// Keyword → theme palette
const THEMES: { keywords: string[]; colors: [string, string]; icon: string }[] = [
  { keywords: ["travel","city","destination","rome","paris","tokyo","hotel","museum","street","coast","beach","mountain","island"],
    colors: ["#0ea5e9","#6366f1"], icon: "🌍" },
  { keywords: ["finance","market","stock","revenue","profit","gdp","economic","fund","invest","bank","price","trade"],
    colors: ["#10b981","#0ea5e9"], icon: "📈" },
  { keywords: ["tech","software","code","computer","digital","ai","data","cloud","system","neural","chip","api"],
    colors: ["#8b5cf6","#6366f1"], icon: "⚡" },
  { keywords: ["nature","forest","ocean","sky","green","plant","earth","climate","energy","solar","wind"],
    colors: ["#10b981","#0d9488"], icon: "🌿" },
  { keywords: ["space","star","galaxy","planet","cosmos","nasa","orbit","rocket","lunar","mars"],
    colors: ["#6366f1","#0ea5e9"], icon: "🚀" },
  { keywords: ["health","medical","science","research","biology","molecule","quantum","physics","lab","study"],
    colors: ["#6ee7b7","#8b5cf6"], icon: "🔬" },
  { keywords: ["art","design","creative","music","film","culture","history","ancient","architecture"],
    colors: ["#f59e0b","#ef4444"], icon: "🎨" },
];

function getTheme(description: string): { colors: [string, string]; icon: string } {
  const d = description.toLowerCase();
  for (const t of THEMES) {
    if (t.keywords.some((k) => d.includes(k))) return { colors: t.colors, icon: t.icon };
  }
  // Deterministic fallback hue from string
  let h = 0;
  for (let i = 0; i < description.length; i++) h = (h * 31 + description.charCodeAt(i)) & 0xffffffff;
  const hue = Math.abs(h) % 360;
  return {
    colors: [`hsl(${hue},55%,55%)`, `hsl(${(hue + 50) % 360},45%,45%)`],
    icon: "◈",
  };
}

export function ImageCard({ title, description }: ImageCardProps) {
  const isUrl =
    typeof description === "string" &&
    (description.startsWith("https://") || description.startsWith("http://"));

  const { colors, icon } = getTheme(description);
  const [c1, c2] = colors;

  return (
    <div
      style={{
        background: "var(--axion-glass-bg)",
        border: "1px solid var(--axion-glass-border)",
        borderRadius: "var(--axion-radius)",
        backdropFilter: "blur(var(--axion-blur)) saturate(130%)",
        WebkitBackdropFilter: "blur(var(--axion-blur)) saturate(130%)",
        boxShadow: "var(--shadow-glass)",
        overflow: "hidden",
        position: "relative",
      }}
    >
      {isUrl ? (
        /* ── URL mode: render actual photo ─────────────────────────────── */
        <div style={{ position: "relative", height: "180px", overflow: "hidden" }}>
          <img
            src={description}
            alt={title ?? ""}
            style={{
              width: "100%",
              height: "180px",
              objectFit: "cover",
              display: "block",
            }}
            onError={(e) => {
              // On broken URL: hide img, reveal the themed gradient fallback
              const img = e.currentTarget;
              img.style.display = "none";
              const fallback = img.nextElementSibling as HTMLElement | null;
              if (fallback) fallback.style.display = "flex";
            }}
          />
          {/* Hidden gradient fallback — shown only if img fails to load */}
          <div
            style={{
              display: "none",
              position: "absolute", inset: 0,
              background: `linear-gradient(135deg, ${c1}22 0%, ${c2}14 60%, transparent 100%)`,
              alignItems: "center", justifyContent: "center",
              fontSize: 52, opacity: 0.22,
            }}
          >
            {icon}
          </div>
        </div>
      ) : (
        /* ── Scene mode: themed gradient placeholder ────────────────────── */
        <div
          style={{
            height: "180px",
            position: "relative",
            overflow: "hidden",
            background: `linear-gradient(135deg, ${c1}22 0%, ${c2}14 60%, transparent 100%)`,
          }}
        >
          {/* Radial glow blobs */}
          <div style={{
            position: "absolute", inset: 0,
            background: `radial-gradient(ellipse 70% 80% at 20% 50%, ${c1}30 0%, transparent 65%),
                         radial-gradient(ellipse 50% 60% at 80% 30%, ${c2}22 0%, transparent 60%)`,
          }} />
          {/* Grid lines for depth */}
          <div style={{
            position: "absolute", inset: 0, opacity: 0.07,
            backgroundImage: `repeating-linear-gradient(0deg, rgba(255,255,255,0.5) 0px, rgba(255,255,255,0.5) 1px, transparent 1px, transparent 32px),
                              repeating-linear-gradient(90deg, rgba(255,255,255,0.5) 0px, rgba(255,255,255,0.5) 1px, transparent 1px, transparent 32px)`,
          }} />
          {/* Centre icon */}
          <div style={{
            position: "absolute", inset: 0,
            display: "flex", alignItems: "center", justifyContent: "center",
            fontSize: 52, opacity: 0.22,
            filter: "drop-shadow(0 0 12px rgba(255,255,255,0.3))",
          }}>
            {icon}
          </div>
          {/* Colour tag top-right */}
          <div style={{
            position: "absolute", top: 12, right: 12,
            width: 8, height: 8, borderRadius: "50%",
            background: c1,
            boxShadow: `0 0 10px ${c1}`,
          }} />
        </div>
      )}

      {/* Caption */}
      <div style={{ padding: "var(--axion-pad, 20px)", paddingTop: "16px" }}>
        {title && (
          <p
            className="text-[11px] font-semibold uppercase tracking-widest mb-1.5"
            style={{ color: "rgba(255,255,255,0.55)", fontFamily: "var(--axion-font-main)" }}
          >
            {title}
          </p>
        )}
        {!isUrl && (
          <p className="axion-scene-text">{description}</p>
        )}
      </div>
    </div>
  );
}
