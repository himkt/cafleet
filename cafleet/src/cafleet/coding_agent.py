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
    permission_args: tuple[str, ...] = ()

    def build_command(
        self,
        prompt: str,
        *,
        display_name: str | None = None,
    ) -> list[str]:
        name_args: tuple[str, ...] = ()
        if display_name and self.display_name_args:
            name_args = (*self.display_name_args, display_name)
        return [
            self.binary,
            *self.extra_args,
            *self.permission_args,
            *name_args,
            prompt,
        ]

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
        "`cafleet --session-id {session_id} message poll --agent-id {agent_id}`.\n"
        "Your harness runs in dontAsk mode — your Bash tool is enabled and permission\n"
        "prompts auto-resolve, so call cafleet (and any other shell command) directly\n"
        "via the Bash tool."
    ),
    display_name_args=("--name",),
    permission_args=("--permission-mode", "dontAsk"),
)
