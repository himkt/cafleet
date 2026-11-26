"""G1 command-parity scenarios: every SPEC §6.3 command, run through the stand-in multiplexer and
native executables.

A scenario is {"name", "world", "steps", optional "files", "help_quotes", "settle"}. A step is a
command ({"argv", optional "env", "stdin", "stdout" sink, "compare", "quiet", "via", "cwd"}) or an
action ({"do": callable(world), optional "record"}). `quiet` waits for runner quiescence after the
command, so asynchronous runner effects land before the next step.
"""
import os
import sqlite3

U = ["--database-url", "{URL}"]
TMUX = "{W}/tmux.sock"
PROMPT = ("FLEET ID: {fleet_id}\nYOUR MEMBER ID: {member_id}\nDIRECTOR MEMBER ID: {director_member_id}\n"
          "CODING AGENT: {coding_agent}\nDATABASE URL: {database_url}\n")
SEED_TS = "2026-11-24T00:00:0{}.000000+00:00"
ESCAPES = "quote\" back\\ slash/ nl\n tab\t cr\r bs\b ff\f nul-ish\x01\x1f del\x7f ls  ps  <&> é 界 🙂"
LONG = "x" * 199 + "界🙂" + "y" * 50
PERMISSION_MODE_HELP = (
    "Launch-only native permission policy for the resolved backend — claude: default, manual, dontAsk, "
    "acceptEdits, auto, bypassPermissions, plan (default auto); codex: on-request, never, approve-for-me "
    "(default approve-for-me); opencode: cafleet, cafleet-ask (default cafleet-ask); hermes: smart (default "
    "smart); pi: cafleet, cafleet-ask (default cafleet-ask)")
PANE_HELP = ("The Director's pane; its tmux %N or herdr <workspace>:<pane> form selects the multiplexer [default: "
             "the caller's TMUX_PANE or HERDR_PANE_ID, verified against the caller's process ancestry]")
SERVER_HELP = "The Director's tmux socket path or herdr session [default: the caller's server on the pane's multiplexer]"

SCENARIOS = []


def scenario(name, world, *steps, **extra):
    SCENARIOS.append({"name": name, "world": world,
                      "steps": [s if isinstance(s, dict) else {"argv": s} for s in steps], **extra})


def setup(*extra):
    return {"argv": ["setup", *extra, *U], "label": "setup"}


def fleet_tmux(*extra):
    return {"argv": ["fleet", "create", "--name", "alpha", "--coding-agent", "claude", "--pane", "%0",
                     "--mux-server", TMUX, *extra, *U], "label": "fleet create (tmux)"}


def fleet_herdr(*extra):
    return {"argv": ["fleet", "create", "--name", "alpha", "--coding-agent", "claude", "--pane", "w1:p1",
                     "--mux-server", "default", *extra, *U], "label": "fleet create (herdr)"}


def member(name, agent, *extra, prompt=PROMPT, fleet="1", quiet=True, env=None):
    step = {"argv": ["member", "create", "--fleet-id", fleet, "--name", name, "--description", f"{name} member",
                     "--coding-agent", agent, *extra, prompt, *U], "quiet": quiet, "label": f"member create {name}"}
    if env:
        step["env"] = env
    return step


def send(frm, to, *rest, quiet=False):
    return {"argv": ["message", "send", "--from-member-id", str(frm), "--to-member-id", str(to), *rest],
            "quiet": quiet}


def quiet(world):
    import harness
    return harness.quiesce(world)


QUIET = {"do": quiet, "label": "quiesce"}


# ---------------------------------------------------------------- seeding

def seed_fleet(conn, name, ts, director_pane):
    fleet = conn.execute("INSERT INTO fleets (name, created_at) VALUES (?, ?) RETURNING fleet_id", (name, ts)).fetchone()[0]
    director = conn.execute(
        "INSERT INTO members (fleet_id, name, description, status, registered_at) "
        "VALUES (?, 'Director', 'Root Director for this fleet', 'active', ?) RETURNING member_id", (fleet, ts)).fetchone()[0]
    conn.execute(
        "INSERT INTO member_placements (member_id, mux_session, mux_window_id, mux_window_name, window_ordinal, "
        "column_side, mux_pane_id, backend, mux_server, coding_agent, created_at) "
        "VALUES (?, 'main', '@1', ?, 0, 'left', ?, 'tmux', '/tmp/tmux-0/default', 'claude', ?)",
        (director, f"cafleet-{fleet}-0", director_pane, ts))
    conn.execute("UPDATE fleets SET director_member_id=? WHERE fleet_id=?", (director, fleet))
    return fleet, director


def seed_member(conn, fleet, name, ts, status="active"):
    member_id = conn.execute(
        "INSERT INTO members (fleet_id, name, description, status, registered_at) "
        "VALUES (?, ?, 'seeded', ?, ?) RETURNING member_id", (fleet, name, status, ts)).fetchone()[0]
    if status == "active":
        conn.execute(
            "INSERT INTO member_placements (member_id, mux_session, mux_window_id, mux_window_name, window_ordinal, "
            "column_side, mux_pane_id, backend, mux_server, coding_agent, created_at) "
            "VALUES (?, 'main', NULL, NULL, 1, 'left', NULL, 'tmux', '/tmp/tmux-0/default', 'claude', ?)",
            (member_id, ts))
    return member_id


def seed_standard(world):
    """fleet 1: Director 1, members 2, 3 (deregistered 3); fleet 2: Director 4, member 5;
    fleet 3: Director 6 deregistered, member 7; fleet 4 deleted: Director 8, member 9."""
    conn = sqlite3.connect(world.db, isolation_level=None)
    conn.execute("PRAGMA foreign_keys=ON")
    conn.execute("BEGIN IMMEDIATE")
    f1, _ = seed_fleet(conn, "alpha", SEED_TS.format(1), "%1")
    seed_member(conn, f1, "worker-a", SEED_TS.format(1))
    seed_member(conn, f1, "gone", SEED_TS.format(1), status="deregistered")
    f2, _ = seed_fleet(conn, "beta", SEED_TS.format(2), "%2")
    seed_member(conn, f2, "worker-b", SEED_TS.format(2))
    f3, d3 = seed_fleet(conn, "gamma", SEED_TS.format(3), "%3")
    seed_member(conn, f3, "orphan", SEED_TS.format(3))
    conn.execute("UPDATE members SET status='deregistered' WHERE member_id=?", (d3,))
    f4, _ = seed_fleet(conn, "delta", SEED_TS.format(3), "%4")
    seed_member(conn, f4, "late", SEED_TS.format(3))
    conn.execute("UPDATE fleets SET deleted_at=? WHERE fleet_id=?", (SEED_TS.format(4), f4))
    conn.execute("COMMIT")
    conn.close()
    files_fixture(world)


def write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)


def files_fixture(world):
    w = world.root
    write(f"{w}/body.txt", "file body\nline two\r\n".encode())
    write(f"{w}/empty.txt", b"")
    write(f"{w}/blank.txt", b" \t\n\x1c\x1f\xe3\x80\x80")
    write(f"{w}/latin1.txt", b"caf\xe9")
    write(f"{w}/nul.txt", b"a\x00b")
    write(f"{w}/big.txt", ("界" * 40000 + "\n").encode())
    write(f"{w}/prompt.txt", PROMPT.encode())
    write(f"{w}/cmd.txt", b"printf from-file\n")
    write(f"{w}/unreadable.txt", b"secret")
    os.chmod(f"{w}/unreadable.txt", 0)
    os.makedirs(f"{w}/adir", exist_ok=True)


def do(fn, label=None, record=False):
    return {"do": fn, "label": label or fn.__name__, "record": record}


def sql(*statements):
    def run(world):
        world.sql(list(statements))
    run.__name__ = "sql: " + "; ".join(statements)[:60]
    return do(run)


def write_file(rel, data, mode=None):
    def run(world):
        path = world.subst(rel)
        write(path, data)
        if mode is not None:
            os.chmod(path, mode)
    run.__name__ = f"write {rel}"
    return do(run)


def remove(rel):
    def run(world):
        path = world.subst(rel)
        if os.path.isdir(path) and not os.path.islink(path):
            import shutil
            shutil.rmtree(path)
        elif os.path.lexists(path):
            os.unlink(path)
    run.__name__ = f"remove {rel}"
    return do(run)


def type_into(pane, data):
    def run(world):
        world.control(op="type", pane=pane, data=data)
    run.__name__ = f"type into {pane}"
    return do(run)


def screen_of(pane):
    def run(world):
        return [line.rstrip() for line in world.control(op="screen", pane=pane)["lines"]]
    run.__name__ = f"screen {pane}"
    return do(run, record=True)


def hold_lock(mode, seconds):
    """Hold a `BEGIN <mode>` SQLite lock on the world database for `seconds` from a background
    thread; the action returns once the lock is held, so the next step starts under it."""
    def run(world):
        import threading
        held = threading.Event()

        def holder():
            conn = sqlite3.connect(world.db, timeout=0, isolation_level=None, check_same_thread=False)
            conn.execute(f"BEGIN {mode}")
            held.set()
            import time
            time.sleep(seconds)
            conn.execute("COMMIT")
            conn.close()
        thread = threading.Thread(target=holder, daemon=True)
        thread.start()
        assert held.wait(5), "the lock holder never acquired its lock"
        world.lock_holder = thread
    run.__name__ = f"hold {mode} for {seconds}s"
    return do(run)


