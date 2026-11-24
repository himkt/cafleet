package core

import (
	"fmt"
	"strconv"
	"strings"
	"time"
)

var tmuxPaneGoneMarkers = []string{"can't find pane", "no such pane"}

func tmuxIsPaneGone(stderr string) bool {
	lowered := strings.ToLower(stderr)
	for _, marker := range tmuxPaneGoneMarkers {
		if strings.Contains(lowered, marker) {
			return true
		}
	}
	return false
}

type tmuxMultiplexer struct {
	backend      *backendCore
	closedWindow opt[string]
}

func tmuxMultiplexerForServer(runner commandRunner, socket string) *tmuxMultiplexer {
	return &tmuxMultiplexer{backend: &backendCore{
		runner:     runner,
		name:       "tmux",
		paneGone:   tmuxIsPaneGone,
		serverFlag: "-S",
		server:     socket,
	}}
}

// display is format for pane as display-message prints it.
func (m *tmuxMultiplexer) display(pane, format string) (string, *multiplexerError) {
	return m.backend.run(m.backend.argv("tmux", "display-message", "-p", "-t", pane, format))
}

func tmuxUnexpectedOutput(out string) *multiplexerError {
	return newMultiplexerError("unexpected tmux display-message output: " + quoted(out))
}

const (
	tmuxScreenMetadata      = "#{pane_height} #{pane_width} #{cursor_y} #{cursor_x} #{alternate_on}"
	tmuxScreenRetryInterval = 50 * time.Millisecond
	tmuxLiteralChunkChars   = 256
)

type tmuxScreen struct {
	rows, cols uint16
	cursor     [2]uint16
	alternate  bool
}

// tmuxParseScreenMetadata parses a tmuxScreenMetadata line into rows, columns, cursor and
// screen mode.
func tmuxParseScreenMetadata(line string) (tmuxScreen, *multiplexerError) {
	fields := strings.Fields(line)
	if len(fields) != 5 {
		return tmuxScreen{}, tmuxUnexpectedOutput(line)
	}
	numbers := make([]uint16, 4)
	for i, field := range fields[:4] {
		parsed, err := parseRustUint16(field)
		if err != nil {
			return tmuxScreen{}, tmuxUnexpectedOutput(line)
		}
		numbers[i] = parsed
	}
	var alternate bool
	switch fields[4] {
	case "0":
	case "1":
		alternate = true
	default:
		return tmuxScreen{}, tmuxUnexpectedOutput(line)
	}
	return tmuxScreen{rows: numbers[0], cols: numbers[1], cursor: [2]uint16{numbers[2], numbers[3]}, alternate: alternate}, nil
}

// parseRustUint16 parses as Rust's u16::from_str: an optional `+`, then decimal digits.
func parseRustUint16(field string) (uint16, error) {
	digits := strings.TrimPrefix(field, "+")
	if digits == "" || digits[0] < '0' || digits[0] > '9' {
		return 0, strconv.ErrSyntax
	}
	parsed, err := strconv.ParseUint(digits, 10, 16)
	return uint16(parsed), err
}

func (m *tmuxMultiplexer) core() *backendCore {
	return m.backend
}

func (m *tmuxMultiplexer) bounded(d deadline) multiplexer {
	return &tmuxMultiplexer{backend: m.backend.bounded(d)}
}

func (m *tmuxMultiplexer) name() string {
	return m.backend.name
}

func (m *tmuxMultiplexer) ensureAvailable() *multiplexerError {
	return m.backend.ensureAvailable()
}

// paneContext reads the pane's context; tmux prints `||` and exits 0 for a missing pane, so an
// empty field or another pane ID means the server has no such pane.
func (m *tmuxMultiplexer) paneContext(pane string) (opt[multiplexerContext], *multiplexerError) {
	out, err := m.display(pane, "#{session_name}|#{window_id}|#{pane_id}")
	if err != nil {
		return none[multiplexerContext](), err
	}
	parts := strings.SplitN(strings.TrimSpace(out), "|", 3)
	if len(parts) != 3 {
		return none[multiplexerContext](), tmuxUnexpectedOutput(out)
	}
	if parts[0] == "" || parts[1] == "" || parts[2] != pane {
		return none[multiplexerContext](), nil
	}
	return some(multiplexerContext{session: parts[0], windowID: parts[1], paneID: parts[2]}), nil
}

func (m *tmuxMultiplexer) renameWindow(windowID, name string) *multiplexerError {
	_, err := m.backend.run(m.backend.argv("tmux", "rename-window", "-t", windowID, name))
	return err
}

// windowName reads the window's name; tmux prints an empty line for a missing window, so the
// echoed window ID must match.
func (m *tmuxMultiplexer) windowName(windowID string) (string, *multiplexerError) {
	out, err := m.display(windowID, "#{window_id}|#{window_name}")
	if err != nil {
		return "", err
	}
	line, terminated := strings.CutSuffix(out, "\n")
	if !terminated {
		return "", tmuxUnexpectedOutput(out)
	}
	echoed, name, found := strings.Cut(line, "|")
	if !found || echoed != windowID {
		return "", tmuxUnexpectedOutput(out)
	}
	return name, nil
}

