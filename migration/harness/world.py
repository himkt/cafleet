"""Same-path worlds for the differential harness.

A world is a directory at one fixed absolute path, rebuilt from scratch for every binary and every
scenario run, so path-bearing output compares without rewriting. It holds a temporary HOME, the
database, a PATH directory with the stand-in `tmux`, `herdr` and native executables, the
stand-in multiplexer daemon and its logs. Nothing outside the world is read or written: the
default database, the user's multiplexer servers and installed assets stay untouched.
"""
import json
import os
import re
import shutil
import signal
import sqlite3
import subprocess
import sys
import time

HARNESS = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HARNESS))
sys.path.insert(0, HARNESS)
import fakemux  # noqa: E402

VERSION_LINES = {
    "claude": "2.1.325 (Claude Code)",
    "codex": "codex-cli 0.169.0",
    "opencode": "opencode v2.3.0",
    "hermes": "Hermes Agent vunknown (2026.9.24)",
    "pi": "1.5.2",
}
MEMBER_IDLE = {
    "claude": "claude-2.1.322-composer-prompt-plain-41-post-submit-tmux-120-1",
    "codex": "codex-0.167.0-composer-idle-empty-tmux-120-1",
    "opencode": "opencode-2.2.7-composer-prompt-plain-41-post-submit-tmux-120-1",
    "hermes": "hermes-2026.9.24-composer-empty-tmux-120-1",
    "pi": "pi-1.5.0-idle-composer-tmux-120-1",
}
DIRECTOR_IDLE = {
    "claude": "claude-2.1.325-director-idle-empty-tmux-120-1",
    "codex": "codex-0.169.0-director-idle-empty-tmux-120-1",
    "opencode": "opencode-2.3.0-director-idle-empty-tmux-120-1",
    "hermes": "hermes-2026.9.24-director-idle-empty-tmux-120-1",
    "pi": "pi-1.5.2-director-idle-empty-tmux-120-1",
}
BACKENDS = tuple(VERSION_LINES)


CONTINUATION = {"claude": "  ", "codex": "  ", "opencode": "┃ ", "hermes": "", "pi": ""}
DIRECTOR_COMPOSER = {"hermes": {"prefix": " ❯ ", "continuation": "   "}}


HERMES_SESSION_RULES = [
    {"when": "submit", "match": "^/stop$", "say": "No running background processes."},
    {"when": "submit", "match": "^/exit$", "exit": 0, "print": "Session: local-0123456789abcdef0123456789abcdef"},
]


def default_native(backend, fixtures, director=False):
    composer = {"composer": True, "continuation": CONTINUATION[backend]}
    if director:
        composer.update(DIRECTOR_COMPOSER.get(backend, {}))
    elif backend == "hermes":
        composer["empty_line"] = "❯ \x1b[7m \x1b[0m"
    return {"version_line": VERSION_LINES[backend], "start": "idle",
            "screens": {"idle": {"fixture": fixtures[backend], **composer}},
            "rules": list(HERMES_SESSION_RULES) if backend == "hermes" else []}


