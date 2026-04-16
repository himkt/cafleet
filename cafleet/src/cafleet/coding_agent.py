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
    display_name_args: tuple[str, ...] = ()

    def build_command(
        self, prompt: str, *, display_name: str | None = None
    ) -> list[str]:
        name_args: tuple[str, ...] = ()
        if display_name and self.display_name_args:
            name_args = (*self.display_name_args, display_name)
        return [self.binary, *self.extra_args, *name_args, prompt]

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
        "`cafleet --session-id {session_id} poll --agent-id {agent_id}`."
    ),
    display_name_args=("--name",),
)

CODEX = CodingAgentConfig(
    name="codex",
    binary="codex",
    extra_args=("--approval-mode", "auto-edit"),
    default_prompt_template=(
        "Your session_id is {session_id} and your agent_id is {agent_id}.\n"
        "You are a member of the team led by {director_name} ({director_agent_id}).\n"
        "Check for instructions using "
        "`cafleet --session-id {session_id} poll --agent-id {agent_id}`.\n"
        "Use `cafleet --session-id {session_id} ack --agent-id {agent_id} --task-id <id>` "
        "to acknowledge messages\n"
        "and `cafleet --session-id {session_id} send --agent-id {agent_id} "
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
