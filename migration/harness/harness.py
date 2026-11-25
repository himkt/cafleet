"""Differential harness for the parity gates G1 (command parity) and G2 (runner parity).

  python3 -I migration/harness/harness.py [--gate g1|g2|all] [--only NAME ...] [--runs N]
                                          [--work DIR] [--report FILE] REFERENCE [CANDIDATE]

Every scenario rebuilds its world at the same absolute path (`<work>/w`) for every run, drives the
binary through `subprocess` with argv[0] = "cafleet", and records each step's exit code, stdout,
stderr and stand-in multiplexer argv, then the database, the stand-in native logs (argv and the
bytes each native child received) and the installed asset files.

With REFERENCE alone the harness runs each scenario `--runs` times (default 1) and compares every
later run with the first: `--gate g2 --runs 3` is the G2 three-run baseline. A G2 scenario whose
reference runs differ depends on timing: it is reported as dropped, and `scenarios_g2.DROPPED`
records it with the behavior G5 must cover, which removes it from the gate. A G1 scenario whose
reference runs differ is unstable, a harness defect that fails the run. With a CANDIDATE, each
stable scenario's candidate run is compared with the first reference run. `--list` prints the
selected scenarios and the dropped ones.

Comparison rules: an `exact` step compares exit code, stdout and stderr bytes and the step's own
multiplexer argv; a `parse` step compares exit code, empty stdout, the first stderr line and the
accepted-value list; a `help` step compares exit 0, empty stderr and the presence of the strings
SPEC quotes. G2 scenarios additionally compare the runner's own multiplexer argv and the bytes each
native child received. Every multiplexer argv comparison collapses each run of consecutive
identical pane captures into one: how many polls a wait takes before the stand-in native redraws is
timing, and two reference runs differ in it too.

A scenario marked `informational` is compared and reported (status `info` when it differs) but
stays outside the pass condition.
"""
import argparse
import hashlib
import json
import os
import re
import sys
import time
import traceback

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import control  # noqa: E402
import world as W  # noqa: E402


# ---------------------------------------------------------------- quiescence

QUIET_POLLS = 3


def quiesce(world, timeout=30.0, interval=0.5):
    """Wait until the world is quiet (SPEC G2 determinism): every live run's health reply shows no
    pending transitions and the same output sequence and observation on polls `interval` apart,
    and the multiplexer log and database stop changing. The snapshot must hold for QUIET_POLLS
    consecutive polls, a window longer than the runner's one-second preview and Director delivery
    sweeps, so a quiet gap between two sweep ticks is not taken for quiescence. Returns True when
    quiet."""
    deadline = time.monotonic() + timeout
    previous, repeats = None, 0
    while time.monotonic() < deadline:
        current = snapshot(world)
        repeats = repeats + 1 if current is not None and current == previous else 0
        if repeats == QUIET_POLLS - 1:
            return True
        previous = current
        time.sleep(interval)
    return False


def snapshot(world):
    state = []
    if os.path.exists(world.db):
        try:
            members = [r[0] for r in world.query(
                "SELECT member_id FROM member_runs WHERE status NOT IN ('exited','failed','lost') ORDER BY member_id")]
            busy = world.query(
                "SELECT count(*) FROM member_input_operations i JOIN member_runs r USING (run_id) "
                "WHERE r.status NOT IN ('exited','failed','lost') "
                "AND (i.state='in_progress' OR (i.state='ready' AND i.deferred_at IS NULL))")[0][0]
        except Exception:  # noqa: BLE001 - a database mid-write or of another format is simply not quiet yet
            return None
        if busy:
            return None
        for member_id in members:
            try:
                result = control.health(world.db, member_id)
            except (OSError, ConnectionError, RuntimeError, ValueError):
                return None
            if result is None:
                continue
            observation = result.get("observation")
            if observation is not None and observation.get("pending_count"):
                return None
            state.append((member_id, result["output_seq"], json.dumps(observation, sort_keys=True)))
        state.append(hashlib.sha256(world.dump_db().encode()).hexdigest())
    state.append(os.path.getsize(world.mux_log) if os.path.exists(world.mux_log) else 0)
    return state


# ---------------------------------------------------------------- execution

class Recorder:
    def __init__(self, world):
        self.world = world
        self.steps = []
        self.pids = {os.getpid()}
        self.raw_mux_other = []

    def note_panes(self):
        if self.world.daemon is None:
            return
        self.pids.add(self.world.daemon.pid)
        try:
            for pane in self.world.control(op="panes")["panes"]:
                if pane["pid"]:
                    self.pids.add(pane["pid"])
        except OSError:
            pass


