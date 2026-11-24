package core

import (
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"
)

// multiplexerContext is the concrete session, window and pane context discovery returns.
type multiplexerContext struct {
	session  string
	windowID string
	paneID   string
}

// columnTarget is an existing column of a window; otherPanes holds the opposite column (the
// Director pane for the Director window).
type columnTarget struct {
	windowID    string
	side        columnSide
	columnPanes []string
	otherPanes  []string
}

//sumtype:decl
type paneTarget interface {
	isPaneTarget()
}

type paneTargetNewWindow struct {
	session    string
	windowName string
}

func (columnTarget) isPaneTarget()        {}
func (paneTargetNewWindow) isPaneTarget() {}

// screenCapture is one screenful of a pane as the multiplexer renders it; cursor (row, column)
// and alternateScreen are absent when the capture source exposes no cursor or screen-mode data.
type screenCapture struct {
	rows            uint16
	cols            uint16
	cursor          opt[[2]uint16]
	alternateScreen opt[bool]
	body            string
}

type spawnedPane struct {
	paneID   string
	windowID string
}

// paneCleanup is the outcome of backend-owned pane compensation.
//
//sumtype:decl
type paneCleanup interface {
	isPaneCleanup()
}

type paneCleanupAttempted struct {
	paneID string
	err    opt[string]
}

type paneCleanupUnknown struct{}

func (paneCleanupAttempted) isPaneCleanup() {}
func (paneCleanupUnknown) isPaneCleanup()   {}

// multiplexerError is a backend failure with optional pane ownership information.
type multiplexerError struct {
	message           string
	paneCleanup       opt[paneCleanup]
	submissionStarted bool
}

func newMultiplexerError(message string) *multiplexerError {
	return &multiplexerError{message: message}
}

func newMultiplexerErrorf(format string, args ...any) *multiplexerError {
	return newMultiplexerError(fmt.Sprintf(format, args...))
}

func (e *multiplexerError) afterWrite() *multiplexerError {
	e.submissionStarted = true
	return e
}

func (e *multiplexerError) withPaneCleanup(cleanup paneCleanup) *multiplexerError {
	e.paneCleanup = some(cleanup)
	return e
}

func (e *multiplexerError) Error() string {
	cleanup, _ := e.paneCleanup.get()
	switch cleanup := cleanup.(type) {
	case paneCleanupAttempted:
		if cause, failed := cleanup.err.get(); failed {
			return e.message + "\ncleanup failed for pane " + cleanup.paneID + ": " + cause
		}
	case paneCleanupUnknown:
		return e.message + "\npane ID unknown; pane cleanup unconfirmed"
	case nil:
	default:
		panic("invalid paneCleanup")
	}
	return e.message
}

// cafleet is the application error a multiplexer failure surfaces as.
func (e *multiplexerError) cafleet() *cafleetError {
	return appError(e.Error())
}

// paneOwnership owns a known pane until finish transfers it or rollback attempts its cleanup.
type paneOwnership struct {
	paneID opt[string]
	kill   func(string) *multiplexerError
}

func newPaneOwnership(paneID string, kill func(string) *multiplexerError) *paneOwnership {
	return &paneOwnership{paneID: some(paneID), kill: kill}
}

func (o *paneOwnership) take() string {
	paneID, owned := o.paneID.get()
	if !owned {
		panic("the pane is still owned")
	}
	o.paneID = none[string]()
	return paneID
}

func (o *paneOwnership) rollback() paneCleanup {
	paneID := o.take()
	cleanup := paneCleanupAttempted{paneID: paneID}
	if err := o.kill(paneID); err != nil {
		cleanup.err = some(err.Error())
	}
	return cleanup
}

func (o *paneOwnership) finish() string {
	return o.take()
}

type runErrorKind uint8

const (
	runErrorBinaryNotFound runErrorKind = iota + 1
	runErrorNotStarted
	runErrorTimeout
	runErrorFailed
)

// runError is a subprocess failure below the backend's message mapping. detail is the cause of
// a binary-not-found or not-started failure and the stderr of a failed run.
type runError struct {
	kind   runErrorKind
	detail string
}

func runErrorDeadlineExpired() *runError {
	return &runError{kind: runErrorNotStarted, detail: "command deadline expired before launch"}
}

type commandRunner interface {
	binaryExists(name string) bool
	run(argv []string) (string, *runError)
	runFor(argv []string, timeout time.Duration) (string, *runError)
}