def release_lock(world):
    world.lock_holder.join()


SEEDED = [setup(), do(seed_standard)]
DB = [setup()]

# ---------------------------------------------------------------- database selection and the schema guard

scenario("missing-database", {},
         ["--database-url", "{URL}", "fleet", "list"],
         ["--database-url", "{URL}", "message", "poll", "1"],
         ["message", "show", "1", "--database-url", "{URL}"],
         send(1, 2, "hi", "--database-url", "{URL}"),
         send(1, 2, "", "--database-url", "{URL}"),
         ["--database-url", "postgres://x", "message", "poll", "1"],
         ["--database-url", "sqlite://relative.db", "fleet", "list"],
         ["--database-url=", "fleet", "list"],
         ["member", "list", "1", *U],
         ["member", "show", "1", *U],
         ["event", "show", "1", *U],
         ["input", "show", "1", *U],
         ["permission", "inspect", "1", "--event-id", "1", "--from-member-id", "1", *U],
         ["fleet", "show", "1", *U],
         ["fleet", "delete", "1", *U],
         ["member", "delete", "2", *U],
         ["member", "capture", "2", *U],
         ["member", "prompt", "2", "--from-member-id", "1", "hi", *U],
         ["member", "exec", "2", "--from-member-id", "1", "true", *U],
         ["member", "press", "2", "--from-member-id", "1", "Enter", *U],
         ["input", "reconcile", "1", "--from-member-id", "1", "--disposition", "submitted", "--evidence-ref", "x", *U],
         ["permission", "respond", "1", "--event-id", "1", "--from-member-id", "1", "--expected-run-id", "1",
          "--expected-owner-epoch", "1", "--request-token", "0" * 64, "--decision", "allow", *U],
         ["member", "create", "--fleet-id", "1", "--name", "x", "--description", "d", "", *U],
         ["member", "create", "--fleet-id", "1", "--name", "x", "--description", "d", "{bogus}", *U],
         ["fleet", "create", "--name", "a", "--coding-agent", "claude", "--pane", "%0", "--mux-server", TMUX, *U],
         ["message", "send", "--from-member-id", "1", "--to-member-id", "2", "--file", "{W}/nope", *U])
for name, prepare in [
    ("empty-file", [write_file("{DB}", b"")]),
    ("garbage-file", [write_file("{DB}", b"this is not an sqlite database" * 200)]),
    ("foreign-app-id", [setup(), sql("PRAGMA application_id=42")]),
    ("other-version", [setup(), sql("UPDATE cafleet_database SET cafleet_version='0.35.5'")]),
    ("no-version-table", [setup(), sql("DROP TABLE cafleet_database")]),
    ("plain-sqlite", [do(lambda w: sqlite3.connect(w.db).execute("CREATE TABLE t(x)").connection.commit(),
                         "create foreign table")]),
    ("db-is-dir", [do(lambda w: os.makedirs(w.db), "make database path a directory")]),
]:
    scenario(f"format-{name}", {}, *prepare,
             ["--database-url", "{URL}", "fleet", "list"],
             ["--database-url", "{URL}", "message", "poll", "1", "--json"],
             send(1, 1, "x", "--database-url", "{URL}"),
             ["member", "list", "1", *U],
             ["doctor", *U],
             ["doctor", "--json", *U],
             setup(),
             ["--database-url", "{URL}", "fleet", "list"])
scenario("db-under-file", {}, write_file("{W}/file", b"x"),
         ["--database-url", "sqlite:///{W}/file/cafleet.db", "fleet", "list"],
         ["--database-url", "sqlite:///{W}/file/cafleet.db", "setup"],
         ["--database-url", "sqlite:///{W}/file/cafleet.db", "doctor"])
scenario("unreadable-db-dir", {}, setup(), do(lambda w: os.chmod(w.root, 0o300), "make the world unreadable"),
         ["--database-url", "{URL}", "fleet", "list"],
         ["--database-url", "{URL}", "doctor"],
         do(lambda w: os.chmod(w.root, 0o700), "restore the world"))
scenario("relative-url", {}, setup(),
         ["--database-url", "sqlite:///cafleet.db", "fleet", "list", "--json"],
         ["--database-url", "sqlite:///./cafleet.db", "message", "poll", "1"],
         {"argv": ["--database-url", "sqlite:///../w/cafleet.db", "fleet", "list"], "cwd": "{W}/home"},
         ["--database-url", "sqlite:///~/cafleet.db", "fleet", "list"],
         ["--database-url", "sqlite:///{W}//cafleet.db", "fleet", "list"],
         ["--database-url", "sqlite:///{W}/./cafleet.db", "message", "poll", "1"],
         do(lambda w: os.symlink(w.db, f"{w.root}/link.db"), "symlink the database"),
         ["--database-url", "sqlite:///{W}/link.db", "fleet", "list", "--json"])
scenario("database-url-no-path", {},
         ["--database-url", "sqlite:///", "setup"],
         ["--database-url", "sqlite:///", "fleet", "list"],
         ["--database-url", "sqlite:///", "doctor"])
scenario("database-url-repeats", {}, setup(),
         ["--database-url", "{URL}", "--database-url", "{URL}", "fleet", "list"],
         ["fleet", "list", "--database-url", "{URL}", "--database-url", "{URL}"],
         ["--database-url", "sqlite:///{W}/other.db", "fleet", "list", "--database-url", "{URL}"],
         ["--database-url", "{URL}", "fleet", "list", "--database-url", "sqlite:///{W}/other.db"],
         ["fleet", "--database-url", "{URL}", "list"],
         ["--database-url={URL}", "fleet", "list"],
         ["fleet", "list", "--database-url={URL}"])
scenario("empty-registry", {}, setup(),
         ["--database-url", "{URL}", "fleet", "list"],
         ["--database-url", "{URL}", "fleet", "list", "--json"],
         ["--database-url", "{URL}", "message", "poll", "1"],
         ["--database-url", "{URL}", "message", "show", "1"],
         ["member", "list", "1", *U],
         ["member", "list", "1", "--json", *U],
         ["fleet", "show", "1", "--json", *U],
         ["fleet", "delete", "1", "--json", *U],
         ["member", "show", "1", "--json", *U],
         ["member", "delete", "1", *U],
         ["member", "capture", "1", *U],
         ["event", "show", "1", "--json", *U],
         ["input", "show", "1", "--json", *U],
         ["permission", "inspect", "1", "--event-id", "1", "--from-member-id", "1", "--json", *U],
         ["member", "prompt", "1", "--from-member-id", "1", "hi", *U],
         ["member", "exec", "1", "--from-member-id", "1", "true", *U],
         ["member", "press", "1", "--from-member-id", "1", "Enter", *U],
         ["member", "create", "--fleet-id", "1", "--name", "x", "--description", "d", PROMPT, *U],
         ["input", "reconcile", "1", "--from-member-id", "1", "--disposition", "submitted", "--evidence-ref", "x", *U])

# ---------------------------------------------------------------- broker

scenario("broker-flow", {}, *SEEDED,
         ["fleet", "list", *U],
         ["fleet", "list", "--json", *U],
         [*U, *send(1, 2, "hello worker")["argv"]],
         send(2, 1, ESCAPES, "--json", *U),
         send(2, 1, LONG, *U),
         send(2, 2, "note to self", "--json", *U),
         send(1, 2, "--file", "{W}/body.txt", *U),
         send(1, 2, "--json", "--file", "{W}/nul.txt", *U),
         {**send(1, 2, "--file", "-", *U), "stdin": "from stdin 界\n".encode()},
         send(1, 2, "--file", "{W}/big.txt", "--json", *U),
         [*U, *send(1, 2, "--", "--looks-like-a-flag")["argv"]],
         send(1, 2, "-", *U),
         ["message", "show", "1", *U],
         ["message", "show", "2", "--json", *U],
         ["message", "show", "3", *U],
         ["message", "poll", "1", *U],
         ["message", "poll", "1", "--json", *U],
         ["message", "poll", "2", "--json", *U],
         ["message", "poll", "2", *U],
         ["message", "poll", "2", "--json", *U],
         ["message", "show", "2", *U],
         ["message", "show", "2", "--json", *U],
         ["message", "poll", "+2", *U],
         [*U, "message", "poll", "--", "-1"],
         ["message", "show", "999", *U],
         [*U, "message", "show", "--", "-5"],
         ["member", "list", "1", *U],
         ["member", "list", "1", "--json", *U],
         ["member", "list", "2", *U],
         ["member", "list", "3", "--json", *U],
         ["member", "list", "4", *U],
         ["member", "list", "99", *U])