def run_scenario(binary, scenario, root):
    """One confined scenario run; aborts the harness if any cafleet-owned entry of the user's
    real home changed while it ran."""
    before = W.real_home_state()
    record = run_confined(binary, scenario, root)
    after = W.real_home_state()
    if before != after:
        changed = sorted(set(before) ^ set(after))
        raise SystemExit(f"scenario {scenario['name']} changed cafleet files of the real home: {changed}")
    return record


def run_confined(binary, scenario, root):
    world = W.World(root, binary, scenario.get("world", {}))
    rec = Recorder(world)
    error = None
    try:
        world.build()
        for step in scenario["steps"]:
            if "do" in step:
                value = step["do"](world)
                if step.get("record"):
                    rec.steps.append({"label": step.get("label", step["do"].__name__), "kind": "action",
                                      "value": json.dumps(value, sort_keys=True, ensure_ascii=False)})
                rec.note_panes()
                continue
            result = world.run(step)
            rec.pids.update(p for p in (result["pid"], result["wrapper_pid"]) if p)
            rec.note_panes()
            quiet = None
            if step.get("quiet"):
                quiet = quiesce(world, timeout=step.get("quiet_timeout", 30.0))
                extra = world.take_mux(-1)
                result["mux"]["other"] += extra["other"] + extra["own"]
                rec.note_panes()
            rec.steps.append({"label": step.get("label") or " ".join(step["argv"][:4]), "kind": step.get("compare", "exact"),
                              "argv": step["argv"], "exit": result["exit"], "stdout": result["stdout"],
                              "stderr": result["stderr"], "mux": result["mux"]["own"], "bg": result["mux"]["other"],
                              "quiet": quiet})
        if scenario.get("settle"):
            quiesce(world, timeout=30.0)
        extra = world.take_mux(-1)
        tail_bg = extra["own"] + extra["other"]
        for pid_row in safe_query(world, "SELECT owner_pid, runner_pid, child_pid FROM member_runs"):
            rec.pids.update(p for p in pid_row if p)
        db = world.dump_db()
        native = world.native_logs()
        files = {base: world.files(base) for base in scenario.get("files", [])}
    except Exception:  # noqa: BLE001 - a harness failure is recorded as the scenario's result
        error = traceback.format_exc()
        db, native, files, tail_bg = "", {}, {}, []
    finally:
        world.stop()
    return normalize(rec, db, native, files, tail_bg, error, binary)


def safe_query(world, statement):
    try:
        return world.query(statement)
    except Exception:  # noqa: BLE001 - absent or foreign databases have no runs
        return []


EXTRA = [
    ("START", re.compile(rb"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}:\d+")),
    ("DEV", re.compile(rb'"device":\d+')),
    ("INO", re.compile(rb'"inode":\d+')),
    ("IDLE", re.compile(rb'"idle":\d+')),
    ("DUR", re.compile(rb"(?<=[ (=])\d+(?:\.\d+)?(?:ns|us|\xc2\xb5s|ms|s|m|h)(?=[ ),;.\n]|$)")),
]


def binary_literals(binary):
    """The binary under test as cafleet reports it (as invoked, and with symlinks resolved)."""
    return {os.fsencode(path): b"<CAFLEET>" for path in {binary, os.path.realpath(binary)}}


RUNTIME_NAMESPACE = re.compile(r"(?:^|(?<=/))[a-z2-7]{26}(?=/|$)")


def normalize_path(norm, path):
    path = RUNTIME_NAMESPACE.sub(lambda m: norm.ordinal("NS", m.group(0).encode()).decode(), path)
    return norm(path)


def normalize(rec, db, native, files, tail_bg, error, binary):
    """Normalize every value with one shared ordinal table, command-owned values first: the
    runner's own multiplexer traffic comes last, so its timing cannot renumber the ordinals of
    the step outputs and rows G1 compares."""
    norm = W.Normalizer(pids=rec.pids, extra=EXTRA, literals=binary_literals(binary))
    steps = []
    for step in rec.steps:
        if step["kind"] == "action":
            steps.append({"label": step["label"], "kind": "action", "value": norm(step["value"])})
            continue
        steps.append({
            "label": step["label"], "kind": step["kind"], "exit": step["exit"],
            "stdout": norm(step["stdout"]).decode("utf-8", "backslashreplace"),
            "stderr": norm(step["stderr"]).decode("utf-8", "backslashreplace"),
            "mux": json.loads(norm(json.dumps(step["mux"]))),
            "quiet": step["quiet"],
        })
    record = {
        "steps": steps,
        "db": norm(db),
        "native": {name: norm(text) for name, text in native.items() if not name.endswith(".in")},
        "files": {base: {normalize_path(norm, path): norm(content) for path, content in listing.items()}
                  for base, listing in files.items()},
        "error": error,
    }
    record["native"].update({name: norm(text) for name, text in native.items() if name.endswith(".in")})
    for normalized, step in zip(steps, rec.steps):
        if step["kind"] != "action":
            normalized["bg"] = json.loads(norm(json.dumps(step["bg"])))
    record["bg_tail"] = json.loads(norm(json.dumps(tail_bg)))
    return record


