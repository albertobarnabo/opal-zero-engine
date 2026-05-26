# opalzero-sdk (Python)

Python client for the [OpalZero Intelligence Kernel](https://github.com/albertobarnabo/opalzero).

## Install

```bash
pip install opalzero-sdk
```

## Quick start

```python
import asyncio
from opalzero import OpalZeroClient

async def main():
    async with OpalZeroClient(base_url="http://localhost:8000") as opalzero:
        async for event in opalzero.execute("Analyse the EV market in Europe"):
            if event.type == "task_completed":
                print(f"[{event.role}] {event.result[:120]}")
            if event.type == "mission_complete":
                print("Mission complete:", event.mission_state.data_payload)

asyncio.run(main())
```

## Self-host

See [docker-compose.yml](../docker-compose.yml) in the monorepo root.
