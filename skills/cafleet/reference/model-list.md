# CAFleet Model List

The catalog of models a Director may pass to `cafleet member create --model`,
with standard API prices and the official sources. The selection policy —
cost efficiency mode, the reviewer rule — lives in
[`roles/director.md`](../roles/director.md) § *Model selection*, not on this
page. The tables are maintained exclusively by the
`cafleet-model-list-refresh` skill from the official sources linked below —
refreshed at least every 30 days (last refreshed: 2026-07-25).
Prices are standard provider USD rates per MTok and are planning estimates,
not an invoice guarantee. Context windows are listed for the `claude` backend,
whose model strings are the ones a context-window suffix can apply to. The
list covers the `claude`, `codex`, and `opencode` backends; opencode models
route through OpenCode Zen and keep the `opencode/` prefix in their `--model`
value. Each backend's table is ordered most → least capable.

## Sources

- [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing.md)
- [OpenAI pricing](https://developers.openai.com/api/docs/pricing)
- [Codex model availability](https://learn.chatgpt.com/docs/models.md)
- [OpenCode Zen models and pricing](https://opencode.ai/docs/zen.md)
- [Claude Code model configuration](https://code.claude.com/docs/en/model-config.md)
  — context windows and `[1m]` applicability for the `claude` backend

## Reviewer defaults

Each backend's current `{reviewer_model}` value — the overlays mirror this
table, and every refresh keeps it in sync with the tables below:

| Backend | Reviewer |
|---|---|
| claude | fable |
| codex | gpt-5.6-sol |
| opencode | opencode/glm-5.2 |

## claude

Either the model name or its alias is a valid `--model` token.

| Model | Alias | Class | Context | Input $/MTok | Output $/MTok |
|---|---|---|---|---|---|
| claude-fable-5 | fable | Mythos-class frontier; highest capability on every dimension | 1M | 10.00 | 50.00 |
| claude-opus-5 | opus | Everyday frontier; strong coding, planning, and review | 1M | 5.00 | 25.00 |
| claude-opus-4-8 | — | Prior frontier generation at the same price tier | 1M | 5.00 | 25.00 |
| claude-sonnet-5 | sonnet | Efficient mid tier for routine work | 1M | 2.00 | 10.00 |
| claude-haiku-4-5 | haiku | Fast low-cost tier; monitoring and quick bounded tasks | 200K | 1.00 | 5.00 |

Every 1M row above runs at that window by default on the Anthropic API, so
its `--model` value needs no `[1m]` suffix; `claude-haiku-4-5` has no 1M
variant and never takes one. The one case that calls for the suffix is an
Opus spawn on a Pro plan, where the 1M window is opt-in and billed to usage
credits — there the Director passes `--model 'claude-opus-5[1m]'`, quoted,
because the brackets otherwise glob in zsh. Confirm the operator's plan
before spending their credits that way.

## codex

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|
| gpt-5.6-sol | Latest frontier agentic coding tier; strongest reviewer | 5.00 | 30.00 |
| gpt-5.5 | Frontier tier for complex coding and research work | 5.00 | 30.00 |
| gpt-5.6-terra | Balanced agentic coding tier for everyday work | 2.50 | 15.00 |
| gpt-5.6-luna | Fast affordable agentic coding tier | 1.00 | 6.00 |

## opencode

A curated subset of the [OpenCode Zen](https://opencode.ai/docs/zen.md)
catalog; the `opencode/` prefix is part of the `--model` value. Any other
`<provider-id>/<model-id>` value remains a manual spawn with explicit
`--coding-agent opencode --model` flags on `member create`. A price of
0.00 is an explicitly free model, currently offered by Zen for a limited
time.

| Model | Class | Input $/MTok | Output $/MTok |
|---|---|---|---|
| opencode/glm-5.2 | Strong general coding tier | 1.40 | 4.40 |
| opencode/kimi-k2.7-code | Strong agentic tier tuned for code | 0.95 | 4.00 |
| opencode/qwen3.5-plus | Efficient mid tier for routine work | 0.20 | 1.20 |
| opencode/big-pickle | Stealth preview model; capability unverified | 0.00 | 0.00 |
| opencode/deepseek-v4-flash-free | Fast light tier for quick bounded tasks | 0.00 | 0.00 |