// paneProcess reads the pane's process; a reply naming another or no pane means the server
// has no such pane.
func (m *tmuxMultiplexer) paneProcess(pane string) (opt[int64], *multiplexerError) {
	out, err := m.display(pane, "#{pane_id}|#{pane_pid}")
	if err != nil {
		return none[int64](), err
	}
	paneID, pidText, found := strings.Cut(strings.TrimSpace(out), "|")
	if !found {
		return none[int64](), tmuxUnexpectedOutput(out)
	}
	if paneID != pane {
		return none[int64](), nil
	}
	pid, parseErr := parseI64(pidText)
	if parseErr != nil || pid <= 0 {
		return none[int64](), tmuxUnexpectedOutput(out)
	}
	return some(pid), nil
}

func (m *tmuxMultiplexer) checkKeyTarget(pane string, p placement, d deadline) *multiplexerError {
	target, err := m.backend.pressRun(m.backend.argv("tmux", "display-message", "-p", "-t", pane, "#{pane_id}|#{window_id}|#{session_name}"), d, false)
	if err != nil {
		return err
	}
	parts := strings.SplitN(strings.TrimSpace(target), "|", 3)
	if len(parts) != 3 || parts[0] != pane || parts[1] != p.concreteWindowID() || parts[2] != p.muxSession {
		return newMultiplexerError(placementMismatch)
	}
	return nil
}

func (m *tmuxMultiplexer) sendKeys(pane string, keys pressKeys, d deadline) *multiplexerError {
	return m.backend.sendKeys(append(m.backend.argv("tmux", "send-keys", "-t", pane), keys.tmuxArguments()...), d)
}

func (m *tmuxMultiplexer) windowHeight(windowID string, _ []string) (int64, *multiplexerError) {
	out, err := m.display(windowID, "#{window_height}")
	if err != nil {
		return 0, err
	}
	height, parseErr := parseI64(strings.TrimSpace(out))
	if parseErr != nil || height <= 0 {
		return 0, tmuxUnexpectedOutput(out)
	}
	return height, nil
}

func (m *tmuxMultiplexer) spawnMemberPane(target paneTarget, paneName, cwd string, command []string) (spawnedPane, *multiplexerError) {
	var spawned spawnedPane
	switch target := target.(type) {
	case columnTarget:
		paneID, err := m.splitInto(target, command)
		if err != nil {
			return spawnedPane{}, err
		}
		spawned = spawnedPane{paneID: paneID, windowID: target.windowID}
	case paneTargetNewWindow:
		created, err := m.newWindow(target.session, target.windowName, cwd, command)
		if err != nil {
			return spawnedPane{}, err
		}
		spawned = created
	default:
		panic("invalid paneTarget")
	}
	// Naming and equalization are best effort once the pane exists.
	_, _ = m.backend.run(m.backend.argv("tmux", "select-pane", "-t", spawned.paneID, "-T", paneName))
	_ = m.equalizeWindow(spawned.windowID)
	return spawned, nil
}

// capturePane captures the last lines drawn lines of the pane buffer through captureWindow.
func (m *tmuxMultiplexer) capturePane(pane string, lines int64) (string, *multiplexerError) {
	if err := checkCaptureLines(lines); err != nil {
		return "", err
	}
	fetchDepth := lines + captureOverFetchLines
	raw, err := m.backend.run(m.backend.argv("tmux", "capture-pane", "-p", "-e", "-t", pane, "-S", fmt.Sprintf("-%d", fetchDepth)))
	if err != nil {
		return "", err
	}
	return captureWindow(raw, lines), nil
}

// captureScreen reads the metadata, the body, then the metadata again, and retries the whole
// capture while the two metadata lines differ.
func (m *tmuxMultiplexer) captureScreen(pane string) (screenCapture, *multiplexerError) {
	bodyArgv := m.backend.argv("tmux", "capture-pane", "-p", "-e", "-N", "-t", pane)
	for {
		before, err := m.display(pane, tmuxScreenMetadata)
		if err != nil {
			return screenCapture{}, err
		}
		body, err := m.backend.run(bodyArgv)
		if err != nil {
			return screenCapture{}, err
		}
		after, err := m.display(pane, tmuxScreenMetadata)
		if err != nil {
			return screenCapture{}, err
		}
		if before == after {
			screen, err := tmuxParseScreenMetadata(before)
			if err != nil {
				return screenCapture{}, err
			}
			return screenCapture{rows: screen.rows, cols: screen.cols, cursor: some(screen.cursor), alternateScreen: some(screen.alternate), body: body}, nil
		}
		time.Sleep(tmuxScreenRetryInterval)
	}
}

