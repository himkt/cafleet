"""Stand-in native coding-agent executables (claude, codex, opencode, hermes, pi).

The world's PATH holds one generated wrapper per backend that calls `main(BACKEND, CONFIG)`.
Non-interactive invocations answer from the world's script: `--version` prints the configured
version line and `hermes config path` prints the configured path.

An interactive run is deterministic: it has no timers. It puts its terminal in raw mode, draws a
bundled native screen fixture and redraws only in response to the bytes it receives. Its
composer echoes typed and pasted text at the fixture's cursor, Enter submits the composer, and
the script's rules move between fixture screens on submitted text or keys. Every run logs its argv,
working directory and backend configuration variables, and appends every byte it receives to its
own `.in` file, which G2 compares.
"""
import fcntl
import json
import os
import re
import sys
import termios
import tty

CONFIG_ENV = ("CLAUDE_CONFIG_DIR", "CODEX_HOME", "OPENCODE_CONFIG_DIR", "HERMES_HOME", "PI_CODING_AGENT_DIR")
PASTE_START, PASTE_END = b"\x1b[200~", b"\x1b[201~"
KEY_BYTES = {"enter": b"\r", "esc": b"\x1b", "ctrl+c": b"\x03", "tab": b"\t", "shift+tab": b"\x1b[Z",
             "up": b"\x1b[A", "down": b"\x1b[B", "right": b"\x1b[C", "left": b"\x1b[D"}


def load_screens(repo, backend):
    with open(os.path.join(repo, "assets", "profiles", f"{backend}.json")) as f:
        return json.load(f)["screens"]


def next_ordinal(log_path):
    with open(log_path, "a+") as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        f.seek(0)
        return sum(1 for _ in f) + 1


def append_log(log_path, entry):
    with open(log_path, "a") as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        f.write(json.dumps(entry) + "\n")


