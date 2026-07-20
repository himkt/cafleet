"""Minimal model-list Markdown fixture builders shared by domain and CLI tests."""

from datetime import UTC, datetime

MODELS_HEADER = (
    "| Backend | Model | Aliases | Active | Rank "
    "| Cod | Pln | Rsc | Rev | Mon "
    "| In | Cached | Write | Out | Max tokens |"
)
MODELS_SEPARATOR = "|" + "---|" * 15

# The base fixture's sources are retrieved 2026-07-19; this instant keeps them
# fresh under freshness_days=30.
SELECTION_NOW = datetime(2026, 7, 20, 12, 0, tzinfo=UTC)
READY_BACKENDS = frozenset({"claude", "codex", "opencode"})


def row(
    *,
    backend="claude",
    model,
    aliases="—",
    active="yes",
    rank,
    cod=4,
    pln=4,
    rsc=4,
    rev=4,
    mon=4,
    inp="1.0",
    cached="0.1",
    write="1.25",
    out="6.0",
    max_tokens=200000,
):
    return (
        f"| {backend} | {model} | {aliases} | {active} | {rank}"
        f" | {cod} | {pln} | {rsc} | {rev} | {mon}"
        f" | {inp} | {cached} | {write} | {out} | {max_tokens} |"
    )


def unpriced_row(**overrides):
    fields = {
        "backend": "opencode",
        "model": "opencode/gpt-5.5",
        "rank": 5,
        "cod": 3,
        "pln": 3,
        "rsc": 3,
        "rev": 3,
        "mon": 3,
        "inp": "—",
        "cached": "—",
        "write": "—",
        "out": "—",
    }
    fields.update(overrides)
    return row(**fields)


def model_list_text(model_rows, *, retrieved_at="2026-07-19T00:00:00Z"):
    rows = "\n".join(model_rows)
    return f"""# Model list fixture

Prose is permitted before the first section.

## Metadata

| Field | Value |
|---|---|
| schema_version | 1 |
| generated_at | {retrieved_at} |
| freshness_days | 30 |

## Sources

| Source | URL | Retrieved at | Content SHA-256 |
|---|---|---|---|
| anthropic | https://platform.claude.com/docs/en/about-claude/pricing | {retrieved_at} | {"a" * 64} |
| openai | https://developers.openai.com/api/docs/pricing | {retrieved_at} | {"b" * 64} |

## Models

{MODELS_HEADER}
{MODELS_SEPARATOR}
{rows}
"""
