"""Stand-in tmux and herdr for the migration harness.

One daemon per world models every tmux server and herdr session the world declares. The `tmux`
and `herdr` executables on the world's PATH are thin clients: each sends its argv to the daemon
over a Unix socket and replays the daemon's exit code, stdout and stderr. The daemon logs every
invocation (with the caller's parent pid, so the harness can attribute it to a command step) and
answers from its model or from a scripted fault.

Panes are real PTYs: a pane command runs on the PTY slave as a session leader with the slave as
its controlling terminal, and the daemon drains the master into a screen model, so the cafleet
runner sees a real outer terminal.

Usage:
  fakemux.py serve CONFIG_JSON          run the daemon (the harness starts it)
  fakemux.py client SOCKET TOOL ARGV... the body of the generated `tmux` / `herdr` stand-ins
"""
import errno
import fcntl
import json
import os
import re
import selectors
import signal
import socket
import struct
import sys
import termios
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import screen as screen_model  # noqa: E402


# ---------------------------------------------------------------- wire

def send_frame(sock, obj):
    body = json.dumps(obj).encode()
    sock.sendall(struct.pack(">I", len(body)) + body)


def recv_exact(sock, n):
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("fakemux peer closed")
        buf += chunk
    return buf


def recv_frame(sock):
    (length,) = struct.unpack(">I", recv_exact(sock, 4))
    return json.loads(recv_exact(sock, length))


def request(socket_path, payload, timeout=30.0):
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(timeout)
    sock.connect(socket_path)
    try:
        send_frame(sock, payload)
        return recv_frame(sock)
    finally:
        sock.close()


def client_main(socket_path, tool, argv):
    reply = request(socket_path, {"tool": tool, "argv": argv, "ppid": os.getppid(), "pid": os.getpid(),
                                  "cwd": os.getcwd(), "env": {k: os.environ.get(k) for k in CALLER_ENV}})
    sys.stdout.buffer.write(os.fsencode(reply["stdout"]))
    sys.stdout.flush()
    sys.stderr.buffer.write(os.fsencode(reply["stderr"]))
    sys.stderr.flush()
    return reply["code"]


CALLER_ENV = ("TMUX", "TMUX_PANE", "HERDR_ENV", "HERDR_SESSION", "HERDR_SOCKET_PATH", "HERDR_PANE_ID")


class Reply(Exception):
    def __init__(self, code=0, stdout="", stderr=""):
        super().__init__(stderr)
        self.code, self.stdout, self.stderr = code, stdout, stderr


def fail(stderr, code=1):
    raise Reply(code, "", stderr if stderr.endswith("\n") else stderr + "\n")


# ---------------------------------------------------------------- panes

def keyname_bytes(name, herdr):
    """The bytes a named key sends; None when `name` is not a key name."""
    table = HERDR_KEYS if herdr else TMUX_KEYS
    if name in table:
        return table[name]
    lowered = name.lower()
    prefix = "ctrl+" if herdr else "c-"
    if lowered.startswith(prefix) and len(name) == len(prefix) + 1 and name[-1].isalpha():
        return bytes([ord(name[-1].lower()) - ord("a") + 1])
    if not herdr and re.fullmatch(r"0x[0-9a-fA-F]{2}", name):
        return bytes([int(name, 16)])
    return None


FKEYS = {1: b"\x1bOP", 2: b"\x1bOQ", 3: b"\x1bOR", 4: b"\x1bOS", 5: b"\x1b[15~", 6: b"\x1b[17~",
         7: b"\x1b[18~", 8: b"\x1b[19~", 9: b"\x1b[20~", 10: b"\x1b[21~", 11: b"\x1b[23~", 12: b"\x1b[24~"}
TMUX_KEYS = {"Enter": b"\r", "Escape": b"\x1b", "Space": b" ", "Tab": b"\t", "BSpace": b"\x7f",
             "Up": b"\x1b[A", "Down": b"\x1b[B", "Right": b"\x1b[C", "Left": b"\x1b[D", "BTab": b"\x1b[Z",
             **{f"F{n}": seq for n, seq in FKEYS.items()}}
HERDR_KEYS = {"enter": b"\r", "esc": b"\x1b", "space": b" ", "tab": b"\t", "backspace": b"\x7f",
              "up": b"\x1b[A", "down": b"\x1b[B", "right": b"\x1b[C", "left": b"\x1b[D", "shift+tab": b"\x1b[Z",
              "plus": b"+", "minus": b"-", "semicolon": b";", **{f"f{n}": seq for n, seq in FKEYS.items()}}


