from cafleet.coding_agent.base import ensure_binary_on_path


class ClaudeCodeAgent:
    name = "claude"
    binary_name = "claude"

    def ensure_available(self) -> None:
        ensure_binary_on_path(self.binary_name)

    def build_spawn_argv(self, prompt: str, *, display_name: str) -> list[str]:
        return [
            self.binary_name,
            "--permission-mode",
            "dontAsk",
            "--name",
            display_name,
            prompt,
        ]
