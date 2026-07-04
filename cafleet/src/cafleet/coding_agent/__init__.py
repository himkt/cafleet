from cafleet.coding_agent.base import CodingAgent
from cafleet.coding_agent.claude import ClaudeCodeAgent
from cafleet.coding_agent.codex import CodexAgent
from cafleet.coding_agent.opencode import OpencodeAgent

CODING_AGENTS: dict[str, CodingAgent] = {
    "claude": ClaudeCodeAgent(),
    "codex": CodexAgent(),
    "opencode": OpencodeAgent(),
}

__all__ = [
    "CODING_AGENTS",
    "ClaudeCodeAgent",
    "CodexAgent",
    "CodingAgent",
    "OpencodeAgent",
]