class Native:
    def __init__(self, backend, script, screens, ordinal, input_path):
        self.backend = backend
        self.script = script
        self.screens = screens
        self.ordinal = ordinal
        self.input = open(input_path, "ab", buffering=0)
        self.state = script["start"]
        self.composer = ""
        self.pending = b""
        self.in_paste = False
        self.exit_code = None
        self.notes = []

    def size(self):
        try:
            rows, cols = struct_winsize()
            return rows, cols
        except OSError:
            return 24, 80

    def screen(self):
        return self.script["screens"][self.state]

    def draw(self):
        spec = self.screen()
        fixture = self.screens[spec["fixture"]]
        rows, cols = self.size()
        out = [b"\x1b[?2004h" if self.script.get("bracketed_paste", True) else b"\x1b[?2004l"]
        out.append(b"\x1b[?1049h" if fixture["alternate_screen"] else b"\x1b[?1049l")
        out.append(b"\x1b[0m\x1b[H\x1b[2J")
        styles = row_styles(fixture)
        cursor_row, cursor_col = fixture["cursor_row"], fixture["cursor_col"]
        rendered = [styled(clip(line, cols), styles.get(k, [])) for k, line in enumerate(fixture["lines"])]
        for k, note in enumerate(self.notes[-cursor_row:] if cursor_row else []):
            rendered[k] = clip(note, cols)
        if spec.get("composer") and not self.composer and "empty_line" in spec:
            rendered[cursor_row] = spec["empty_line"]
            cursor_col = display_width(re.sub(r"\x1b\[[0-9;]*m", "", spec["empty_line"]).rstrip())
        if spec.get("composer") and self.composer:
            composer_line = fixture["lines"][cursor_row]
            prefix = spec.get("prefix", clip(composer_line, cursor_col))
            continuation = spec.get("continuation", " " * display_width(prefix))
            wrapped = word_wrap(self.composer.replace("\n", " "), cols - display_width(prefix),
                                cols - display_width(continuation))
            composer_rows = [prefix + wrapped[0]] + [continuation + row for row in wrapped[1:]]
            rendered = rendered[:cursor_row] + composer_rows + rendered[cursor_row + 1:]
            cursor_row = cursor_row + len(composer_rows) - 1
            cursor_col = display_width(composer_rows[-1])
        overflow = max(0, len(rendered) - rows)
        if overflow and cursor_row >= overflow:
            rendered = rendered[overflow:]
            cursor_row -= overflow
        for k, line in enumerate(rendered[:rows]):
            out.append(f"\x1b[{k + 1};1H".encode() + line.encode())
        out.append(f"\x1b[{min(rows, cursor_row + 1)};{min(cols, cursor_col + 1)}H".encode())
        os.write(1, b"".join(out))

    def feed(self, data):
        self.input.write(data)
        data = self.pending + data
        self.pending = b""
        k = 0
        while k < len(data):
            if data.startswith(PASTE_START, k):
                self.in_paste = True
                k += len(PASTE_START)
                continue
            if data.startswith(PASTE_END, k):
                self.in_paste = False
                k += len(PASTE_END)
                continue
            if data[k] == 0x1B and len(data) - k < 6 and (PASTE_START.startswith(data[k:]) or PASTE_END.startswith(data[k:])):
                self.pending = data[k:]
                return
            if self.in_paste:
                end = data.find(PASTE_END, k)
                chunk = data[k:end if end >= 0 else len(data)]
                self.composer += chunk.decode("utf-8", "replace")
                k += len(chunk)
                continue
            if data[k] == 0x1B:
                sequence = escape_sequence(data, k)
                if sequence is None:
                    self.pending = data[k:]
                    return
                self.key(sequence)
                k += len(sequence)
                continue
            byte = data[k]
            if byte == 0x0D:
                self.submit()
            elif byte in (0x7F, 0x08):
                self.composer = self.composer[:-1]
            elif byte < 0x20:
                self.key(bytes([byte]))
            else:
                end = k
                while end < len(data) and data[end] >= 0x20 and data[end] != 0x7F and data[end] != 0x1B:
                    end += 1
                self.type_text(data[k:end].decode("utf-8", "replace"))
                k = end
                continue
            k += 1

    def type_text(self, text):
        rule = self.match("key", text)
        if rule is not None:
            self.apply(rule)
            return
        if self.screen().get("composer"):
            self.composer += text

    def key(self, sequence):
        name = next((n for n, b in KEY_BYTES.items() if b == sequence), sequence.hex())
        rule = self.match("key", name)
        if rule is not None:
            self.apply(rule)

    def submit(self):
        text = self.composer
        self.composer = ""
        rule = self.match("submit", text)
        if rule is None:
            rule = self.match("key", "enter")
        if rule is not None:
            self.apply(rule)

    def match(self, when, value):
        for rule in self.script.get("rules", []):
            if rule["when"] != when:
                continue
            if "state" in rule and rule["state"] != self.state:
                continue
            if re.search(rule["match"], value):
                return rule
        return None

    def apply(self, rule):
        if "goto" in rule:
            self.state = rule["goto"]
        if "say" in rule:
            self.notes.append(rule["say"])
        if "exit" in rule:
            if rule.get("print"):
                os.write(1, b"\x1b[?1049l\r\n\x1b[J" + rule["print"].encode() + b"\r\n")
            self.exit_code = rule["exit"]

    def run(self):
        self.draw()
        while self.exit_code is None:
            try:
                data = os.read(0, 65536)
            except InterruptedError:
                continue
            except OSError:
                break
            if not data:
                break
            self.feed(data)
            if self.exit_code is None:
                self.draw()
        return self.exit_code or 0


def escape_sequence(data, k):
    """The complete escape sequence starting at data[k], or None when it is still incomplete.

    A lone trailing ESC is the Escape key: writers send each key sequence in one write, and the
    stand-in has no timer to disambiguate it."""
    if k + 1 >= len(data):
        return data[k:k + 1]
    second = data[k + 1]
    if second == ord("["):
        j = k + 2
        while j < len(data) and not 0x40 <= data[j] <= 0x7E:
            j += 1
        return data[k:j + 1] if j < len(data) else None
    if second == ord("O"):
        return data[k:k + 3] if k + 2 < len(data) else None
    return data[k:k + 1]