# ---------------------------------------------------------------- comparison

POSSIBLE = re.compile(r"\[possible values: [^\]]*\]")


def parse_view(step):
    stderr = step["stderr"]
    return {"exit": step["exit"], "stdout_empty": step["stdout"] == "", "first": stderr.split("\n", 1)[0],
            "possible": POSSIBLE.findall(stderr)}


def help_view(step, quoted):
    return {"exit": step["exit"], "stderr": step["stderr"], "quoted": [q for q in quoted if q in step["stdout"]]}


def compare(scenario, ref, got):
    """Every difference between two normalized scenario records, as (where, reference, got)."""
    diffs = []
    if ref["error"] or got["error"]:
        if ref["error"] != got["error"]:
            diffs.append(("harness error", ref["error"], got["error"]))
        return diffs
    if len(ref["steps"]) != len(got["steps"]):
        diffs.append(("step count", len(ref["steps"]), len(got["steps"])))
    runner_gate = scenario["gate"] == "g2"
    for k, (a, b) in enumerate(zip(ref["steps"], got["steps"])):
        where = f"step {k} ({a['label']})"
        if a["kind"] == "action":
            if a["value"] != b["value"]:
                diffs.append((where, a["value"], b["value"]))
            continue
        if a["kind"] == "parse":
            if parse_view(a) != parse_view(b):
                diffs.append((where + " parse", parse_view(a), parse_view(b)))
            continue
        if a["kind"] == "help":
            quoted = scenario.get("help_quotes", [])
            if help_view(a, quoted) != help_view(b, quoted):
                diffs.append((where + " help", help_view(a, quoted), help_view(b, quoted)))
            continue
        for field in ("exit", "stdout", "stderr"):
            if a[field] != b[field]:
                diffs.append((f"{where} {field}", a[field], b[field]))
        if collapse_polls(a["mux"]) != collapse_polls(b["mux"]):
            diffs.append((f"{where} mux", collapse_polls(a["mux"]), collapse_polls(b["mux"])))
    if runner_gate and collapse_polls(runner_mux(ref)) != collapse_polls(runner_mux(got)):
        diffs.append(("runner mux sequence", collapse_polls(runner_mux(ref)), collapse_polls(runner_mux(got))))
    if ref["db"] != got["db"]:
        diffs.append(("database", ref["db"], got["db"]))
    for name in sorted(set(ref["native"]) | set(got["native"])):
        if not runner_gate and name.endswith(".in"):
            continue
        if ref["native"].get(name) != got["native"].get(name):
            diffs.append((f"native {name}", ref["native"].get(name), got["native"].get(name)))
    if ref["files"] != got["files"]:
        diffs.append(("installed files", ref["files"], got["files"]))
    return diffs


def is_capture(call):
    """A pane capture: tmux `capture-pane` or herdr `pane read`."""
    args = call[1]
    return "capture-pane" in args or any(args[k:k + 2] == ["pane", "read"] for k in range(len(args) - 1))


def collapse_polls(calls):
    """calls with each run of consecutive identical pane captures reduced to one."""
    collapsed = []
    for call in calls:
        if collapsed and is_capture(call) and call == collapsed[-1]:
            continue
        collapsed.append(call)
    return collapsed


def runner_mux(record):
    """The runners' own multiplexer calls of a whole scenario, in order. A runner call that a
    settle interval delays past one step's quiet window lands in the next step's log, so the
    sequence, not its split across steps, is the runner's behavior."""
    calls = [call for step in record["steps"] if step["kind"] != "action" for call in step["bg"]]
    return calls + record["bg_tail"]


# ---------------------------------------------------------------- driver

def load_scenarios(gate, include_dropped=False):
    """The gate's scenarios. G2 scenarios that `scenarios_g2.DROPPED` lists (their Rust runs
    differed in the three-run baseline) leave the gate unless `include_dropped`."""
    import scenarios_g1
    import scenarios_g2
    chosen = []
    if gate in ("g1", "all"):
        chosen += [{**s, "gate": "g1"} for s in scenarios_g1.SCENARIOS]
    if gate in ("g2", "all"):
        chosen += [{**s, "gate": "g2"} for s in scenarios_g2.SCENARIOS
                   if include_dropped or s["name"] not in scenarios_g2.DROPPED]
    names = [s["name"] for s in chosen]
    duplicates = {n for n in names if names.count(n) > 1}
    assert not duplicates, f"duplicate scenario names: {sorted(duplicates)}"
    unknown = set(scenarios_g2.DROPPED) - {s["name"] for s in scenarios_g2.SCENARIOS}
    assert not unknown, f"DROPPED names no G2 scenario: {sorted(unknown)}"
    return chosen