scenario("send-errors", {}, *SEEDED,
         send(2, 5, "cross fleet", *U),
         send(99, 1, "unknown sender", *U),
         send(1, 98, "unknown recipient", *U),
         send(3, 1, "deregistered sender", *U),
         send(1, 3, "deregistered recipient", *U),
         send(7, 7, "inactive director fleet", *U),
         send(9, 8, "deleted fleet", *U),
         send(1, 2, "", *U),
         send(1, 2, " \t\n　\x1c", *U),
         send(1, 2, "--file", "{W}/empty.txt", *U),
         send(1, 2, "--file", "{W}/blank.txt", *U),
         send(1, 2, "--file", "{W}/missing.txt", *U),
         send(1, 2, "--file", "{W}/adir", *U),
         send(1, 2, "--file", "{W}/latin1.txt", *U),
         send(1, 2, "--file", "{W}/unreadable.txt", *U),
         send(1, 2, "--file", "/dev/null", *U),
         send(1, 2, "--file=", *U),
         {**send(1, 2, "--file", "-", *U), "stdin": b""},
         {**send(1, 2, "--file", "-", *U), "stdin": b"  \n"},
         {**send(1, 2, "--file", "-", *U), "stdin": b"\xff\xfe"},
         ["message", "poll", "99", *U],
         ["message", "poll", "3", *U],
         ["message", "poll", "0", "--json", *U],
         ["message", "poll", "7", *U],
         ["message", "poll", "9", *U])
scenario("output-failures", {}, *SEEDED,
         {**send(1, 2, "to /dev/full", *U), "stdout": "devfull"},
         {"argv": ["message", "show", "1", *U], "stdout": "devfull"},
         {"argv": ["fleet", "list", "--json", *U], "stdout": "devfull"},
         send(1, 2, "second", *U),
         {"argv": ["message", "poll", "2", *U], "stdout": "broken-pipe"},
         ["message", "poll", "2", "--json", *U],
         ["message", "show", "2", "--json", *U],
         {"argv": ["member", "list", "1", *U], "stdout": "broken-pipe"},
         {"argv": ["member", "list", "1", "--json", *U], "stdout": "devfull"},
         {"argv": ["fleet", "show", "1", *U], "stdout": "devfull"},
         {"argv": ["--version"], "stdout": "devfull"},
         {"argv": ["--help"], "stdout": "devfull"},
         {"argv": ["doctor", *U], "stdout": "devfull"},
         {"argv": ["setup", *U], "stdout": "devfull"},
         {"argv": ["setup", *U], "stdout": "broken-pipe"})
LOCKED_COMMANDS = [send(1, 2, "under a lock", *U)["argv"], ["message", "poll", "2", *U], ["fleet", "list", *U],
                   ["member", "list", "1", "--json", *U], ["message", "show", "1", *U], ["fleet", "delete", "2", *U],
                   ["member", "delete", "5", *U], ["setup", *U], ["doctor", *U]]
# Well past the five-second busy timeout: on macOS CI runners the reference binary was observed to
# keep waiting for more than seven wall-clock seconds before reporting the lock.
OUTLASTING_LOCK_SECONDS = 12
scenario("database-locks", {}, *SEEDED, send(1, 2, "before the locks", *U),
         *[step for argv in LOCKED_COMMANDS
           for step in (hold_lock("EXCLUSIVE", OUTLASTING_LOCK_SECONDS), {"argv": argv}, do(release_lock))],
         *[step for argv in LOCKED_COMMANDS[:3]
           for step in (hold_lock("IMMEDIATE", OUTLASTING_LOCK_SECONDS), {"argv": argv}, do(release_lock))],
         *[step for argv in LOCKED_COMMANDS[:2] for step in (hold_lock("EXCLUSIVE", 1.5), {"argv": argv}, do(release_lock))],
         ["message", "poll", "2", "--json", *U])
scenario("seeded-reads", {}, *SEEDED,
         ["fleet", "show", "1", *U],
         ["fleet", "show", "4", "--json", *U],
         ["fleet", "show", "99", *U],
         ["member", "show", "1", *U],
         ["member", "show", "2", "--json", *U],
         ["member", "show", "3", *U],
         ["member", "show", "1", "--json", *U],
         ["member", "capture", "2", *U],
         ["member", "capture", "1", *U],
         ["member", "prompt", "2", "--from-member-id", "1", "hi", *U],
         ["member", "exec", "2", "--from-member-id", "1", "true", *U],
         ["member", "press", "2", "--from-member-id", "1", "Enter", *U],
         ["member", "prompt", "2", "--from-member-id", "2", "hi", *U],
         ["member", "prompt", "5", "--from-member-id", "1", "hi", *U],
         ["member", "prompt", "1", "--from-member-id", "1", "hi", *U],
         ["member", "exec", "1", "--from-member-id", "1", "true", *U],
         ["member", "press", "1", "--from-member-id", "1", "Enter", *U],
         ["member", "prompt", "7", "--from-member-id", "6", "hi", *U],
         ["member", "prompt", "9", "--from-member-id", "8", "hi", *U],
         ["member", "delete", "1", *U],
         ["member", "delete", "3", *U],
         ["member", "delete", "99", *U],
         ["member", "delete", "2", "--json", *U],
         ["member", "list", "1", "--json", *U],
         ["event", "show", "1", *U],
         ["input", "show", "1", *U],
         ["permission", "inspect", "1", "--event-id", "1", "--from-member-id", "1", "--json", *U],
         ["member", "create", "--fleet-id", "4", "--name", "x", "--description", "d", "--coding-agent", "claude",
          PROMPT, *U],
         ["member", "create", "--fleet-id", "3", "--name", "x", "--description", "d", "--coding-agent", "claude",
          PROMPT, *U],
         ["member", "create", "--fleet-id", "99", "--name", "x", "--description", "d", PROMPT, *U],
         ["fleet", "delete", "4", *U],
         ["fleet", "delete", "4", "--json", *U],
         ["fleet", "delete", "99", *U],
         ["fleet", "list", *U])

# ---------------------------------------------------------------- version, help and parse errors

HELP_TARGETS = [[], ["setup"], ["doctor"], ["fleet"], ["fleet", "create"], ["fleet", "list"], ["fleet", "show"],
                ["fleet", "delete"], ["member"], ["member", "create"], ["member", "delete"], ["member", "show"],
                ["member", "list"], ["member", "prompt"], ["member", "exec"], ["member", "press"],
                ["member", "capture"], ["event"], ["event", "show"], ["input"], ["input", "show"],
                ["input", "reconcile"], ["permission"], ["permission", "inspect"], ["permission", "respond"],
                ["message"], ["message", "send"], ["message", "poll"], ["message", "show"]]
scenario("version-and-help", {},
         ["--version"], ["-V"],
         *[{"argv": [*target, "--help"], "compare": "help"} for target in HELP_TARGETS],
         *[{"argv": [*target, "-h"], "compare": "help"} for target in HELP_TARGETS[:4]],
         {"argv": ["help", "message"], "compare": "help"},
         {"argv": ["message", "help", "poll"], "compare": "help"},
         {"argv": ["fleet", "create", "--help", *U], "compare": "help"},
         {"argv": [], "compare": "parse"},
         help_quotes=[PERMISSION_MODE_HELP, PANE_HELP, SERVER_HELP, "Initialize the version's database and install assets",
                      "Diagnose multiplexer, database and agent assets", "Persist a point-to-point message",
                      "Read and consume incoming deliveries", "Read one message", "Register the calling Director atomically",
                      "Read fleet registry", "Register and spawn an independent agent", "Kill its owned pane and deregister",
                      "Submit authorized native input", "Request explicit ordered keys", "Read a pane tail",
                      "Read a durable runtime event and current state", "Inspect / answer the current native permission request",
                      "allow", "deny", "submitted", "not-submitted", "abandoned-uncertain"])

