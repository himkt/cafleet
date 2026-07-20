# CAFleet Model List

The catalog of models a Director may pass to `cafleet member create --model`,
with standard API prices and the official sources. A Director reads this page
before every spawn and applies the selection policy in
[`roles/director.md`](../roles/director.md) § *Model selection* /
[`reference/director.md`](director.md) § *Model selection before member
create*. The tables are maintained exclusively by the
`cafleet-model-list-refresh` skill from the official pricing pages linked
below — refreshed at least every 30 days (last refreshed: 2026-07-20).
Prices are standard provider USD rates per MTok and are planning estimates,
not an invoice guarantee. The list covers the `claude`, `codex`, and
`opencode` backends; opencode models route through OpenCode Zen and keep the
`opencode/` prefix in their `--model` value.

## Sources

- [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing)
- [OpenAI pricing](https://developers.openai.com/api/docs/pricing)
- [OpenCode Zen models and pricing](https://opencode.ai/docs/zen/)

## claude

Rows are ordered most → least capable. Either the model name or its alias is a
valid `--model` token.

| Model | Alias | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|---|
| claude-fable-5 | fable | Mythos-class frontier; highest capability on every dimension | 10.00 | 50.00 |
| claude-opus-4-8 | opus | Everyday frontier; strong coding, planning, and review | 5.00 | 25.00 |
| claude-sonnet-5 | sonnet | Efficient mid tier for routine work | 2.00 | 10.00 |
| claude-haiku-4-5 | haiku | Fast low-cost tier; monitoring and quick bounded tasks | 1.00 | 5.00 |

## codex

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|
| gpt-5.6-sol | Latest frontier agentic coding tier; strongest reviewer | 5.00 | 30.00 |
| gpt-5.5 | Frontier tier for complex coding and research work | 5.00 | 30.00 |
| gpt-5.6-terra | Balanced agentic coding tier for everyday work | 2.50 | 15.00 |
| gpt-5.4 | Strong everyday coding tier | 2.50 | 15.00 |
| gpt-5.6-luna | Fast affordable agentic coding tier | 1.00 | 6.00 |
| gpt-5.4-mini | Fast efficient mini tier for responsive tasks and subagents | 0.75 | 4.50 |

## opencode

A curated subset of the [OpenCode Zen](https://opencode.ai/docs/zen/)
catalog; the `opencode/` prefix is part of the `--model` value. A price of
0.00 is an explicitly free model, currently offered by Zen for a limited
time.

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|
| opencode/glm-5.2 | Strong general coding tier | 1.40 | 4.40 |
| opencode/kimi-k2.7-code | Strong agentic tier tuned for code | 0.95 | 4.00 |
| opencode/qwen3.5-plus | Efficient mid tier for routine work | 0.20 | 1.20 |
| opencode/big-pickle | Stealth preview model; capability unverified | 0.00 | 0.00 |
| opencode/deepseek-v4-flash-free | Fast light tier for quick bounded tasks | 0.00 | 0.00 |
