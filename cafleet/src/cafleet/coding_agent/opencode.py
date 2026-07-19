from pathlib import Path

from cafleet.coding_agent.base import ensure_binary_on_path


class OpencodeAgent:
    name = "opencode"
    binary_name = "opencode"

    def ensure_available(self) -> None:
        ensure_binary_on_path(self.binary_name)
        preset = Path("~/.opencode/agents/cafleet.md").expanduser()
        if not preset.is_file():
            raise RuntimeError(
                f"opencode agent preset not found at {preset}; "
                "run 'cafleet setup' first"
            )

    def validate_model(self, model: str | None) -> None:
        if model is None:
            return
        provider, sep, model_id = model.partition("/")
        if not sep or not provider or not model_id:
            raise ValueError(
                "--model for the opencode backend must be "
                f"'<provider-id>/<model-id>' (got '{model}')."
            )

    def validate_effort(self, effort: str | None) -> None:
        if effort is None:
            return
        raise ValueError("opencode does not support reasoning effort.")

    def build_spawn_argv(
        self,
        prompt: str,
        *,
        display_name: str,
        model: str | None = None,
        effort: str | None = None,
    ) -> list[str]:
        del display_name
        assert effort is None, "opencode effort must be rejected by validate_effort"
        argv = [self.binary_name, "--agent", "cafleet"]
        if model is not None:
            argv.extend(["--model", model])
        argv.extend(["--prompt", prompt])
        return argv
