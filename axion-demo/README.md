# Axion Demo

A full-featured bento-grid UI for [Axion](https://github.com/albertobarnabo/axion-lab) missions. Stream structured multi-agent output into a live glassmorphism dashboard — metric cards, comparison tables, charts, timelines, and image cards appear in real time as agents complete their work.

Built with [Next.js 15](https://nextjs.org), [Recharts](https://recharts.org), and [`axion-sdk`](https://www.npmjs.com/package/axion-sdk).

![Axion Demo screenshot](https://albertobarnabo.it/axion/og.png)

---

## Prerequisites

You need a running **Axion server** to connect to. Either:

- **Local:** clone [`axion-lab`](https://github.com/albertobarnabo/axion-lab) and run `cargo run -p axion-server`
- **Docker:** `docker run -p 8080:8000 -e OPENAI_API_KEY=sk-... ghcr.io/albertobarnabo/axion-server:latest`

The server needs `OPENAI_API_KEY` set. Optionally set `TAVILY_API_KEY` for live web search.

---

## Quick start

```bash
git clone https://github.com/albertobarnabo/axion-demo
cd axion-demo
npm install
cp .env.example .env.local   # edit if your server is not on localhost:8080
npm run dev
```

Open [http://localhost:3001](http://localhost:3001). Type a mission intent and hit Execute.

---

## Configuration

| Variable | Default | Description |
|---|---|---|
| `NEXT_PUBLIC_AXION_URL` | `http://localhost:8080` | URL of your axion-server instance |

API keys (OpenAI, Tavily, Axion auth) can also be entered in the Settings panel (⚙) at runtime — they are stored in localStorage and sent as request headers, never to any third party.

---

## Using `axion-sdk` in your own app

This demo uses [`axion-sdk`](https://www.npmjs.com/package/axion-sdk) for all server communication. You can embed the same capability in any React app:

```bash
npm install axion-sdk
```

```tsx
import { AxionClient } from 'axion-sdk'
import { useMission } from 'axion-sdk/react'

const client = new AxionClient({ baseUrl: 'http://localhost:8080' })

export function MissionRunner() {
  const { run, status, cards, activeAgent, error } = useMission({ client })

  return (
    <div>
      <button onClick={() => run('Research the best EVs under $50k')} disabled={status === 'running'}>
        {status === 'running' ? `Running… ${activeAgent?.role ?? ''}` : 'Run mission'}
      </button>
      {error && <p style={{ color: 'red' }}>{error}</p>}
      {cards.map(card => (
        <div key={card.key}>
          <strong>{card.widget}</strong>: {JSON.stringify(card.props)}
        </div>
      ))}
    </div>
  )
}
```

`useMission` returns:

| Field | Type | Description |
|---|---|---|
| `run(intent, model?)` | `(string, string?) => Promise<void>` | Execute a new mission |
| `refine(id, intent, model?)` | `(string, string, string?) => Promise<void>` | Refine an existing mission |
| `status` | `"idle" \| "running" \| "complete" \| "failed"` | Current lifecycle state |
| `cards` | `BentoCard[]` | Parsed output cards, ready to render |
| `missionState` | `MissionState \| null` | Raw mission state (build your own renderer) |
| `activeAgent` | `{ role, intent } \| null` | Currently executing agent |
| `missionId` | `string \| null` | ID of the completed mission (use for refine) |
| `error` | `string \| null` | Error message if status is "failed" |
| `reset()` | `() => void` | Clear all state back to idle |

---

## Stack

- **Framework:** Next.js 15 (App Router)
- **Styling:** Tailwind CSS + custom glassmorphism design system
- **Charts:** Recharts
- **SDK:** [`axion-sdk`](https://www.npmjs.com/package/axion-sdk)
- **Animations:** Framer Motion

---

## License

MIT
