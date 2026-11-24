package core

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"
)

// hermesConfig runs `hermes config path`; calls of one concurrent batch pass the same deadline.
type hermesConfig interface {
	// configPath is the trimmed stdout of `hermes config path`, or the failure's cause.
	configPath(until time.Time) (string, error)
}

// hermesOutput is a completed hermes process: its exit code, stdout and stderr.
type hermesOutput struct {
	exitCode int
	stdout   string
	stderr   string
}

// succeeded is the stdout of a command that exited 0, or its stderr text as the cause.
func succeeded(output hermesOutput) (string, error) {
	stderr := strings.TrimFunc(output.stderr, unicode.IsSpace)
	switch {
	case output.exitCode == 0:
		return output.stdout, nil
	case stderr == "":
		return "", appErrorf("exited with code %d", output.exitCode)
	default:
		return "", appError(stderr)
	}
}

// nativeSession is the member run's Hermes session identity, stored in
// member_runs.native_session_json.
type nativeSession struct {
	// hermesHome is the effective Hermes home, the directory of `hermes config path` at launch,
	// which holds processes.json.
	hermesHome string
	sessionID  opt[string]
}

type rawNativeSession struct {
	HermesHome string      `json:"hermes_home"`
	SessionID  opt[string] `json:"session_id"`
}

func (s nativeSession) MarshalJSONTo(encoder *jsontext.Encoder) error {
	object := jsonObject{{"hermes_home", s.hermesHome}}
	if id, ok := s.sessionID.get(); ok {
		object = append(object, jsonMember{"session_id", id})
	}
	return object.MarshalJSONTo(encoder)
}

func (s *nativeSession) UnmarshalJSONFrom(decoder *jsontext.Decoder) error {
	value, err := decoder.ReadValue()
	if err != nil {
		return err
	}
	var raw rawNativeSession
	if err := decodeStrict(value, &raw); err != nil {
		return err
	}
	*s = nativeSession{hermesHome: raw.HermesHome, sessionID: raw.SessionID}
	return nil
}

// hermesSettlementHost is the effects settlement needs: native input and screen reads on the
// member's pane, the run's child state, the session record and the Hermes process registry.
type hermesSettlementHost interface {
	clientExited() (bool, error)
	sendKeys(keys []string) error
	// capture is the native capture, preserving SGR for placeholder detection.
	capture() (string, error)
	recordSessionID(sessionID string) error
	// processesJSON is the text of home's processes.json; false when the file is absent.
	processesJSON(home string) (string, bool, error)
	// registeredProcessRunning reports whether the registered process still runs: pid alive
	// with hostStartTime, Hermes's /proc/<pid>/stat start field. A platform that cannot compare
	// start times reports a live pid as running.
	registeredProcessRunning(pid uint32, hostStartTime uint64) (bool, error)
	pause()
	now() time.Time
}

var hermesInterruptKeys = []string{"Ctrl+C"}

var hermesStopCommand = []string{"/", "s", "t", "o", "p"}

var hermesExitCommand = []string{"/", "e", "x", "i", "t"}

func backspaces(first string) []string {
	keys := make([]string, 64)
	for i := range keys {
		keys[i] = "Backspace"
	}
	keys[0] = first
	return keys
}

// Ctrl+C on an empty composer ends the client without stopping its session. Backspace also
// clears the multiline drafts that Hermes's line-kill shortcuts leave behind.
var (
	hermesClearFirstBatch     = backspaces("Escape")
	hermesClearBackspaceBatch = backspaces("Backspace")
)

const hermesClearBatchesPerRound = 16

var hermesMenuCloseKeys = []string{"Escape"}

var hermesSubmitKeys = []string{"Enter"}

// A typed slash command opens a completion menu once it has rendered, so the menu is closed
// before the first Enter and each Enter waits for its effect before the next one.
const (
	hermesMaxSubmits   = 3
	hermesSubmitWindow = 1000 * time.Millisecond
)

var hermesBusyMarkers = []string{"type to redirect this turn", "↑/↓ to select"}

const hermesBusyTailLines = 8

type settlement uint8

const (
	// settlementVerified is a session whose work stopped and none of whose registered processes
	// runs.
	settlementVerified settlement = iota + 1
	// settlementClientAlreadyExited is a client that had exited before settlement could stop its
	// session.
	settlementClientAlreadyExited
)