def color(spec, base):
    if spec["kind"] == "indexed":
        return f"{base};5;{spec['index']}"
    return f"{base};2;{spec['r']};{spec['g']};{spec['b']}"


def row_styles(fixture):
    """Per row, (start_col, end_col, sgr) spans from the fixture's recorded cell styles."""
    spans = {}
    for style in fixture.get("cell_styles", []):
        codes = (["2"] if style["faint"] else []) + ([color(style["foreground"], 38)] if style["foreground"] else [])
        spans.setdefault(style["row"], []).append((style["start_col"], style["end_col"], codes))
    for background in fixture.get("cell_backgrounds", []):
        spans.setdefault(background["row"], []).append(
            (background["start_col"], background["end_col"], [color(background["bg"], 48)]))
    return spans


def styled(text, spans):
    if not spans:
        return text
    out, col, current = [], 0, None
    for ch in text:
        codes = [code for start, end, span in spans if start <= col < end for code in span]
        sgr = "\x1b[0;" + ";".join(codes) + "m" if codes else "\x1b[0m"
        if sgr != current:
            out.append(sgr)
            current = sgr
        out.append(ch)
        col += char_width(ch)
    out.append("\x1b[0m")
    return "".join(out)


def word_wrap(text, first_width, width):
    """Rows of `text`, breaking at the last space that fits (the space is dropped) or, in a word
    longer than the row, at the row width."""
    rows, limit = [], max(1, first_width)
    while display_width(text) > limit:
        head = clip(text, limit)
        cut = head.rfind(" ")
        if cut > 0:
            rows.append(head[:cut])
            text = text[cut + 1:]
        else:
            rows.append(head)
            text = text[len(head):]
        limit = max(1, width)
    rows.append(text)
    return rows


def clip(text, width):
    out, used = [], 0
    for ch in text:
        w = char_width(ch)
        if used + w > width:
            break
        out.append(ch)
        used += w
    return "".join(out)


def display_width(text):
    return sum(char_width(ch) for ch in text)


def char_width(ch):
    import unicodedata
    if unicodedata.combining(ch) or 0xFE00 <= ord(ch) <= 0xFE0F:
        return 0
    return 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1


def struct_winsize():
    import struct
    packed = fcntl.ioctl(1, termios.TIOCGWINSZ, b"\0" * 8)
    rows, cols, _, _ = struct.unpack("HHHH", packed)
    return rows, cols


def main(backend, script_name, config_path):
    with open(config_path) as f:
        config = json.load(f)
    script = config["natives"][script_name]
    argv = sys.argv[1:]
    log_dir = config["log_dir"]
    os.makedirs(log_dir, exist_ok=True)
    log_path = os.path.join(log_dir, f"{script_name}.log")
    env = {k: os.environ.get(k) for k in CONFIG_ENV}
    if argv == ["--version"]:
        append_log(log_path, {"kind": "version", "argv": argv, "env": env})
        sys.stdout.write(script["version_line"] + "\n")
        return script.get("version_exit", 0)
    if backend == "hermes" and argv[:2] == ["config", "path"]:
        append_log(log_path, {"kind": "config-path", "argv": argv, "env": env})
        home = os.environ.get("HERMES_HOME") or os.path.join(os.environ["HOME"], ".hermes")
        sys.stdout.write(script.get("config_path_line", os.path.join(home, "config.yaml")) + "\n")
        return script.get("config_path_exit", 0)
    ordinal = next_ordinal(log_path)
    append_log(log_path, {"kind": "interactive", "ordinal": ordinal, "argv": argv, "cwd": os.getcwd(), "env": env})
    screens = load_screens(config["repo"], backend)
    native = Native(backend, script, screens, ordinal, os.path.join(log_dir, f"{script_name}-{ordinal}.in"))
    saved = None
    if os.isatty(0):
        saved = termios.tcgetattr(0)
        tty.setraw(0)
    try:
        return native.run()
    finally:
        if saved is not None:
            try:
                termios.tcsetattr(0, termios.TCSANOW, saved)
            except termios.error:
                pass