PARSE_CASES = [
    ["message", "send"],
    ["message", "send", "--from-member-id", "1", "--to-member-id", "2"],
    ["message", "send", "--from-member-id", "1", "--to-member-id", "2", "a", "--file", "x"],
    ["message", "send", "--from-member-id", "x", "--to-member-id", "2", "a"],
    ["message", "send", "--from-member-id", "-1", "--to-member-id", "2", "a"],
    ["message", "send", "--from-member-id", "1", "--to-member-id", "2", "a", "b"],
    ["message", "send", "--from-member-id", "1", "--from-member-id", "1", "--to-member-id", "2", "a"],
    ["message", "send", "--from-member-id", "1", "--to-member-id", "2", "a", "--json=1"],
    ["message", "send", "--from-member-id", "1", "--to-member-id", "2", "a", "--bogus"],
    ["message", "send", "--from-member-id", "1", "--to-member-id", "2", "a", "-x"],
    ["message", "send", "--from-member-id"],
    ["message", "send", "--json", "--from-member-id", "1"],
    ["--database-url", "x", "message", "send"],
    ["message", "send", "--file", "f", "--bogus"],
    ["message", "send", "--file"],
    ["message", "send", "--to-member-id", "2", "--from-member-id", "1", "--json", "x", "--file", "y"],
    ["message", "send", "--file", "y", "x"],
    ["message", "send", "--from-member-id", "--json"],
    ["message", "send", "--from-member-id", "-"],
    ["message", "send", "--from-member-id", "1", "--to-member-id", "2", "--file", "a", "--file", "b"],
    ["message", "send", "--from-member-id=", "--to-member-id", "2", "t"],
    ["message", "send", "-from-member-id", "1"],
    ["message", "send", "--from-member-id", "1", "--to-member-id", "99999999999999999999", "t"],
    ["message", "send", "--from-member-id", "1", "--to-member-id", "--", "t"],
    ["message", "poll"],
    ["message", "poll", "abc"],
    ["message", "poll", "99999999999999999999"],
    ["message", "poll", "--", "-99999999999999999999"],
    ["message", "poll", "1", "2"],
    ["message", "poll", "1", "--database-url"],
    ["message", "poll", "--json"],
    ["message", "poll", "1", "--json", "--json"],
    ["message", "poll", " 5"],
    ["message", "poll", ""],
    ["message", "poll", "-"],
    ["message", "poll", "+"],
    ["message", "poll", "1_000"],
    ["message", "poll", "٣"],
    ["message", "poll", "1", "--database-url", "--json"],
    ["message", "poll", "1", "--database-url", "-x"],
    ["--database-url", "--json", "message", "poll", "1"],
    ["message", "poll", "1", "--json", "extra"],
    ["message", "poll", "--jso"],
    ["message", "poll", "--from-member-id", "1"],
    ["message", "poll", "1", "-V"],
    ["message", "--database-url", "x", "poll"],
    ["message", "show"],
    ["message", "show", "1", "--json=x"],
    ["message", "ack", "1"],
    ["message", "Send"],
    ["message", "sen"],
    ["messag", "send"],
    ["message"],
    ["fleet", "list", "extra"],
    ["fleet", "list", "--json", "--json"],
    ["fleet", "list", "--bogus"],
    ["fleet", "list", "--json=1"],
    ["fleet", "list", "--", "x"],
    ["fleet"],
    ["--bogus"],
    ["--json"],
    ["setup", "--json"],
    ["setup", "--coding-agent"],
    ["setup", "--coding-agent", "bogus"],
    ["setup", "--coding-agent", "Claude"],
    ["setup", "extra"],
    ["doctor", "extra"],
    ["fleet", "create"],
    ["fleet", "create", "--name", "a"],
    ["fleet", "create", "--name", "a", "--coding-agent", "bogus"],
    ["fleet", "create", "--coding-agent", "claude"],
    ["fleet", "create", "--name", "a", "--coding-agent", "claude", "--mux-server"],
    ["fleet", "show"],
    ["fleet", "show", "0"],
    ["fleet", "show", "x"],
    ["fleet", "delete"],
    ["fleet", "delete", "-3"],
    ["member"],
    ["member", "create"],
    ["member", "create", "--fleet-id", "1", "--name", "a", "--description", "d"],
    ["member", "create", "--fleet-id", "1", "--name", "a", "--description", "d", "p", "--file", "f"],
    ["member", "create", "--fleet-id", "x", "--name", "a", "--description", "d", "p"],
    ["member", "create", "--fleet-id", "1", "--name", "a", "--description", "d", "--coding-agent", "bogus", "p"],
    ["member", "create", "--fleet-id", "1", "--name", "a", "p"],
    ["member", "create", "--fleet-id", "1", "--description", "d", "p"],
    ["member", "delete"],
    ["member", "delete", "x"],
    ["member", "show"],
    ["member", "list"],
    ["member", "list", "0"],
    ["member", "prompt", "2", "hi"],
    ["member", "prompt", "2", "--from-member-id", "0", "hi"],
    ["member", "prompt", "2", "--from-member-id", "x", "hi"],
    ["member", "prompt", "--from-member-id", "1", "hi"],
    ["member", "prompt", "2", "--from-member-id", "1"],
    ["member", "prompt", "2", "--from-member-id", "1", "a", "b"],
    ["member", "exec", "2", "--from-member-id", "1"],
    ["member", "exec", "2", "--from-member-id", "1", "a", "--file", "f"],
    ["member", "exec", "2", "true"],
    ["member", "press", "2", "--from-member-id", "1"],
    ["member", "press", "0", "--from-member-id", "1", "Enter"],
    ["member", "press", "2", "Enter"],
    ["member", "press", "2", "--from-member-id", "1", "--json"],
    ["member", "capture"],
    ["member", "capture", "2", "--lines"],
    ["member", "capture", "2", "--lines", "x"],
    ["member", "capture", "2", "--lines", "-1"],
    ["member", "capture", "2", "--ansi=1"],
    ["event"],
    ["event", "show"],
    ["event", "show", "0"],
    ["event", "show", "1", "2"],
    ["input", "show"],
    ["input", "show", "-1"],
    ["input", "reconcile", "1"],
    ["input", "reconcile", "1", "--from-member-id", "1", "--disposition", "bogus", "--evidence-ref", "x"],
    ["input", "reconcile", "1", "--from-member-id", "1", "--disposition", "not_submitted", "--evidence-ref", "x"],
    ["input", "reconcile", "1", "--from-member-id", "1", "--evidence-ref", "x"],
    ["input", "reconcile", "1", "--from-member-id", "1", "--disposition", "submitted"],
    ["permission"],
    ["permission", "inspect", "1"],
    ["permission", "inspect", "1", "--event-id", "1"],
    ["permission", "inspect", "1", "--from-member-id", "1"],
    ["permission", "inspect", "0", "--event-id", "1", "--from-member-id", "1"],
    ["permission", "respond", "1", "--event-id", "1", "--from-member-id", "1"],
    ["permission", "respond", "1", "--event-id", "1", "--from-member-id", "1", "--expected-run-id", "1",
     "--expected-owner-epoch", "1", "--request-token", "0" * 64],
    ["permission", "respond", "1", "--event-id", "1", "--from-member-id", "1", "--expected-run-id", "1",
     "--expected-owner-epoch", "1", "--request-token", "0" * 64, "--decision", "allow", "--choice", "x"],
    ["permission", "respond", "1", "--event-id", "1", "--from-member-id", "1", "--expected-run-id", "1",
     "--expected-owner-epoch", "1", "--request-token", "0" * 64, "--decision", "maybe"],
    ["permission", "respond", "1", "--event-id", "1", "--from-member-id", "1", "--expected-run-id", "1",
     "--expected-owner-epoch", "1", "--request-token", "0" * 64, "--choice", "Not-Valid"],
    ["permission", "respond", "1", "--event-id", "1", "--from-member-id", "1", "--expected-run-id", "1",
     "--expected-owner-epoch", "1", "--request-token", "0" * 64, "--choice", "a" * 65],
    ["permission", "respond", "1", "--event-id", "1", "--from-member-id", "1", "--expected-run-id", "1",
     "--expected-owner-epoch", "0", "--request-token", "0" * 64, "--decision", "allow"],
    ["__member-run"],
    ["__member-run", "--descriptor"],
    ["message", "send", "--from-member-id", "1", "--to-member-id", "2", b"\xff\xfe".decode("utf-8", "surrogateescape")],
    ["message", "poll", b"\xff".decode("utf-8", "surrogateescape")],
    ["fleet", "list", "--database-url", b"sqlite:///\xff".decode("utf-8", "surrogateescape")],
]
scenario("parse-errors", {}, *SEEDED,
         *[{"argv": [*case, "--database-url", "{URL}"] if i % 2 else case, "compare": "parse"}
           for i, case in enumerate(PARSE_CASES)],
         ["fleet", "list", *U])
# The full parse-error bytes, beyond the first line G1 compares: an informational report outside
# the gate's pass condition.
scenario("parse-errors-exact", {}, *SEEDED,
         *[[*case, "--database-url", "{URL}"] if i % 2 else case for i, case in enumerate(PARSE_CASES)],
         informational=True)

# ---------------------------------------------------------------- setup

scenario("setup-variants", {},
         setup("--coding-agent", "pi"),
         setup("--coding-agent", "claude", "--coding-agent", "claude", "--coding-agent", "codex"),
         setup("--coding-agent", "opencode", "hermes"),
         setup(),
         setup("--coding-agent", "codex"),
         {**setup("--coding-agent", "claude"), "env": {"CLAUDE_CONFIG_DIR": "{W}/claude-home"}},
         {**setup("--coding-agent", "codex"), "env": {"CODEX_HOME": "relative/codex"}},
         {**setup(), "env": {"CODEX_HOME": ""}},
         {**setup("--coding-agent", "claude"), "env": {"CODEX_HOME": ""}},
         {**setup("--coding-agent", "hermes"), "env": {"HERMES_HOME": "~/hermes"}},
         {**setup("--coding-agent", "opencode"), "env": {"OPENCODE_CONFIG_DIR": "{W}/oc"}},
         {**setup("--coding-agent", "pi"), "env": {"PI_CODING_AGENT_DIR": "{W}/pi"}},
         {**setup("--coding-agent", "pi"), "env": {"PI_CODING_AGENT_DIR": "{W}/pi-" + b"\xff".decode("utf-8", "surrogateescape")}},
         files=["{HOME}", "{W}/claude-home", "{W}/oc", "{W}/pi"])
