"""Builders for canonical model-catalog Markdown fixtures shared by the model-selection tests."""

import json
from datetime import UTC, datetime

from cafleet.model_selection import parse_catalog_markdown

APPROVED_ANTHROPIC_URL = "https://platform.claude.com/docs/en/about-claude/pricing"
APPROVED_OPENAI_URL = "https://developers.openai.com/api/docs/pricing"
CATALOG_MARKER = "<!-- cafleet-model-catalog: v1 -->"

CAPABILITY_DIMENSIONS = ("coding", "planning", "research", "review", "monitor")

# Normative role-profile table from design 0000145 § Task profile and selection algorithm.
WORKFLOW_ROLE_PROFILES = {
    "monitor": {
        "task_kind": "monitoring",
        "requires": {"monitor": 2},
        "token_profile": "small",
    },
    "drafter": {
        "task_kind": "design_doc_drafting",
        "requires": {"planning": 3, "research": 2, "review": 1},
        "token_profile": "standard",
    },
    "reviewer": {
        "task_kind": "review",
        "requires": {"review": 4, "planning": 3},
        "token_profile": "standard",
    },
    "analyzer": {
        "task_kind": "requirements_analysis",
        "requires": {"planning": 4, "research": 3, "review": 3},
        "token_profile": "standard",
    },
    "programmer": {
        "task_kind": "implementation",
        "requires": {"coding": 4, "planning": 3, "review": 2},
        "token_profile": "large",
    },
    "tester": {
        "task_kind": "test_design",
        "requires": {"coding": 3, "planning": 3, "review": 3},
        "token_profile": "standard",
    },
    "verifier": {
        "task_kind": "verification",
        "requires": {"coding": 3, "planning": 4, "review": 4},
        "token_profile": "standard",
    },
    "manager": {
        "task_kind": "research_coordination",
        "requires": {"planning": 4, "research": 3},
        "token_profile": "standard",
    },
    "scout": {
        "task_kind": "source_discovery",
        "requires": {"research": 3, "planning": 2},
        "token_profile": "small",
    },
    "researcher": {
        "task_kind": "research_synthesis",
        "requires": {"research": 4, "planning": 3, "review": 2},
        "token_profile": "large",
    },
    "web_researcher": {
        "task_kind": "web_research",
        "requires": {"research": 4, "planning": 3},
        "token_profile": "large",
    },
    "transcript": {
        "task_kind": "research_transcript",
        "requires": {"planning": 3, "research": 2, "review": 2},
        "token_profile": "standard",
    },
    "presentation": {
        "task_kind": "presentation_authoring",
        "requires": {"planning": 3, "research": 2, "review": 2},
        "token_profile": "standard",
    },
    "visual_reviewer": {
        "task_kind": "visual_review",
        "requires": {"review": 4, "planning": 2},
        "token_profile": "standard",
    },
}


def known_rate_card(
    *,
    card_id="standard",
    input_price=3.0,
    cached_input_price=0.3,
    cache_write_price=3.75,
    output_price=15.0,
    pricing_source="anthropic",
    max_total_tokens=200000,
    effective_from="2026-07-01",
    effective_until=None,
):
    return {
        "id": card_id,
        "status": "known",
        "max_total_tokens": max_total_tokens,
        "components": {
            "input": {"mode": "supported", "usd_per_mtok": input_price},
            "cached_input": {"mode": "supported", "usd_per_mtok": cached_input_price},
            "cache_write": {"mode": "supported", "usd_per_mtok": cache_write_price},
            "output": {"mode": "supported", "usd_per_mtok": output_price},
        },
        "effective_from": effective_from,
        "effective_until": effective_until,
        "pricing_source": pricing_source,
    }


def _provenance():
    return {
        "type": "maintainer_judgment",
        "rationale": "Reviewed classification; not a provider benchmark claim.",
        "reviewed_at": "2026-07-19T00:00:00Z",
    }


def claude_model():
    return {
        "key": "claude:claude-sonnet-5",
        "backend": "claude",
        "provider_sku": "claude-sonnet-5",
        "provider": "anthropic",
        "active": True,
        "capability": {
            "global_rank": 20,
            "levels": {
                "coding": 4,
                "planning": 4,
                "research": 4,
                "review": 4,
                "monitor": 4,
            },
            "provenance": _provenance(),
        },
        "rate_cards": [known_rate_card(pricing_source="anthropic")],
        "availability": {"requires_backend": "claude"},
    }


def codex_model():
    return {
        "key": "codex:gpt-5.6-luna",
        "backend": "codex",
        "provider_sku": "gpt-5.6-luna",
        "provider": "openai",
        "active": True,
        "capability": {
            "global_rank": 30,
            "levels": {
                "coding": 4,
                "planning": 4,
                "research": 4,
                "review": 4,
                "monitor": 4,
            },
            "provenance": _provenance(),
        },
        "rate_cards": [
            known_rate_card(
                input_price=1.0,
                cached_input_price=0.1,
                cache_write_price=1.25,
                output_price=6.0,
                pricing_source="openai",
            )
        ],
        "availability": {"requires_backend": "codex"},
    }