const placementMismatch = "press target context does not match the recorded placement"

func mapRunError(name string, argv []string, err *runError) *multiplexerError {
	joined := strings.Join(argv, " ")
	switch err.kind {
	case runErrorNotStarted:
		return newMultiplexerErrorf("%s command was not started: %s: %s", name, err.detail, joined)
	case runErrorBinaryNotFound:
		return newMultiplexerErrorf("%s binary not found: %s", name, err.detail)
	case runErrorTimeout:
		return newMultiplexerErrorf("%s command timed out at the pane input deadline: %s", name, joined)
	case runErrorFailed:
		return newMultiplexerErrorf("%s command failed: %s\nstderr: %s", name, joined, strings.TrimSpace(err.detail))
	default:
		panic("invalid runErrorKind")
	}
}

func checkDeadline(d deadline) *multiplexerError {
	if _, err := d.remaining(); err != nil {
		return newMultiplexerError(cafleetErrorOf(err).message())
	}
	return nil
}

func checkCaptureLines(lines int64) *multiplexerError {
	if lines <= 0 {
		return newMultiplexerErrorf("capture_pane: lines must be positive, got %d", lines)
	}
	return nil
}

func memberPaneName(memberID int64, memberName string) string {
	return fmt.Sprintf("%d - %s", memberID, memberName)
}

// backendCore is the runner, server selector and error mapping shared by every backend; a
// backend adds only its own argv construction and output parsing.
type backendCore struct {
	runner     commandRunner
	name       string
	paneGone   func(string) bool
	serverFlag string
	server     string
}

func (c *backendCore) bounded(d deadline) *backendCore {
	bounded := *c
	bounded.runner = deadlineRunnerWrap(c.runner, d)
	return &bounded
}

// argv is parts with the server selector inserted after the binary.
func (c *backendCore) argv(parts ...string) []string {
	return append([]string{parts[0], c.serverFlag, c.server}, parts[1:]...)
}

func (c *backendCore) mapError(argv []string, err *runError) *multiplexerError {
	return mapRunError(c.name, argv, err)
}

func (c *backendCore) run(argv []string) (string, *multiplexerError) {
	out, err := c.runner.run(argv)
	if err != nil {
		return "", c.mapError(argv, err)
	}
	return out, nil
}

func (c *backendCore) runToleratingMissing(argv []string) *multiplexerError {
	_, err := c.runner.run(argv)
	if err == nil || (err.kind == runErrorFailed && c.paneGone(err.detail)) {
		return nil
	}
	return c.mapError(argv, err)
}

func (c *backendCore) ensureAvailable() *multiplexerError {
	if !c.runner.binaryExists(c.name) {
		return newMultiplexerErrorf("%s binary not found on PATH", c.name)
	}
	return nil
}

func (c *backendCore) pressRun(argv []string, d deadline, writing bool) (string, *multiplexerError) {
	remaining, err := d.remaining()
	if err != nil {
		return "", newMultiplexerError(cafleetErrorOf(err).message())
	}
	out, runErr := c.runner.runFor(argv, remaining)
	switch {
	case runErr == nil:
		return out, nil
	case writing:
		return "", c.writeError(argv, runErr)
	default:
		return "", c.mapError(argv, runErr)
	}
}

// writeError maps a writing process's failure; one that failed or timed out started, so its
// input may have reached the pane.
func (c *backendCore) writeError(argv []string, err *runError) *multiplexerError {
	mapped := c.mapError(argv, err)
	if err.kind == runErrorFailed || err.kind == runErrorTimeout {
		return mapped.afterWrite()
	}
	return mapped
}

// acknowledgedRun runs one writing process bounded by the runner's own deadline, acknowledged
// like acknowledgedWrite.
func (c *backendCore) acknowledgedRun(argv []string, what string) *multiplexerError {
	out, err := c.runner.run(argv)
	if err != nil {
		return c.writeError(argv, err)
	}
	return c.acknowledge(out, what)
}

func (c *backendCore) acknowledge(output, what string) *multiplexerError {
	if strings.TrimSpace(output) != "" {
		return newMultiplexerErrorf("%s %s acknowledgment was unreadable", c.name, what).afterWrite()
	}
	return nil
}

func (c *backendCore) sendKeys(argv []string, d deadline) *multiplexerError {
	return c.acknowledgedWrite(argv, d, "key")
}