scenario("setup-install-targets", {}, setup(),
         write_file("{HOME}/.claude/skills/unrelated/SKILL.md", b"keep me"),
         write_file("{HOME}/.claude/skills/cafleet/stale-extra.md", b"stale"),
         remove("{HOME}/.claude/skills/cafleet/SKILL.md"),
         do(lambda w: os.symlink(f"{w.root}/body-target", f"{w.home}/.pi/agent/extensions/cafleet.ts")
            if not os.unlink(f"{w.home}/.pi/agent/extensions/cafleet.ts") else None, "symlink the pi extension"),
         write_file("{W}/body-target", b"referent"),
         setup(),
         remove("{HOME}/.codex/rules"),
         write_file("{HOME}/.codex/rules", b"a file where a directory belongs"),
         setup("--coding-agent", "codex"),
         setup("--coding-agent", "claude", "--coding-agent", "codex", "--coding-agent", "pi"),
         remove("{HOME}/.config/opencode/plugins"),
         write_file("{HOME}/.config/opencode/plugins", b"file"),
         setup("--coding-agent", "opencode"),
         remove("{HOME}/.pi/agent/extensions"),
         write_file("{HOME}/.pi/agent/extensions", b"file"),
         setup("--coding-agent", "pi"),
         remove("{HOME}/.hermes"),
         write_file("{HOME}/.hermes", b"file"),
         setup("--coding-agent", "hermes"),
         files=["{HOME}", "{W}/body-target"])
scenario("setup-concurrent-format", {}, write_file("{DB}", b""), setup(), setup(),
         sql("PRAGMA application_id=1128351289"), setup())

# ---------------------------------------------------------------- doctor

scenario("doctor-environments", {"layout": "tmux"}, setup(),
         ["doctor", *U],
         ["doctor", "--json", *U],
         {"argv": ["doctor", *U], "env": {"TMUX": "{W}/tmux.sock,1,0", "TMUX_PANE": "%0"}},
         {"argv": ["doctor", "--json", *U], "env": {"TMUX": "{W}/tmux.sock,1,0", "TMUX_PANE": "%0"}},
         {"argv": ["doctor", *U], "env": {"TMUX": "{W}/tmux.sock,1,0"}},
         {"argv": ["doctor", *U], "env": {"TMUX": "{W}/tmux.sock,1,0", "TMUX_PANE": "%9"}},
         {"argv": ["doctor", *U], "env": {"TMUX": "{W}/missing.sock,1,0", "TMUX_PANE": "%0"}},
         {"argv": ["doctor", *U], "env": {"TMUX": "{W}/tmux.sock,1,0", "HERDR_ENV": "1"}},
         {"argv": ["doctor", *U], "env": {"HERDR_ENV": "1", "HERDR_PANE_ID": "w1:p1"}},
         {"argv": ["doctor", *U], "env": {"TMUX": "{W}/tmux.sock,1,0", "TMUX_PANE": "%0"},
          "via": ["codex", "app-server"]},
         {"argv": ["doctor", *U], "env": {"TMUX": "{W}/tmux.sock,1,0", "TMUX_PANE": "%0"},
          "via": ["/opt/hermes", "--verbose", "gateway", "run"]},
         {"argv": ["doctor", *U], "env": {"PATH": "/usr/bin:/bin"}},
         {"argv": ["doctor", *U], "env": {"CODEX_HOME": "rel", "HERMES_HOME": ""}},
         remove("{HOME}/.pi/agent/extensions/cafleet.ts"),
         remove("{HOME}/.config/opencode/agents/cafleet-ask.md"),
         write_file("{HOME}/.claude/skills/cafleet/SKILL.md", b"changed"),
         write_file("{HOME}/.config/opencode/plugins/cafleet.executable", b"/nonexistent/cafleet\n"),
         ["doctor", *U],
         ["doctor", "--json", *U],
         {"argv": ["doctor", *U], "stdout": "broken-pipe"})
scenario("doctor-herdr", {"layout": "herdr"}, setup(),
         {"argv": ["doctor", *U], "env": {"HERDR_ENV": "1", "HERDR_PANE_ID": "w1:p1"}},
         {"argv": ["doctor", *U], "env": {"HERDR_ENV": "1", "HERDR_PANE_ID": "w1:p1", "HERDR_SESSION": "default"}},
         {"argv": ["doctor", *U], "env": {"HERDR_ENV": "1", "HERDR_PANE_ID": "w1:p1", "HERDR_SOCKET_PATH": "{W}/herdr.sock"}},
         {"argv": ["doctor", *U], "env": {"HERDR_ENV": "1", "HERDR_PANE_ID": "w1:p1", "HERDR_SESSION": "nosuch"}},
         {"argv": ["doctor", *U], "env": {"HERDR_ENV": "1"}},
         {"argv": ["doctor", "--json", *U], "env": {"HERDR_ENV": "1", "HERDR_PANE_ID": "w1:p9"}})
scenario("doctor-no-default-tmux", {"layout": "tmux", "default_tmux": False}, setup(),
         {"argv": ["doctor", *U], "env": {"TMUX_PANE": "%0", "TMUX": "x"}},
         {"argv": ["doctor", *U], "env": {"TMUX": ",1,0", "TMUX_PANE": "%0"}})

# ---------------------------------------------------------------- fleet create / list / show / delete

scenario("fleet-create-tmux", {"layout": "tmux"}, setup(),
         fleet_tmux("--json"),
         ["fleet", "create", "--name", "beta", "--coding-agent", "codex", "--pane", "%0", "--mux-server", TMUX, *U],
         ["fleet", "create", "--name", "gamma", "--coding-agent", "claude", "--pane", "%9", "--mux-server", TMUX, *U],
         ["fleet", "create", "--name", "gamma", "--coding-agent", "claude", "--pane", "%0", "--mux-server",
          "{W}/nosuch.sock", *U],
         ["fleet", "create", "--name", "gamma", "--coding-agent", "claude", "--pane", "%x", *U],
         ["fleet", "create", "--name", "gamma", "--coding-agent", "claude", "--pane", "a:b:c", *U],
         ["fleet", "create", "--name", "gamma", "--coding-agent", "claude", "--pane", "%0", *U],
         {"argv": ["fleet", "create", "--name", "env", "--coding-agent", "claude", *U],
          "env": {"TMUX": "{W}/tmux.sock,1,0", "TMUX_PANE": "%0"}},
         {"argv": ["fleet", "create", "--name", "env", "--coding-agent", "claude", "--json", *U],
          "env": {"TMUX": "{W}/tmux.sock,1,0", "TMUX_PANE": "%0", "HERDR_ENV": "1"}},
         {"argv": ["fleet", "create", "--name", "env", "--coding-agent", "claude", *U],
          "env": {"TMUX": "{W}/tmux.sock,1,0"}},
         {"argv": ["fleet", "create", "--name", "env", "--coding-agent", "claude", *U]},
         {"argv": ["fleet", "create", "--name", "env", "--coding-agent", "claude", *U],
          "env": {"TMUX": "{W}/tmux.sock,1,0", "TMUX_PANE": "%9"}},
         {"argv": ["fleet", "create", "--name", "shared", "--coding-agent", "codex", *U],
          "env": {"TMUX": "{W}/tmux.sock,1,0", "TMUX_PANE": "%0"}, "via": ["codex", "app-server"]},
         {"argv": ["fleet", "create", "--name", "both", "--coding-agent", "claude", "--pane", "%0", *U],
          "env": {"TMUX": "{W}/tmux.sock,1,0", "HERDR_ENV": "1"}},
         ["fleet", "list", *U],
         ["fleet", "list", "--json", *U],
         ["fleet", "show", "1", *U],
         ["fleet", "show", "2", "--json", *U],
         ["member", "list", "1", *U],
         ["member", "show", "1", "--json", *U],
         screen_of("%0"))
scenario("fleet-create-tmux-faults", {"layout": "tmux", "faults": [
    {"tool": "tmux", "match": "rename-window -t @0 cafleet-1-0", "times": 1,
     "reply": {"code": 1, "stderr": "can't rename window\n"}},
    {"tool": "tmux", "match": r"#\{window_id\}\|#\{window_name\}", "times": 1, "reply": {"stdout": "@0|wrong\n"}},
    {"tool": "tmux", "match": r"#\{window_id\}\|#\{window_name\}", "times": 1, "reply": {"stdout": "garbage\n"}},
    {"tool": "tmux", "match": r"display-message -p -t %0 #\{session_name\}", "times": 1, "reply": {"stdout": "||\n"}},
]}, setup(), fleet_tmux(), fleet_tmux("--json"), fleet_tmux(), fleet_tmux(), fleet_tmux(), ["fleet", "list", *U])
scenario("fleet-create-default-tmux", {"layout": "tmux"}, setup(),
         ["fleet", "create", "--name", "d", "--coding-agent", "claude", "--pane", "%0", *U],
         {"argv": ["fleet", "create", "--name", "e", "--coding-agent", "claude", "--pane", "%0", *U],
          "env": {"TMUX": "{W}/tmux.sock,1,0"}})
scenario("fleet-create-no-default-tmux", {"layout": "tmux", "default_tmux": False}, setup(),
         ["fleet", "create", "--name", "d", "--coding-agent", "claude", "--pane", "%0", *U],
         {"argv": ["fleet", "create", "--name", "d", "--coding-agent", "claude", *U], "env": {"TMUX_PANE": "%0", "TMUX": ""}})
