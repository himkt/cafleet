from cafleet.coding_agent.base import ensure_binary_on_path


class ClaudeCodeAgent:
    name = "claude"
    binary_name = "claude"

    def ensure_available(self) -> None:
        ensure_binary_on_path(self.binary_name)

    def validate_model(self, model: str | None) -> None:
        # Pass-through: the claude binary itself rejects unknown models.
        del model

    def build_spawn_argv(
        self, prompt: str, *, display_name: str, model: str | None = None
    ) -> list[str]:
        argv = [
            self.binary_name,
            "--permission-mode",
            "dontAsk",
            "--name",
            display_name,
        ]
        if model is not None:
            argv.extend(["--model", model])
        argv.append(prompt)
        return argv
