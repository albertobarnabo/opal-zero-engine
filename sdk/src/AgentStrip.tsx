import type { AgentState, AgentStatus } from "./types.js";

interface AgentStripProps {
  agents: AgentState[];
  style?: React.CSSProperties;
}

export function AgentStrip({ agents, style }: AgentStripProps) {
  if (!agents.length) return null;
  return (
    <div
      style={{
        display: "flex",
        flexWrap: "wrap",
        alignItems: "center",
        gap: "8px",
        ...style,
      }}
    >
      <span
        style={{
          fontSize: "10px",
          textTransform: "uppercase",
          letterSpacing: "0.22em",
          color: "rgba(255,255,255,0.5)",
          marginRight: "4px",
        }}
      >
        Agents
      </span>
      {agents.map((a, i) => (
        <AgentPill key={`${a.slug}-${i}`} agent={a} />
      ))}
    </div>
  );
}

function AgentPill({ agent }: { agent: AgentState }) {
  const isFailed    = agent.status === "failed";
  const isCompleted = agent.status === "completed";
  const isRunning   = agent.status === "running";

  const color = isFailed
    ? "rgba(239,68,68,0.9)"   // red
    : isCompleted
    ? "rgba(255,255,255,0.9)"
    : "rgba(255,255,255,1)";  // running — full white

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "8px",
        background: "rgba(255,255,255,0.06)",
        border: "1px solid rgba(255,255,255,0.08)",
        backdropFilter: "blur(12px)",
        borderRadius: "9999px",
        paddingLeft: "10px",
        paddingRight: "12px",
        paddingTop: "4px",
        paddingBottom: "4px",
        fontSize: "12px",
        color,
      }}
    >
      <StatusIcon status={agent.status} />
      <span style={{ fontWeight: 500, letterSpacing: "-0.01em" }}>{agent.role}</span>
    </div>
  );
}

function StatusIcon({ status }: { status: AgentStatus }) {
  if (status === "running") {
    return (
      <span
        style={{
          position: "relative",
          display: "flex",
          width: "8px",
          height: "8px",
        }}
      >
        <span
          style={{
            position: "absolute",
            inset: 0,
            borderRadius: "50%",
            background: "rgba(99,179,237,1)",
            animation: "axion-pulse 1.5s ease-in-out infinite",
          }}
        />
        <style>{`
          @keyframes axion-pulse {
            0%, 100% { opacity: 1; transform: scale(1); }
            50% { opacity: 0.5; transform: scale(1.4); }
          }
        `}</style>
      </span>
    );
  }

  if (status === "completed") {
    return (
      <span
        style={{
          display: "flex",
          width: "14px",
          height: "14px",
          alignItems: "center",
          justifyContent: "center",
          borderRadius: "50%",
          background: "rgba(52,211,153,0.2)",
          flexShrink: 0,
        }}
      >
        <svg
          width="10"
          height="10"
          viewBox="0 0 10 10"
          fill="none"
          stroke="rgb(52,211,153)"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <polyline points="1.5,5 4,7.5 8.5,2.5" />
        </svg>
      </span>
    );
  }

  // failed
  return (
    <span
      style={{
        display: "flex",
        width: "14px",
        height: "14px",
        alignItems: "center",
        justifyContent: "center",
        borderRadius: "50%",
        background: "rgba(239,68,68,0.2)",
        flexShrink: 0,
      }}
    >
      <svg
        width="10"
        height="10"
        viewBox="0 0 10 10"
        fill="none"
        stroke="rgb(239,68,68)"
        strokeWidth="2"
        strokeLinecap="round"
      >
        <line x1="2" y1="2" x2="8" y2="8" />
        <line x1="8" y1="2" x2="2" y2="8" />
      </svg>
    </span>
  );
}
