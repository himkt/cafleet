from cafleet.coding_agent.base import CodingAgent, ensure_binary_on_path
from cafleet.coding_agent.claude import ClaudeCodeAgent
from cafleet.coding_agent.codex import CodexAgent

CODING_AGENTS: dict[str, CodingAgent] = {
    "claude": ClaudeCodeAgent(),
    "codex": CodexAgent(),
}

__all__ = [
    "CODING_AGENTS",
    "ClaudeCodeAgent",
    "CodexAgent",
    "CodingAgent",
    "ensure_binary_on_path",
]
