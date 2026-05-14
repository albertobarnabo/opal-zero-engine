# axion-sdk (Python)

Python client for the [Axion Intelligence Kernel](https://github.com/albertobarnabo/axion).

## Install

```bash
pip install axion-sdk
```

## Quick start

```python
import asyncio
from axion import AxionClient

async def main():
    async with AxionClient(base_url="http://localhost:8000") as axion:
        async for event in axion.execute("Analyse the EV market in Europe"):
            if event.type == "task_completed":
                print(f"[{event.role}] {event.result[:120]}")
            if event.type == "mission_complete":
                print("Mission complete:", event.mission_state.data_payload)

asyncio.run(main())
```

## Self-host

See [docker-compose.yml](../docker-compose.yml) in the monorepo root.
