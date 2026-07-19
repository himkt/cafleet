# CAFleet Model Catalog

The machine-readable source of truth for CAFleet model availability, reviewed
capability policy, and standard token-price estimates. The payload block below
is maintained exclusively by the `cafleet-model-catalog-refresh` skill from the
two approved official pricing sources recorded in `sources`; capability levels
and global ranks are reviewed maintainer judgment, not provider benchmark
claims. Prices are standard direct-provider USD API rates and are estimates,
not an invoice guarantee.

<!-- cafleet-model-catalog: v1 -->
```json
{
  "currency": "USD",
  "freshness_days": 30,
  "generated_at": "2026-07-19T12:12:20Z",
  "model_tokens": [
    {
      "backend": "claude",
      "model_key": "claude:claude-fable-5",
      "primary": true,
      "token": "fable"
    },
    {
      "backend": "claude",
      "model_key": "claude:claude-fable-5",
      "primary": false,
      "token": "claude-fable-5"
    },
    {
      "backend": "claude",
      "model_key": "claude:claude-opus-4-8",
      "primary": true,
      "token": "opus"
    },
    {
      "backend": "claude",
      "model_key": "claude:claude-opus-4-8",
      "primary": false,
      "token": "claude-opus-4-8"
    },
    {
      "backend": "claude",
      "model_key": "claude:claude-sonnet-5",
      "primary": true,
      "token": "sonnet"
    },
    {
      "backend": "claude",
      "model_key": "claude:claude-sonnet-5",
      "primary": false,
      "token": "claude-sonnet-5"
    },
    {
      "backend": "claude",
      "model_key": "claude:claude-haiku-4-5",
      "primary": true,
      "token": "haiku"
    },
    {
      "backend": "claude",
      "model_key": "claude:claude-haiku-4-5",
      "primary": false,
      "token": "claude-haiku-4-5"
    },
    {
      "backend": "codex",
      "model_key": "codex:gpt-5.6-sol",
      "primary": true,
      "token": "gpt-5.6-sol"
    },
    {
      "backend": "codex",
      "model_key": "codex:gpt-5.6-terra",
      "primary": true,
      "token": "gpt-5.6-terra"
    },
    {
      "backend": "codex",
      "model_key": "codex:gpt-5.6-luna",
      "primary": true,
      "token": "gpt-5.6-luna"
    },
    {
      "backend": "codex",
      "model_key": "codex:gpt-5.5",
      "primary": true,
      "token": "gpt-5.5"
    },
    {
      "backend": "codex",
      "model_key": "codex:gpt-5.4",
      "primary": true,
      "token": "gpt-5.4"
    },
    {
      "backend": "codex",
      "model_key": "codex:gpt-5.4-mini",
      "primary": true,
      "token": "gpt-5.4-mini"
    },
    {
      "backend": "opencode",
      "model_key": "opencode:opencode/gpt-5.5-pro",
      "primary": true,
      "token": "opencode/gpt-5.5-pro"
    },
    {
      "backend": "opencode",
      "model_key": "opencode:opencode/gpt-5.5",
      "primary": true,
      "token": "opencode/gpt-5.5"
    },
    {
      "backend": "opencode",
      "model_key": "opencode:opencode/claude-opus-4-8",
      "primary": true,
      "token": "opencode/claude-opus-4-8"
    },
    {
      "backend": "opencode",
      "model_key": "opencode:opencode/claude-sonnet-4-6",
      "primary": true,
      "token": "opencode/claude-sonnet-4-6"
    },
    {
      "backend": "opencode",
      "model_key": "opencode:opencode/claude-haiku-4-5",
      "primary": true,
      "token": "opencode/claude-haiku-4-5"
    }
  ],
  "models": [
    {
      "active": true,
      "availability": {
        "requires_backend": "claude"
      },
      "backend": "claude",
      "capability": {
        "global_rank": 100,
        "levels": {
          "coding": 5,
          "monitor": 5,
          "planning": 5,
          "research": 5,
          "review": 5
        },
        "provenance": {
          "rationale": "Mythos-class frontier tier; highest reviewed capability on every dimension.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "claude:claude-fable-5",
      "provider": "anthropic",
      "provider_sku": "claude-fable-5",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "supported",
              "usd_per_mtok": 12.5
            },
            "cached_input": {
              "mode": "supported",
              "usd_per_mtok": 1.0
            },
            "input": {
              "mode": "supported",
              "usd_per_mtok": 10.0
            },
            "output": {
              "mode": "supported",
              "usd_per_mtok": 50.0
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": null,
          "id": "standard",
          "max_total_tokens": 1000000,
          "pricing_source": "anthropic",
          "status": "known"
        }
      ]
    },
    {
      "active": true,
      "availability": {
        "requires_backend": "claude"
      },
      "backend": "claude",
      "capability": {
        "global_rank": 85,
        "levels": {
          "coding": 5,
          "monitor": 5,
          "planning": 5,
          "research": 4,
          "review": 5
        },
        "provenance": {
          "rationale": "Everyday frontier tier with 1M context; strong coding, planning, and review.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "claude:claude-opus-4-8",
      "provider": "anthropic",
      "provider_sku": "claude-opus-4-8",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "supported",
              "usd_per_mtok": 6.25
            },
            "cached_input": {
              "mode": "supported",
              "usd_per_mtok": 0.5
            },
            "input": {
              "mode": "supported",
              "usd_per_mtok": 5.0
            },
            "output": {
              "mode": "supported",
              "usd_per_mtok": 25.0
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": null,
          "id": "standard",
          "max_total_tokens": 1000000,
          "pricing_source": "anthropic",
          "status": "known"
        }
      ]
    },
    {
      "active": true,
      "availability": {
        "requires_backend": "claude"
      },
      "backend": "claude",
      "capability": {
        "global_rank": 70,
        "levels": {
          "coding": 4,
          "monitor": 4,
          "planning": 4,
          "research": 4,
          "review": 4
        },
        "provenance": {
          "rationale": "Efficient mid tier for routine work; solid across all dimensions.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "claude:claude-sonnet-5",
      "provider": "anthropic",
      "provider_sku": "claude-sonnet-5",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "supported",
              "usd_per_mtok": 2.5
            },
            "cached_input": {
              "mode": "supported",
              "usd_per_mtok": 0.2
            },
            "input": {
              "mode": "supported",
              "usd_per_mtok": 2.0
            },
            "output": {
              "mode": "supported",
              "usd_per_mtok": 10.0
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": "2026-08-31",
          "id": "standard_intro",
          "max_total_tokens": 1000000,
          "pricing_source": "anthropic",
          "status": "known"
        },
        {
          "components": {
            "cache_write": {
              "mode": "supported",
              "usd_per_mtok": 3.75
            },
            "cached_input": {
              "mode": "supported",
              "usd_per_mtok": 0.3
            },
            "input": {
              "mode": "supported",
              "usd_per_mtok": 3.0
            },
            "output": {
              "mode": "supported",
              "usd_per_mtok": 15.0
            }
          },
          "effective_from": "2026-09-01",
          "effective_until": null,
          "id": "standard",
          "max_total_tokens": 1000000,
          "pricing_source": "anthropic",
          "status": "known"
        }
      ]
    },
    {
      "active": true,
      "availability": {
        "requires_backend": "claude"
      },
      "backend": "claude",
      "capability": {
        "global_rank": 40,
        "levels": {
          "coding": 2,
          "monitor": 4,
          "planning": 2,
          "research": 2,
          "review": 1
        },
        "provenance": {
          "rationale": "Fast low-cost tier; reliable for monitoring and quick bounded tasks.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "claude:claude-haiku-4-5",
      "provider": "anthropic",
      "provider_sku": "claude-haiku-4-5",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "supported",
              "usd_per_mtok": 1.25
            },
            "cached_input": {
              "mode": "supported",
              "usd_per_mtok": 0.1
            },
            "input": {
              "mode": "supported",
              "usd_per_mtok": 1.0
            },
            "output": {
              "mode": "supported",
              "usd_per_mtok": 5.0
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": null,
          "id": "standard",
          "max_total_tokens": 200000,
          "pricing_source": "anthropic",
          "status": "known"
        }
      ]
    },
    {
      "active": true,
      "availability": {
        "requires_backend": "codex"
      },
      "backend": "codex",
      "capability": {
        "global_rank": 90,
        "levels": {
          "coding": 5,
          "monitor": 4,
          "planning": 5,
          "research": 4,
          "review": 5
        },
        "provenance": {
          "rationale": "Latest frontier agentic coding tier; codex default and strongest reviewer.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "codex:gpt-5.6-sol",
      "provider": "openai",
      "provider_sku": "gpt-5.6-sol",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "supported",
              "usd_per_mtok": 0.0
            },
            "cached_input": {
              "mode": "supported",
              "usd_per_mtok": 0.5
            },
            "input": {
              "mode": "supported",
              "usd_per_mtok": 5.0
            },
            "output": {
              "mode": "supported",
              "usd_per_mtok": 30.0
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": null,
          "id": "standard",
          "max_total_tokens": 400000,
          "pricing_source": "openai",
          "status": "known"
        }
      ]
    },
    {
      "active": true,
      "availability": {
        "requires_backend": "codex"
      },
      "backend": "codex",
      "capability": {
        "global_rank": 75,
        "levels": {
          "coding": 4,
          "monitor": 4,
          "planning": 4,
          "research": 4,
          "review": 4
        },
        "provenance": {
          "rationale": "Balanced agentic coding tier for everyday work.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "codex:gpt-5.6-terra",
      "provider": "openai",
      "provider_sku": "gpt-5.6-terra",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "supported",
              "usd_per_mtok": 0.0
            },
            "cached_input": {
              "mode": "supported",
              "usd_per_mtok": 0.25
            },
            "input": {
              "mode": "supported",
              "usd_per_mtok": 2.5
            },
            "output": {
              "mode": "supported",
              "usd_per_mtok": 15.0
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": null,
          "id": "standard",
          "max_total_tokens": 400000,
          "pricing_source": "openai",
          "status": "known"
        }
      ]
    },
    {
      "active": true,
      "availability": {
        "requires_backend": "codex"
      },
      "backend": "codex",
      "capability": {
        "global_rank": 55,
        "levels": {
          "coding": 3,
          "monitor": 4,
          "planning": 3,
          "research": 3,
          "review": 3
        },
        "provenance": {
          "rationale": "Fast affordable agentic coding tier.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "codex:gpt-5.6-luna",
      "provider": "openai",
      "provider_sku": "gpt-5.6-luna",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "supported",
              "usd_per_mtok": 0.0
            },
            "cached_input": {
              "mode": "supported",
              "usd_per_mtok": 0.1
            },
            "input": {
              "mode": "supported",
              "usd_per_mtok": 1.0
            },
            "output": {
              "mode": "supported",
              "usd_per_mtok": 6.0
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": null,
          "id": "standard",
          "max_total_tokens": 400000,
          "pricing_source": "openai",
          "status": "known"
        }
      ]
    },
    {
      "active": true,
      "availability": {
        "requires_backend": "codex"
      },
      "backend": "codex",
      "capability": {
        "global_rank": 80,
        "levels": {
          "coding": 5,
          "monitor": 4,
          "planning": 5,
          "research": 4,
          "review": 4
        },
        "provenance": {
          "rationale": "Frontier tier for complex coding and research work.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "codex:gpt-5.5",
      "provider": "openai",
      "provider_sku": "gpt-5.5",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "supported",
              "usd_per_mtok": 0.0
            },
            "cached_input": {
              "mode": "supported",
              "usd_per_mtok": 0.5
            },
            "input": {
              "mode": "supported",
              "usd_per_mtok": 5.0
            },
            "output": {
              "mode": "supported",
              "usd_per_mtok": 30.0
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": null,
          "id": "standard",
          "max_total_tokens": 400000,
          "pricing_source": "openai",
          "status": "known"
        }
      ]
    },
    {
      "active": true,
      "availability": {
        "requires_backend": "codex"
      },
      "backend": "codex",
      "capability": {
        "global_rank": 60,
        "levels": {
          "coding": 4,
          "monitor": 4,
          "planning": 3,
          "research": 3,
          "review": 3
        },
        "provenance": {
          "rationale": "Strong everyday coding tier.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "codex:gpt-5.4",
      "provider": "openai",
      "provider_sku": "gpt-5.4",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "supported",
              "usd_per_mtok": 0.0
            },
            "cached_input": {
              "mode": "supported",
              "usd_per_mtok": 0.25
            },
            "input": {
              "mode": "supported",
              "usd_per_mtok": 2.5
            },
            "output": {
              "mode": "supported",
              "usd_per_mtok": 15.0
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": null,
          "id": "standard",
          "max_total_tokens": 400000,
          "pricing_source": "openai",
          "status": "known"
        }
      ]
    },
    {
      "active": true,
      "availability": {
        "requires_backend": "codex"
      },
      "backend": "codex",
      "capability": {
        "global_rank": 30,
        "levels": {
          "coding": 2,
          "monitor": 3,
          "planning": 2,
          "research": 2,
          "review": 2
        },
        "provenance": {
          "rationale": "Fast efficient mini tier for responsive tasks and subagents.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "codex:gpt-5.4-mini",
      "provider": "openai",
      "provider_sku": "gpt-5.4-mini",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "supported",
              "usd_per_mtok": 0.0
            },
            "cached_input": {
              "mode": "supported",
              "usd_per_mtok": 0.075
            },
            "input": {
              "mode": "supported",
              "usd_per_mtok": 0.75
            },
            "output": {
              "mode": "supported",
              "usd_per_mtok": 4.5
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": null,
          "id": "standard",
          "max_total_tokens": 400000,
          "pricing_source": "openai",
          "status": "known"
        }
      ]
    },
    {
      "active": true,
      "availability": {
        "requires_backend": "opencode"
      },
      "backend": "opencode",
      "capability": {
        "global_rank": 82,
        "levels": {
          "coding": 5,
          "monitor": 4,
          "planning": 5,
          "research": 4,
          "review": 5
        },
        "provenance": {
          "rationale": "Zen gateway pro tier; gateway price not evidenced by an approved source.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "opencode:opencode/gpt-5.5-pro",
      "provider": "openai",
      "provider_sku": "opencode/gpt-5.5-pro",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "cached_input": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "input": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "output": {
              "mode": "unsupported",
              "usd_per_mtok": null
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": null,
          "id": "gateway_unknown",
          "max_total_tokens": 400000,
          "pricing_source": "openai",
          "status": "unknown"
        }
      ]
    },
    {
      "active": true,
      "availability": {
        "requires_backend": "opencode"
      },
      "backend": "opencode",
      "capability": {
        "global_rank": 78,
        "levels": {
          "coding": 5,
          "monitor": 4,
          "planning": 5,
          "research": 4,
          "review": 4
        },
        "provenance": {
          "rationale": "Zen gateway frontier tier; gateway price not evidenced by an approved source.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "opencode:opencode/gpt-5.5",
      "provider": "openai",
      "provider_sku": "opencode/gpt-5.5",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "cached_input": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "input": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "output": {
              "mode": "unsupported",
              "usd_per_mtok": null
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": null,
          "id": "gateway_unknown",
          "max_total_tokens": 400000,
          "pricing_source": "openai",
          "status": "unknown"
        }
      ]
    },
    {
      "active": true,
      "availability": {
        "requires_backend": "opencode"
      },
      "backend": "opencode",
      "capability": {
        "global_rank": 76,
        "levels": {
          "coding": 5,
          "monitor": 5,
          "planning": 5,
          "research": 4,
          "review": 5
        },
        "provenance": {
          "rationale": "Zen gateway Opus tier; gateway price not evidenced by an approved source.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "opencode:opencode/claude-opus-4-8",
      "provider": "anthropic",
      "provider_sku": "opencode/claude-opus-4-8",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "cached_input": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "input": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "output": {
              "mode": "unsupported",
              "usd_per_mtok": null
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": null,
          "id": "gateway_unknown",
          "max_total_tokens": 1000000,
          "pricing_source": "anthropic",
          "status": "unknown"
        }
      ]
    },
    {
      "active": true,
      "availability": {
        "requires_backend": "opencode"
      },
      "backend": "opencode",
      "capability": {
        "global_rank": 65,
        "levels": {
          "coding": 4,
          "monitor": 4,
          "planning": 4,
          "research": 4,
          "review": 4
        },
        "provenance": {
          "rationale": "Zen gateway Sonnet tier; gateway price not evidenced by an approved source.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "opencode:opencode/claude-sonnet-4-6",
      "provider": "anthropic",
      "provider_sku": "opencode/claude-sonnet-4-6",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "cached_input": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "input": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "output": {
              "mode": "unsupported",
              "usd_per_mtok": null
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": null,
          "id": "gateway_unknown",
          "max_total_tokens": 1000000,
          "pricing_source": "anthropic",
          "status": "unknown"
        }
      ]
    },
    {
      "active": true,
      "availability": {
        "requires_backend": "opencode"
      },
      "backend": "opencode",
      "capability": {
        "global_rank": 35,
        "levels": {
          "coding": 2,
          "monitor": 4,
          "planning": 2,
          "research": 2,
          "review": 1
        },
        "provenance": {
          "rationale": "Zen gateway Haiku tier; gateway price not evidenced by an approved source.",
          "reviewed_at": "2026-07-19T12:12:20Z",
          "type": "maintainer_judgment"
        }
      },
      "key": "opencode:opencode/claude-haiku-4-5",
      "provider": "anthropic",
      "provider_sku": "opencode/claude-haiku-4-5",
      "rate_cards": [
        {
          "components": {
            "cache_write": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "cached_input": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "input": {
              "mode": "unsupported",
              "usd_per_mtok": null
            },
            "output": {
              "mode": "unsupported",
              "usd_per_mtok": null
            }
          },
          "effective_from": "2026-07-19",
          "effective_until": null,
          "id": "gateway_unknown",
          "max_total_tokens": 200000,
          "pricing_source": "anthropic",
          "status": "unknown"
        }
      ]
    }
  ],
  "role_profiles": {
    "analyzer": {
      "requires": {
        "planning": 4,
        "research": 3,
        "review": 3
      },
      "task_kind": "requirements_analysis",
      "token_profile": "standard"
    },
    "drafter": {
      "requires": {
        "planning": 3,
        "research": 2,
        "review": 1
      },
      "task_kind": "design_doc_drafting",
      "token_profile": "standard"
    },
    "manager": {
      "requires": {
        "planning": 4,
        "research": 3
      },
      "task_kind": "research_coordination",
      "token_profile": "standard"
    },
    "monitor": {
      "requires": {
        "monitor": 2
      },
      "task_kind": "monitoring",
      "token_profile": "small"
    },
    "presentation": {
      "requires": {
        "planning": 3,
        "research": 2,
        "review": 2
      },
      "task_kind": "presentation_authoring",
      "token_profile": "standard"
    },
    "programmer": {
      "requires": {
        "coding": 4,
        "planning": 3,
        "review": 2
      },
      "task_kind": "implementation",
      "token_profile": "large"
    },
    "researcher": {
      "requires": {
        "planning": 3,
        "research": 4,
        "review": 2
      },
      "task_kind": "research_synthesis",
      "token_profile": "large"
    },
    "reviewer": {
      "requires": {
        "planning": 3,
        "review": 4
      },
      "task_kind": "review",
      "token_profile": "standard"
    },
    "scout": {
      "requires": {
        "planning": 2,
        "research": 3
      },
      "task_kind": "source_discovery",
      "token_profile": "small"
    },
    "tester": {
      "requires": {
        "coding": 3,
        "planning": 3,
        "review": 3
      },
      "task_kind": "test_design",
      "token_profile": "standard"
    },
    "transcript": {
      "requires": {
        "planning": 3,
        "research": 2,
        "review": 2
      },
      "task_kind": "research_transcript",
      "token_profile": "standard"
    },
    "verifier": {
      "requires": {
        "coding": 3,
        "planning": 4,
        "review": 4
      },
      "task_kind": "verification",
      "token_profile": "standard"
    },
    "visual_reviewer": {
      "requires": {
        "planning": 2,
        "review": 4
      },
      "task_kind": "visual_review",
      "token_profile": "standard"
    },
    "web_researcher": {
      "requires": {
        "planning": 3,
        "research": 4
      },
      "task_kind": "web_research",
      "token_profile": "large"
    }
  },
  "schema_version": 1,
  "sources": {
    "anthropic": {
      "content_sha256": "465c891550c135716901a2679c1c7a693b03fe3d2eac2aff200f9f7f745a1ea3",
      "retrieved_at": "2026-07-19T12:12:20Z",
      "url": "https://platform.claude.com/docs/en/about-claude/pricing"
    },
    "openai": {
      "content_sha256": "6b5ed5e1cdd1edded9e5cc4b187f1603d150318b9a03191bd549a8790c193b9d",
      "retrieved_at": "2026-07-19T12:12:20Z",
      "url": "https://developers.openai.com/api/docs/pricing"
    }
  },
  "token_profiles": {
    "large": {
      "cache_write": 0,
      "cached_input": 0,
      "input": 24000,
      "output": 12000
    },
    "small": {
      "cache_write": 0,
      "cached_input": 0,
      "input": 4000,
      "output": 1000
    },
    "standard": {
      "cache_write": 0,
      "cached_input": 0,
      "input": 12000,
      "output": 6000
    }
  }
}
```