func (c *backendCore) acknowledgedWrite(argv []string, d deadline, what string) *multiplexerError {
	out, err := c.pressRun(argv, d, true)
	if err != nil {
		return err
	}
	return c.acknowledge(out, what)
}

type deadlineRunner struct {
	runner   commandRunner
	deadline deadline
}

func deadlineRunnerWrap(runner commandRunner, d deadline) commandRunner {
	return deadlineRunner{runner: runner, deadline: d}
}

func (r deadlineRunner) remaining() (time.Duration, *runError) {
	left, err := r.deadline.remaining()
	if err != nil {
		return 0, runErrorDeadlineExpired()
	}
	return left, nil
}

func (r deadlineRunner) binaryExists(name string) bool {
	return r.runner.binaryExists(name)
}

func (r deadlineRunner) run(argv []string) (string, *runError) {
	remaining, err := r.remaining()
	if err != nil {
		return "", err
	}
	return r.runFor(argv, remaining)
}

func (r deadlineRunner) runFor(argv []string, timeout time.Duration) (string, *runError) {
	remaining, err := r.remaining()
	if err != nil {
		return "", err
	}
	return r.runner.runFor(argv, min(timeout, remaining))
}

// captureOverFetchLines is the capture over-fetch margin: both backends fetch lines + 1000 so a
// blank tail deeper than the requested window still leaves drawn lines to keep.
const captureOverFetchLines int64 = 1000

// captureWindow splits the fetched buffer on `\n` only (so `\r` survives for the CLI-side
// defrag), drops the trailing run of visually blank lines (kept lines keep their bytes) and
// keeps the last lines remaining lines, so a small window shows the pane's drawn bottom.
func captureWindow(raw string, lines int64) string {
	if lines <= 0 {
		panic("capture lines are positive")
	}
	parts := strings.Split(raw, "\n")
	for len(parts) > 0 && isVisuallyBlank(parts[len(parts)-1]) {
		parts = parts[:len(parts)-1]
	}
	start := max(int64(len(parts))-lines, 0)
	return strings.Join(parts[start:], "\n")
}

type multiplexer interface {
	core() *backendCore
	bounded(d deadline) multiplexer
	name() string
	ensureAvailable() *multiplexerError
	// checkKeyTarget confirms that the recorded pane still sits at the placement's session
	// and window.
	checkKeyTarget(pane string, p placement, d deadline) *multiplexerError
	sendKeys(pane string, keys pressKeys, d deadline) *multiplexerError
	// paneContext is the session, window and pane of pane on the targeted server; false when
	// the server has no such pane.
	paneContext(pane string) (opt[multiplexerContext], *multiplexerError)
	// paneProcess is the pid of pane's process on the targeted server; false when the server
	// has no such pane.
	paneProcess(pane string) (opt[int64], *multiplexerError)
	renameWindow(windowID, name string) *multiplexerError
	// windowName is the current name of windowID on the targeted server; an empty name is
	// valid.
	windowName(windowID string) (string, *multiplexerError)
	// windowHeight is the live height of windowID in rows; herdr reads it through the first
	// answering anchor.
	windowHeight(windowID string, anchors []string) (int64, *multiplexerError)
	// spawnMemberPane creates the member pane for target, named paneName. Naming,
	// equalization and herdr re-parenting are best effort; a failure after the pane ID is known
	// closes that pane.
	spawnMemberPane(target paneTarget, paneName, cwd string, command []string) (spawnedPane, *multiplexerError)
	capturePane(pane string, lines int64) (string, *multiplexerError)
	// captureScreen is the visible screen of pane with its geometry, plus the cursor and
	// screen mode when the backend exposes them.
	captureScreen(pane string) (screenCapture, *multiplexerError)
	// typeLiteralUntil types literal as plain text, checking stopped before each write process
	// after the first; false when it stopped before writing every piece.
	typeLiteralUntil(pane, literal string, stopped func() bool) (bool, *multiplexerError)
	// pressSubmit presses the key sequence whose bytes are submit.
	pressSubmit(pane string, submit []byte) *multiplexerError
	// closePane closes the pane and records windowID for the next restoreAfterClose.
	closePane(pane, windowID string) *multiplexerError
	// restoreAfterClose equalizes every column of the window the most recent closePane
	// recorded.
	restoreAfterClose() *multiplexerError
}