// settle stops the member's Hermes session work and verifies that no process it registered
// still runs.
func settle(memberID int64, session *nativeSession, host hermesSettlementHost, until time.Time) (settlement, error) {
	outcome, err := settleSession(session, host, until)
	if err != nil {
		return 0, appErrorf("hermes work for member %d could not be verified as stopped: %s; member was not deleted.", memberID, cafleetErrorOf(err).message())
	}
	return outcome, nil
}

func settleSession(session *nativeSession, host hermesSettlementHost, until time.Time) (settlement, error) {
	exited, err := host.clientExited()
	if err != nil {
		return 0, err
	}
	recorded, recordedSet := session.sessionID.get()
	var sessionID string
	switch {
	case exited && recordedSet:
		sessionID = recorded
	case exited:
		return settlementClientAlreadyExited, nil
	default:
		if sessionID, err = stopClient(host, until); err != nil {
			return 0, err
		}
		if err := host.recordSessionID(sessionID); err != nil {
			return 0, err
		}
	}
	if err := verifyProcesses(host, session.hermesHome, sessionID); err != nil {
		return 0, err
	}
	return settlementVerified, nil
}

func alreadyExitedWarning(memberID int64) string {
	return fmt.Sprintf("warning: hermes client for member %d had already exited; its session work may still be running in the Hermes gateway", memberID)
}

const hermesSettlementBudget = 20 * time.Second

// settlementDeadline is the budget of one Hermes member's settlement, which takes several native
// round trips.
func settlementDeadline() deadline { return deadlineUntil(time.Now().Add(hermesSettlementBudget)) }

func stopClient(host hermesSettlementHost, until time.Time) (string, error) {
	screen, err := host.capture()
	if err != nil {
		return "", err
	}
	if busy(screen) {
		if err := host.sendKeys(hermesInterruptKeys); err != nil {
			return "", err
		}
		if _, err := waitFor(host, until, "the Hermes turn did not stop after Ctrl-C", func(screen string) (struct{}, bool) {
			return struct{}{}, !busy(screen)
		}); err != nil {
			return "", err
		}
	}
	if screen, err = host.capture(); err != nil {
		return "", err
	}
	reports := stopReports(screen)
	if _, err := submitCommand(host, until, hermesStopCommand, "Hermes did not report /stop", func(screen string) (struct{}, bool) {
		return struct{}{}, stopReports(screen) > reports
	}); err != nil {
		return "", err
	}
	sessionID, err := submitCommand(host, until, hermesExitCommand, "Hermes printed no exit summary after /exit", exitSummarySession)
	if err != nil {
		return "", err
	}
	_, exited, err := poll(host, until, func(host hermesSettlementHost) (struct{}, bool, error) {
		exited, err := host.clientExited()
		return struct{}{}, exited, err
	})
	if err != nil {
		return "", err
	}
	if !exited {
		return "", appError("the Hermes client did not exit after /exit")
	}
	return sessionID, nil
}

// poll asks done until it yields a value or until passes, pausing between attempts; false once
// until passes.
func poll[T any](host hermesSettlementHost, until time.Time, done func(hermesSettlementHost) (T, bool, error)) (T, bool, error) {
	for {
		value, ok, err := done(host)
		if err != nil || ok {
			return value, ok, err
		}
		if !host.now().Before(until) {
			var zero T
			return zero, false, nil
		}
		host.pause()
	}
}

func captureDone[T any](done func(string) (T, bool)) func(hermesSettlementHost) (T, bool, error) {
	return func(host hermesSettlementHost) (T, bool, error) {
		screen, err := host.capture()
		if err != nil {
			var zero T
			return zero, false, err
		}
		value, ok := done(screen)
		return value, ok, nil
	}
}