def opencode_gateway_model():
    """Gateway model without an approved actual price: only an `unknown` rate card."""
    return {
        "key": "opencode:opencode/gpt-5.5",
        "backend": "opencode",
        "provider_sku": "opencode/gpt-5.5",
        "provider": "openai",
        "active": True,
        "capability": {
            "global_rank": 10,
            "levels": {
                "coding": 3,
                "planning": 3,
                "research": 3,
                "review": 3,
                "monitor": 3,
            },
            "provenance": _provenance(),
        },
        "rate_cards": [
            {
                "id": "gateway_unknown",
                "status": "unknown",
                "max_total_tokens": 128000,
                "components": {
                    "input": {"mode": "unsupported", "usd_per_mtok": None},
                    "cached_input": {"mode": "unsupported", "usd_per_mtok": None},
                    "cache_write": {"mode": "unsupported", "usd_per_mtok": None},
                    "output": {"mode": "unsupported", "usd_per_mtok": None},
                },
                "effective_from": "2026-07-01",
                "effective_until": None,
                "pricing_source": "openai",
            }
        ],
        "availability": {"requires_backend": "opencode"},
    }


def base_catalog():
    """A fully valid schema-v1 catalog payload covering every workflow role."""
    return {
        "schema_version": 1,
        "generated_at": "2026-07-19T00:00:00Z",
        "freshness_days": 30,
        "currency": "USD",
        "token_profiles": {
            "small": {
                "input": 4000,
                "cached_input": 0,
                "cache_write": 0,
                "output": 1000,
            },
            "standard": {
                "input": 12000,
                "cached_input": 0,
                "cache_write": 0,
                "output": 6000,
            },
            "large": {
                "input": 24000,
                "cached_input": 0,
                "cache_write": 0,
                "output": 12000,
            },
        },
        "role_profiles": {
            role: dict(profile) for role, profile in WORKFLOW_ROLE_PROFILES.items()
        },
        "sources": {
            "anthropic": {
                "url": APPROVED_ANTHROPIC_URL,
                "retrieved_at": "2026-07-19T00:00:00Z",
                "content_sha256": "a" * 64,
            },
            "openai": {
                "url": APPROVED_OPENAI_URL,
                "retrieved_at": "2026-07-19T00:00:00Z",
                "content_sha256": "b" * 64,
            },
        },
        "models": [claude_model(), codex_model()],
        "model_tokens": [
            {
                "backend": "claude",
                "token": "sonnet",
                "model_key": "claude:claude-sonnet-5",
                "primary": True,
            },
            {
                "backend": "codex",
                "token": "gpt-5.6-luna",
                "model_key": "codex:gpt-5.6-luna",
                "primary": True,
            },
        ],
    }


def canonical_payload(payload: dict) -> str:
    return json.dumps(payload, sort_keys=True, indent=2, ensure_ascii=False) + "\n"


def render_markdown(payload_text: str, *, marker: str = CATALOG_MARKER) -> str:
    return (
        "# CAFleet Model Catalog\n"
        "\n"
        "Maintained via the cafleet-model-catalog-refresh skill.\n"
        "\n"
        f"{marker}\n"
        "```json\n"
        f"{payload_text}"
        "```\n"
    )


def catalog_markdown(payload: dict) -> str:
    return render_markdown(canonical_payload(payload))


def parsed_catalog(payload: dict):
    return parse_catalog_markdown(catalog_markdown(payload))


# Selection-time inputs shared by the selector tests. The base catalog's sources
# were retrieved 2026-07-19, so this instant keeps them fresh (freshness_days=30).
SELECTION_NOW = datetime(2026, 7, 20, 12, 0, tzinfo=UTC)
READY_BACKENDS = frozenset({"claude", "codex", "opencode"})


def uniform_levels(level=4, **overrides):
    levels = dict.fromkeys(CAPABILITY_DIMENSIONS, level)
    levels.update(overrides)
    return levels


def make_model(
    *,
    backend="claude",
    sku,
    rank,
    levels=None,
    active=True,
    input_price=1.0,
    cached_input_price=0.1,
    cache_write_price=1.25,
    output_price=6.0,
    max_total_tokens=200000,
    effective_from="2026-07-01",
    effective_until=None,
    rate_cards=None,
):
    if levels is None:
        levels = uniform_levels()
    provider = "anthropic" if backend == "claude" else "openai"
    if rate_cards is None:
        rate_cards = [
            known_rate_card(
                input_price=input_price,
                cached_input_price=cached_input_price,
                cache_write_price=cache_write_price,
                output_price=output_price,
                pricing_source=provider,
                max_total_tokens=max_total_tokens,
                effective_from=effective_from,
                effective_until=effective_until,
            )
        ]
    return {
        "key": f"{backend}:{sku}",
        "backend": backend,
        "provider_sku": sku,
        "provider": provider,
        "active": active,
        "capability": {
            "global_rank": rank,
            "levels": levels,
            "provenance": _provenance(),
        },
        "rate_cards": rate_cards,
        "availability": {"requires_backend": backend},
    }


def primary_token_entry(model, token=None):
    return {
        "backend": model["backend"],
        "token": token or model["provider_sku"],
        "model_key": model["key"],
        "primary": True,
    }


def catalog_with(models, tokens=None, extra_tokens=()):
    catalog = base_catalog()
    catalog["models"] = list(models)
    if tokens is None:
        tokens = [primary_token_entry(model) for model in models if model["active"]]
    catalog["model_tokens"] = list(tokens) + list(extra_tokens)
    return catalog