class Pane:
    def __init__(self, daemon, pane_id, rows, cols, report_pid=None, title=""):
        self.daemon = daemon
        self.pane_id = pane_id
        self.rows, self.cols = rows, cols
        self.screen = screen_model.Screen(rows, cols)
        self.title = title
        self.report_pid = report_pid
        self.proc_pid = None
        self.master = None
        self.cwd = None
        self.dead = False

    @property
    def pid(self):
        if self.report_pid is not None:
            return self.report_pid
        return self.proc_pid if self.proc_pid is not None else self.daemon.pid

    def start(self, argv, cwd, env):
        master, slave = os.openpty()
        set_winsize(slave, self.rows, self.cols)
        pid = os.fork()
        if pid == 0:
            try:
                os.close(master)
                os.setsid()
                fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
                for fd in (0, 1, 2):
                    os.dup2(slave, fd)
                if slave > 2:
                    os.close(slave)
                os.closerange(3, 65536)
                for sig in (signal.SIGPIPE, signal.SIGCHLD, signal.SIGHUP, signal.SIGINT, signal.SIGTERM,
                            signal.SIGQUIT):
                    signal.signal(sig, signal.SIG_DFL)
                signal.pthread_sigmask(signal.SIG_SETMASK, [])
                os.chdir(cwd)
                os.execvpe(argv[0], argv, env)
            except BaseException as error:  # noqa: BLE001 - the forked child must never return
                os.write(2, f"fakemux exec {argv!r} failed: {error}\n".encode())
            finally:
                os._exit(127)
        os.close(slave)
        os.set_blocking(master, False)
        self.master, self.proc_pid, self.cwd = master, pid, cwd
        self.daemon.register_master(self)

    def resize(self, rows, cols):
        if (rows, cols) == (self.rows, self.cols):
            return
        self.rows, self.cols = rows, cols
        self.screen.resize(rows, cols)
        if self.master is not None:
            set_winsize(self.master, rows, cols)

    def write(self, data):
        if self.master is None:
            return
        view = memoryview(data)
        deadline = time.monotonic() + 5
        while view:
            try:
                n = os.write(self.master, view)
                view = view[n:]
            except BlockingIOError:
                if time.monotonic() > deadline:
                    fail(f"pane {self.pane_id} input is not draining")
                self.drain()
                time.sleep(0.001)

    def drain(self):
        if self.master is None:
            return
        while True:
            try:
                data = os.read(self.master, 65536)
            except BlockingIOError:
                return
            except OSError as error:
                if error.errno == errno.EIO:
                    data = b""
                else:
                    raise
            if not data:
                self.daemon.close_master(self)
                return
            self.screen.feed(data)

    def kill(self):
        if self.proc_pid is not None:
            try:
                os.killpg(self.proc_pid, signal.SIGHUP)
            # darwin reports EPERM instead of ESRCH for a group whose members have all exited
            # but are not yet reaped; tmux closes such a pane successfully.
            except (ProcessLookupError, PermissionError):
                pass
        if self.master is not None:
            self.daemon.close_master(self)

    # rendering ----------------------------------------------------

    def capture(self, ansi, keep_trailing, start):
        lines = self.screen.visible_lines(ansi, keep_trailing)
        if start is not None and start < 0:
            history = self.screen.history_lines(ansi, keep_trailing)
            lines = history[max(0, len(history) + start):] + lines
        return "".join(line + "\n" for line in lines)


def set_winsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


# ---------------------------------------------------------------- tmux model

class TLeaf:
    def __init__(self, pane):
        self.pane = pane
        self.parent = None


class TSplit:
    """axis 'h' lays children left to right, 'v' top to bottom; weights are relative sizes."""

    def __init__(self, axis, children, weights):
        self.axis = axis
        self.children = children
        self.weights = weights
        self.parent = None
        for child in children:
            child.parent = self


def allocate(total, weights):
    available = total - (len(weights) - 1)
    whole = sum(weights)
    sizes = [max(1, int(available * w / whole)) for w in weights]
    sizes[-1] = max(1, available - sum(sizes[:-1]))
    return sizes


class TmuxWindow:
    def __init__(self, window_id, name, index, rows, cols, root_pane):
        self.window_id, self.name, self.index = window_id, name, index
        self.rows, self.cols = rows, cols
        self.root = TLeaf(root_pane)
        self.active = root_pane

    def leaves(self, node=None):
        node = node or self.root
        if isinstance(node, TLeaf):
            return [node]
        return [leaf for child in node.children for leaf in self.leaves(child)]

    def panes(self):
        return [leaf.pane for leaf in self.leaves()]

    def find(self, pane):
        return next(leaf for leaf in self.leaves() if leaf.pane is pane)

    def geometry(self):
        out = {}

        def walk(node, left, top, width, height):
            if isinstance(node, TLeaf):
                out[node.pane.pane_id] = (left, top, width, height)
                return
            sizes = allocate(width if node.axis == "h" else height, node.weights)
            offset = left if node.axis == "h" else top
            for child, size in zip(node.children, sizes):
                if node.axis == "h":
                    walk(child, offset, top, size, height)
                else:
                    walk(child, left, offset, width, size)
                offset += size + 1

        walk(self.root, 0, 0, self.cols, self.rows)
        return out

    def apply_geometry(self):
        for leaf in self.leaves():
            _, _, width, height = self.geometry()[leaf.pane.pane_id]
            leaf.pane.resize(height, width)

    def replace(self, old, new):
        parent = old.parent
        new.parent = parent
        if parent is None:
            self.root = new
        else:
            parent.children[parent.children.index(old)] = new

    def split(self, target, pane, axis, before, full, percent):
        new_leaf = TLeaf(pane)
        if full:
            root = self.root
            if isinstance(root, TSplit) and root.axis == axis:
                share = sum(root.weights) * percent / (100 - percent)
                if before:
                    root.children.insert(0, new_leaf)
                    root.weights.insert(0, share)
                else:
                    root.children.append(new_leaf)
                    root.weights.append(share)
                new_leaf.parent = root
            else:
                children = [new_leaf, root] if before else [root, new_leaf]
                weights = [percent, 100 - percent] if before else [100 - percent, percent]
                self.root = TSplit(axis, children, weights)
        else:
            leaf = self.find(target)
            parent = leaf.parent
            if isinstance(parent, TSplit) and parent.axis == axis:
                k = parent.children.index(leaf)
                half = parent.weights[k] / 2
                parent.weights[k] = half
                at = k if before else k + 1
                parent.children.insert(at, new_leaf)
                parent.weights.insert(at, half)
                new_leaf.parent = parent
            else:
                split = TSplit(axis, [], [1, 1])
                split.parent = parent
                if parent is None:
                    self.root = split
                else:
                    parent.children[parent.children.index(leaf)] = split
                split.children = [new_leaf, leaf] if before else [leaf, new_leaf]
                for child in split.children:
                    child.parent = split
        self.apply_geometry()

    def remove(self, pane):
        leaf = self.find(pane)
        parent = leaf.parent
        if parent is None:
            self.root = None
            return
        k = parent.children.index(leaf)
        weight = parent.weights.pop(k)
        parent.children.pop(k)
        neighbour = k - 1 if k > 0 else 0
        parent.weights[neighbour] += weight
        if len(parent.children) == 1:
            self.replace(parent, parent.children[0])
        if self.active is pane:
            self.active = self.panes()[0]
        self.apply_geometry()

    def spread(self, pane):
        parent = self.find(pane).parent
        if parent is not None:
            parent.weights = [1] * len(parent.children)
            self.apply_geometry()


