from cafleet.coding_agent.base import ensure_binary_on_path


class CodexAgent:
    name = "codex"
    binary_name = "codex"

    def ensure_available(self) -> None:
        ensure_binary_on_path(self.binary_name)

    def validate_model(self, model: str | None) -> None:
        # Pass-through: the codex binary itself rejects unknown models.
        del model

    def build_spawn_argv(
        self, prompt: str, *, display_name: str, model: str | None = None
    ) -> list[str]:
        # display_name silently ignored — codex has no `--name` analog.
        del display_name
        argv = [
            self.binary_name,
            "--ask-for-approval",
            "never",
            "--sandbox",
            "workspace-write",
        ]
        if model is not None:
            argv.extend(["--model", model])
        argv.append(prompt)
        return argv