scenario("fleet-create-herdr", {"layout": "herdr"}, setup(),
         fleet_herdr("--json"),
         ["fleet", "create", "--name", "b", "--coding-agent", "pi", "--pane", "w1:p1", *U],
         ["fleet", "create", "--name", "c", "--coding-agent", "pi", "--pane", "w1:p9", *U],
         ["fleet", "create", "--name", "c", "--coding-agent", "pi", "--pane", "w1:p1", "--mux-server", "nosuch", *U],
         {"argv": ["fleet", "create", "--name", "env", "--coding-agent", "claude", *U],
          "env": {"HERDR_ENV": "1", "HERDR_PANE_ID": "w1:p1"}},
         {"argv": ["fleet", "create", "--name", "env", "--coding-agent", "claude", *U],
          "env": {"HERDR_ENV": "1", "HERDR_PANE_ID": "w1:p1", "HERDR_SOCKET_PATH": "{W}/herdr.sock"}},
         {"argv": ["fleet", "create", "--name", "env", "--coding-agent", "claude", *U],
          "env": {"HERDR_ENV": "1", "HERDR_PANE_ID": "w1:p1", "HERDR_SESSION": "default"}},
         {"argv": ["fleet", "create", "--name", "env", "--coding-agent", "claude", *U], "env": {"HERDR_ENV": "1"}},
         {"argv": ["fleet", "create", "--name", "env", "--coding-agent", "claude", *U],
          "env": {"HERDR_ENV": "1", "HERDR_PANE_ID": "w1:p1"}, "via": ["hermes_cli.main", "gateway", "run"]},
         ["fleet", "list", "--json", *U],
         ["member", "list", "1", "--json", *U])
scenario("fleet-create-not-ancestor", {"layout": "tmux", "director_is_ancestor": False}, setup(),
         {"argv": ["fleet", "create", "--name", "env", "--coding-agent", "claude", *U],
          "env": {"TMUX": "{W}/tmux.sock,1,0", "TMUX_PANE": "%0"}},
         {"argv": ["doctor", *U], "env": {"TMUX": "{W}/tmux.sock,1,0", "TMUX_PANE": "%0"}})

# ---------------------------------------------------------------- member lifecycle (tmux)

MEMBER_FILES = ["{W}/.cafleet-runs"]
scenario("member-lifecycle-tmux", {"layout": "tmux"}, setup(), fleet_tmux(),
         member("m1", "claude"),
         ["member", "list", "1", *U],
         ["member", "list", "1", "--json", *U],
         ["member", "show", "2", *U],
         ["member", "show", "2", "--json", *U],
         {**member("m2", "claude", "--json")},
         member("m3", "claude"),
         member("m4", "claude"),
         ["member", "list", "1", *U],
         ["fleet", "show", "1", "--json", *U],
         ["doctor", *U],
         ["doctor", "--json", *U],
         {"argv": ["member", "delete", "3", *U], "quiet": True},
         ["member", "list", "1", "--json", *U],
         {"argv": ["member", "delete", "3", *U], "quiet": True},
         {"argv": ["member", "delete", "1", *U]},
         {"argv": ["member", "delete", "5", "--json", *U], "quiet": True},
         member("m5", "claude"),
         ["member", "list", "1", *U],
         {"argv": ["fleet", "delete", "1", *U], "quiet": True},
         {"argv": ["fleet", "delete", "1", "--json", *U], "quiet": True},
         ["fleet", "list", "--json", *U],
         ["fleet", "show", "1", *U],
         ["member", "list", "1", *U],
         ["message", "poll", "1", "--json", *U],
         ["event", "show", "1", *U],
         ["event", "show", "1", "--json", *U],
         screen_of("%0"))
scenario("member-create-backends-tmux", {"layout": "tmux"}, setup(), fleet_tmux(),
         member("cl", "claude", "--model", "opus", "--effort", "high"),
         member("cx", "codex", "--model", "gpt-6-luna", "--effort", "minimal", "--permission-mode", "never"),
         member("oc", "opencode", "--model", "opencode-go/gpt-6-luna", "--permission-mode", "cafleet"),
         member("he", "hermes", "--model", "openai/gpt-6-luna", "--effort", "ultra"),
         member("pi", "pi", "--model", "openai/gpt-6-luna:free", "--effort", "off", "--permission-mode", "cafleet-ask"),
         member("dflt", "claude", "--permission-mode", "plan"),
         ["member", "list", "1", *U],
         ["member", "list", "1", "--json", *U],
         *[{"argv": ["member", "show", str(k), "--json", *U]} for k in range(2, 8)],
         ["doctor", *U],
         {"argv": ["fleet", "delete", "1", *U], "quiet": True, "quiet_timeout": 60},
         files=MEMBER_FILES, settle=True)
scenario("member-create-inherit-backend", {"layout": "tmux"}, setup(), do(files_fixture),
         ["fleet", "create", "--name", "alpha", "--coding-agent", "codex", "--pane", "%0", "--mux-server", TMUX, *U],
         {"argv": ["member", "create", "--fleet-id", "1", "--name", "i", "--description", "d", PROMPT, *U], "quiet": True},
         {"argv": ["member", "create", "--fleet-id", "1", "--name", "j", "--description", "d", "--file", "{W}/prompt.txt", *U],
          "quiet": True},
         {"argv": ["member", "create", "--fleet-id", "1", "--name", "k", "--description", "d", "--file", "-", "--json", *U],
          "stdin": PROMPT.encode(), "quiet": True},
         ["member", "list", "1", *U],
         {"argv": ["fleet", "delete", "1", *U], "quiet": True})
scenario("member-create-errors", {"layout": "tmux"}, setup(), fleet_tmux(), do(files_fixture),
         member("e1", "claude", "--effort", "ultra", quiet=False),
         member("e2", "codex", "--effort", "max", quiet=False),
         member("e3", "opencode", "--effort", "low", quiet=False),
         member("e4", "hermes", "--effort", "extreme", quiet=False),
         member("e5", "pi", "--effort", "ultra", quiet=False),
         member("e6", "opencode", "--model", "abc", quiet=False),
         member("e7", "opencode", "--model", "a/", quiet=False),
         member("e8", "opencode", "--model", "/b", quiet=False),
         member("e9", "pi", "--model", "anthropic/fable:high", quiet=False),
         member("e10", "claude", "--permission-mode", "never", quiet=False),
         member("e11", "codex", "--permission-mode", "auto", quiet=False),
         member("e12", "hermes", "--permission-mode", "cafleet", quiet=False),
         member("e13", "pi", prompt="@file.md please", quiet=False),
         member("e14", "claude", prompt="{bogus}", quiet=False),
         member("e15", "claude", prompt="{fleet_id!r}", quiet=False),
         member("e16", "claude", prompt="{0}", quiet=False),
         member("e17", "claude", prompt="unbalanced { brace", quiet=False),
         member("e18", "claude", prompt="unbalanced } brace", quiet=False),
         member("e19", "claude", prompt="{{literal}} {fleet_id}", quiet=False),
         member("e20", "claude", prompt="no database placeholder", quiet=False),
         member("e21", "claude", prompt="   ", quiet=False),
         {"argv": ["member", "create", "--fleet-id", "1", "--name", "f", "--description", "d", "--file", "{W}/empty.txt", *U]},
         {"argv": ["member", "create", "--fleet-id", "1", "--name", "f", "--description", "d", "--file", "{W}/latin1.txt", *U]},
         {"argv": ["member", "create", "--fleet-id", "1", "--name", "f", "--description", "d", "--file", "{W}/missing", *U]},
         {"argv": ["member", "create", "--fleet-id", "1", "--name", "f", "--description", "d", "--file", "-", *U], "stdin": b" "},
         {"argv": ["member", "create", "--fleet-id", "1", "--name", "f", "--description", "d", "--file", "-", *U],
          "stdin": b"\xff"},
         member("p1", "claude", env={"PATH": "{W}/empty-path:/usr/bin:/bin"}, quiet=False),
         member("p2", "claude", env={"CLAUDE_CONFIG_DIR": "relative"}, quiet=False),
         member("p3", "opencode", env={"OPENCODE_CONFIG_DIR": "{W}/no-presets"}, quiet=False),
         member("p4", "pi", env={"PI_CODING_AGENT_DIR": "{W}/no-pi"}, quiet=False),
         member("p5", "hermes", env={"HERMES_HOME": ""}, quiet=False),
         remove("{HOME}/.config/opencode/agents/cafleet.md"),
         member("p6", "opencode", "--permission-mode", "cafleet", quiet=False),
         ["member", "list", "1", "--json", *U],
         ["fleet", "list", *U])
scenario("member-create-native-faults", {"layout": "tmux", "natives": {
    "claude": {"version_line": "tool 2.1"}, "codex": {"version_line": "v2.1.4 2.1.5"},
    "pi": {"version_line": "1.5.2", "version_exit": 3}, "opencode": {"version_line": "opencode v2.3.0-beta.1+build.7"},
    "hermes": {"version_line": "Hermes Agent vunknown (2026.09.24)"}}}, setup(), fleet_tmux(),
         member("v1", "claude"), member("v2", "codex"), member("v3", "pi"), member("v4", "opencode"), member("v5", "hermes"),
         ["member", "list", "1", "--json", *U],
         *[{"argv": ["member", "show", str(k), "--json", *U]} for k in range(2, 7)],
         {"argv": ["fleet", "delete", "1", *U], "quiet": True, "quiet_timeout": 60}, settle=True)
scenario("member-create-hermes-config-faults", {"layout": "tmux", "natives": {
    "hermes": {"config_path_exit": 2, "config_path_line": "boom"}}}, setup(), fleet_tmux(),
         member("h1", "hermes"), ["member", "list", "1", *U])