class TmuxServer:
    def __init__(self, daemon, socket_path, config):
        self.daemon = daemon
        self.socket_path = socket_path
        self.sessions = {}
        self.windows = []
        self.next_window = 0
        self.rows = config.get("rows", 39)
        self.cols = config.get("cols", 241)
        for session in config.get("sessions", []):
            self.sessions[session["name"]] = []
            for window in session["windows"]:
                pane = self.daemon.new_pane("tmux", self.rows, self.cols, window.get("report_pid"))
                self.add_window(session["name"], window.get("name", ""), pane)
                pane.title = window.get("title", "")
                command = window.get("command")
                if command:
                    pane.start(command, window.get("cwd", daemon.config["world"]), self.pane_env(pane, session["name"]))

    def pane_env(self, pane, session):
        env = dict(self.daemon.config["pane_env"])
        env["TMUX"] = f"{self.socket_path},{self.daemon.pid},0"
        env["TMUX_PANE"] = pane.pane_id
        env["TERM"] = "tmux-256color"
        return env

    def add_window(self, session, name, pane):
        window = TmuxWindow(f"@{self.next_window}", name, len(self.sessions[session]), self.rows, self.cols, pane)
        window.session = session
        self.next_window += 1
        self.sessions[session].append(window)
        self.windows.append(window)
        window.apply_geometry()
        return window

    # lookup -------------------------------------------------------

    def all_panes(self):
        return [(window, pane) for window in self.windows for pane in window.panes()]

    def pane_by_id(self, pane_id):
        return next(((w, p) for w, p in self.all_panes() if p.pane_id == pane_id), (None, None))

    def resolve(self, target, caller_pane=None):
        """(session, window, pane) for a target, or None when nothing matches."""
        if target is None:
            if caller_pane:
                window, pane = self.pane_by_id(caller_pane)
                if pane:
                    return window.session, window, pane
            if not self.windows:
                return None
            window = self.windows[0]
            return window.session, window, window.active
        if target.startswith("%"):
            window, pane = self.pane_by_id(target)
            return (window.session, window, pane) if pane else None
        if target.startswith("@"):
            window = next((w for w in self.windows if w.window_id == target), None)
            return (window.session, window, window.active) if window else None
        session, _, rest = target.partition(":")
        if session in self.sessions and self.sessions[session]:
            windows = self.sessions[session]
            window = windows[0]
            if rest:
                window = next((w for w in windows if w.name == rest or str(w.index) == rest), None)
                if window is None:
                    return None
            return session, window, window.active
        return None

    def expand(self, fmt, resolved):
        session, window, pane = resolved if resolved else (None, None, None)
        geometry = window.geometry() if window else {}
        left, top, width, height = geometry.get(pane.pane_id, (0, 0, 0, 0)) if pane else (0, 0, 0, 0)
        g = pane.screen.grid if pane else None
        values = {
            "session_name": session or "",
            "window_id": window.window_id if window else "",
            "window_name": window.name if window else "",
            "window_index": str(window.index) if window else "",
            "window_height": str(window.rows) if window else "",
            "window_width": str(window.cols) if window else "",
            "pane_id": pane.pane_id if pane else "",
            "pane_pid": str(pane.pid) if pane else "",
            "pane_title": pane.title if pane else "",
            "pane_left": str(left) if pane else "",
            "pane_top": str(top) if pane else "",
            "pane_width": str(width) if pane else "",
            "pane_height": str(height) if pane else "",
            "cursor_x": str(g.cursor[1]) if pane else "",
            "cursor_y": str(g.cursor[0]) if pane else "",
            "alternate_on": ("1" if pane.screen.alternate is not None else "0") if pane else "",
            "socket_path": self.socket_path,
            "pane_dead": ("1" if pane.dead else "0") if pane else "",
        }
        return re.sub(r"#\{([a-z_]+)\}", lambda m: values.get(m.group(1), ""), fmt)

    # commands -----------------------------------------------------

    def run(self, args, caller):
        if not args:
            fail("usage: tmux [-S socket-path] command")
        command, rest = args[0], args[1:]
        handler = getattr(self, "cmd_" + command.replace("-", "_"), None)
        if handler is None:
            fail(f"unknown command: {command}")
        return handler(parse_opts(rest, TMUX_VALUED_OPTIONS.get(command, "")), caller)

    def need_pane(self, target, caller):
        resolved = self.resolve(target, caller.get("env", {}).get("TMUX_PANE"))
        if resolved is None:
            fail(f"can't find pane: {target}")
        return resolved

    def cmd_display_message(self, opts, caller):
        resolved = self.resolve(opts.get("t"), caller.get("env", {}).get("TMUX_PANE"))
        fmt = opts["args"][0] if opts["args"] else ""
        return self.expand(fmt, resolved) + "\n"

    def cmd_rename_window(self, opts, caller):
        resolved = self.resolve(opts.get("t"))
        if resolved is None:
            fail(f"can't find window: {opts.get('t')}")
        resolved[1].name = opts["args"][0] if opts["args"] else ""
        return ""

    def cmd_select_pane(self, opts, caller):
        _, _, pane = self.need_pane(opts.get("t"), caller)
        if "T" in opts:
            pane.title = opts["T"]
        return ""

    def cmd_list_panes(self, opts, caller):
        target = opts.get("t")
        resolved = self.resolve(target)
        if resolved is None:
            fail(f"can't find window: {target}")
        window = resolved[1]
        fmt = opts.get("F", "#{pane_id}")
        return "".join(self.expand(fmt, (window.session, window, pane)) + "\n" for pane in window.panes())

    def cmd_select_layout(self, opts, caller):
        _, window, pane = self.need_pane(opts.get("t"), caller)
        if "E" in opts:
            window.spread(pane)
        return ""

    def cmd_kill_pane(self, opts, caller):
        _, window, pane = self.need_pane(opts.get("t"), caller)
        self.close(window, pane)
        return ""

    def close(self, window, pane):
        pane.kill()
        window.remove(pane)
        if window.root is None:
            self.windows.remove(window)
            self.sessions[window.session].remove(window)

    def spawn(self, pane, window, opts, caller):
        command = opts["args"]
        cwd = opts.get("c") or caller.get("cwd") or self.daemon.config["world"]
        if not command:
            command = self.daemon.config.get("default_command", ["/bin/sh"])
        elif len(command) == 1:
            command = ["/bin/sh", "-c", command[0]]
        pane.start(command, cwd, self.pane_env(pane, window.session))

    def cmd_split_window(self, opts, caller):
        _, window, target = self.need_pane(opts.get("t"), caller)
        axis = "h" if "h" in opts else "v"
        percent = 50
        if "l" in opts and opts["l"].endswith("%"):
            percent = int(opts["l"][:-1])
        pane = self.daemon.new_pane("tmux", 1, 1)
        window.split(target, pane, axis, "b" in opts, "f" in opts, percent)
        self.spawn(pane, window, opts, caller)
        if "d" not in opts:
            window.active = pane
        fmt = opts.get("F", "#{session_name}:#{window_index}.#{pane_index}")
        return self.expand(fmt, (window.session, window, pane)) + "\n" if "P" in opts else ""

    def cmd_new_window(self, opts, caller):
        target = opts.get("t", "")
        session = target.split(":")[0] if target else next(iter(self.sessions))
        if session not in self.sessions:
            fail(f"can't find session: {session}")
        pane = self.daemon.new_pane("tmux", self.rows, self.cols)
        window = self.add_window(session, opts.get("n", ""), pane)
        self.spawn(pane, window, opts, caller)
        fmt = opts.get("F", "#{session_name}:#{window_index}.#{pane_index}")
        return self.expand(fmt, (session, window, pane)) + "\n" if "P" in opts else ""

    def cmd_send_keys(self, opts, caller):
        _, _, pane = self.need_pane(opts.get("t"), caller)
        data = b""
        if "l" in opts:
            data = "".join(opts["args"]).encode()
        elif "H" in opts:
            data = bytes(int(token, 16) for token in opts["args"])
        else:
            for token in opts["args"]:
                key = keyname_bytes(token, herdr=False)
                data += key if key is not None else token.encode()
        pane.write(data)
        return ""

    def cmd_capture_pane(self, opts, caller):
        _, _, pane = self.need_pane(opts.get("t"), caller)
        pane.drain()
        start = int(opts["S"]) if "S" in opts and opts["S"] not in ("-",) else None
        return pane.capture("e" in opts, "N" in opts, start) if "p" in opts else ""

    def cmd_resize_pane(self, opts, caller):
        return ""

    def cmd_resize_window(self, opts, caller):
        return ""

    def cmd_kill_server(self, opts, caller):
        for window in list(self.windows):
            for pane in window.panes():
                self.close(window, pane)
        return ""

    def cmd_list_sessions(self, opts, caller):
        fmt = opts.get("F", "#{session_name}")
        return "".join(self.expand(fmt, (name, windows[0], windows[0].active)) + "\n"
                       for name, windows in self.sessions.items() if windows)