class World:
    def __init__(self, root, binary, spec):
        self.root = root
        self.binary = binary
        self.spec = spec
        self.home = f"{root}/home"
        self.bin = f"{root}/bin"
        self.db = f"{root}/cafleet.db"
        self.url = f"sqlite:///{self.db}"
        self.mux_socket = f"{root}/mux/daemon.sock"
        self.mux_log = f"{root}/mux/calls.log"
        self.native_dir = f"{root}/native"
        self.daemon = None
        self.log_offset = 0

    # ------------------------------------------------------------ construction

    def env(self, extra=None):
        env = {"HOME": self.home, "PATH": f"{self.bin}:/usr/bin:/bin", "LANG": "C.UTF-8", "TZ": "UTC",
               "TERM": "xterm-256color", "SHELL": "/bin/sh"}
        for key, value in (extra or {}).items():
            if value is None:
                env.pop(key, None)
            else:
                env[key] = self.subst(value)
        return env

    def subst(self, value):
        if isinstance(value, bytes):
            return value
        return (value.replace("{URL}", self.url).replace("{W}", self.root).replace("{DB}", self.db)
                .replace("{HOME}", self.home))

    def build(self):
        shutil.rmtree(self.root, ignore_errors=True)
        for path in (self.home, self.bin, f"{self.root}/mux", self.native_dir):
            os.makedirs(path)
        os.chmod(self.root, 0o700)
        natives = {b: default_native(b, MEMBER_IDLE) for b in BACKENDS}
        natives.update({f"director-{b}": default_native(b, DIRECTOR_IDLE, director=True) for b in BACKENDS})
        for name, override in self.spec.get("natives", {}).items():
            natives[name] = {**natives[name], **override}
        os.makedirs(f"{self.root}/director")
        with open(f"{self.root}/native.json", "w") as f:
            json.dump({"natives": natives, "log_dir": self.native_dir, "repo": REPO}, f)
        for backend in self.spec.get("path_natives", BACKENDS):
            self.write_native(f"{self.bin}/{backend}", backend, backend)
        for backend in BACKENDS:
            self.write_native(f"{self.root}/director/{backend}", backend, f"director-{backend}")
        if self.spec.get("layout", "tmux"):
            for tool in self.spec.get("mux_tools", ("tmux", "herdr")):
                self.write_exec(f"{self.bin}/{tool}",
                                f"#!{sys.executable} -I\nimport sys\nsys.path.insert(0, {HARNESS!r})\nimport fakemux\n"
                                f"sys.exit(fakemux.client_main({self.mux_socket!r}, {tool!r}, sys.argv[1:]))\n")
            self.start_daemon()
        for step in self.spec.get("prepare", []):
            step(self)

    def write_exec(self, path, text):
        with open(path, "w") as f:
            f.write(text)
        os.chmod(path, 0o755)

    def write_native(self, path, backend, script):
        self.write_exec(path, f"#!{sys.executable} -I\nimport sys\nsys.path.insert(0, {HARNESS!r})\nimport fakenative\n"
                              f"sys.exit(fakenative.main({backend!r}, {script!r}, {self.root + '/native.json'!r}))\n")

    def native_command(self, backend):
        return [f"{self.root}/director/{backend}"]

    def mux_config(self):
        layout = self.spec.get("layout", "tmux")
        director = self.spec.get("director_backend", "claude")
        command = self.spec.get("director_command", self.native_command(director))
        report_pid = os.getpid() if self.spec.get("director_is_ancestor", True) else None
        config = {"socket": self.mux_socket, "log": self.mux_log, "world": self.root, "ready": f"{self.root}/mux/ready",
                  "pane_env": self.env(), "tmux": [], "herdr": [], "faults": self.spec.get("faults", [])}
        if layout in ("tmux", "both"):
            config["tmux"].append({"socket": f"{self.root}/tmux.sock", "rows": 39, "cols": 241, "sessions": [
                {"name": "main", "windows": [{"name": "director", "command": command, "report_pid": report_pid}]}]})
            if self.spec.get("default_tmux", True):
                config["default_tmux"] = f"{self.root}/tmux.sock"
        if layout in ("herdr", "both"):
            config["herdr"].append({"name": "default", "socket_path": f"{self.root}/herdr.sock", "rows": 39, "cols": 241,
                                    "workspaces": [{"id": "w1", "tabs": [{"label": "director", "command": command,
                                                                          "report_pid": report_pid}]}]})
        for extra in self.spec.get("extra_tmux", []):
            config["tmux"].append(extra)
        return config

    def start_daemon(self):
        config_path = f"{self.root}/mux/config.json"
        with open(config_path, "w") as f:
            json.dump(self.mux_config(), f)
        self.daemon = subprocess.Popen([sys.executable, "-I", f"{HARNESS}/fakemux.py", "serve", config_path],
                                       stdin=subprocess.DEVNULL, stdout=open(f"{self.root}/mux/daemon.out", "wb"),
                                       stderr=subprocess.STDOUT, cwd=self.root, env=self.env())
        deadline = time.monotonic() + 10
        while not os.path.exists(f"{self.root}/mux/ready"):
            if self.daemon.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError(f"fakemux daemon failed: {open(self.root + '/mux/daemon.out').read()}")
            time.sleep(0.01)
        self.wait_director_drawn(deadline)

    def wait_director_drawn(self, deadline):
        """Every world starts from a Director pane whose native has drawn its first screen."""
        if "director_command" in self.spec:
            return
        for pane in self.control(op="panes")["panes"]:
            while not any(line.strip() for line in self.control(op="screen", pane=pane["pane"])["lines"]):
                if time.monotonic() > deadline:
                    raise RuntimeError(f"the Director pane {pane['pane']} never drew its native screen")
                time.sleep(0.01)

    def control(self, **payload):
        return fakemux.request(self.mux_socket, {"tool": "control", **payload})

    def stop(self):
        if self.daemon is not None:
            try:
                self.control(op="stop")
            except OSError:
                self.daemon.send_signal(signal.SIGTERM)
            try:
                self.daemon.wait(timeout=15)
            except subprocess.TimeoutExpired:
                os.killpg(self.daemon.pid, signal.SIGKILL)
                self.daemon.wait()
            self.daemon = None
        self.kill_strays()

    def kill_strays(self):
        """Kill any process still running with the world in its command line (a runner or native
        the daemon no longer tracks), so no world outlives its scenario."""
        for pid, cmdline in process_table():
            if self.root.encode() in cmdline and pid != os.getpid():
                try:
                    os.kill(pid, signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass

    # ------------------------------------------------------------ execution

    def confined(self, argv, env):
        """`argv` with an explicit world database, after checking that `env` confines the command
        to the world: HOME is the world's home, every backend config-location variable that is
        set lies inside the world, and an explicit `--database-url` names a world path. A command
        without one gets the world database prepended as its global option. Configuration homes
        and databases outside the world are never reachable, whatever the binary under test does
        with HOME."""
        assert env.get("HOME") == self.home, f"HOME must be the world home, got {env.get('HOME')!r}"
        for name in ("CLAUDE_CONFIG_DIR", "CODEX_HOME", "OPENCODE_CONFIG_DIR", "HERMES_HOME", "PI_CODING_AGENT_DIR"):
            value = env.get(name)
            assert value is None or not value.startswith("/") or value.startswith(self.root + "/"), \
                f"{name} must stay inside the world, got {value!r}"
        urls = [argv[k + 1] for k, a in enumerate(argv[:-1]) if a == "--database-url"]
        urls += [a.split("=", 1)[1] for a in argv if a.startswith("--database-url=")]
        for url in urls:
            path = url.removeprefix("sqlite:///")
            assert not path.startswith("/") or path.startswith(self.root + "/"), f"database outside the world: {url!r}"
        return argv if urls else [argv[0], "--database-url", self.url, *argv[1:]]

    def run(self, step, binary=None):
        binary = binary or self.binary
        env = self.env(step.get("env"))
        argv = self.confined(["cafleet", *[self.subst(a) for a in step["argv"]]], env)
        stdin = step.get("stdin")
        sink = step.get("stdout", "pipe")
        kwargs = dict(executable=binary, env=env, cwd=self.subst(step.get("cwd", self.root)))
        pidfile = None
        if "via" in step:
            pidfile = f"{self.root}/via.pid"
            argv = [sys.executable, "-I", f"{HARNESS}/via.py", pidfile, *step["via"], "--", binary, *argv[1:]]
            kwargs["executable"] = sys.executable
        timeout = step.get("timeout", 120)
        if sink == "pipe":
            p = subprocess.Popen(argv, stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kwargs)
            out, err = p.communicate(stdin, timeout=timeout)
        elif sink == "devfull" and not os.path.exists("/dev/full"):
            return {"exit": None, "stdout": b"<skipped: this platform has no /dev/full>", "stderr": b"", "pid": None,
                    "mux": {"own": [], "other": []}, "wrapper_pid": None}
        elif sink == "devfull":
            with open("/dev/full", "wb") as full:
                p = subprocess.Popen(argv, stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL,
                                     stdout=full, stderr=subprocess.PIPE, **kwargs)
                _, err = p.communicate(stdin, timeout=timeout)
            out = b"<dev/full>"
        elif sink == "broken-pipe":
            read_end, write_end = os.pipe()
            os.close(read_end)
            p = subprocess.Popen(argv, stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL,
                                 stdout=write_end, stderr=subprocess.PIPE, **kwargs)
            os.close(write_end)
            _, err = p.communicate(stdin, timeout=timeout)
            out = b"<broken pipe>"
        else:
            raise ValueError(sink)
        pid = p.pid
        if pidfile is not None:
            with open(pidfile) as f:
                pid = int(f.read())
            os.unlink(pidfile)
        return {"exit": p.returncode, "stdout": out, "stderr": err, "pid": pid, "mux": self.take_mux(pid),
                "wrapper_pid": p.pid}

    def take_mux(self, pid):
        """The stand-in invocations made since the previous step: the step's own, and the others."""
        if not os.path.exists(self.mux_log):
            return {"own": [], "other": []}
        with open(self.mux_log) as f:
            f.seek(self.log_offset)
            lines = f.readlines()
            self.log_offset = f.tell()
        own, other = [], []
        for line in lines:
            entry = json.loads(line)
            (own if entry["ppid"] == pid else other).append([entry["tool"], entry["argv"], entry["code"]])
        return {"own": own, "other": other}

    # ------------------------------------------------------------ observation

    def sql(self, statements, params=()):
        conn = sqlite3.connect(self.db, isolation_level=None)
        conn.execute("PRAGMA foreign_keys=ON")
        conn.execute("BEGIN IMMEDIATE")
        for statement in statements:
            conn.execute(statement, params)
        conn.execute("COMMIT")
        conn.close()

    def query(self, statement, params=()):
        conn = sqlite3.connect(f"file:{self.db}?mode=ro", uri=True)
        try:
            return conn.execute(statement, params).fetchall()
        finally:
            conn.close()

    def dump_db(self):
        if not os.path.isfile(self.db) or os.path.getsize(self.db) == 0:
            return "<no database file>"
        try:
            conn = sqlite3.connect(f"file:{self.db}?mode=ro", uri=True)
            tables = [r[0] for r in conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
            out = []
            for table in tables:
                columns = [r[1] for r in conn.execute(f"PRAGMA table_info({table})")]
                rows = conn.execute(f"SELECT * FROM {table} ORDER BY rowid").fetchall()
                out.append(f"{table}: {columns!r}")
                out.extend(f"  {row!r}" for row in rows)
            conn.close()
        except sqlite3.DatabaseError as error:
            return f"<undumpable: {error}>"
        return "\n".join(out)

    def native_logs(self):
        """Each native's invocation log and received bytes. A log's lines are sorted: member
        creation runs the version probe and `hermes config path` concurrently, so their order is
        not part of the contract; each interactive entry carries its launch ordinal."""
        out = {}
        if not os.path.isdir(self.native_dir):
            return out
        for name in sorted(os.listdir(self.native_dir)):
            path = os.path.join(self.native_dir, name)
            with open(path, "rb") as f:
                data = f.read().decode("utf-8", "backslashreplace")
            out[name] = "".join(sorted(data.splitlines(keepends=True))) if name.endswith(".log") else data
        return out

    def files(self, base):
        """Relative path → content for every file under `base`: small text files verbatim (the
        harness normalizes them like any output), others by digest, SQLite files by kind only
        (their bytes hold a random instance id)."""
        import hashlib
        out = {}
        base = self.subst(base)
        for dirpath, dirnames, filenames in os.walk(base):
            dirnames.sort()
            for name in sorted(filenames):
                path = os.path.join(dirpath, name)
                rel = os.path.relpath(path, base)
                if os.path.islink(path):
                    out[rel] = "-> " + os.readlink(path)
                elif not os.path.isfile(path):
                    out[rel] = "<not a regular file>"
                else:
                    with open(path, "rb") as f:
                        data = f.read()
                    if data.startswith(b"SQLite format 3\0"):
                        out[rel] = "<sqlite database>"
                    elif len(data) <= 4096 and b"\0" not in data:
                        out[rel] = data.decode("utf-8", "backslashreplace")
                    else:
                        out[rel] = hashlib.sha256(data).hexdigest()[:16]
        return out


REAL_HOME_TARGETS = (".claude/skills", ".codex/skills", ".codex/rules", ".config/opencode/skills",
                     ".config/opencode/agents", ".config/opencode/plugins", ".hermes/skills", ".pi/agent/skills",
                     ".pi/agent/extensions", ".local/share/cafleet")


def real_home_state():
    """(path, mtime_ns, size) of every cafleet-owned entry under the invoking user's real homes
    (the account's home directory and the harness's own HOME): installed skills, presets and
    extensions, and the default database directory's entries by name only, because a live fleet
    of the installed cafleet legitimately writes its own database during a run. The harness
    compares it around every scenario."""
    import pwd
    homes = {pwd.getpwuid(os.getuid()).pw_dir, os.environ.get("HOME", "")} - {""}
    state = []
    for home in sorted(homes):
        for target in REAL_HOME_TARGETS:
            base = os.path.join(home, target)
            if not os.path.isdir(base):
                continue
            for dirpath, dirnames, filenames in os.walk(base):
                dirnames.sort()
                for name in sorted(dirnames + filenames):
                    if "cafleet" not in os.path.join(dirpath, name)[len(home):]:
                        continue
                    path = os.path.join(dirpath, name)
                    if target == ".local/share/cafleet":
                        if dirpath == base and name.endswith(".db"):
                            state.append((path, None, None))
                        continue
                    info = os.lstat(path)
                    state.append((path, info.st_mtime_ns, info.st_size))
    return state


def process_table():
    """(pid, command line bytes) of every visible process: /proc on Linux, `ps` elsewhere."""
    if os.path.isdir("/proc/self"):
        rows = []
        for entry in os.listdir("/proc"):
            if entry.isdigit():
                try:
                    with open(f"/proc/{entry}/cmdline", "rb") as f:
                        rows.append((int(entry), f.read()))
                except OSError:
                    continue
        return rows
    listing = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, check=True).stdout
    rows = []
    for line in listing.splitlines():
        fields = line.strip().split(None, 1)
        if fields:
            rows.append((int(fields[0]), fields[1] if len(fields) > 1 else b""))
    return rows


# ---------------------------------------------------------------- normalization

TS_PATTERNS = [
    re.compile(rb"\d{4}-\d\d-\d\d[T ]\d\d:\d\d:\d\d(?:\.\d{1,9})?(?:Z|[+-]\d\d:\d\d)"),
]
HEX64 = re.compile(rb"(?<![0-9a-f])[0-9a-f]{64}(?![0-9a-f])")
HEX32 = re.compile(rb"(?<![0-9a-f])[0-9a-f]{32}(?![0-9a-f])")
NAMESPACE = re.compile(rb"\.cafleet-runs/([a-z2-7]{26})")


class Normalizer:
    """Replaces run-dependent values by first-occurrence ordinals shared across one world run.

    Timestamps, pids, process start identities, runtime namespaces, launch nonces, request tokens
    and database file identities differ between any two runs; ordinals keep their equality
    structure comparable. `literals` maps fixed byte strings, such as the path of the binary
    under test, to their placeholders.
    """

    def __init__(self, pids=(), extra=(), literals=None):
        self.tables = {}
        self.pids = {str(p).encode() for p in pids}
        self.extra = list(extra)
        self.literals = sorted((literals or {}).items(), key=lambda item: -len(item[0]))

    def ordinal(self, kind, value):
        table = self.tables.setdefault(kind, {})
        if value not in table:
            table[value] = len(table) + 1
        return b"<%s%d>" % (kind.encode(), table[value])

    def __call__(self, data):
        if isinstance(data, str):
            return self(data.encode("utf-8", "surrogateescape")).decode("utf-8", "surrogateescape")
        for literal, placeholder in self.literals:
            data = data.replace(literal, placeholder)
        for pattern in TS_PATTERNS:
            data = pattern.sub(lambda m: self.ordinal("TS", m.group(0)), data)
        data = NAMESPACE.sub(lambda m: b".cafleet-runs/" + self.ordinal("NS", m.group(1)), data)
        data = HEX64.sub(lambda m: self.ordinal("HEX", m.group(0)), data)
        data = HEX32.sub(lambda m: self.ordinal("ID", m.group(0)), data)
        for kind, pattern in self.extra:
            data = pattern.sub(lambda m: self.ordinal(kind, m.group(0)), data)
        if self.pids:
            alternatives = b"|".join(sorted((re.escape(p) for p in self.pids), key=len, reverse=True))
            data = re.sub(rb"(?<![0-9])(?:" + alternatives + rb")(?![0-9])", lambda m: self.ordinal("PID", m.group(0)), data)
        return data
