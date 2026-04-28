"""Coding-agent registry: parameterizes tmux spawn per backend."""

import shutil
from dataclasses import dataclass


@dataclass(frozen=True)
class CodingAgentConfig:
    name: str
    binary: str
    extra_args: tuple[str, ...] = ()
    default_prompt_template: str = ""
    display_name_args: tuple[str, ...] = ()
    disallow_tools_args: tuple[str, ...] = ()

    def build_command(
        self,
        prompt: str,
        *,
        display_name: str | None = None,
        deny_bash: bool = False,
    ) -> list[str]:
        deny_args: tuple[str, ...] = ()
        if deny_bash and self.disallow_tools_args:
            deny_args = self.disallow_tools_args
        name_args: tuple[str, ...] = ()
        if display_name and self.display_name_args:
            name_args = (*self.display_name_args, display_name)
        return [self.binary, *self.extra_args, *deny_args, *name_args, prompt]

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
        "`cafleet --session-id {session_id} poll --agent-id {agent_id}`.\n"
        "Your Bash tool is denied. Route any shell command through your Director —\n"
        "see Skill(cafleet) > Routing Bash via the Director for the bash_request JSON envelope."
    ),
    display_name_args=("--name",),
    disallow_tools_args=("--disallowedTools", "Bash"),
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
        '--to <id> --text "..."` to reply.\n'
        "\n"
        "When you need to run a shell command, do NOT use your shell tool directly. "
        "Instead send a JSON\n"
        "bash_request to your Director ({director_agent_id}) via `cafleet send`:\n"
        '  {{"type":"bash_request","cmd":"<shell-command>","cwd":"<absolute-path>","reason":"<short-reason>"}}\n'
        "Then poll for a bash_result reply correlated by in_reply_to == <your-send-task-id>."
    ),
    disallow_tools_args=(),
)

CODING_AGENTS: dict[str, CodingAgentConfig] = {
    "claude": CLAUDE,
    "codex": CODEX,
}


def get_coding_agent(name: str) -> CodingAgentConfig:
    try:
        return CODING_AGENTS[name]
    except KeyError as exc:
        raise ValueError(
            f"Unknown coding agent '{name}'. "
            f"Available: {', '.join(sorted(CODING_AGENTS))}"
        ) from exc