scenario("member-create-mux-faults", {"layout": "tmux", "faults": [
    {"tool": "tmux", "match": "split-window", "times": 1, "reply": {"code": 1, "stderr": "create pane failed: no space\n"}},
    {"tool": "tmux", "match": "split-window", "times": 1, "reply": {"stdout": "\n"}},
    {"tool": "tmux", "match": r"#\{window_height\}", "times": 1, "reply": {"stdout": "tall\n"}},
]}, setup(), fleet_tmux(),
         member("f1", "claude"), member("f2", "claude"), member("f3", "claude"), member("ok", "claude"),
         ["member", "list", "1", "--json", *U],
         {"argv": ["fleet", "delete", "1", *U], "quiet": True})
scenario("member-create-overflow-windows", {"layout": "tmux"}, setup(), fleet_tmux(),
         *[member(f"o{k}", "claude") for k in range(1, 6)],
         ["member", "list", "1", *U],
         {"argv": ["member", "delete", "3", *U], "quiet": True},
         member("o6", "claude"),
         ["member", "list", "1", "--json", *U],
         screen_of("%0"),
         {"argv": ["fleet", "delete", "1", "--json", *U], "quiet": True, "quiet_timeout": 60}, settle=True)
scenario("member-create-overflow-faults", {"layout": "tmux", "faults": [
    {"tool": "tmux", "match": "new-window", "times": 1, "reply": {"stdout": "garbage\n"}},
]}, setup(), fleet_tmux(), *[member(f"o{k}", "claude") for k in range(1, 5)],
         ["member", "list", "1", *U],
         {"argv": ["fleet", "delete", "1", *U], "quiet": True, "quiet_timeout": 60}, settle=True)

# ---------------------------------------------------------------- member lifecycle (herdr)

scenario("member-lifecycle-herdr", {"layout": "herdr"}, setup(), fleet_herdr(),
         member("h1", "claude"), member("h2", "codex"), member("h3", "pi"), member("h4", "opencode"),
         ["member", "list", "1", *U],
         ["member", "list", "1", "--json", *U],
         ["member", "show", "3", "--json", *U],
         {"argv": ["member", "capture", "2", *U]},
         {"argv": ["member", "capture", "2", "--lines", "3", "--json", *U]},
         {"argv": ["member", "delete", "2", *U], "quiet": True},
         member("h5", "claude"),
         ["member", "list", "1", *U],
         {"argv": ["fleet", "delete", "1", *U], "quiet": True, "quiet_timeout": 60}, settle=True)
scenario("member-create-herdr-faults", {"layout": "herdr", "faults": [
    {"tool": "herdr", "match": "pane split", "times": 1, "reply": {"stdout": "{\"result\":{\"pane\":{}}}\n"}},
    {"tool": "herdr", "match": "pane run", "times": 1, "reply": {"code": 1, "stderr": "{\"error\":{\"code\":\"run_failed\"}}\n"}},
    {"tool": "herdr", "match": "pane split", "times": 1, "reply": {"stdout": "not json\n"}},
]}, setup(), fleet_herdr(), member("f1", "claude"), member("f2", "claude"), member("f3", "claude"), member("ok", "claude"),
         ["member", "list", "1", "--json", *U],
         {"argv": ["fleet", "delete", "1", *U], "quiet": True})

# ---------------------------------------------------------------- pane input: prompt / exec / press / capture

PANE = "%1"
scenario("member-input-tmux", {"layout": "tmux"}, setup(), fleet_tmux(), member("m1", "claude"), do(files_fixture),
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "first prompt", *U], "quiet": True},
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "second prompt", "--json", *U], "quiet": True},
         {"argv": ["member", "exec", "2", "--from-member-id", "1", "printf hello", *U], "quiet": True},
         {"argv": ["member", "exec", "2", "--from-member-id", "1", "--file", "{W}/cmd.txt", "--json", *U], "quiet": True},
         {"argv": ["member", "exec", "2", "--from-member-id", "1", "--file", "-", *U], "stdin": b"echo stdin\r\n",
          "quiet": True},
         {"argv": ["member", "prompt", "2", "--from-member-id", "2", "guarded", *U]},
         {"argv": ["member", "exec", "2", "--from-member-id", "99", "true", *U]},
         {"argv": ["member", "press", "2", "--from-member-id", "2", "Enter", *U]},
         {"argv": ["member", "prompt", "1", "--from-member-id", "1", "self", *U]},
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "two\nlines", *U]},
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "tab\there", *U]},
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "  ", *U]},
         {"argv": ["member", "exec", "2", "--from-member-id", "1", "a\x7fb", *U]},
         {"argv": ["member", "exec", "2", "--from-member-id", "1", "--file", "{W}/body.txt", *U]},
         {"argv": ["member", "exec", "2", "--from-member-id", "1", "--file", "{W}/empty.txt", *U]},
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "x" * 457, *U], "quiet": True},
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "界" * 456, *U], "quiet": True},
         {"argv": ["member", "press", "2", "--from-member-id", "1", "a", "B", "Enter", "--json", *U], "quiet": True},
         {"argv": ["member", "press", "2", "--from-member-id", "1", "--json", "--", "-", "+", ";", *U], "quiet": True},
         {"argv": ["member", "press", "2", "--from-member-id", "1", "ctrl+u", "SHIFT+TAB", "f12", "space", "backspace",
                   "escape", "tab", "up", "down", "left", "right", *U], "quiet": True},
         {"argv": ["member", "press", "2", "--from-member-id", "1", "Ctrl+1", *U]},
         {"argv": ["member", "press", "2", "--from-member-id", "1", "F13", *U]},
         {"argv": ["member", "press", "2", "--from-member-id", "1", "x" * 33, *U]},
         {"argv": ["member", "press", "2", "--from-member-id", "1", *(["a"] * 65), *U]},
         {"argv": ["member", "press", "2", "--from-member-id", "1", "é", *U]},
         {"argv": ["member", "press", "1", "--from-member-id", "1", "Enter", *U]},
         {"argv": ["member", "capture", "2", *U]},
         {"argv": ["member", "capture", "2", "--lines", "5", *U]},
         {"argv": ["member", "capture", "2", "--lines", "5", "--ansi", *U]},
         {"argv": ["member", "capture", "2", "--lines", "3", "--json", *U]},
         {"argv": ["member", "capture", "2", "--ansi", "--json", *U]},
         {"argv": ["member", "capture", "2", "--lines", "0", *U]},
         {"argv": ["member", "capture", "1", *U]},
         {"argv": ["member", "capture", "1", "--json", *U]},
         ["input", "show", "1", *U],
         ["input", "show", "1", "--json", *U],
         ["input", "show", "2", "--json", *U],
         ["input", "show", "99", *U],
         ["member", "show", "2", "--json", *U],
         ["event", "show", "1", "--json", *U],
         ["message", "poll", "1", "--json", *U],
         screen_of(PANE),
         {"argv": ["member", "delete", "2", "--json", *U], "quiet": True},
         ["event", "show", "1", *U],
         ["input", "show", "1", "--json", *U],
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "after delete", *U]},
         {"argv": ["input", "reconcile", "1", "--from-member-id", "1", "--disposition", "submitted", "--evidence-ref",
                   "pane capture", *U]})
scenario("member-input-busy", {"layout": "tmux", "natives": {"claude": {
    "start": "idle",
    "screens": {"idle": {"fixture": "claude-2.1.322-composer-prompt-plain-41-post-submit-tmux-120-1", "composer": True,
                         "continuation": "  "},
                "busy": {"fixture": "claude-2.1.322-queued-hint-tmux-120-1"}},
    "rules": [{"when": "submit", "match": "^go busy", "goto": "busy"}, {"when": "key", "match": "^z$", "state": "busy",
                                                                        "goto": "idle"}]}}},
         setup(), fleet_tmux(), member("m1", "claude"),
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "go busy", *U], "quiet": True},
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "queued one", *U], "quiet": True},
         {"argv": ["member", "exec", "2", "--from-member-id", "1", "queued exec", "--json", *U], "quiet": True},
         send(1, 2, "queued message", *U, quiet=True),
         ["member", "show", "2", "--json", *U],
         ["input", "show", "2", "--json", *U],
         ["input", "show", "3", *U],
         {"argv": ["member", "press", "2", "--from-member-id", "1", "z", *U], "quiet": True, "quiet_timeout": 60},
         ["input", "show", "2", "--json", *U],
         ["input", "show", "3", "--json", *U],
         ["message", "show", "1", "--json", *U],
         ["message", "poll", "1", "--json", *U],
         screen_of(PANE),
         {"argv": ["member", "delete", "2", *U], "quiet": True})
scenario("member-input-herdr", {"layout": "herdr"}, setup(), fleet_herdr(), member("m1", "claude"),
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "herdr prompt", *U], "quiet": True},
         {"argv": ["member", "exec", "2", "--from-member-id", "1", "printf hi", "--json", *U], "quiet": True},
         {"argv": ["member", "press", "2", "--from-member-id", "1", "-", "+", ";", "Ctrl+a", "F1", "Shift+Tab", *U],
          "quiet": True},
         {"argv": ["member", "capture", "2", "--lines", "4", *U]},
         {"argv": ["member", "capture", "2", "--ansi", "--json", *U]},
         ["input", "show", "1", "--json", *U],
         {"argv": ["member", "delete", "2", *U], "quiet": True})