func validateKeyTarget(m multiplexer, p placement, d deadline) *multiplexerError {
	if err := checkDeadline(d); err != nil {
		return err
	}
	pane, concrete := p.muxPaneID.get()
	if !concrete {
		return newMultiplexerError("press target has no pane")
	}
	return m.checkKeyTarget(pane, p, d)
}

// killPane closes the pane, then restores windowID best effort.
func killPane(m multiplexer, pane, windowID string) *multiplexerError {
	if err := m.closePane(pane, windowID); err != nil {
		return err
	}
	// Restoring the layout is best effort once the pane is closed.
	_ = m.restoreAfterClose()
	return nil
}

type muxBackend uint8

const (
	muxBackendTmux muxBackend = iota + 1
	muxBackendHerdr
)

func (m muxBackend) name() string {
	switch m {
	case muxBackendTmux:
		return "tmux"
	case muxBackendHerdr:
		return "herdr"
	default:
		panic("invalid muxBackend")
	}
}

// namedKey is a key's press-token name, tmux key name and herdr key name.
type namedKey struct{ token, tmux, herdr string }

var namedKeys = []namedKey{
	{"Space", "Space", "space"},
	{"Escape", "Escape", "esc"},
	{"Enter", "Enter", "enter"},
	{"Tab", "Tab", "tab"},
	{"Backspace", "BSpace", "backspace"},
	{"Up", "Up", "up"},
	{"Down", "Down", "down"},
	{"Left", "Left", "left"},
	{"Right", "Right", "right"},
}

//sumtype:decl
type pressKey interface{ isPressKey() }

type pressKeyLiteral struct{ b uint8 }

type pressKeyNamed struct{ named *namedKey }

type pressKeyFunction struct{ n uint8 }

type pressKeyControl struct{ b uint8 }

type pressKeyShiftTab struct{}

func (pressKeyLiteral) isPressKey()  {}
func (pressKeyNamed) isPressKey()    {}
func (pressKeyFunction) isPressKey() {}
func (pressKeyControl) isPressKey()  {}
func (pressKeyShiftTab) isPressKey() {}

type pressKeys struct{ keys []pressKey }

func namedKeyOf(token string) *namedKey {
	for i := range namedKeys {
		if namedKeys[i].token == token {
			return &namedKeys[i]
		}
	}
	panic("a named key the byte mapping names")
}

// pressKeysFromBytes is the keys whose bytes are bytes; false when a byte has no explicit-key
// mapping or bytes is empty.
func pressKeysFromBytes(bytes []byte) (pressKeys, bool) {
	keys := make([]pressKey, 0, len(bytes))
	for _, b := range bytes {
		switch {
		case b >= '!' && b <= '~':
			keys = append(keys, pressKeyLiteral{b: b})
		case b == ' ':
			keys = append(keys, pressKeyNamed{named: namedKeyOf("Space")})
		case b == 0x1b:
			keys = append(keys, pressKeyNamed{named: namedKeyOf("Escape")})
		case b == '\r':
			keys = append(keys, pressKeyNamed{named: namedKeyOf("Enter")})
		case b == '\t':
			keys = append(keys, pressKeyNamed{named: namedKeyOf("Tab")})
		case b == 0x7f:
			keys = append(keys, pressKeyNamed{named: namedKeyOf("Backspace")})
		case b >= 0x01 && b <= 0x1a:
			keys = append(keys, pressKeyControl{b: 'a' + b - 1})
		default:
			return pressKeys{}, false
		}
	}
	if len(keys) == 0 {
		return pressKeys{}, false
	}
	return pressKeys{keys: keys}, true
}

func muxBackendParse(name string) (muxBackend, *multiplexerError) {
	switch name {
	case "tmux":
		return muxBackendTmux, nil
	case "herdr":
		return muxBackendHerdr, nil
	default:
		return 0, newMultiplexerErrorf("unsupported recorded multiplexer backend '%s'", name)
	}
}

// forServer is the multiplexer that targets server on this backend explicitly.
func (m muxBackend) forServer(server string, runner commandRunner) multiplexer {
	switch m {
	case muxBackendTmux:
		return tmuxMultiplexerForServer(runner, server)
	case muxBackendHerdr:
		return herdrMultiplexerForServer(runner, server)
	default:
		panic("invalid muxBackend")
	}
}

