# axion-kernel

Premium extensions for the Axion Intelligence Kernel. Provides an OpenAI-backed AI provider and an enhanced Governor with full mission context injection.

## What it adds

- **OpenAIProvider** — production-grade LLM provider using `gpt-4o-mini` (text) and `gpt-4o` (vision), with configurable timeouts and token limits
- **AxionGovernor** — enhanced quality evaluator with five structured rubric criteria and full context injection (intent + task results + prior state)

## Usage

axion-kernel is used server-side when `OPENAI_API_KEY` is present. It slots in as a drop-in replacement for the built-in `BuiltinGovernor` and `SimpleProvider` in axion-core.

## Configuration

Inherits all env vars from axion-core. No additional configuration required.

## Docs

[albertobarnabo.it/axion/docs](https://albertobarnabo.it/axion/docs)

## License

MIT
