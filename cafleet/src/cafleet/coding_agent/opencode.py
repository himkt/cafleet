from cafleet.coding_agent.base import ensure_binary_on_path
from cafleet.coding_agent.opencode_preset import (
    CAFLEET_AGENT,
    materialize_cafleet_agent,
)


class OpencodeAgent:
    name = "opencode"
    binary_name = "opencode"

    def ensure_available(self) -> None:
        ensure_binary_on_path(self.binary_name)
        materialize_cafleet_agent(CAFLEET_AGENT)

    def validate_model(self, model: str | None) -> None:
        if model is None:
            return
        provider, sep, model_id = model.partition("/")
        if not sep or not provider or not model_id:
            raise ValueError(
                "--model for the opencode backend must be "
                f"'<provider-id>/<model-id>' (got '{model}')."
            )

    def build_spawn_argv(
        self, prompt: str, *, display_name: str, model: str | None = None
    ) -> list[str]:
        del display_name
        argv = [self.binary_name, "--agent", "cafleet"]
        if model is not None:
            argv.extend(["--model", model])
        argv.extend(["--prompt", prompt])
        return argv
