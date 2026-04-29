"""Coding-agent spawn config."""

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
        "`! cafleet --session-id {session_id} message poll --agent-id {agent_id}`.\n"
        "If your Bash tool is denied for this session, you MUST prefix every cafleet call\n"
        "(poll/send/ack) with `! ` — for example, `! cafleet --session-id {session_id} message send\n"
        '--agent-id {agent_id} --to <id> --text "..."`. Do NOT attempt cafleet via the Bash tool;\n'
        "it is denied. The `!` shortcut is independent of the Bash-tool deny posture. Likewise route\n"
        "any other shell command through your Director — send a plain message asking for it; the\n"
        "Director will respond with `! command` keystrokes that your harness will execute via the\n"
        "same `!` CLI shortcut."
    ),
    display_name_args=("--name",),
    disallow_tools_args=("--disallowedTools", "Bash"),
)