// muxBackendDetect auto-detects the backend from HERDR_ENV / TMUX (an empty value counts as
// unset); ambiguity and absence are hard errors.
func muxBackendDetect(env func(string) opt[string]) (muxBackend, *multiplexerError) {
	present := func(name string) bool {
		value, set := env(name).get()
		return set && value != ""
	}
	switch herdr, tmux := present("HERDR_ENV"), present("TMUX"); {
	case herdr && tmux:
		return 0, newMultiplexerError("ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; unset the one that does not belong to this session")
	case herdr:
		return muxBackendHerdr, nil
	case tmux:
		return muxBackendTmux, nil
	default:
		return 0, newMultiplexerError("no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; run cafleet inside a tmux or herdr session")
	}
}

// callerEnv holds the set caller-environment variables that multiplexer detection and
// `fleet create` read; a non-UTF-8 one fails with its name.
func callerEnv() (map[string]string, error) {
	env := map[string]string{}
	for _, name := range strings.Split("TMUX TMUX_PANE HERDR_ENV HERDR_SESSION HERDR_SOCKET_PATH HERDR_PANE_ID", " ") {
		value, err := processEnv(name)
		if err != nil {
			return nil, err
		}
		if set, present := value.get(); present {
			env[name] = set
		}
	}
	return env, nil
}

// multiplexerForServer is the multiplexer that targets server on the recorded backend
// explicitly.
func multiplexerForServer(backend, server string, runner commandRunner) (multiplexer, *multiplexerError) {
	parsed, err := muxBackendParse(backend)
	if err != nil {
		return nil, err
	}
	return parsed.forServer(server, runner), nil
}

// callerServer is the caller's server on backend when `fleet create` receives no
// --mux-server.
func callerServer(backend muxBackend, env func(string) opt[string], runner commandRunner) (string, *multiplexerError) {
	present := func(name string) (string, bool) {
		value, set := env(name).get()
		return value, set && value != ""
	}
	switch backend {
	case muxBackendTmux:
		tmux, set := present("TMUX")
		if !set {
			return tmuxSocketReply(runner, defaultTmuxSocket)
		}
		for range 2 {
			if comma := strings.LastIndexByte(tmux, ','); comma >= 0 {
				tmux = tmux[:comma]
			}
		}
		return tmux, nil
	case muxBackendHerdr:
		if session, set := present("HERDR_SESSION"); set {
			return session, nil
		}
		if socket, set := present("HERDR_SOCKET_PATH"); set {
			return herdrSessionForSocket(runner, socket)
		}
		return defaultHerdrSession, nil
	default:
		panic("invalid muxBackend")
	}
}

const defaultHerdrSession = "default"

// defaultTmuxSocket asks the default tmux server for its socket. The command runner passes the
// process environment to every child, so a stale TMUX would select its server instead of the
// default one.
var defaultTmuxSocket = []string{"env", "-u", "TMUX", "tmux", "display-message", "-p", "#{socket_path}"}

// tmuxSocketReply is the default tmux server's socket path as argv prints it.
func tmuxSocketReply(runner commandRunner, argv []string) (string, *multiplexerError) {
	noServer := newMultiplexerError("no tmux server found; pass --mux-server <socket path>")
	out, err := runner.run(argv)
	switch {
	case err != nil && err.kind == runErrorFailed:
		return "", noServer
	case err != nil:
		return "", mapRunError("tmux", argv, err)
	}
	socket := strings.TrimSpace(out)
	if socket == "" {
		return "", noServer
	}
	return socket, nil
}

func herdrSessionForSocket(runner commandRunner, socket string) (string, *multiplexerError) {
	argv := []string{"herdr", "session", "list", "--json"}
	out, err := runner.run(argv)
	if err != nil {
		return "", mapRunError("herdr", argv, err)
	}
	listing, parsed := jsonParse(out)
	if !parsed {
		return "", newMultiplexerErrorf("herdr session list returned non-JSON output: %s", quoted(out))
	}
	sessions, listed := jsonArray(jsonIndex(listing, "sessions"))
	if !listed {
		return "", newMultiplexerError("herdr session list missing 'sessions' field")
	}
	for _, session := range sessions {
		if path, isText := jsonString(jsonIndex(session, "socket_path")); isText && path == socket {
			if name, named := jsonString(jsonIndex(session, "name")); named {
				return name, nil
			}
			break
		}
	}
	return defaultHerdrSession, nil
}