// submitCommand clears the composer, types command, closes its completion menu, then presses
// Enter up to hermesMaxSubmits times, each after the previous one showed no effect within
// hermesSubmitWindow, and keeps waiting for done until until.
func submitCommand[T any](host hermesSettlementHost, until time.Time, command []string, timeout string, done func(string) (T, bool)) (T, error) {
	var zero T
	if err := clearComposer(host, until); err != nil {
		return zero, err
	}
	if err := host.sendKeys(command); err != nil {
		return zero, err
	}
	host.pause()
	if err := host.sendKeys(hermesMenuCloseKeys); err != nil {
		return zero, err
	}
	for range hermesMaxSubmits {
		host.pause()
		exited, err := host.clientExited()
		if err != nil {
			return zero, err
		}
		if exited {
			break
		}
		if err := host.sendKeys(hermesSubmitKeys); err != nil {
			return zero, err
		}
		window := host.now().Add(hermesSubmitWindow)
		if until.Before(window) {
			window = until
		}
		value, ok, err := poll(host, window, captureDone(done))
		if err != nil {
			return zero, err
		}
		if ok {
			return value, nil
		}
	}
	return waitFor(host, until, timeout, done)
}

func clearComposer(host hermesSettlementHost, until time.Time) error {
	failed := func() error { return appError("the Hermes composer still holds text after clearing") }
	screen, err := host.capture()
	if err != nil {
		return err
	}
	for {
		if !host.now().Before(until) {
			return failed()
		}
		if composerEmpty(screen) {
			return nil
		}
		region, ok := composerRegion(screen)
		if !ok {
			return failed()
		}
		previous := stripAnsi(strings.Join(region, "\n"))
		for batch := range hermesClearBatchesPerRound {
			if !host.now().Before(until) {
				return failed()
			}
			keys := hermesClearBackspaceBatch
			if batch == 0 {
				keys = hermesClearFirstBatch
			}
			if err := host.sendKeys(keys); err != nil {
				return err
			}
		}
		host.pause()
		if screen, err = host.capture(); err != nil {
			return err
		}
		current, ok := composerRegion(screen)
		if !ok || stripAnsi(strings.Join(current, "\n")) == previous {
			return failed()
		}
	}
}

func waitFor[T any](host hermesSettlementHost, until time.Time, timeout string, done func(string) (T, bool)) (T, error) {
	value, ok, err := poll(host, until, captureDone(done))
	if err != nil {
		return value, err
	}
	if !ok {
		return value, appError(timeout)
	}
	return value, nil
}

func busy(screen string) bool {
	var nonblank []string
	for _, line := range lines(stripAnsi(screen)) {
		if strings.TrimFunc(line, unicode.IsSpace) != "" {
			nonblank = append(nonblank, line)
		}
	}
	for _, line := range nonblank[max(0, len(nonblank)-hermesBusyTailLines):] {
		for _, marker := range hermesBusyMarkers {
			if strings.Contains(line, marker) {
				return true
			}
		}
	}
	return false
}

var hermesStopReport = regexp.MustCompile(`Stopped [0-9]+ process\(es\)\.|No running background processes\.`)

func stopReports(screen string) int {
	return len(hermesStopReport.FindAllStringIndex(stripAnsi(screen), -1))
}

var hermesExitSummary = regexp.MustCompile(`(?m)^Session:[\t\n\v\f\r ]+(local-[0-9a-f]+)[\t\n\v\f\r ]*$`)

func exitSummarySession(screen string) (string, bool) {
	all := hermesExitSummary.FindAllStringSubmatch(stripAnsi(screen), -1)
	if len(all) == 0 {
		return "", false
	}
	return all[len(all)-1][1], true
}

type registeredProcess struct {
	Pid           *uint32 `json:"pid"`
	HostStartTime *uint64 `json:"host_start_time"`
	OwnerTaskID   *string `json:"owner_task_id"`
}

func decodeRegistry(text string) ([]registeredProcess, error) {
	if !strings.HasPrefix(strings.TrimLeft(text, " \t\n\r"), "[") {
		return nil, appError("invalid type: expected a sequence")
	}
	var processes []registeredProcess
	if err := json.Unmarshal([]byte(text), &processes); err != nil {
		return nil, appError(err.Error())
	}
	for _, p := range processes {
		switch {
		case p.Pid == nil:
			return nil, appError("missing field `pid`")
		case p.HostStartTime == nil:
			return nil, appError("missing field `host_start_time`")
		case p.OwnerTaskID == nil:
			return nil, appError("missing field `owner_task_id`")
		}
	}
	return processes, nil
}

