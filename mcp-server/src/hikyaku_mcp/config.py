"""Configuration loaded from environment variables.

HIKYAKU_URL      — broker base URL (required)
HIKYAKU_API_KEY  — API key for authentication (required)
HIKYAKU_AGENT_ID — agent ID (required)
"""

from __future__ import annotations

import os


def get_config() -> dict[str, str]:
    """Load and validate configuration from environment variables."""
    broker_url = os.environ.get("HIKYAKU_URL", "")
    api_key = os.environ.get("HIKYAKU_API_KEY", "")
    agent_id = os.environ.get("HIKYAKU_AGENT_ID", "")

    if not broker_url:
        raise ValueError("HIKYAKU_URL environment variable is required")
    if not api_key:
        raise ValueError("HIKYAKU_API_KEY environment variable is required")
    if not agent_id:
        raise ValueError("HIKYAKU_AGENT_ID environment variable is required")

    return {
        "broker_url": broker_url,
        "api_key": api_key,
        "agent_id": agent_id,
    }
