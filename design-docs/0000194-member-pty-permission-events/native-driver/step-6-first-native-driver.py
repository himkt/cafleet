"""Director-owned one-case native capture driver; executing a mode launches real processes."""
import argparse
import ctypes
import errno
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time

BASE = Path(__file__).resolve().parents[2]
if not BASE.is_absolute() or str(BASE) == "<unset>":
    raise RuntimeError("Error: BASE is <unset>; refusing to fall back to /tmp")
# Committed native fixtures derive from these captures, so their visible paths must not name a home directory.
NEUTRAL = Path("/tmp/cafleet-194")
# The repository mount reports root-created files as owned by another user, which the private
# ownership checks refuse, so case databases and sockets live on the local filesystem.
PRIVATE = Path("/tmp/cafleet-194-p")
ANSWER_WINDOW_MS = 60_000
# Mirrors APPROVED_CASES in verification.rs; the driver rejects any other binding.
CASES = {
    "cells-002": {"backend": "claude", "mode": "no_answer", "decision": None, "stage_hex": None,
                  "probe_argument": "allow", "database": "nc02.db", "socket": "tc02", "session": "cafleet194nc02"},
    "ans-allow-001": {"backend": "claude", "mode": "answer_stage", "decision": "allow", "stage_hex": "31",
                      "probe_argument": "allow", "database": "na01.db", "socket": "ta01", "session": "cafleet194na01"},
    "ans-deny-001": {"backend": "claude", "mode": "answer_stage", "decision": "deny", "stage_hex": "34",
                     "probe_argument": "deny", "database": "nd01.db", "socket": "td01", "session": "cafleet194nd01"},
    "idle-001": {"backend": "claude", "mode": "no_answer", "decision": None, "stage_hex": None,
                 "probe_argument": None, "database": "ni01.db", "socket": "ti01", "session": "cafleet194ni01"},
    "codex-trust-001": {"backend": "codex", "mode": "no_answer", "decision": None, "stage_hex": None,
                        "probe_argument": None, "database": "xt01.db", "socket": "xt01", "session": "cafleet194xt01"},
    "codex-cells-001": {"backend": "codex", "mode": "no_answer", "decision": None, "stage_hex": None,
                        "probe_argument": "allow", "database": "xc01.db", "socket": "xc01", "session": "cafleet194xc01"},
    "codex-idle-001": {"backend": "codex", "mode": "no_answer", "decision": None, "stage_hex": None,
                       "probe_argument": None, "database": "xi01.db", "socket": "xi01", "session": "cafleet194xi01"},
}
# The reviewed native executable and permission-probe helper per backend, pinned by digest.
NATIVES = {
    "claude": {"path": Path("/root/.local/share/mise/installs/aqua-anthropics-claude-code/v2.1.302/claude"),
               "sha256": "60d7eb7f12fbb28a52533fe8be5884097403562aa02dc73991a021c7067e29a5",
               "version": "2.1.302",
               "helper": BASE / ".execution/194/native-fixtures/claude-default-linux/permission-probe",
               "helper_sha256": "de637bad7b3c569c448634f24ec3cb09d97d0a528e2eac27f3be544b5fbd03c0"},
    "codex": {"path": Path("/root/.local/share/mise/installs/aqua-openai-codex/v0.163.0/bin/codex"),
              "sha256": "399ff69c431c066e40de318fba45c2b07f45a76b19939d247ee4828fc70c4215",
              "version": "0.163.0",
              "helper": BASE / ".execution/194/native-fixtures/codex-on-request-linux/permission-probe",
              "helper_sha256": "a7368024d7452112753355570f3c64705f53996a6b7cdb0f2dbc7edcef0c9f8e"},
}
VERSION_TOKEN = re.compile(r"^[0-9]+(\.[0-9]+)+([-+][0-9A-Za-z.]+)?$")
LIBTEST_ROOT = BASE / ".execution/194/native-target-002/debug/deps"
CASE_ID = CWD = OUTPUT = SOCKET = DATABASE = SESSION = SELECTED = None


