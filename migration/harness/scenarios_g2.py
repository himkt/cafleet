"""G2 runner-parity scenarios: `__member-run` driven through member creation, broker previews,
Director delivery, prompts, queued input, permission answers, manual input, child exit and
deletion. Every step waits for quiescence, and the comparison covers the rows, events and input
receipts, the runner's own multiplexer argv and the bytes each native child received."""
from scenarios_g1 import PERMISSION_NATIVE, U, do, fleet_herdr, fleet_tmux, member, respond, send, setup, type_into

SCENARIOS = []

# Scenarios whose three Rust runs differed in the G2 baseline. Each leaves G2; the value names the
# timing-dependent behavior a clock-driven white-box test must cover under G5.
DROPPED = {
    "g2-director-pi-tmux": (
        "Director delivery of a second message to a pi Director right after the first submission: whether "
        "the runner types it before the Director's next poll consumes it depends on the post-submit settle "
        "interval and the one-second Director sweep (one of three Rust runs left it unnotified)"),
}

IDLE ={"claude": "claude-2.1.322-composer-prompt-plain-41-post-submit-tmux-120-1"}


def scenario(name, world, *steps, **extra):
    SCENARIOS.append({"name": name, "world": world, "settle": True,
                      "steps": [s if isinstance(s, dict) else {"argv": s, "quiet": True} for s in steps], **extra})


def q(argv, **extra):
    return {"argv": argv, "quiet": True, **extra}


def busy_native(backend="claude"):
    return {backend: {
        "start": "idle",
        "screens": {"idle": {"fixture": IDLE[backend], "composer": True, "continuation": "  "},
                    "busy": {"fixture": "claude-2.1.322-queued-hint-tmux-120-1"}},
        "rules": [{"when": "submit", "match": "^go busy", "goto": "busy"},
                  {"when": "key", "match": "^z$", "state": "busy", "goto": "idle"},
                  {"when": "submit", "match": "^/exit$", "exit": 0, "print": "bye"},
                  {"when": "submit", "match": "^/fail$", "exit": 3, "print": "failing"}]}}


BODY_2000 = "long body " + "word " * 400

scenario("g2-preview-sweep-tmux", {"layout": "tmux"}, setup(), fleet_tmux(), member("m1", "claude"),
         q(send(1, 2, "first message", *U)["argv"]),
         q(send(1, 2, "second message with quote\" and back\\slash", *U)["argv"]),
         q(send(1, 2, "界🙂 wide characters wrap " * 6, *U)["argv"]),
         q(send(1, 2, BODY_2000, *U)["argv"]),
         q(send(2, 2, "self send never previews", *U)["argv"]),
         q(["message", "poll", "2", "--json", *U]),
         q(["member", "show", "2", "--json", *U]))
scenario("g2-preview-batch-tmux", {"layout": "tmux"}, setup(), fleet_tmux(), member("m1", "claude"),
         do(lambda w: [w.run({"argv": send(1, 2, f"burst {k}", *U)["argv"]}) for k in range(5)], "send five messages"),
         q(["message", "show", "5", "--json", *U]),
         q(["message", "poll", "2", "--json", *U]))
scenario("g2-director-delivery-tmux", {"layout": "tmux"}, setup(), fleet_tmux(), member("m1", "claude"),
         q(send(2, 1, "report from member", *U)["argv"]),
         q(send(2, 1, "a much longer report from the member " * 12, *U)["argv"]),
         q(send(2, 1, BODY_2000, *U)["argv"]),
         q(["message", "poll", "1", "--json", *U]))
scenario("g2-director-delivery-two-members", {"layout": "tmux"}, setup(), fleet_tmux(), member("m1", "claude"),
         member("m2", "claude"),
         q(send(2, 1, "from member two", *U)["argv"]),
         q(send(3, 1, "from member three", *U)["argv"]),
         q(send(3, 2, "member to member", *U)["argv"]),
         q(["message", "poll", "1", "--json", *U]))
scenario("g2-prompt-exec-press-tmux", {"layout": "tmux"}, setup(), fleet_tmux(), member("m1", "claude"),
         q(["member", "prompt", "2", "--from-member-id", "1", "plain prompt", *U]),
         q(["member", "exec", "2", "--from-member-id", "1", "printf exec", *U]),
         q(["member", "prompt", "2", "--from-member-id", "1", "p" * 300, *U]),
         q(["member", "press", "2", "--from-member-id", "1", "h", "i", "Backspace", "Enter", *U]),
         q(["input", "show", "1", "--json", *U]),
         q(["input", "show", "2", "--json", *U]))