def short(value, limit=1500):
    text = value if isinstance(value, str) else json.dumps(value, ensure_ascii=False)
    return text if len(text) <= limit else text[:limit] + f"... <{len(text)} chars>"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("reference", nargs="?")
    parser.add_argument("candidate", nargs="?")
    parser.add_argument("--gate", choices=("g1", "g2", "all"), default="all")
    parser.add_argument("--only", action="append", default=[])
    parser.add_argument("--runs", type=int, default=1)
    parser.add_argument("--work", default=f"/tmp/cafleet-harness-{os.getuid()}")
    parser.add_argument("--report", default=None)
    parser.add_argument("--records", default=None, help="write every normalized reference run to this JSON file")
    parser.add_argument("--include-dropped", action="store_true", help="also run the G2 scenarios DROPPED lists")
    parser.add_argument("--list", action="store_true", help="print the selected scenario names and exit")
    args = parser.parse_args(argv)
    selected = [s for s in load_scenarios(args.gate, args.include_dropped) if not args.only or s["name"] in args.only]
    assert selected, f"no scenario matches {args.only}"
    if args.list:
        import scenarios_g2
        for s in selected:
            print(s["gate"], s["name"])
        for name, reason in scenarios_g2.DROPPED.items():
            print("g2-dropped", name, "-", reason)
        return 0
    if args.reference is None:
        parser.error("REFERENCE is required")
    reference = os.path.abspath(args.reference)
    candidate = os.path.abspath(args.candidate) if args.candidate else None
    root = os.path.join(args.work, "w")
    os.makedirs(args.work, exist_ok=True)
    scenarios = selected
    report = {"reference": reference, "candidate": candidate, "runs": args.runs, "scenarios": []}
    records = {}
    failures = 0
    for scenario in scenarios:
        started = time.monotonic()
        runs = [run_scenario(reference, scenario, root) for _ in range(args.runs)]
        records[scenario["name"]] = runs
        unstable = [(k, compare(scenario, runs[0], run)) for k, run in enumerate(runs[1:], start=2)]
        unstable = [(k, d) for k, d in unstable if d]
        entry = {"name": scenario["name"], "gate": scenario["gate"], "steps": len(runs[0]["steps"]),
                 "error": runs[0]["error"], "unstable_runs": [
                     {"run": k, "diffs": [{"where": w, "first": short(a), "this": short(b)} for w, a, b in d]}
                     for k, d in unstable]}
        unstable_status = "dropped" if scenario["gate"] == "g2" else "unstable"
        status = "error" if runs[0]["error"] else unstable_status if unstable else "stable"
        if candidate and not unstable and not runs[0]["error"]:
            got = run_scenario(candidate, scenario, root)
            diffs = compare(scenario, runs[0], got)
            entry["diffs"] = [{"where": w, "reference": short(a), "candidate": short(b)} for w, a, b in diffs]
            if scenario.get("informational"):
                status = "pass" if not diffs else "info"
            else:
                status = "pass" if not diffs else "FAIL"
                failures += bool(diffs)
        failures += status in ("error", "unstable")
        entry["status"] = status
        report["scenarios"].append(entry)
        unquiet = sum(1 for step in runs[0]["steps"] if step.get("quiet") is False)
        entry["unquiet_steps"] = unquiet
        print(f"{scenario['gate']} {scenario['name']:<44} {status:<8} steps={len(runs[0]['steps'])} "
              f"unquiet={unquiet} {time.monotonic() - started:6.1f}s", flush=True)
        if runs[0]["error"]:
            print(runs[0]["error"], flush=True)
        if status == "FAIL":
            for diff in entry["diffs"][:3]:
                print(f"  {diff['where']}\n    reference: {diff['reference']!r}\n    candidate: {diff['candidate']!r}",
                      flush=True)
    dropped = [s["name"] for s in report["scenarios"] if s["status"] == "dropped"]
    unstable = [s["name"] for s in report["scenarios"] if s["status"] == "unstable"]
    report["dropped"], report["unstable"] = dropped, unstable
    print(f"scenarios={len(report['scenarios'])} dropped={len(dropped)} unstable={len(unstable)} failing={failures}")
    for name in dropped:
        print(f"  G2 dropped (reference runs differ; cover in G5): {name}")
    for name in unstable:
        print(f"  G1 unstable (reference runs differ; a harness defect): {name}")
    if args.report:
        with open(args.report, "w", errors="backslashreplace") as f:
            json.dump(report, f, indent=1, ensure_ascii=False)
    if args.records:
        with open(args.records, "w") as f:
            json.dump(records, f, indent=1)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