func verifyProcesses(host hermesSettlementHost, home, sessionID string) error {
	text, present, err := host.processesJSON(home)
	if err != nil || !present {
		return err
	}
	registry := joinPath(home, "processes.json")
	processes, err := decodeRegistry(text)
	if err != nil {
		return appErrorf("%s is not a Hermes process registry: %s", registry, cafleetErrorOf(err).message())
	}
	var running []string
	for _, p := range processes {
		if *p.OwnerTaskID != sessionID {
			continue
		}
		alive, err := host.registeredProcessRunning(*p.Pid, *p.HostStartTime)
		if err != nil {
			return err
		}
		if alive {
			running = append(running, strconv.FormatUint(uint64(*p.Pid), 10))
		}
	}
	if len(running) == 0 {
		return nil
	}
	return appErrorf("background processes of session %s are still running: %s", sessionID, strings.Join(running, ", "))
}

func composerRegion(screen string) ([]string, bool) {
	rows := lines(screen)
	stripped := make([]string, len(rows))
	for i, row := range rows {
		stripped[i] = stripAnsi(row)
	}
	allRule := func(row string) bool {
		return row != "" && strings.Trim(row, "─") == ""
	}
	bottom := -1
	for i := len(stripped) - 1; i >= 0; i-- {
		if allRule(stripped[i]) {
			bottom = i
			break
		}
	}
	if bottom < 0 {
		return nil, false
	}
	width := utf8.RuneCountInString(stripped[bottom])
	for top := bottom - 1; top >= 0; top-- {
		row := stripped[top]
		if strings.HasPrefix(row, "─") && strings.HasSuffix(row, "─") && utf8.RuneCountInString(row) == width {
			return rows[top+1 : bottom], true
		}
	}
	return nil, false
}

func composerEmpty(screen string) bool {
	region, ok := composerRegion(screen)
	return ok && len(region) == 1 && promptRowEmpty(region[0])
}

var hermesCaptureToken = regexp.MustCompile("\x1b\\[([0-9;]*)m|\x1b\\[[0-?]*[ -/]*[@-~]|\x1b\\][^\x07\x1b]*(?:\x07|\x1b\\\\)|([^\x1b])")

func promptRowEmpty(row string) bool {
	if !strings.HasPrefix(stripAnsi(row), "❯ ") {
		return false
	}
	italic := false
	type visibleChar struct {
		text   string
		italic bool
	}
	var visible []visibleChar
	for _, token := range hermesCaptureToken.FindAllStringSubmatchIndex(row, -1) {
		if token[2] >= 0 {
			codes := strings.Split(row[token[2]:token[3]], ";")
			for i := 0; i < len(codes); i++ {
				switch codes[i] {
				case "", "0", "23":
					italic = false
				case "3":
					italic = true
				// Extended colours carry their arguments in the following parameters.
				case "38", "48", "58":
					if i+1 < len(codes) {
						i++
						switch codes[i] {
						case "5":
							i++
						case "2":
							i += 3
						}
					}
				}
			}
		} else if token[4] >= 0 {
			visible = append(visible, visibleChar{text: row[token[4]:token[5]], italic: italic})
		}
	}
	for i, v := range visible {
		if i >= 2 && !v.italic && strings.TrimFunc(v.text, unicode.IsSpace) != "" {
			return false
		}
	}
	return true
}

// effectiveHome is the effective Hermes home for the configuration base: the directory of the
// file `hermes config path` names, which follows a sticky active_profile.
func effectiveHome(hermes hermesConfig, until time.Time) (string, error) {
	config, err := hermes.configPath(until)
	if err == nil {
		var home string
		if home, err = homeOfConfigPath(config); err == nil {
			return home, nil
		}
	}
	return "", appError("cannot read hermes configuration through 'hermes config path': " + cafleetErrorOf(err).message())
}

// homeOfConfigPath is the Hermes home that owns the file `hermes config path` printed.
func homeOfConfigPath(configPath string) (string, error) {
	parent := rustParent(configPath)
	if parent == "" || !filepath.IsAbs(parent) {
		return "", appError("printed no absolute file path: " + configPath)
	}
	return parent, nil
}

// rustParent is Rust's Path::parent: the path without its final component, "" when the path has
// none or is a root.
func rustParent(path string) string {
	trimmed := strings.TrimRight(path, "/")
	if trimmed == "" {
		return ""
	}
	index := strings.LastIndex(trimmed, "/")
	switch {
	case index < 0:
		if trimmed == "." || trimmed == ".." {
			return ""
		}
		return "."
	case index == 0:
		return "/"
	default:
		return strings.TrimRight(trimmed[:index], "/")
	}
}
