"""Coding agent configuration for multi-runner support.

Encapsulates agent-specific details — binary name, extra args, default prompt
template — so that tmux pane spawning is parameterized by agent type.
"""

from dataclasses import dataclass
import shutil


@dataclass(frozen=True)
class CodingAgentConfig:
    """Configuration for a coding agent binary that runs inside a tmux pane."""

    name: str
    binary: str
    extra_args: tuple[str, ...] = ()
    default_prompt_template: str = ""

    def build_command(self, prompt: str) -> list[str]:
        return [self.binary, *self.extra_args, prompt]

    def ensure_available(self) -> None:
        if shutil.which(self.binary) is None:
            raise RuntimeError(f"'{self.binary}' binary not found on PATH")


CLAUDE = CodingAgentConfig(
    name="claude",
    binary="claude",
    extra_args=(),
    default_prompt_template=(
        "Load Skill(cafleet). Your session_id is {session_id} and your agent_id is {agent_id}.\n"
        "You are a member of the team led by {director_name} ({director_agent_id}).\n"
        "Wait for instructions via "
        "`cafleet --session-id {session_id} --agent-id {agent_id} poll`."
    ),
)

CODEX = CodingAgentConfig(
    name="codex",
    binary="codex",
    extra_args=("--approval-mode", "auto-edit"),
    default_prompt_template=(
        "Your session_id is {session_id} and your agent_id is {agent_id}.\n"
        "You are a member of the team led by {director_name} ({director_agent_id}).\n"
        "Check for instructions using "
        "`cafleet --session-id {session_id} --agent-id {agent_id} poll`.\n"
        "Use `cafleet --session-id {session_id} --agent-id {agent_id} ack --task-id <id>` "
        "to acknowledge messages\n"
        "and `cafleet --session-id {session_id} --agent-id {agent_id} send "
        '--to <id> --text "..."` to reply.'
    ),
)

CODING_AGENTS: dict[str, CodingAgentConfig] = {
    "claude": CLAUDE,
    "codex": CODEX,
}


def get_coding_agent(name: str) -> CodingAgentConfig:
    """Return the CodingAgentConfig for the given name.

    Raises ValueError if the name is not in the registry.
    """
    try:
        return CODING_AGENTS[name]
    except KeyError:
        raise ValueError(
            f"Unknown coding agent '{name}'. "
            f"Available: {', '.join(sorted(CODING_AGENTS))}"
        )