TMUX_VALUED_OPTIONS = {
    "display-message": "tcF",
    "rename-window": "t",
    "select-pane": "tT",
    "list-panes": "tF",
    "select-layout": "t",
    "kill-pane": "t",
    "split-window": "tlFce",
    "new-window": "tnFce",
    "send-keys": "tN",
    "capture-pane": "tSEb",
    "resize-pane": "txy",
    "resize-window": "txy",
    "list-sessions": "F",
}


def parse_opts(args, valued):
    opts = {"args": []}
    k = 0
    while k < len(args):
        token = args[k]
        if token == "--":
            opts["args"].extend(args[k + 1:])
            break
        if token.startswith("-") and len(token) > 1 and not opts["args"] and not re.fullmatch(r"-\d+", token):
            letters = token[1:]
            j = 0
            while j < len(letters):
                letter = letters[j]
                if letter in valued:
                    value = letters[j + 1:] or (args[k + 1] if k + 1 < len(args) else "")
                    if not letters[j + 1:]:
                        k += 1
                    opts[letter] = value
                    break
                opts[letter] = True
                j += 1
        else:
            opts["args"].append(token)
        k += 1
    return opts


# ---------------------------------------------------------------- herdr model

class HLeaf:
    def __init__(self, pane):
        self.pane = pane
        self.parent = None