scenario("member-input-pending-placement", {"layout": "tmux"}, setup(), fleet_tmux(),
         sql("INSERT INTO members (fleet_id, name, description, status, registered_at) VALUES "
             "(1, 'pending', 'd', 'active', '2026-11-24T00:00:00.000000+00:00')",
             "INSERT INTO member_placements (member_id, mux_session, mux_window_id, mux_window_name, window_ordinal, "
             "column_side, mux_pane_id, backend, mux_server, coding_agent, created_at) VALUES "
             "(2, 'main', NULL, NULL, 0, 'right', NULL, 'tmux', '/tmp/x', 'claude', '2026-11-24T00:00:00.000000+00:00')"),
         ["member", "capture", "2", *U],
         ["member", "prompt", "2", "--from-member-id", "1", "hi", *U],
         ["member", "exec", "2", "--from-member-id", "1", "true", *U],
         ["member", "press", "2", "--from-member-id", "1", "Enter", *U],
         ["member", "show", "2", "--json", *U],
         ["member", "list", "1", *U],
         ["doctor", *U],
         ["member", "delete", "2", "--json", *U],
         ["fleet", "delete", "1", *U])

# ---------------------------------------------------------------- the hidden runner command

scenario("member-run-direct", {"layout": "tmux"}, setup(), fleet_tmux(), do(files_fixture),
         ["__member-run", "--descriptor", "{W}/missing.json", *U],
         ["__member-run", "--descriptor", "relative.json", *U],
         ["__member-run", "--descriptor", "{W}/body.txt", *U],
         ["__member-run", "--descriptor", "{W}/adir", *U],
         write_file("{W}/descriptor.json", b'{"protocol_version":1}', 0o600),
         ["__member-run", "--descriptor", "{W}/descriptor.json", *U],
         ["--database-url", "sqlite:///{W}/other.db", "__member-run", "--descriptor", "{W}/descriptor.json"],
         member("m1", "claude"),
         do(lambda w: __import__("shutil").copy(next(
             os.path.join(d, f) for d, _, fs in os.walk(f"{w.root}/.cafleet-runs") for f in fs if f == "owner.lock"),
             f"{w.root}/lock-copy"), "copy a runtime file"),
         ["__member-run", "--descriptor", "{W}/lock-copy", *U],
         {"argv": ["member", "delete", "2", *U], "quiet": True})

# ---------------------------------------------------------------- input reconcile

scenario("input-reconcile", {"layout": "tmux", "natives": {"claude": {
    "start": "idle",
    "screens": {"idle": {"fixture": "claude-2.1.322-composer-prompt-plain-41-post-submit-tmux-120-1", "composer": True,
                         "continuation": "  "},
                "frozen": {"fixture": "claude-2.1.322-composer-prompt-plain-41-literal-tmux-120-1"}},
    "rules": [{"when": "submit", "match": "^freeze", "goto": "frozen"}]}}},
         setup(), fleet_tmux(), member("m1", "claude"),
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "freeze", *U], "quiet": True},
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "never confirmed", *U], "quiet": True},
         ["input", "show", "2", "--json", *U],
         ["input", "reconcile", "2", "--from-member-id", "2", "--disposition", "submitted", "--evidence-ref", "x", *U],
         ["input", "reconcile", "2", "--from-member-id", "1", "--disposition", "submitted", "--evidence-ref", "", *U],
         ["input", "reconcile", "2", "--from-member-id", "1", "--disposition", "not-submitted", "--evidence-ref",
          "capture shows the draft", "--json", *U],
         ["input", "reconcile", "2", "--from-member-id", "1", "--disposition", "not-submitted", "--evidence-ref",
          "again", *U],
         ["input", "reconcile", "2", "--from-member-id", "1", "--disposition", "submitted", "--evidence-ref", "conflict", *U],
         ["input", "reconcile", "1", "--from-member-id", "1", "--disposition", "abandoned-uncertain", "--evidence-ref",
          "x", *U],
         ["input", "reconcile", "99", "--from-member-id", "1", "--disposition", "submitted", "--evidence-ref", "x", *U],
         ["input", "show", "2", *U],
         ["member", "show", "2", "--json", *U],
         {"argv": ["member", "delete", "2", *U], "quiet": True},
         ["input", "reconcile", "2", "--from-member-id", "1", "--disposition", "submitted", "--evidence-ref", "x", *U])

# ---------------------------------------------------------------- permission inspect / respond

PERMISSION_NATIVE = {"claude": {
    "start": "idle",
    "screens": {"idle": {"fixture": "claude-2.1.322-composer-prompt-plain-41-post-submit-tmux-120-1", "composer": True,
                         "continuation": "  "},
                "perm": {"fixture": "claude-2.1.322-bash-allow-pre-stage-tmux-120-1"},
                "done": {"fixture": "claude-2.1.322-bash-allow-post-stage-tmux-120-1", "composer": True,
                         "continuation": "  "},
                "nudge": {"fixture": "claude-2.1.322-nudge-decline-pre-stage-tmux-120-1"}},
    "rules": [{"when": "submit", "match": "^run the probe", "goto": "perm"},
              {"when": "submit", "match": "^nudge", "goto": "nudge"},
              {"when": "key", "match": "^[1-4]$", "state": "perm", "goto": "done"},
              {"when": "key", "match": ".", "state": "nudge", "goto": "done"}]}}


def event_ids(world):
    return world.query("SELECT event_id, kind, interaction_id FROM member_runtime_events ORDER BY event_id")


def respond_argv(world, interaction, decision=None, choice=None, from_id="1", token=None, epoch=None, run=None):
    rows = world.query("SELECT event_id FROM member_runtime_events WHERE interaction_id=? AND kind='interaction_opened'",
                       (interaction,))
    event = rows[0][0] if rows else 1
    run_row = world.query("SELECT run_id, owner_epoch FROM member_runs ORDER BY run_id DESC LIMIT 1")[0]
    import json
    import subprocess
    if token is None:
        env = world.env()
        out = subprocess.run(world.confined(["cafleet", "permission", "inspect", str(interaction), "--event-id",
                                             str(event), "--from-member-id", "1", "--json", "--database-url", world.url], env),
                             executable=world.binary, env=env, capture_output=True, cwd=world.root)
        token = json.loads(out.stdout)["request"]["request_token"] if out.returncode == 0 else "0" * 64
    argv = ["permission", "respond", str(interaction), "--event-id", str(event), "--from-member-id", from_id,
            "--expected-run-id", str(run or run_row[0]), "--expected-owner-epoch", str(epoch or run_row[1]),
            "--request-token", token or "0" * 64]
    argv += ["--decision", decision] if decision else ["--choice", choice]
    return argv + ["--database-url", world.url]


def respond(interaction, **kwargs):
    def run(world):
        argv = respond_argv(world, interaction, **kwargs)
        result = world.run({"argv": argv})
        import harness
        harness.quiesce(world)
        return {"exit": result["exit"], "stdout": result["stdout"].decode(), "stderr": result["stderr"].decode()}
    run.__name__ = f"permission respond {interaction} {kwargs}"
    return do(run, record=True)


def inspect(interaction, json_flag=True, from_id="1", event=None):
    def run(world):
        rows = world.query("SELECT event_id FROM member_runtime_events WHERE interaction_id=? AND kind='interaction_opened'",
                           (interaction,))
        ev = event or (rows[0][0] if rows else 1)
        argv = ["permission", "inspect", str(interaction), "--event-id", str(ev), "--from-member-id", from_id,
                *(["--json"] if json_flag else []), "--database-url", "{URL}"]
        result = world.run({"argv": argv})
        return {"exit": result["exit"], "stdout": result["stdout"].decode(), "stderr": result["stderr"].decode()}
    run.__name__ = f"permission inspect {interaction}"
    return do(run, record=True)


scenario("permission-tmux", {"layout": "tmux", "natives": PERMISSION_NATIVE}, setup(), fleet_tmux(),
         member("m1", "claude"),
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "run the probe", *U], "quiet": True},
         do(event_ids, record=True),
         inspect(1), inspect(1, json_flag=False), inspect(1, from_id="2"), inspect(99), inspect(1, event=99),
         ["member", "show", "2", "--json", *U],
         ["event", "show", "1", "--json", *U],
         ["event", "show", "1", *U],
         respond(1, choice="allow-once"),
         respond(1, decision="allow", token="f" * 64),
         respond(1, decision="allow", epoch=99),
         respond(1, decision="allow", from_id="2"),
         respond(1, decision="allow"),
         respond(1, decision="deny"),
         do(event_ids, record=True),
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "run the probe again", *U], "quiet": True},
         do(event_ids, record=True),
         inspect(2),
         respond(2, decision="deny"),
         {"argv": ["member", "prompt", "2", "--from-member-id", "1", "nudge", *U], "quiet": True},
         do(event_ids, record=True),
         inspect(3),
         respond(3, decision="allow"),
         respond(3, choice="nonexistent-choice"),
         respond(3, choice="decline"),
         ["event", "show", "2", "--json", *U],
         ["message", "poll", "1", "--json", *U],
         screen_of(PANE),
         {"argv": ["member", "delete", "2", *U], "quiet": True},
         inspect(1),
         ["event", "show", "1", "--json", *U])