def select_case(case_id):
    global CASE_ID, CWD, OUTPUT, SOCKET, DATABASE, SESSION, SELECTED
    SELECTED = CASES[case_id]
    CASE_ID = case_id
    CWD = NEUTRAL / case_id
    OUTPUT = NEUTRAL / f"{case_id}-capture"
    SOCKET = PRIVATE / SELECTED["socket"]
    DATABASE = PRIVATE / SELECTED["database"]
    SESSION = SELECTED["session"]


def answer_window_ms():
    return ANSWER_WINDOW_MS if SELECTED["mode"] == "answer_stage" else 0


def forbidden_markers():
    allowed = "allow.executed" if SELECTED["decision"] == "allow" else None
    return [marker for marker in ("allow.executed", "deny.executed") if marker != allowed]


def native():
    return NATIVES[SELECTED["backend"]]


def version_token(line):
    tokens = [token for token in line.split() if VERSION_TOKEN.match(token)]
    require(len(tokens) == 1, "exactly one native version token required")
    return tokens[0]


ENTRY = "runtime::runner::native_capture::verification::native_capture_prepare_start_driver"


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def monotonic_ns():
    # The Rust driver samples POSIX CLOCK_MONOTONIC; time.monotonic_ns() is mach_absolute_time on macOS.
    return time.clock_gettime_ns(time.CLOCK_MONOTONIC)


def digest(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as source:
        while chunk := source.read(65536):
            value.update(chunk)
    return value.hexdigest()


def private_directory(path):
    path = Path(path)
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    meta = path.lstat()
    require(stat.S_ISDIR(meta.st_mode) and meta.st_uid == os.getuid()
            and stat.S_IMODE(meta.st_mode) == 0o700, "private owned directory required")
    return path


def owned_parent(path):
    path = Path(path)
    path.mkdir(mode=0o700, exist_ok=True)
    meta = path.lstat()
    require(stat.S_ISDIR(meta.st_mode) and meta.st_uid == os.getuid()
            and stat.S_IMODE(meta.st_mode) & 0o022 == 0, "owned directory without group/other write required")
    return path


def read_bytes(path, limit=65536, private=True):
    path = Path(path)
    meta = path.lstat()
    require(path.is_absolute() and stat.S_ISREG(meta.st_mode) and meta.st_size <= limit,
            "bounded absolute regular file required")
    if private:
        require(meta.st_uid == os.getuid() and stat.S_IMODE(meta.st_mode) == 0o600,
                "private owned0600 file required")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd, "rb") as source:
        actual = os.fstat(source.fileno())
        require((meta.st_dev, meta.st_ino) == (actual.st_dev, actual.st_ino), "file replaced")
        value = source.read(limit + 1)
    require(len(value) <= limit, "file exceeds bound")
    return value


def read_json(path):
    return json.loads(read_bytes(path))


def publish(path, value):
    path = Path(path)
    data = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    require(len(data) <= 65536, "private record exceeds bound")
    temporary = path.with_name(path.name + ".pending-" + secrets.token_hex(8))
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        with os.fdopen(fd, "wb") as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.link(temporary, path)
        parent = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(parent)
        finally:
            os.close(parent)
    finally:
        temporary.unlink()


def command(argv, until_ns, cwd=None, environment=None):
    remaining = (until_ns - monotonic_ns()) / 1_000_000_000
    require(remaining > 0.25, "original command bound expired before spawn")
    with tempfile.TemporaryFile(dir=PRIVATE) as stdout, tempfile.TemporaryFile(dir=PRIVATE) as stderr:
        child = subprocess.Popen([str(arg) for arg in argv], stdin=subprocess.DEVNULL,
                                 stdout=stdout, stderr=stderr, cwd=cwd, env=environment)
        limit = until_ns - 250_000_000
        cause = None
        while child.poll() is None:
            if monotonic_ns() >= limit:
                cause = "original command deadline expired"
                break
            if os.fstat(stdout.fileno()).st_size > 65536 or os.fstat(stderr.fileno()).st_size > 65536:
                cause = "command output exceeds private bound"
                break
            time.sleep(min(0.01, max(0, (limit - monotonic_ns()) / 1_000_000_000)))
        if cause is not None:
            child.kill()
            remaining = max(0, (until_ns - monotonic_ns()) / 1_000_000_000)
            try:
                child.wait(timeout=remaining)
            except subprocess.TimeoutExpired as error:
                raise RuntimeError(f"{cause}; owned PID{child.pid} cleanup unconfirmed") from error
            raise RuntimeError(cause)
        require(os.fstat(stdout.fileno()).st_size <= 65536
                and os.fstat(stderr.fileno()).st_size <= 65536, "command output exceeds bound")
        stdout.seek(0)
        stderr.seek(0)
        result, errors = stdout.read(), stderr.read()
    require(child.returncode == 0, f"owned command exit{child.returncode}: {errors[:2048]!r}")
    return result.decode("utf-8")