scenario("g2-queued-input-tmux", {"layout": "tmux", "natives": busy_native()}, setup(), fleet_tmux(), member("m1", "claude"),
         q(["member", "prompt", "2", "--from-member-id", "1", "go busy", *U]),
         q(["member", "exec", "2", "--from-member-id", "1", "queued exec", *U]),
         q(["member", "prompt", "2", "--from-member-id", "1", "queued prompt", *U]),
         q(send(1, 2, "queued preview", *U)["argv"]),
         q(["member", "press", "2", "--from-member-id", "1", "z", *U], quiet_timeout=60),
         q(["input", "show", "2", "--json", *U]),
         q(["input", "show", "3", "--json", *U]),
         q(["message", "poll", "1", "--json", *U]))
scenario("g2-manual-input-tmux", {"layout": "tmux"}, setup(), fleet_tmux(), member("m1", "claude"),
         type_into("%1", "typed by a human"),
         {"do": lambda w: __import__("harness").quiesce(w), "label": "quiesce"},
         q(send(1, 2, "preview after manual draft", *U)["argv"]),
         type_into("%1", "\x7f" * 16 + "\r"),
         {"do": lambda w: __import__("harness").quiesce(w), "label": "quiesce"},
         q(["member", "show", "2", "--json", *U]))
scenario("g2-permission-tmux", {"layout": "tmux", "natives": PERMISSION_NATIVE}, setup(), fleet_tmux(), member("m1", "claude"),
         q(["member", "prompt", "2", "--from-member-id", "1", "run the probe", *U]),
         q(send(1, 2, "preview held by the open interaction", *U)["argv"]),
         respond(1, decision="allow"),
         q(["member", "prompt", "2", "--from-member-id", "1", "run the probe again", *U]),
         respond(2, decision="deny"),
         q(["message", "poll", "1", "--json", *U]),
         q(["message", "poll", "2", "--json", *U]))
scenario("g2-child-exit-tmux", {"layout": "tmux", "natives": busy_native()}, setup(), fleet_tmux(), member("m1", "claude"),
         member("m2", "claude"),
         q(["member", "prompt", "2", "--from-member-id", "1", "/exit", *U]),
         q(["member", "prompt", "3", "--from-member-id", "1", "/fail", *U]),
         q(["member", "show", "2", "--json", *U]),
         q(["member", "list", "1", "--json", *U]),
         q(["message", "poll", "1", "--json", *U]),
         q(["member", "delete", "2", *U]),
         q(["member", "delete", "3", "--json", *U]))
scenario("g2-resize-tmux", {"layout": "tmux"}, setup(), fleet_tmux(), member("m1", "claude"), member("m2", "claude"),
         q(send(1, 3, "preview into a half-height pane", *U)["argv"]),
         q(["member", "delete", "2", *U]),
         q(send(1, 3, "preview after the pane grew back", *U)["argv"]))
scenario("g2-delete-and-fleet-delete-tmux", {"layout": "tmux"}, setup(), fleet_tmux(), member("m1", "claude"),
         member("m2", "claude"), member("m3", "claude"),
         q(["member", "delete", "3", *U]),
         q(["fleet", "delete", "1", "--json", *U], quiet_timeout=60))
for backend in ("codex", "opencode", "hermes", "pi"):
    scenario(f"g2-backend-{backend}-tmux", {"layout": "tmux"}, setup(), fleet_tmux(), member("m1", backend),
             q(send(1, 2, f"preview for {backend}", *U)["argv"]),
             q(["member", "prompt", "2", "--from-member-id", "1", f"prompt for {backend}", *U]),
             q(["member", "exec", "2", "--from-member-id", "1", "printf hi", *U]),
             q(send(2, 1, f"report from {backend}", *U)["argv"]),
             q(["input", "show", "1", "--json", *U]),
             q(["member", "delete", "2", *U], quiet_timeout=60))
for director in ("codex", "opencode", "hermes", "pi"):
    scenario(f"g2-director-{director}-tmux", {"layout": "tmux", "director_backend": director}, setup(),
             q(["fleet", "create", "--name", "alpha", "--coding-agent", director, "--pane", "%0", "--mux-server",
                "{W}/tmux.sock", *U]),
             member("m1", "claude"),
             q(send(2, 1, f"report to a {director} Director", *U)["argv"]),
             q(send(2, 1, "a longer report " * 20, *U)["argv"]),
             q(["message", "poll", "1", "--json", *U]))
scenario("g2-preview-and-delivery-herdr", {"layout": "herdr"}, setup(), fleet_herdr(), member("m1", "claude"),
         member("m2", "codex"),
         q(send(1, 2, "herdr preview", *U)["argv"]),
         q(send(1, 3, "herdr preview for codex", *U)["argv"]),
         q(send(2, 1, "herdr report", *U)["argv"]),
         q(["member", "prompt", "2", "--from-member-id", "1", "herdr prompt", *U]),
         q(["member", "press", "3", "--from-member-id", "1", "x", "Enter", *U]),
         q(["fleet", "delete", "1", *U], quiet_timeout=60))