// typeLiteralUntil types literal in chunks; a failure after the first chunk's process started
// may leave a partial literal.
func (m *tmuxMultiplexer) typeLiteralUntil(pane, literal string, stopped func() bool) (bool, *multiplexerError) {
	characters := []rune(literal)
	for index, start := 0, 0; start < len(characters); index, start = index+1, start+tmuxLiteralChunkChars {
		if index > 0 && stopped() {
			return false, nil
		}
		chunk := string(characters[start:min(start+tmuxLiteralChunkChars, len(characters))])
		if err := m.backend.acknowledgedRun(m.backend.argv("tmux", "send-keys", "-t", pane, "-l", "--", chunk), "literal"); err != nil {
			if index > 0 {
				return false, err.afterWrite()
			}
			return false, err
		}
	}
	return true, nil
}

func (m *tmuxMultiplexer) pressSubmit(pane string, submit []byte) *multiplexerError {
	argv := m.backend.argv("tmux", "send-keys", "-t", pane, "-H")
	for _, b := range submit {
		argv = append(argv, fmt.Sprintf("%02x", b))
	}
	return m.backend.acknowledgedRun(argv, "submit key")
}

func (m *tmuxMultiplexer) closePane(pane, windowID string) *multiplexerError {
	if err := m.backend.runToleratingMissing(m.backend.argv("tmux", "kill-pane", "-t", pane)); err != nil {
		return err
	}
	m.closedWindow = some(windowID)
	return nil
}

func (m *tmuxMultiplexer) restoreAfterClose() *multiplexerError {
	windowID, closed := m.closedWindow.get()
	m.closedWindow = none[string]()
	if !closed {
		return nil
	}
	return m.equalizeWindow(windowID)
}

// splitInto is the new pane's ID from a split; any failure leaves no confirmed pane to clean
// up.
func (m *tmuxMultiplexer) splitInto(column columnTarget, command []string) (string, *multiplexerError) {
	var args []string
	if len(column.columnPanes) > 0 {
		args = m.backend.argv("tmux", "split-window", "-v", "-d", "-t", column.columnPanes[len(column.columnPanes)-1])
	} else {
		if len(column.otherPanes) == 0 {
			panic("an empty column's window holds the opposite column")
		}
		parts := []string{"tmux", "split-window", "-h", "-f"}
		if column.side == columnSideLeft {
			parts = append(parts, "-b")
		}
		parts = append(parts, "-l", "50%", "-d", "-t", column.otherPanes[0])
		args = m.backend.argv(parts...)
	}
	args = append(append(args, "-P", "-F", "#{pane_id}"), command...)
	out, err := m.backend.run(args)
	if err != nil {
		return "", err.withPaneCleanup(paneCleanupUnknown{})
	}
	paneID := strings.TrimSpace(out)
	if paneID == "" {
		return "", newMultiplexerError("tmux split-window returned an empty pane ID").withPaneCleanup(paneCleanupUnknown{})
	}
	return paneID, nil
}

func (m *tmuxMultiplexer) newWindow(session, windowName, cwd string, command []string) (spawnedPane, *multiplexerError) {
	args := append(m.backend.argv("tmux", "new-window", "-d", "-t", session+":", "-n", windowName, "-c", cwd, "-P", "-F", "#{window_id}|#{pane_id}"), command...)
	out, err := m.backend.run(args)
	if err != nil {
		return spawnedPane{}, err.withPaneCleanup(paneCleanupUnknown{})
	}
	windowID, paneID, found := strings.Cut(strings.TrimSpace(out), "|")
	if !found || windowID == "" || paneID == "" {
		return spawnedPane{}, newMultiplexerError("unexpected tmux new-window output: " + quoted(out)).withPaneCleanup(paneCleanupUnknown{})
	}
	return spawnedPane{paneID: paneID, windowID: windowID}, nil
}

// equalizeWindow spreads each column (panes sharing pane_left) of windowID evenly; a window
// that no longer exists has nothing to restore.
func (m *tmuxMultiplexer) equalizeWindow(windowID string) *multiplexerError {
	argv := m.backend.argv("tmux", "list-panes", "-t", windowID, "-F", "#{pane_id}|#{pane_left}")
	listing, runErr := m.backend.runner.run(argv)
	switch {
	case runErr != nil && runErr.kind == runErrorFailed && strings.Contains(runErr.detail, "can't find window"):
		return nil
	case runErr != nil:
		return m.backend.mapError(argv, runErr)
	}
	type column struct {
		left  string
		panes []string
	}
	var columns []column
	for _, line := range lines(listing) {
		if strings.TrimSpace(line) == "" {
			continue
		}
		paneID, left, found := strings.Cut(line, "|")
		if !found {
			return newMultiplexerError("unexpected tmux list-panes output: " + quoted(listing))
		}
		matched := false
		for i := range columns {
			if columns[i].left == left {
				columns[i].panes = append(columns[i].panes, paneID)
				matched = true
				break
			}
		}
		if !matched {
			columns = append(columns, column{left: left, panes: []string{paneID}})
		}
	}
	for _, column := range columns {
		if len(column.panes) < 2 {
			continue
		}
		if _, err := m.backend.run(m.backend.argv("tmux", "select-layout", "-E", "-t", column.panes[0])); err != nil {
			return err
		}
	}
	return nil
}
