from cafleet.coding_agent.base import ensure_binary_on_path


class CodexAgent:
    name = "codex"
    binary_name = "codex"

    def ensure_available(self) -> None:
        ensure_binary_on_path(self.binary_name)

    def build_spawn_argv(self, prompt: str, *, display_name: str) -> list[str]:
        # display_name silently ignored — codex has no `--name` analog.
        del display_name
        return [
            self.binary_name,
            "--ask-for-approval",
            "never",
            "--sandbox",
            "workspace-write",
            prompt,
        ]