# CODEX_HOME may select a private assets-guard home without credentials; the native child uses its own.
SERVER_REMOVED_NAMES = ("CLAUDECODE", "CLAUDE_EFFORT", "CLAUDE_PID", "CODEX_HOME")
SERVER_REMOVED_PREFIXES = ("CLAUDE_CODE_", "HERDR_")


class PreOwnershipFailure(RuntimeError):
    pass


def server_removed(name):
    return name in SERVER_REMOVED_NAMES or name.startswith(SERVER_REMOVED_PREFIXES)


def native_environment():
    # Production members launch from the multiplexer server, not from the Director's session shell,
    # whose session markers and credentials must not reach the dedicated server or native child.
    return {key: value for key, value in os.environ.items() if not server_removed(key)}


def tmux(parts, until_ns):
    return command(["tmux", "-S", SOCKET, *parts], until_ns, environment=native_environment())


class BSDInfo(ctypes.Structure):
    _fields_ = [("prefix", ctypes.c_ubyte * 120),
                ("start_seconds", ctypes.c_uint64), ("start_microseconds", ctypes.c_uint64)]


def process_identity(pid):
    if sys.platform == "linux":
        return linux_process_identity(pid)
    require(sys.platform == "darwin" and ctypes.sizeof(BSDInfo) == 136,
            "this case requires the reviewed macOS proc_bsdinfo layout")
    library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
    function = library.proc_pidinfo
    function.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
    function.restype = ctypes.c_int
    info = BSDInfo()
    ctypes.set_errno(0)
    count = function(pid, 3, 0, ctypes.byref(info), ctypes.sizeof(info))
    if count == 0 and ctypes.get_errno() in (errno.ESRCH, errno.ENOENT):
        return None
    require(count == ctypes.sizeof(info), "complete process birth identity unavailable")
    return {"pid": pid, "start_identity": f"{info.start_seconds}:{info.start_microseconds}"}


def linux_process_identity(pid):
    # Matches ProcessIdentity::read on Linux: boot ID plus the stat start time (field 22).
    try:
        text = Path(f"/proc/{pid}/stat").read_text()
    except FileNotFoundError:
        return None
    fields = text.rsplit(")", 1)[1].split()
    require(len(fields) > 19, "malformed process stat")
    boot = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    return {"pid": pid, "start_identity": f"{boot}:{fields[19]}"}


def reference(source, name):
    data = read_bytes(source)
    target = CWD / name
    fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())
    return {"path": str(target), "sha256": digest(target)}


def case():
    value = read_json(CWD / "case.json")
    require(value["schema_version"] == 1 and value["case_id"] == CASE_ID
            and value["cwd"] == str(CWD) and value["database_url"] == f"sqlite:///{DATABASE}"
            and value["tmux"]["socket_path"] == str(SOCKET), "case resources differ")
    return value