// asciiEqualFold compares two strings ignoring the case of ASCII letters only.
func asciiEqualFold(left, right string) bool {
	if len(left) != len(right) {
		return false
	}
	for i := range len(left) {
		if asciiLower(left[i]) != asciiLower(right[i]) {
			return false
		}
	}
	return true
}

func asciiLower(b byte) byte {
	if b >= 'A' && b <= 'Z' {
		return b + 'a' - 'A'
	}
	return b
}

func pressKeysParse(keys []string) (pressKeys, error) {
	if len(keys) < 1 || len(keys) > 64 {
		return pressKeys{}, usageError("press requires between 1 and 64 keys")
	}
	parsed := make([]pressKey, 0, len(keys))
	for index, token := range keys {
		if len(token) > 32 {
			return pressKeys{}, usageErrorf("press key at position %d exceeds 32 UTF-8 bytes", index+1)
		}
		key, ok := pressKeyOf(token)
		if !ok {
			encoded, err := encodeJSON(token)
			if err != nil {
				panic("a press token is a valid UTF-8 string: " + err.Error())
			}
			return pressKeys{}, usageErrorf("invalid press key at position %d: %s", index+1, encoded)
		}
		parsed = append(parsed, key)
	}
	return pressKeys{keys: parsed}, nil
}

func pressKeyOf(token string) (pressKey, bool) {
	if len(token) == 1 && token[0] >= '!' && token[0] <= '~' {
		return pressKeyLiteral{b: token[0]}, true
	}
	for i := range namedKeys {
		if asciiEqualFold(token, namedKeys[i].token) {
			return pressKeyNamed{named: &namedKeys[i]}, true
		}
	}
	if asciiEqualFold(token, "Shift+Tab") {
		return pressKeyShiftTab{}, true
	}
	if len(token) == 6 && asciiEqualFold(token[:5], "Ctrl+") && isASCIILetter(token[5]) {
		return pressKeyControl{b: asciiLower(token[5])}, true
	}
	for n := uint8(1); n <= 12; n++ {
		if asciiEqualFold(token, "F"+strconv.Itoa(int(n))) {
			return pressKeyFunction{n: n}, true
		}
	}
	return nil, false
}

func isASCIILetter(b byte) bool {
	return (b >= 'a' && b <= 'z') || (b >= 'A' && b <= 'Z')
}

func (k pressKeys) render(render func(pressKey) string) []string {
	rendered := make([]string, len(k.keys))
	for i, key := range k.keys {
		rendered[i] = render(key)
	}
	return rendered
}

func (k pressKeys) canonical() []string {
	return k.render(func(key pressKey) string {
		switch key := key.(type) {
		case pressKeyLiteral:
			return string(rune(key.b))
		case pressKeyNamed:
			return key.named.token
		case pressKeyFunction:
			return fmt.Sprintf("F%d", key.n)
		case pressKeyControl:
			return "Ctrl+" + string(rune(key.b))
		case pressKeyShiftTab:
			return "Shift+Tab"
		default:
			panic("invalid pressKey")
		}
	})
}

func (k pressKeys) tmuxArguments() []string {
	return k.render(func(key pressKey) string {
		switch key := key.(type) {
		case pressKeyLiteral:
			return fmt.Sprintf("0x%02x", key.b)
		case pressKeyNamed:
			return key.named.tmux
		case pressKeyFunction:
			return fmt.Sprintf("F%d", key.n)
		case pressKeyControl:
			return "C-" + string(rune(key.b))
		case pressKeyShiftTab:
			return "BTab"
		default:
			panic("invalid pressKey")
		}
	})
}

func (k pressKeys) herdrArguments() []string {
	return k.render(func(key pressKey) string {
		switch key := key.(type) {
		case pressKeyLiteral:
			switch key.b {
			case '+':
				return "plus"
			case '-':
				return "minus"
			case ';':
				return "semicolon"
			default:
				return string(rune(key.b))
			}
		case pressKeyNamed:
			return key.named.herdr
		case pressKeyFunction:
			return fmt.Sprintf("f%d", key.n)
		case pressKeyControl:
			return "ctrl+" + string(rune(key.b))
		case pressKeyShiftTab:
			return "shift+tab"
		default:
			panic("invalid pressKey")
		}
	})
}

// reportLayoutReparenting reports a failed herdr re-parenting move while the spawn continues.
func reportLayoutReparenting(cause *multiplexerError) {
	fmt.Fprintln(os.Stderr, "layout re-parenting incomplete: "+cause.Error())
}
