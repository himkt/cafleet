import json
from dataclasses import asdict, dataclass, field
from pathlib import Path


@dataclass(frozen=True)
class PermissionRuleset:
    bash: dict[str, str] = field(default_factory=dict)
    read: dict[str, str] = field(default_factory=dict)
    edit: dict[str, str] = field(default_factory=dict)
    external_directory: str = "deny"
    webfetch: str = "deny"
    websearch: str = "deny"
    repo_clone: str = "deny"
    question: str = "deny"
    plan_enter: str = "deny"
    plan_exit: str = "deny"


@dataclass(frozen=True)
class OpencodeAgentDefinition:
    description: str
    mode: str
    permission: PermissionRuleset
    body: str

    def to_markdown(self) -> str:
        frontmatter = {
            "description": self.description,
            "mode": self.mode,
            "permission": asdict(self.permission),
        }
        return (
            f"---\n{json.dumps(frontmatter, indent=2, ensure_ascii=False)}\n"
            f"---\n\n{self.body}\n"
        )


CAFLEET_AGENT = OpencodeAgentDefinition(
    description=(
        "CAFleet-spawned member with workspace-scoped permission floor; "
        "matches Claude Code dontAsk safety posture."
    ),
    mode="primary",
    permission=PermissionRuleset(
        bash={
            "*": "allow",
            "bash -c*": "deny",
            "sh -c*": "deny",
            "zsh -c*": "deny",
            "python -c*": "deny",
            "python3 -c*": "deny",
            "perl -e*": "deny",
            "node -e*": "deny",
            "node --eval*": "deny",
            "ruby -e*": "deny",
            "eval*": "deny",
            "exec*": "deny",
            "rm -rf*": "deny",
            "sudo*": "deny",
            "git push*": "deny",
            "git reset --hard*": "deny",
            "chmod*": "deny",
            "chown*": "deny",
            "curl*": "deny",
            "wget*": "deny",
            "nc*": "deny",
            "ssh*": "deny",
            "scp*": "deny",
            "rsync*": "deny",
            "osascript*": "deny",
        },
        read={
            "*": "allow",
            "**/.env": "deny",
            "**/.env.*": "deny",
        },
        edit={
            "*": "allow",
            "**/.env": "deny",
            "**/.env.*": "deny",
        },
    ),
    body=(
        "# CAFleet member agent\n\n"
        "You are a CAFleet member spawned by the Director. The bash, read, "
        "and edit permission rulesets in your frontmatter enforce a "
        "workspace-scoped safety floor that mirrors Claude Code's `dontAsk` "
        "posture: dangerous shell-indirection wrappers, destructive "
        "operations, network egress utilities, and `.env` files are denied; "
        "everything else is allowed without user prompts. Refer to your "
        "Director's spawn-prompt instructions for the task."
    ),
)


def materialize_cafleet_agent(definition: OpencodeAgentDefinition) -> None:
    target = Path("~/.opencode/agents/cafleet.md").expanduser()
    if target.exists():
        return
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(definition.to_markdown(), encoding="utf-8")
