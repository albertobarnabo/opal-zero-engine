// ── Mission Templates ─────────────────────────────────────────────────────────
// Curated quick-start templates for the idle gallery.
// 12 templates across 4 categories: Research, Finance, Engineering, Strategy.

export interface MissionTemplate {
  id: string;
  category: "Research" | "Analysis" | "Technical" | "Creative";
  title: string;
  description: string;   // one sentence, shown on the card
  intent: string;        // pre-fill text; use [BRACKETS] for user-customizable slots
  accent: string;        // CSS color string matching the existing card accent pattern
  icon: string;          // single emoji
}

export const TEMPLATES: MissionTemplate[] = [
  // Research (3)
{ id: "competitive-analysis", category: "Research",
  title: "Competitive Analysis",
  description: "Map the landscape around any product or company.",
  intent: "Competitive analysis: [Your Company/Product] vs its top 3 competitors. Cover positioning, pricing, strengths, weaknesses, and market share.",
  accent: "#6ee7b7", icon: "🔍" },

{ id: "market-sizing", category: "Research",
  title: "Market Sizing",
  description: "Estimate TAM/SAM/SOM for any industry.",
  intent: "Market sizing for [Industry] in [Year]. Estimate TAM, SAM, and SOM with methodology, data sources, and key assumptions.",
  accent: "#6ee7b7", icon: "📊" },

{ id: "topic-deep-dive", category: "Research",
  title: "Topic Deep-Dive",
  description: "Comprehensive research synthesis on any subject.",
  intent: "Research the current state of [Topic]. Cover history, key players, recent developments, open questions, and future outlook.",
  accent: "#6ee7b7", icon: "🌐" },

// Analysis (3)
{ id: "investment-thesis", category: "Analysis",
  title: "Investment Thesis",
  description: "Build a structured thesis on any asset or company.",
  intent: "Build an investment thesis for [Company or Asset]. Cover business model, financials, competitive moat, risks, and valuation.",
  accent: "#fbbf24", icon: "📈" },

{ id: "tech-evaluation", category: "Analysis",
  title: "Technology Evaluation",
  description: "Compare two technologies for a specific use case.",
  intent: "Technology evaluation: [Tech A] vs [Tech B] for [use case]. Cover performance, ecosystem, learning curve, cost, and a final recommendation.",
  accent: "#fbbf24", icon: "⚖️" },

{ id: "decision-analysis", category: "Analysis",
  title: "Decision Analysis",
  description: "Structured pros/cons and recommendation for any decision.",
  intent: "Analyse the decision: [Decision]. Context: [Context]. Map pros, cons, second-order effects, and give a final recommendation with confidence level.",
  accent: "#fbbf24", icon: "🧭" },

// Technical (3)
{ id: "system-architecture", category: "Technical",
  title: "System Architecture",
  description: "Design a system architecture for any problem.",
  intent: "Architect a system for [Problem Description]. Define components, data flows, technology choices, trade-offs, and an implementation roadmap.",
  accent: "#818cf8", icon: "🏗️" },

{ id: "code-review", category: "Technical",
  title: "Code Review Plan",
  description: "Produce a structured review and improvement plan.",
  intent: "Code review and improvement plan for a [Language] [component/service] that does [description]. Cover correctness, performance, security, and maintainability.",
  accent: "#818cf8", icon: "🔧" },

{ id: "debug-plan", category: "Technical",
  title: "Debug & Fix Strategy",
  description: "Systematic debugging plan for any technical problem.",
  intent: "Debug and fix: [Problem Description]. Stack: [Tech Stack]. Hypothesise root causes, propose diagnostic steps, and outline the fix strategy.",
  accent: "#818cf8", icon: "🐛" },

// Creative (3)
{ id: "research-report", category: "Creative",
  title: "Research Report",
  description: "A publishable report with charts and narrative.",
  intent: "Write a comprehensive research report on [Topic]. Include an executive summary, key findings with data, visualisations, and actionable conclusions.",
  accent: "#f472b6", icon: "📝" },

{ id: "business-plan", category: "Creative",
  title: "Business Plan",
  description: "Full business plan for any venture idea.",
  intent: "Create a business plan for [Business Idea]. Cover problem, solution, market, GTM strategy, revenue model, team needs, and 12-month milestones.",
  accent: "#f472b6", icon: "🚀" },

{ id: "insights-synthesis", category: "Creative",
  title: "Insights Synthesis",
  description: "Turn raw research into clear, actionable insights.",
  intent: "Synthesise research on [Topic] into 5–7 actionable insights. For each insight: evidence, implication, and recommended action.",
  accent: "#f472b6", icon: "💡" },
];
