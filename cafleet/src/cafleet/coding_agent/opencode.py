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

    def build_spawn_argv(self, prompt: str, *, display_name: str) -> list[str]:
        del display_name
        return [
            self.binary_name,
            "--agent",
            "cafleet",
            "--prompt",
            prompt,
        ]
