# axion-stack

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Node](https://img.shields.io/badge/node-18%2B-green.svg)

**Real-time bento-grid UI for Axion missions. Stream structured agent output into an interactive glassmorphism dashboard — no configuration required.**

axion-stack connects to an [axion-server](https://github.com/albertobarnabo/axion-engine) instance via SSE and renders the structured results of multi-agent missions as a live bento grid. Metric cards, comparison tables, charts, timelines, and image cards snap into place as agents complete their tasks. When the mission finishes you can refine it, export it, or archive it — all from the same interface.

---

## Getting started

```bash
git clone https://github.com/albertobarnabo/axion-ui
cd axion-ui
npm install
npm run dev
```

Open [http://localhost:3000](http://localhost:3000). That's it.

Point the UI at your axion-server instance from the **Settings** drawer (gear icon, bottom-left), or set it at build time:

```bash
NEXT_PUBLIC_AXION_URL=http://your-server:8080 npm run dev
```

---

## Features

- **Live SSE streaming** — task cards appear and update in real time as each agent finishes; no polling, no page refresh
- **Bento grid renderer** — MetricCard, ChartCard, ComparisonTable, Timeline, and ImageCard components auto-layout based on the mission payload
- **Iterative refinement** — submit a follow-up intent without clearing the grid; new findings merge in and new cards highlight automatically
- **Execution trace panel** — expand any mission to see every agent's raw output, timing, and status
- **Template gallery** — one-click presets for common mission types (market research, travel planning, competitive analysis, code audit)
- **Export** — download mission results as Markdown, CSV, or styled HTML
- **File upload** — attach images or data files (CSV, JSON, TXT) for agents to analyse
- **Browser notifications** — get notified when a long-running mission completes, even if the tab is in the background
- **API key drawer** — configure `OPENAI_API_KEY` and `TAVILY_API_KEY` from the UI without touching environment files

---

## Stack

Next.js 14 · TypeScript · Tailwind CSS · Framer Motion · Recharts

---

## License

MIT

⭐ Star the repo if the UI saves you from building a dashboard from scratch.
