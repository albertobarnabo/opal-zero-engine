# axion-stack

The Next.js 14 web UI for Axion — a glassmorphism bento-grid dashboard that streams multi-agent mission results in real time.

## What it does

axion-stack connects to an axion-server instance via SSE and renders structured mission output as an interactive bento grid: metric cards, charts, comparison tables, timelines, and image cards. Supports iterative mission refinement, mission history, file upload, and export (MD/CSV/HTML).

## Key features

- Real-time SSE streaming with typewriter rendering
- Bento grid with MetricCard, ChartCard, ComparisonTable, Timeline, ImageCard
- Iterative refinement — submit follow-up intents without clearing the grid
- Template gallery for common mission types
- Execution trace panel
- Browser notifications on mission completion
- Settings drawer for API key configuration

## Getting started

```bash
cd axion-stack
npm install
npm run dev
```

Set the server URL in the UI settings (default: `http://localhost:3000`).

## Docs

[albertobarnabo.it/axion/docs](https://albertobarnabo.it/axion/docs)

## License

MIT
