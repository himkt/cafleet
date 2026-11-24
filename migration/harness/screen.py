"""A small deterministic VT screen model for the stand-in multiplexer panes.

It renders what a pane shows for `tmux capture-pane` and `herdr pane read`. Both binaries under
test see the same model, so parity needs determinism, not xterm fidelity: the model keeps cells
with SGR attributes, a primary screen with scrollback, an alternate screen, a scroll region,
autowrap with pending wrap, the cursor and the bracketed-paste mode.
"""
import codecs
import unicodedata

HISTORY_LIMIT = 2000


def char_width(ch):
    code = ord(ch)
    if code < 0x20 or 0x7F <= code < 0xA0:
        return 0
    if unicodedata.combining(ch) or unicodedata.category(ch) in ("Mn", "Me", "Cf") or 0xFE00 <= code <= 0xFE0F:
        return 0
    if unicodedata.east_asian_width(ch) in ("W", "F"):
        return 2
    return 1


class Cell:
    __slots__ = ("text", "attrs", "wide", "cont")

    def __init__(self, text=" ", attrs=(), wide=False, cont=False):
        self.text = text
        self.attrs = attrs
        self.wide = wide
        self.cont = cont


def blank_row(cols, attrs=()):
    return [Cell(" ", attrs) for _ in range(cols)]


class Grid:
    def __init__(self, rows, cols):
        self.rows = rows
        self.cols = cols
        self.lines = [blank_row(cols) for _ in range(rows)]
        self.wrapped = [False] * rows
        self.history = []
        self.history_wrapped = []
        self.cursor = [0, 0]
        self.pending_wrap = False
        self.saved = (0, 0, ())
        self.top = 0
        self.bottom = rows - 1

    def resize(self, rows, cols):
        for k, line in enumerate(self.lines):
            if len(line) < cols:
                line.extend(blank_row(cols - len(line)))
            else:
                del line[cols:]
        while len(self.lines) > rows:
            if self.cursor[0] > 0:
                self.push_history(self.lines.pop(0), self.wrapped.pop(0))
                self.cursor[0] -= 1
            else:
                self.lines.pop()
                self.wrapped.pop()
        while len(self.lines) < rows:
            self.lines.append(blank_row(cols))
            self.wrapped.append(False)
        self.rows, self.cols = rows, cols
        self.top, self.bottom = 0, rows - 1
        self.cursor[0] = min(self.cursor[0], rows - 1)
        self.cursor[1] = min(self.cursor[1], cols - 1)
        self.pending_wrap = False

    def push_history(self, line, wrapped):
        self.history.append(line)
        self.history_wrapped.append(wrapped)
        if len(self.history) > HISTORY_LIMIT:
            del self.history[0]
            del self.history_wrapped[0]