class HSplit:
    """direction 'right' puts `first` left of `second`; 'down' puts it above."""

    def __init__(self, direction, ratio, first, second):
        self.direction, self.ratio = direction, ratio
        self.first, self.second = first, second
        self.parent = None
        first.parent = self
        second.parent = self


class HerdrTab:
    def __init__(self, tab_id, workspace, label, root_pane, rows, cols):
        self.tab_id, self.workspace, self.label = tab_id, workspace, label
        self.root = HLeaf(root_pane)
        self.rows, self.cols = rows, cols

    def leaves(self, node="root"):
        node = self.root if node == "root" else node
        if node is None:
            return []
        if isinstance(node, HLeaf):
            return [node]
        return self.leaves(node.first) + self.leaves(node.second)

    def panes(self):
        return [leaf.pane for leaf in self.leaves()]

    def find(self, pane):
        return next(leaf for leaf in self.leaves() if leaf.pane is pane)

    def layout(self):
        panes, splits = [], []

        def walk(node, x, y, w, h):
            if isinstance(node, HLeaf):
                panes.append((node.pane, {"x": x, "y": y, "width": w, "height": h}))
                return
            splits.append((node, {"x": x, "y": y, "width": w, "height": h}))
            if node.direction == "right":
                first = min(w - 1, max(1, int(round(w * node.ratio))))
                walk(node.first, x, y, first, h)
                walk(node.second, x + first, y, w - first, h)
            else:
                first = min(h - 1, max(1, int(round(h * node.ratio))))
                walk(node.first, x, y, w, first)
                walk(node.second, x, y + first, w, h - first)

        if self.root is not None:
            walk(self.root, 0, 0, self.cols, self.rows)
        return panes, splits

    def apply_geometry(self):
        for pane, rect in self.layout()[0]:
            pane.resize(rect["height"], rect["width"])

    def replace(self, old, new):
        parent = old.parent
        new.parent = parent
        if parent is None:
            self.root = new
        elif parent.first is old:
            parent.first = new
        else:
            parent.second = new

    def insert(self, target, pane, direction):
        leaf = self.find(target)
        parent = leaf.parent
        split = HSplit(direction, 0.5, HLeaf(target), HLeaf(pane))
        split.parent = parent
        if parent is None:
            self.root = split
        elif parent.first is leaf:
            parent.first = split
        else:
            parent.second = split
        self.apply_geometry()

    def detach(self, pane):
        leaf = self.find(pane)
        parent = leaf.parent
        if parent is None:
            self.root = None
            return
        sibling = parent.second if parent.first is leaf else parent.first
        self.replace(parent, sibling)
        self.apply_geometry()

    def resize(self, pane, direction, amount):
        node = self.find(pane)
        axis = "down" if direction in ("down", "up") else "right"
        grow_first = direction in ("down", "right")
        while node.parent is not None:
            parent = node.parent
            in_first = parent.first is node
            if parent.direction == axis and in_first == grow_first:
                ratio = parent.ratio + amount if grow_first else parent.ratio - amount
                parent.ratio = round(min(0.95, max(0.05, ratio)), 4)
                self.apply_geometry()
                return
            node = parent