def validate_live(value, started=None):
    ownership = read_json(value["driver_ownership_path"])
    require(ownership["case_manifest_sha256"] == digest(CWD / "case.json")
            and ownership["case_nonce"] == value["case_nonce"], "ownership binding differs")
    for identity in (ownership["driver"], ownership["server"]):
        require(process_identity(identity["pid"]) == identity, "owned process generation changed")
    meta = SOCKET.lstat()
    require(stat.S_ISSOCK(meta.st_mode) and (meta.st_dev, meta.st_ino) ==
            (value["tmux"]["socket_device"], value["tmux"]["socket_inode"]), "socket changed")
    context = tmux(["display-message", "-p", "-t", value["tmux"]["pane_id"],
                    "#{pid}|#{session_name}|#{window_id}|#{pane_id}"], value["expires_at_mono_ns"]).strip()
    expected = value["tmux"]
    require(context == f'{expected["server_pid"]}|{expected["session"]}|{expected["window_id"]}|{expected["pane_id"]}',
            "dedicated controller changed")
    if started is not None:
        require(started["case_manifest_sha256"] == digest(CWD / "case.json")
                and started["driver_ownership_sha256"] == digest(value["driver_ownership_path"]),
                "started record binding differs")
        for identity in (started["runner"], started["child"]):
            require(process_identity(identity["pid"]) == identity, "native generation changed")
        connection = sqlite3.connect(f"file:{DATABASE}?mode=ro", uri=True, timeout=0)
        connection.row_factory = sqlite3.Row
        connection.set_progress_handler(lambda: int(monotonic_ns() >= value["expires_at_mono_ns"]), 100)
        try:
            row = connection.execute("SELECT * FROM member_runs WHERE run_id=?",
                                     (started["identity"]["run_id"],)).fetchone()
            require(row is not None and row["status"] == "running", "actual Running row required")
            identity = dict(started["identity"])
            for key in ("fleet_id", "member_id", "director_member_id", "run_id", "owner_epoch", "launch_nonce"):
                require(row[key] == identity[key], f"current {key} changed")
            require(json.loads(row["database_identity_json"]) == identity["database_identity_json"],
                    "database identity changed")
            require(row["owner_pid"] == started["runner"]["pid"]
                    and row["owner_start_identity"] == started["runner"]["start_identity"]
                    and row["child_pid"] == started["child"]["pid"]
                    and row["child_start_identity"] == started["child"]["start_identity"], "run process identity changed")
            placement = connection.execute("SELECT mux_pane_id FROM member_placements WHERE member_id=?",
                                           (identity["member_id"],)).fetchone()
            require(placement is not None and placement[0] == started["pane_id"], "placement changed")
        finally:
            connection.close()
        geometry = tmux(["display-message", "-p", "-t", started["pane_id"],
                         "#{pane_width}|#{pane_height}"], value["expires_at_mono_ns"]).strip()
        require(geometry == "120|36", "actual geometry differs")
    return ownership