class Screen:
    def __init__(self, rows, cols, keep_history=True):
        self.primary = Grid(rows, cols)
        self.alternate = None
        self.keep_history = keep_history
        self.attrs = ()
        self.bracketed_paste = False
        self.cursor_visible = True
        self.title = ""
        self.state = "ground"
        self.params = ""
        self.intermediate = ""
        self.osc = ""
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")

    @property
    def grid(self):
        return self.alternate if self.alternate is not None else self.primary

    @property
    def rows(self):
        return self.grid.rows

    @property
    def cols(self):
        return self.grid.cols

    def resize(self, rows, cols):
        self.primary.resize(rows, cols)
        if self.alternate is not None:
            self.alternate.resize(rows, cols)

    # ------------------------------------------------------------ input

    def feed(self, data):
        for ch in self.decoder.decode(data):
            self.step(ch)

    def step(self, ch):
        state = self.state
        if state == "ground":
            if ch == "\x1b":
                self.state = "escape"
                self.intermediate = ""
            elif ord(ch) < 0x20 or ch == "\x7f":
                self.control(ch)
            else:
                self.put(ch)
        elif state == "escape":
            if ch == "[":
                self.state, self.params, self.intermediate = "csi", "", ""
            elif ch == "]":
                self.state, self.osc = "osc", ""
            elif ch in "PX^_":
                self.state = "string"
            elif ch in "()*+#%":
                self.state = "charset"
            else:
                self.state = "ground"
                self.escape(ch)
        elif state == "charset":
            self.state = "ground"
        elif state == "csi":
            if "0" <= ch <= "?":
                self.params += ch
            elif " " <= ch <= "/":
                self.intermediate += ch
            elif "@" <= ch <= "~":
                self.state = "ground"
                self.csi(ch)
            elif ch == "\x1b":
                self.state = "escape"
            else:
                self.control(ch)
        elif state == "osc":
            if ch == "\x07":
                self.state = "ground"
                self.osc_done()
            elif ch == "\x1b":
                self.state = "osc_esc"
            else:
                self.osc += ch
        elif state == "osc_esc":
            self.state = "ground"
            if ch == "\\":
                self.osc_done()
            else:
                self.step(ch)
        elif state == "string":
            if ch == "\x1b":
                self.state = "string_esc"
            elif ch == "\x07":
                self.state = "ground"
        elif state == "string_esc":
            self.state = "ground" if ch == "\\" else "string"

    def osc_done(self):
        code, _, value = self.osc.partition(";")
        if code in ("0", "2"):
            self.title = value

    # ------------------------------------------------------------ primitives

    def put(self, ch):
        g = self.grid
        width = char_width(ch)
        if width == 0:
            row, col = g.cursor
            col = col - 1 if not g.pending_wrap else col
            if 0 <= col < g.cols:
                cell = g.lines[row][col]
                if cell.cont and col > 0:
                    cell = g.lines[row][col - 1]
                cell.text += ch
            return
        if g.pending_wrap:
            g.wrapped[g.cursor[0]] = True
            self.newline_raw()
            g.cursor[1] = 0
            g.pending_wrap = False
        row, col = g.cursor
        if width == 2 and col == g.cols - 1:
            g.lines[row][col] = Cell(" ", self.attrs)
            g.wrapped[row] = True
            self.newline_raw()
            g.cursor[1] = 0
            row, col = g.cursor
        g.lines[row][col] = Cell(ch, self.attrs, wide=width == 2)
        if width == 2:
            g.lines[row][col + 1] = Cell("", (), cont=True)
        col += width
        if col >= g.cols:
            g.cursor[1] = g.cols - 1
            g.pending_wrap = True
        else:
            g.cursor[1] = col

    def newline_raw(self):
        g = self.grid
        if g.cursor[0] == g.bottom:
            self.scroll_up(1)
        elif g.cursor[0] < g.rows - 1:
            g.cursor[0] += 1

    def scroll_up(self, n):
        g = self.grid
        for _ in range(n):
            line = g.lines.pop(g.top)
            wrapped = g.wrapped.pop(g.top)
            if g is self.primary and g.top == 0 and self.keep_history:
                g.push_history(line, wrapped)
            g.lines.insert(g.bottom, blank_row(g.cols, self.attrs))
            g.wrapped.insert(g.bottom, False)

    def scroll_down(self, n):
        g = self.grid
        for _ in range(n):
            del g.lines[g.bottom]
            del g.wrapped[g.bottom]
            g.lines.insert(g.top, blank_row(g.cols, self.attrs))
            g.wrapped.insert(g.top, False)

    def control(self, ch):
        g = self.grid
        if ch == "\r":
            g.cursor[1] = 0
            g.pending_wrap = False
        elif ch in "\n\x0b\x0c":
            self.newline_raw()
            g.pending_wrap = False
        elif ch == "\b":
            if g.pending_wrap:
                g.pending_wrap = False
            elif g.cursor[1] > 0:
                g.cursor[1] -= 1
        elif ch == "\t":
            g.cursor[1] = min(g.cols - 1, (g.cursor[1] // 8 + 1) * 8)
            g.pending_wrap = False

    def escape(self, ch):
        g = self.grid
        if ch == "7":
            g.saved = (g.cursor[0], g.cursor[1], self.attrs)
        elif ch == "8":
            g.cursor[0], g.cursor[1], self.attrs = g.saved
            g.pending_wrap = False
        elif ch == "M":
            if g.cursor[0] == g.top:
                self.scroll_down(1)
            elif g.cursor[0] > 0:
                g.cursor[0] -= 1
        elif ch == "D":
            self.newline_raw()
        elif ch == "E":
            self.newline_raw()
            g.cursor[1] = 0
        elif ch == "c":
            rows, cols = g.rows, g.cols
            self.__init__(rows, cols, self.keep_history)

    def numbers(self, default):
        parts = self.params.lstrip("?>=<").split(";") if self.params else [""]
        out = []
        for part in parts:
            head = part.split(":")[0]
            out.append(int(head) if head.isdigit() else default)
        return out

    def csi(self, final):
        g = self.grid
        private = self.params.startswith("?")
        if self.intermediate or self.params[:1] in (">", "=", "<"):
            return
        nums = self.numbers(0)
        n = nums[0] or 1
        row, col = g.cursor
        if final in "hl" and private:
            for mode in nums:
                self.private_mode(mode, final == "h")
            return
        if final == "m":
            self.sgr()
            return
        g.pending_wrap = False
        if final == "A":
            g.cursor[0] = max(g.top if row >= g.top else 0, row - n)
        elif final in "Be":
            g.cursor[0] = min(g.bottom if row <= g.bottom else g.rows - 1, row + n)
        elif final in "Ca":
            g.cursor[1] = min(g.cols - 1, col + n)
        elif final == "D":
            g.cursor[1] = max(0, col - n)
        elif final == "E":
            g.cursor[0] = min(g.rows - 1, row + n)
            g.cursor[1] = 0
        elif final == "F":
            g.cursor[0] = max(0, row - n)
            g.cursor[1] = 0
        elif final in "G`":
            g.cursor[1] = min(g.cols - 1, n - 1)
        elif final == "d":
            g.cursor[0] = min(g.rows - 1, n - 1)
        elif final in "Hf":
            r = (nums[0] or 1) - 1
            c = ((nums[1] if len(nums) > 1 else 0) or 1) - 1
            g.cursor[0] = min(g.rows - 1, r)
            g.cursor[1] = min(g.cols - 1, c)
        elif final == "J":
            self.erase_display(nums[0])
        elif final == "K":
            self.erase_line(nums[0])
        elif final == "X":
            for c in range(col, min(g.cols, col + n)):
                g.lines[row][c] = Cell(" ", self.attrs)
        elif final == "P":
            line = g.lines[row]
            del line[col:col + n]
            line.extend(blank_row(g.cols - len(line), self.attrs))
        elif final == "@":
            line = g.lines[row]
            for _ in range(min(n, g.cols - col)):
                line.insert(col, Cell(" ", self.attrs))
            del line[g.cols:]
        elif final == "L":
            if g.top <= row <= g.bottom:
                for _ in range(min(n, g.bottom - row + 1)):
                    del g.lines[g.bottom]
                    del g.wrapped[g.bottom]
                    g.lines.insert(row, blank_row(g.cols, self.attrs))
                    g.wrapped.insert(row, False)
        elif final == "M":
            if g.top <= row <= g.bottom:
                for _ in range(min(n, g.bottom - row + 1)):
                    del g.lines[row]
                    del g.wrapped[row]
                    g.lines.insert(g.bottom, blank_row(g.cols, self.attrs))
                    g.wrapped.insert(g.bottom, False)
        elif final == "S":
            self.scroll_up(n)
        elif final == "T":
            self.scroll_down(n)
        elif final == "r":
            top = (nums[0] or 1) - 1
            bottom = ((nums[1] if len(nums) > 1 else 0) or g.rows) - 1
            if top < bottom <= g.rows - 1:
                g.top, g.bottom = top, bottom
                g.cursor[0], g.cursor[1] = 0, 0
        elif final == "s":
            g.saved = (row, col, self.attrs)
        elif final == "u":
            g.cursor[0], g.cursor[1], self.attrs = g.saved

    def private_mode(self, mode, on):
        if mode in (1049, 1047, 47):
            if on and self.alternate is None:
                if mode == 1049:
                    self.primary.saved = (self.primary.cursor[0], self.primary.cursor[1], self.attrs)
                self.alternate = Grid(self.primary.rows, self.primary.cols)
            elif not on and self.alternate is not None:
                self.alternate = None
                if mode == 1049:
                    p = self.primary
                    p.cursor[0], p.cursor[1], self.attrs = p.saved
        elif mode == 2004:
            self.bracketed_paste = on
        elif mode == 25:
            self.cursor_visible = on

    def sgr(self):
        params = self.params.split(";") if self.params else ["0"]
        attrs = dict(self.attrs)
        k = 0
        while k < len(params):
            raw = params[k]
            code = int(raw.split(":")[0]) if raw.split(":")[0].isdigit() else 0
            if code == 0:
                attrs = {}
            elif code in (1, 2, 3, 4, 5, 7, 8, 9):
                attrs[str(code)] = True
            elif code == 22:
                attrs.pop("1", None)
                attrs.pop("2", None)
            elif code in (23, 24, 25, 27, 28, 29):
                attrs.pop(str(code - 20), None)
            elif 30 <= code <= 37 or 90 <= code <= 97:
                attrs["fg"] = str(code)
            elif code == 39:
                attrs.pop("fg", None)
            elif 40 <= code <= 47 or 100 <= code <= 107:
                attrs["bg"] = str(code)
            elif code == 49:
                attrs.pop("bg", None)
            elif code in (38, 48):
                key = "fg" if code == 38 else "bg"
                if ":" in raw:
                    attrs[key] = raw
                elif k + 1 < len(params) and params[k + 1] == "5" and k + 2 < len(params):
                    attrs[key] = f"{code};5;{params[k + 2]}"
                    k += 2
                elif k + 1 < len(params) and params[k + 1] == "2" and k + 4 < len(params):
                    attrs[key] = f"{code};2;{params[k + 2]};{params[k + 3]};{params[k + 4]}"
                    k += 4
            k += 1
        self.attrs = tuple(sorted(attrs.items()))

    def erase_line(self, mode):
        g = self.grid
        row, col = g.cursor
        line = g.lines[row]
        span = range(col, g.cols) if mode == 0 else range(0, col + 1) if mode == 1 else range(g.cols)
        for c in span:
            line[c] = Cell(" ", self.attrs)
        if mode != 1:
            g.wrapped[row] = False

    def erase_display(self, mode):
        g = self.grid
        row = g.cursor[0]
        if mode == 0:
            self.erase_line(0)
            rows = range(row + 1, g.rows)
        elif mode == 1:
            self.erase_line(1)
            rows = range(0, row)
        else:
            rows = range(g.rows)
        for r in rows:
            g.lines[r] = blank_row(g.cols, self.attrs)
            g.wrapped[r] = False
        if mode == 3:
            g.history.clear()
            g.history_wrapped.clear()

    # ------------------------------------------------------------ rendering

    @staticmethod
    def render_line(line, ansi, keep_trailing):
        out = []
        current = ()
        cells = list(line)
        if not keep_trailing:
            while cells and cells[-1].text == " " and not cells[-1].attrs:
                cells.pop()
        for cell in cells:
            if cell.cont:
                continue
            if ansi and cell.attrs != current:
                out.append("\x1b[0m" if current else "")
                if cell.attrs:
                    out.append("\x1b[" + ";".join(sgr_code(k, v) for k, v in cell.attrs) + "m")
                current = cell.attrs
            out.append(cell.text)
        if ansi and current:
            out.append("\x1b[0m")
        return "".join(out)

    def visible_lines(self, ansi=False, keep_trailing=False):
        g = self.grid
        return [self.render_line(line, ansi, keep_trailing) for line in g.lines]

    def history_lines(self, ansi=False, keep_trailing=False):
        if self.alternate is not None:
            return []
        return [self.render_line(line, ansi, keep_trailing) for line in self.primary.history]

    def unwrapped_lines(self, ansi=False):
        """History plus the visible rows, with each automatically wrapped row joined to the next."""
        g = self.grid
        rows = [] if self.alternate is not None else list(zip(g.history, g.history_wrapped))
        rows += list(zip(g.lines, g.wrapped))
        out, pending = [], ""
        for line, wrapped in rows:
            text = self.render_line(line, ansi, wrapped)
            if wrapped:
                pending += text
            else:
                out.append(pending + text)
                pending = ""
        if pending:
            out.append(pending)
        return out


def sgr_code(key, value):
    if key in ("fg", "bg"):
        return value
    return key
