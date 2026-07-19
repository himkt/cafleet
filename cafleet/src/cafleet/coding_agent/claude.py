from cafleet.coding_agent.base import ensure_binary_on_path

EFFORT_LEVELS = ("low", "medium", "high", "xhigh", "max")


class ClaudeCodeAgent:
    name = "claude"
    binary_name = "claude"

    def ensure_available(self) -> None:
        ensure_binary_on_path(self.binary_name)

    def validate_model(self, model: str | None) -> None:
        # Pass-through: the claude binary itself rejects unknown models.
        del model

    def validate_effort(self, effort: str | None) -> None:
        if effort is None:
            return
        if effort not in EFFORT_LEVELS:
            raise ValueError(
                "--effort for the claude backend must be one of "
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
        argv = [
            self.binary_name,
            "--permission-mode",
            "dontAsk",
            "--name",
            display_name,
        ]
        if model is not None:
            argv.extend(["--model", model])
        if effort is not None:
            argv.extend(["--effort", effort])
        argv.append(prompt)
        return argv