def wait_result(value):
    until = value["expires_at_mono_ns"]
    stop_at = until - value["cleanup_reserve_ms"] * 1_000_000
    next_controller_poll = 0
    while monotonic_ns() < until:
        target = Path(value["result_path"])
        if not Path(value["driver_ownership_path"]).exists() and monotonic_ns() >= next_controller_poll:
            next_controller_poll = monotonic_ns() + 250_000_000
            status = tmux(["display-message", "-p", "-t", value["tmux"]["pane_id"],
                           "#{pane_dead}|#{pane_dead_status}"], until).strip()
            if status.startswith("1|") and not Path(value["driver_ownership_path"]).exists():
                raise PreOwnershipFailure(f"driver exited with status {status.split('|', 1)[1]} "
                                          "before ownership publication; no native launch was attempted")
        if target.exists():
            result = read_json(target)
            require(result["case_manifest_sha256"] == digest(CWD / "case.json")
                    and result["driver_ownership_sha256"] == digest(value["driver_ownership_path"]),
                    "result binding differs")
            markers = {"allow.executed": result["allow_marker_absent"], "deny.executed": result["deny_marker_absent"]}
            require(result["shutdown"] is not None and result["shutdown"]["cleanup_state"] == "complete"
                    and result["pane_closed"] and result["runner_absent"] and result["native_absent"]
                    and result["case_id"] == CASE_ID
                    and all(markers[marker] for marker in forbidden_markers()),
                    "owned cleanup is unconfirmed or a probe outside the reviewed decision executed; "
                    "preserve dedicated placement")
            ownership = read_json(value["driver_ownership_path"])
            while process_identity(ownership["driver"]["pid"]) == ownership["driver"] and monotonic_ns() < until:
                time.sleep(0.01)
            require(process_identity(ownership["driver"]["pid"]) is None, "driver exit remains unconfirmed")
            meta = SOCKET.lstat()
            require((meta.st_dev, meta.st_ino) == (value["tmux"]["socket_device"], value["tmux"]["socket_inode"])
                    and process_identity(ownership["server"]["pid"]) == ownership["server"],
                    "dedicated cleanup identity changed")
            require(tmux(["list-panes", "-a", "-F", "#{pane_id}"], until).split() == [value["tmux"]["pane_id"]],
                    "unexpected dedicated pane remains")
            controller_status = tmux(["display-message", "-p", "-t", value["tmux"]["pane_id"],
                                      "#{pane_dead}|#{pane_dead_status}"], until).strip()
            publish(CWD / "final-observation.json", {"schema_version": 1, "result_sha256": digest(target),
                    "controller_status": controller_status, "result": result})
            tmux(["kill-server"], until)
            require(result["observation"] is not None and result["work_error"] is None,
                    "capture work/export incomplete; actual cleanup and outcome preserved")
            service = result["observation"]["service"]
            require(service["result"] == "ok", "service/export did not return Ok")
            probe = SELECTED["probe_argument"]
            require(result["requested_marker"] == (f"{probe}.executed" if probe else None),
                    "result names a different requested marker")
            print(json.dumps({"case": str(CWD / "case.json"), "result": str(target),
                              "service_status": service["status"], "decision": SELECTED["decision"],
                              "stage_hex": SELECTED["stage_hex"],
                              "requested_marker_present": result["requested_marker_present"],
                              "post_stage": result["post_stage"]}))
            return
        if monotonic_ns() >= stop_at and Path(value["driver_ownership_path"]).exists() and not Path(value["stop_path"]).exists():
            ownership = validate_live(value)
            publish(value["stop_path"], {"schema_version": 1, "case_manifest_sha256": digest(CWD / "case.json"),
                    "driver_ownership_sha256": digest(value["driver_ownership_path"]), "reason": "director_stop"})
        if Path(value["driver_ownership_path"]).exists() and not target.exists():
            ownership = read_json(value["driver_ownership_path"])
            require(process_identity(ownership["driver"]["pid"]) == ownership["driver"],
                    "driver exited without a confirmed cleanup record; preserve owned placement")
        time.sleep(min(0.02, max(0, (until - monotonic_ns()) / 1_000_000_000)))
    raise RuntimeError("original case expired; cleanup unconfirmed; preserve owned records and placement")