class HerdrServer:
    def __init__(self, daemon, name, config):
        self.daemon = daemon
        self.name = name
        self.socket_path = config.get("socket_path", f"{daemon.config['world']}/herdr-{name}.sock")
        self.rows = config.get("rows", 39)
        self.cols = config.get("cols", 241)
        self.workspaces = {}
        self.tabs = []
        self.next_tab = 1
        self.focused = None
        for workspace in config.get("workspaces", []):
            self.workspaces[workspace["id"]] = workspace.get("label", workspace["id"])
            for tab in workspace["tabs"]:
                pane = self.new_pane(workspace["id"], tab.get("report_pid"))
                self.add_tab(workspace["id"], tab.get("label", ""), pane)
                pane.title = tab.get("title", "")
                if tab.get("command"):
                    pane.start(tab["command"], tab.get("cwd", daemon.config["world"]), self.pane_env(pane))
                self.focused = self.focused or pane

    def new_pane(self, workspace, report_pid=None):
        pane = self.daemon.new_pane("herdr", self.rows, self.cols, report_pid, workspace=workspace)
        return pane

    def pane_env(self, pane):
        env = dict(self.daemon.config["pane_env"])
        env.update({"HERDR_ENV": "1", "HERDR_PANE_ID": pane.pane_id, "HERDR_SOCKET_PATH": self.socket_path,
                    "TERM": "xterm-256color"})
        return env

    def add_tab(self, workspace, label, pane):
        tab = HerdrTab(f"{workspace}:t{self.next_tab}", workspace, label, pane, self.rows, self.cols)
        self.next_tab += 1
        self.tabs.append(tab)
        tab.apply_geometry()
        return tab

    def all_panes(self):
        return [(tab, pane) for tab in self.tabs for pane in tab.panes()]

    def pane(self, pane_id):
        for tab, pane in self.all_panes():
            if pane.pane_id == pane_id:
                return tab, pane
        raise Reply(1, "", json.dumps({"error": {"code": "pane_not_found", "message": f"pane {pane_id} not found"}}) + "\n")

    def tab(self, tab_id):
        tab = next((t for t in self.tabs if t.tab_id == tab_id), None)
        if tab is None:
            raise Reply(1, "", json.dumps({"error": {"code": "tab_not_found", "message": f"tab {tab_id} not found"}}) + "\n")
        return tab

    def pane_json(self, tab, pane):
        return {"pane_id": pane.pane_id, "workspace_id": tab.workspace, "tab_id": tab.tab_id,
                "label": pane.title, "focused": pane is self.focused}

    def run(self, args, caller):
        words = [a for a in args]
        if not words:
            fail("usage: herdr <command>")
        if words[0] == "api" and words[1:2] == ["snapshot"]:
            return envelope({"snapshot": self.snapshot()})
        group, verb, rest = words[0], (words[1] if len(words) > 1 else ""), words[2:]
        handler = getattr(self, f"cmd_{group}_{verb.replace('-', '_')}", None)
        if handler is None:
            fail(f"unknown herdr command: {' '.join(words[:2])}")
        return handler(parse_long(rest), caller)

    def snapshot(self):
        layouts, panes = [], []
        for tab in self.tabs:
            placed, _ = tab.layout()
            layouts.append({"tab_id": tab.tab_id, "panes": [{"pane_id": p.pane_id, "rect": r} for p, r in placed]})
            panes += [{"pane_id": p.pane_id, "scroll": {"viewport_rows": r["height"]}} for p, r in placed]
        return {"layouts": layouts, "panes": panes}

    def cmd_pane_current(self, opts, caller):
        if "pane" in opts:
            tab, pane = self.pane(opts["pane"])
        else:
            env_pane = caller.get("env", {}).get("HERDR_PANE_ID")
            tab, pane = self.pane(env_pane) if env_pane else self.pane(self.focused.pane_id)
        return envelope({"pane": self.pane_json(tab, pane)})

    def cmd_pane_process_info(self, opts, caller):
        tab, pane = self.pane(opts["pane"])
        return envelope({"process_info": {"pane_id": pane.pane_id, "shell_pid": pane.pid}})

    def cmd_pane_list(self, opts, caller):
        return envelope({"panes": [self.pane_json(tab, pane) for tab, pane in self.all_panes()]})

    def cmd_pane_layout(self, opts, caller):
        tab, pane = self.pane(opts["pane"])
        placed, splits = tab.layout()
        return envelope({"layout": {
            "tab_id": tab.tab_id,
            "area": {"x": 0, "y": 0, "width": tab.cols, "height": tab.rows},
            "panes": [{"pane_id": p.pane_id, "rect": r} for p, r in placed],
            "splits": [{"direction": s.direction, "ratio": s.ratio, "rect": r} for s, r in splits],
        }})

    def cmd_pane_split(self, opts, caller):
        tab, target = self.pane(opts["args"][0])
        pane = self.new_pane(tab.workspace)
        pane.cwd = opts.get("cwd")
        tab.insert(target, pane, opts.get("direction", "right"))
        return envelope({"pane": self.pane_json(tab, pane)})

    def cmd_tab_create(self, opts, caller):
        workspace = opts.get("workspace")
        if workspace not in self.workspaces:
            raise Reply(1, "", json.dumps({"error": {"code": "workspace_not_found", "message": f"workspace {workspace} not found"}}) + "\n")
        pane = self.new_pane(workspace)
        pane.cwd = opts.get("cwd")
        tab = self.add_tab(workspace, opts.get("label", ""), pane)
        return envelope({"tab": {"tab_id": tab.tab_id, "label": tab.label, "workspace_id": workspace},
                         "root_pane": self.pane_json(tab, pane)})

    def cmd_tab_rename(self, opts, caller):
        tab = self.tab(opts["args"][0])
        tab.label = opts["args"][1] if len(opts["args"]) > 1 else ""
        return envelope({"tab": {"tab_id": tab.tab_id, "label": tab.label}})

    def cmd_tab_get(self, opts, caller):
        tab = self.tab(opts["args"][0])
        return envelope({"tab": {"tab_id": tab.tab_id, "label": tab.label, "workspace_id": tab.workspace}})

    def cmd_pane_run(self, opts, caller):
        tab, pane = self.pane(opts["args"][0])
        command = opts["args"][1] if len(opts["args"]) > 1 else ""
        pane.start(["/bin/sh", "-c", command], pane.cwd or self.daemon.config["world"], self.pane_env(pane))
        return envelope({"pane_id": pane.pane_id})

    def cmd_pane_rename(self, opts, caller):
        tab, pane = self.pane(opts["args"][0])
        pane.title = opts["args"][1] if len(opts["args"]) > 1 else ""
        return envelope({"pane": self.pane_json(tab, pane)})

    def cmd_pane_close(self, opts, caller):
        tab, pane = self.pane(opts["args"][0])
        self.close(tab, pane)
        return envelope({"closed": pane.pane_id})

    def close(self, tab, pane):
        pane.kill()
        tab.detach(pane)
        if tab.root is None:
            self.tabs.remove(tab)
        if self.focused is pane:
            self.focused = self.all_panes()[0][1] if self.all_panes() else None

    def cmd_pane_move(self, opts, caller):
        tab, pane = self.pane(opts["args"][0])
        source = tab
        if "new-tab" in opts:
            source.detach(pane)
            if source.root is None:
                self.tabs.remove(source)
            created = self.add_tab(source.workspace, "", pane)
            return envelope({"move_result": {"changed": True, "created_tab": {"tab_id": created.tab_id}}})
        destination = self.tab(opts["tab"])
        _, target = self.pane(opts["target-pane"])
        source.detach(pane)
        if source.root is None:
            self.tabs.remove(source)
        destination.insert(target, pane, opts.get("split", "right"))
        return envelope({"move_result": {"changed": True}})

    def cmd_pane_resize(self, opts, caller):
        tab, pane = self.pane(opts["pane"])
        tab.resize(pane, opts["direction"], float(opts["amount"]))
        return envelope({"pane_id": pane.pane_id})

    def cmd_pane_read(self, opts, caller):
        tab, pane = self.pane(opts["args"][0])
        pane.drain()
        ansi = opts.get("format") == "ansi"
        if opts.get("source") == "visible":
            lines = pane.screen.visible_lines(ansi)
            while lines and not lines[-1].strip():
                lines.pop()
        else:
            lines = pane.screen.unwrapped_lines(ansi)
            if "lines" in opts:
                lines = lines[-int(opts["lines"]):]
        return "".join(line + "\n" for line in lines)

    def cmd_pane_send_text(self, opts, caller):
        tab, pane = self.pane(opts["args"][0])
        pane.write(opts["args"][1].encode() if len(opts["args"]) > 1 else b"")
        return ""

    def cmd_pane_send_keys(self, opts, caller):
        tab, pane = self.pane(opts["args"][0])
        data = b""
        for token in opts["args"][1:]:
            key = keyname_bytes(token, herdr=True)
            if key is None and len(token) == 1:
                key = token.encode()
            if key is None:
                raise Reply(1, "", json.dumps({"error": {"code": "invalid_key", "message": f"unknown key {token}"}}) + "\n")
            data += key
        pane.write(data)
        return ""


