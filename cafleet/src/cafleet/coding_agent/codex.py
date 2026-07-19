from cafleet.coding_agent.base import ensure_binary_on_path

EFFORT_LEVELS = ("minimal", "low", "medium", "high", "xhigh")


class CodexAgent:
    name = "codex"
    binary_name = "codex"

    def ensure_available(self) -> None:
        ensure_binary_on_path(self.binary_name)

    def validate_model(self, model: str | None) -> None:
        # Pass-through: the codex binary itself rejects unknown models.
        del model

    def validate_effort(self, effort: str | None) -> None:
        if effort is None:
            return
        if effort not in EFFORT_LEVELS:
            raise ValueError(
                "--effort for the codex backend must be one of "
                f"{', '.join(EFFORT_LEVELS)} (got '{effort}')."
            )

    def build_spawn_argv(
        self,
        prompt: str,
        *,
        display_name: str,
        model: str | None = None,
        effort: str | None = None,
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
        if effort is not None:
            argv.append(f"--config=model_reasoning_effort={effort}")
        argv.append(prompt)
        return argv