def prepare_start_case(args):
    until = monotonic_ns() + 120_000_000_000 + answer_window_ms() * 1_000_000
    work_until = until - 4_000_000_000
    require(not CWD.exists() and not OUTPUT.exists() and not DATABASE.exists() and not SOCKET.exists(),
            "fresh isolated resources required")
    libtest = Path(args.libtest).resolve(strict=True)
    require(libtest.is_relative_to(LIBTEST_ROOT), "reviewed separate-target libtest required")
    manifest = read_json(Path(args.source_manifest).resolve(strict=True))
    require((BASE / manifest["executable"]["path"]).resolve(strict=True) == libtest
            and digest(libtest) == manifest["executable"]["sha256"],
            "libtest differs from the reviewed source manifest executable")
    for row in manifest["source"]:
        require(digest(BASE / row["source"]) == row["sha256"], f"reviewed source changed: {row['source']}")
    owned_parent(PRIVATE)
    private_directory(NEUTRAL)
    private_directory(CWD)
    private_directory(OUTPUT)
    publish(CWD / "removed-environment.json", {"schema_version": 1,
            "removed_names": sorted(name for name in os.environ if server_removed(name))})
    pinned = native()
    require(digest(pinned["path"]) == pinned["sha256"], "pinned native executable changed")
    helper_source = pinned["helper"]
    helper_bytes = read_bytes(helper_source, 4096, private=False)
    require(hashlib.sha256(helper_bytes).hexdigest() == pinned["helper_sha256"], "reviewed helper differs")
    helper = CWD / "permission-probe"
    descriptor = os.open(helper, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o700)
    with os.fdopen(descriptor, "wb") as output:
        output.write(helper_bytes)
    require(digest(helper) == digest(helper_source), "helper changed")
    version_output = command([pinned["path"], "--version"], work_until, cwd=CWD,
                             environment=native_environment())
    require(len(version_output.splitlines()) == 1 and version_token(version_output) == pinned["version"],
            "fresh same-file normalized version differs")
    version = CWD / "version-evidence.json"
    publish(version, {"schema_version": 1, "executable": str(pinned["path"]), "sha256": pinned["sha256"],
                     "argv": [str(pinned["path"]), "--version"], "stdout": version_output, "exit_code": 0})
    source = reference(Path(args.source_manifest).resolve(strict=True), "source-manifest.json")
    environment = reference(Path(args.environment_evidence).resolve(strict=True), "environment-evidence.json")
    redaction = reference(Path(args.redaction).resolve(strict=True), "event-redaction.json")
    policy = reference(Path(args.capture_policy).resolve(strict=True), "capture-policy.json")
    fd = os.open(DATABASE, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    os.close(fd)
    tmux(["-f", "/dev/null", "new-session", "-d", "-s", SESSION, "-x", "180", "-y", "80",
          "-c", CWD, "/bin/sleep", "120"], work_until)
    tmux(["set-option", "-g", "remain-on-exit", "on"], work_until)
    tmux(["set-option", "-g", "status", "off"], work_until)
    server_names = {line.split("=", 1)[0]
                    for line in tmux(["show-environment", "-g"], work_until).splitlines()
                    if not line.startswith("-")}
    require(not any(server_removed(name) for name in server_names),
            "dedicated server environment retains a removed variable")
    context = tmux(["display-message", "-p", "-t", SESSION, "#{pid}|#{session_name}|#{window_id}|#{pane_id}"],
                   work_until).strip().split("|")
    require(len(context) == 4, "actual dedicated context required")
    metadata = SOCKET.lstat()
    publish(CWD / "script-server.json", {"socket_device": metadata.st_dev,
            "socket_inode": metadata.st_ino, "server": process_identity(int(context[0])),
            "expires_at_mono_ns": until})
    value = {"schema_version": 1, "case_id": CASE_ID, "mode": SELECTED["mode"],
             "answer_window_ms": answer_window_ms(), "decision": SELECTED["decision"],
             "stage_hex": SELECTED["stage_hex"], "probe_argument": SELECTED["probe_argument"],
             "case_nonce": secrets.token_hex(32),
             "expires_at_mono_ns": until, "cleanup_reserve_ms": 4000,
             "database_url": f"sqlite:///{DATABASE}", "cwd": str(CWD), "output_directory": str(OUTPUT),
             "native_version": pinned["version"],
             "native_executable": {"path": str(pinned["path"]), "sha256": pinned["sha256"]},
             "helper": {"path": str(helper), "sha256": digest(helper)},
             "libtest": {"path": str(libtest), "sha256": digest(libtest)},
             "source_manifest": source, "version_evidence": {"path": str(version), "sha256": digest(version)},
             "environment_evidence": environment, "event_redaction": redaction, "capture_policy": policy,
             "director_backend": args.director_backend,
             "tmux": {"socket_path": str(SOCKET), "socket_device": metadata.st_dev, "socket_inode": metadata.st_ino,
                      "server_pid": int(context[0]), "session": context[1], "window_id": context[2], "pane_id": context[3]}}
    names = {"config_path": "capture-config.json", "arm_path": "capture-arm.json",
             "producer_manifest_path": "producer.json", "launch_evidence_path": "launch.json",
             "driver_ownership_path": "driver-ownership.json", "started_record_path": "started.json",
             "stop_path": "stop.json", "privacy_confirmation_path": "privacy-confirmation.json",
             "result_path": "driver-result.json", "descriptor_copy_path": "descriptor-copy.json"}
    value.update({key: str(CWD / name) for key, name in names.items()})
    publish(CWD / "case.json", value)
    publish(CWD / "driver-start-attempt.json", {"schema_version": 1,
            "case_manifest_sha256": digest(CWD / "case.json")})
    tmux(["respawn-pane", "-k", "-t", context[3], "-e", f'CAFLEET_DATABASE_URL={value["database_url"]}',
          "-e", f'CAFLEET_NATIVE_CAPTURE_CONFIG={value["config_path"]}', libtest, "--exact", ENTRY,
          "--ignored", "--nocapture", "--test-threads=1"], work_until)
    print(json.dumps({"case": str(CWD / "case.json"), "controller": context[3],
                      "expires_at_mono_ns": until}), flush=True)
    wait_result(value)


def prepare_start(args):
    try:
        prepare_start_case(args)
    except BaseException as primary:
        cleanup = "native launch not attempted"
        server_record = CWD / "script-server.json"
        if SOCKET.exists() and not server_record.exists():
            cleanup = "dedicated server may exist; generation unconfirmed; preserve socket for reconciliation"
        if server_record.exists():
            owned = read_json(server_record)
            if (CWD / "final-observation.json").exists():
                cleanup = "driver result confirmed owned cleanup; dedicated server close attempted after final observation"
            elif isinstance(primary, PreOwnershipFailure):
                cleanup = ("no native launch; dead controller pane and dedicated server preserved "
                           "for failure capture before an explicit kill-server")
            elif (CWD / "driver-start-attempt.json").exists():
                cleanup = "driver may have started; preserve owned placement and original-bound cleanup records"
            else:
                try:
                    metadata = SOCKET.lstat()
                    require((metadata.st_dev, metadata.st_ino) ==
                            (owned["socket_device"], owned["socket_inode"])
                            and process_identity(owned["server"]["pid"]) == owned["server"],
                            "prelaunch server generation changed")
                    tmux(["kill-server"], owned["expires_at_mono_ns"])
                    cleanup = "fresh prelaunch server closed"
                except BaseException as failure:
                    cleanup = f"prelaunch cleanup unconfirmed: {failure}"
        if CWD.exists():
            try:
                publish(CWD / "preparation-error.json", {"schema_version": 1,
                        "primary": str(primary)[:2048], "cleanup": cleanup[:2048]})
            except BaseException as failure:
                primary.add_note(f"private failure record unavailable: {failure}")
        raise


def arm(args):
    require(args.privacy_reviewed, "actual visual privacy review is required")
    value = case()
    started = read_json(value["started_record_path"])
    validate_live(value, started)
    for name in ("capture_policy", "event_redaction"):
        require(digest(value[name]["path"]) == value[name]["sha256"], "reviewed privacy input changed")
    require(not (CWD / "allow.executed").exists() and not (CWD / "deny.executed").exists(), "probe already executed")
    now = monotonic_ns()
    timeout =min(5000, (value["expires_at_mono_ns"] - 4_000_000_000 - now) // 1_000_000)
    require(timeout >= 101, "original work bound cannot contain a parser pair")
    publish(value["privacy_confirmation_path"], {"schema_version": 1,
            "case_manifest_sha256": digest(CWD / "case.json"),
            "started_record_sha256": digest(value["started_record_path"]), "rows": 36, "cols": 120,
            "capture_policy_sha256": value["capture_policy"]["sha256"],
            "event_redaction_sha256": value["event_redaction"]["sha256"]})
    publish(value["arm_path"], {"schema_version": 1, "capture_id": CASE_ID,
            "run_id": started["identity"]["run_id"], "owner_epoch": started["identity"]["owner_epoch"],
            "minimum_interval_ms": 100, "timeout_ms": timeout})
    print(json.dumps({"armed": CASE_ID, "native_decision": "none before the reviewed stage"}))


def stop(_):
    value = case()
    validate_live(value)
    publish(value["stop_path"], {"schema_version": 1, "case_manifest_sha256": digest(CWD / "case.json"),
            "driver_ownership_sha256": digest(value["driver_ownership_path"]), "reason": "director_stop"})
    print(json.dumps({"stop_requested": True, "result": value["result_path"]}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="mode", required=True)
    prepare = modes.add_parser("prepare-start")
    for argument in ("libtest", "source-manifest", "environment-evidence", "redaction", "capture-policy"):
        prepare.add_argument("--" + argument, required=True)
    prepare.add_argument("--director-backend", required=True)
    prepare.set_defaults(action=prepare_start)
    capture = modes.add_parser("arm")
    capture.add_argument("--privacy-reviewed", action="store_true", required=True)
    capture.set_defaults(action=arm)
    finish = modes.add_parser("stop")
    finish.set_defaults(action=stop)
    for selected in (prepare, capture, finish):
        selected.add_argument("--case", required=True, choices=sorted(CASES))
    args = parser.parse_args()
    select_case(args.case)
    args.action(args)


if __name__ == "__main__":
    main()