def envelope(result):
    return json.dumps({"result": result}) + "\n"


HERDR_BOOLEAN = {"no-focus", "new-tab", "json", "current"}


def parse_long(args):
    opts = {"args": []}
    k = 0
    while k < len(args):
        token = args[k]
        if token.startswith("--") and len(token) > 2:
            name = token[2:]
            if name in HERDR_BOOLEAN:
                opts[name] = True
            else:
                opts[name] = args[k + 1] if k + 1 < len(args) else ""
                k += 1
        else:
            opts["args"].append(token)
        k += 1
    return opts


# ---------------------------------------------------------------- daemon

class Daemon:
    def __init__(self, config):
        self.config = config
        self.pid = os.getpid()
        self.selector = selectors.DefaultSelector()
        self.counters = {"tmux": 0, "herdr": {}}
        self.masters = {}
        self.log = open(config["log"], "a", buffering=1)
        self.seq = 0
        self.faults = config.get("faults", [])
        self.tmux = {}
        self.herdr = {}
        for server in config.get("tmux", []):
            self.tmux[server["socket"]] = TmuxServer(self, server["socket"], server)
        for server in config.get("herdr", []):
            self.herdr[server["name"]] = HerdrServer(self, server["name"], server)
        self.default_tmux = config.get("default_tmux")

    def new_pane(self, tool, rows, cols, report_pid=None, workspace=None):
        if tool == "tmux":
            pane_id = f"%{self.counters['tmux']}"
            self.counters["tmux"] += 1
        else:
            n = self.counters["herdr"].get(workspace, 0) + 1
            self.counters["herdr"][workspace] = n
            pane_id = f"{workspace}:p{n}"
        return Pane(self, pane_id, rows, cols, report_pid)

    def herdr_sessions(self):
        return [{"name": name, "socket_path": server.socket_path} for name, server in self.herdr.items()]

    def register_master(self, pane):
        self.masters[pane.master] = pane
        self.selector.register(pane.master, selectors.EVENT_READ, ("pane", pane))

    def close_master(self, pane):
        if pane.master is None:
            return
        self.selector.unregister(pane.master)
        del self.masters[pane.master]
        os.close(pane.master)
        pane.master = None

    def reap(self):
        while True:
            try:
                pid, status = os.waitpid(-1, os.WNOHANG)
            except ChildProcessError:
                return
            if pid == 0:
                return
            self.exited(pid)

    def exited(self, pid):
        """A pane whose process exits stays in place as a dead pane, as tmux's `remain-on-exit on`
        keeps it, until `kill-pane` closes it. Closing it on exit would race every `kill-pane`
        against the runner's own exit."""
        for server in self.tmux.values():
            for window, pane in server.all_panes():
                if pane.proc_pid == pid:
                    pane.drain()
                    pane.dead = True
                    self.close_master(pane)
                    return
        for server in self.herdr.values():
            for tab, pane in server.all_panes():
                if pane.proc_pid == pid:
                    pane.drain()
                    self.close_master(pane)
                    pane.proc_pid = None

    def server_for(self, tool, argv):
        if tool == "tmux":
            socket_path = None
            if len(argv) >= 2 and argv[0] == "-S":
                socket_path, argv = argv[1], argv[2:]
            elif argv[:1] == ["-f"]:
                argv = argv[2:]
            if socket_path is None:
                socket_path = self.default_tmux
                if socket_path is None:
                    fail(f"no server running on {self.config.get('default_tmux_path', '/tmp/tmux-0/default')}")
            server = self.tmux.get(socket_path)
            if server is None:
                fail(f"no server running on {socket_path}")
            return server, argv
        name = "default"
        if len(argv) >= 2 and argv[0] == "--session":
            name, argv = argv[1], argv[2:]
        if argv[:2] == ["session", "list"]:
            return HerdrSessions(self), argv
        server = self.herdr.get(name)
        if server is None:
            raise Reply(1, "", json.dumps({"error": {"code": "session_not_found", "message": f"herdr session {name} is not running"}}) + "\n")
        return server, argv

    def fault(self, tool, argv):
        joined = " ".join(argv)
        for fault in self.faults:
            if fault["tool"] == tool and re.search(fault["match"], joined) and fault.get("times", 1) != 0:
                fault["times"] = fault.get("times", 1) - 1
                if fault.get("delay"):
                    time.sleep(fault["delay"])
                reply = fault.get("reply")
                if reply is not None:
                    return Reply(reply.get("code", 0), reply.get("stdout", ""), reply.get("stderr", ""))
        return None

    def handle(self, req):
        if req["tool"] == "control":
            return self.control(req)
        tool, argv = req["tool"], req["argv"]
        self.seq += 1
        entry = {"seq": self.seq, "tool": tool, "argv": argv, "ppid": req["ppid"]}
        try:
            injected = self.fault(tool, argv)
            if injected is not None:
                raise injected
            server, rest = self.server_for(tool, argv)
            out = server.run(rest, req)
            reply = {"code": 0, "stdout": out, "stderr": ""}
        except Reply as r:
            reply = {"code": r.code, "stdout": r.stdout, "stderr": r.stderr}
        entry["code"] = reply["code"]
        self.log.write(json.dumps(entry) + "\n")
        return reply

    def control(self, req):
        op = req["op"]
        if op == "ping":
            return {"ok": True}
        if op == "panes":
            panes = []
            for socket_path, server in self.tmux.items():
                for window, pane in server.all_panes():
                    panes.append({"tool": "tmux", "server": socket_path, "window": window.window_id,
                                  "pane": pane.pane_id, "pid": pane.proc_pid, "rows": pane.rows, "cols": pane.cols})
            for name, server in self.herdr.items():
                for tab, pane in server.all_panes():
                    panes.append({"tool": "herdr", "server": name, "window": tab.tab_id, "pane": pane.pane_id,
                                  "pid": pane.proc_pid, "rows": pane.rows, "cols": pane.cols})
            return {"panes": panes}
        if op == "screen":
            pane = self.find_pane(req["pane"])
            pane.drain()
            return {"lines": pane.screen.visible_lines(), "history": pane.screen.history_lines()}
        if op == "type":
            self.find_pane(req["pane"]).write(os.fsencode(req["data"]))
            return {"ok": True}
        if op == "stop":
            self.stopping = True
            return {"ok": True}
        raise ValueError(op)

    def find_pane(self, pane_id):
        for server in list(self.tmux.values()) + list(self.herdr.values()):
            for _, pane in server.all_panes():
                if pane.pane_id == pane_id:
                    return pane
        raise KeyError(pane_id)

    def shutdown(self):
        pids = [p.proc_pid for p in self.masters.values() if p.proc_pid]
        for server in self.tmux.values():
            pids += [p.proc_pid for _, p in server.all_panes() if p.proc_pid]
        for server in self.herdr.values():
            pids += [p.proc_pid for _, p in server.all_panes() if p.proc_pid]
        for sig in (signal.SIGHUP, signal.SIGTERM):
            for pid in set(pids):
                try:
                    os.killpg(pid, sig)
                except (ProcessLookupError, PermissionError):
                    pass
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                self.reap()
                if not any(alive(pid) for pid in set(pids)):
                    return
                time.sleep(0.05)
        for pid in set(pids):
            try:
                os.killpg(pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
        time.sleep(0.1)
        self.reap()

    def serve(self):
        path = self.config["socket"]
        if os.path.exists(path):
            os.unlink(path)
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        listener.bind(path)
        listener.listen(64)
        listener.setblocking(False)
        self.selector.register(listener, selectors.EVENT_READ, ("listen", None))
        self.stopping = False
        signal.signal(signal.SIGTERM, lambda *_: setattr(self, "stopping", True))
        ready_path = self.config.get("ready")
        if ready_path:
            with open(ready_path, "w") as f:
                f.write(str(self.pid))
        while not self.stopping:
            for key, _ in self.selector.select(timeout=0.05):
                kind, obj = key.data
                if kind == "listen":
                    try:
                        conn, _ = listener.accept()
                    except BlockingIOError:
                        continue
                    self.serve_connection(conn)
                else:
                    obj.drain()
            self.reap()
        self.shutdown()
        listener.close()
        os.unlink(path)

    def serve_connection(self, conn):
        conn.setblocking(True)
        conn.settimeout(10)
        try:
            req = recv_frame(conn)
            reply = self.handle(req)
            send_frame(conn, reply)
        except (ConnectionError, OSError, ValueError, KeyError) as error:
            try:
                send_frame(conn, {"code": 1, "stdout": "", "stderr": f"fakemux: {error!r}\n", "error": repr(error)})
            except OSError:
                pass
        finally:
            conn.close()


class HerdrSessions:
    def __init__(self, daemon):
        self.daemon = daemon

    def run(self, args, caller):
        return json.dumps({"sessions": self.daemon.herdr_sessions()}) + "\n"


def alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    try:
        done, _ = os.waitpid(pid, os.WNOHANG)
        return done == 0
    except ChildProcessError:
        return True


def main():
    mode = sys.argv[1]
    if mode == "serve":
        with open(sys.argv[2]) as f:
            config = json.load(f)
        os.setsid()
        Daemon(config).serve()
        return 0
    if mode == "client":
        return client_main(sys.argv[2], sys.argv[3], [os.fsdecode(a) for a in sys.argv[4:]])
    raise SystemExit(f"unknown mode {mode}")


if __name__ == "__main__":
    sys.exit(main())
