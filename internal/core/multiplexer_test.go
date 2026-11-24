package core

import (
	"fmt"
	"slices"
	"strings"
	"testing"
	"time"
)

type fakeResponse struct {
	out string
	err *runError
}

// fakeRunner answers each process call from its queued responses in order and records every
// argv. An unqueued call fails the test and returns a failed run.
type fakeRunner struct {
	t         *testing.T
	binaries  []string
	calls     [][]string
	responses []fakeResponse
}

var _ commandRunner = (*fakeRunner)(nil)

func fakeRunnerWithBinary(t *testing.T, name string) *fakeRunner {
	t.Helper()
	return &fakeRunner{t: t, binaries: []string{name}}
}

func fakeRunnerWithoutBinaries(t *testing.T) *fakeRunner {
	t.Helper()
	return &fakeRunner{t: t}
}

func (f *fakeRunner) succeed(calls int) {
	for range calls {
		f.respond("")
	}
}

func (f *fakeRunner) respond(out string) {
	f.responses = append(f.responses, fakeResponse{out: out})
}

func (f *fakeRunner) fail(err *runError) {
	f.responses = append(f.responses, fakeResponse{err: err})
}

func (f *fakeRunner) binaryExists(name string) bool {
	return slices.Contains(f.binaries, name)
}

func (f *fakeRunner) run(argv []string) (string, *runError) {
	f.calls = append(f.calls, slices.Clone(argv))
	if len(f.responses) == 0 {
		f.t.Errorf("unexpected process call: %q", argv)
		return "", failedRun("unexpected process call")
	}
	response := f.responses[0]
	f.responses = f.responses[1:]
	return response.out, response.err
}

func (f *fakeRunner) runFor(argv []string, _ time.Duration) (string, *runError) {
	return f.run(argv)
}

func (f *fakeRunner) runArgvs() [][]string {
	return slices.Clone(f.calls)
}

func failedRun(stderr string) *runError {
	return &runError{kind: runErrorFailed, detail: stderr}
}

func assertCalls(t *testing.T, name string, got, want [][]string) {
	t.Helper()
	if !slices.EqualFunc(got, want, slices.Equal[[]string]) {
		t.Fatalf("%s: process calls\n%q\nwant\n%q", name, got, want)
	}
}

func encodedText(t *testing.T, value any) string {
	t.Helper()
	encoded, err := encodeJSON(value)
	if err != nil {
		t.Fatal(err)
	}
	return string(encoded)
}

// herdrEnvelope is a herdr success envelope wrapping result.
func herdrEnvelope(t *testing.T, result any) string {
	t.Helper()
	return encodedText(t, map[string]any{"id": 1, "result": result, "type": "response"})
}

// herdrErrorStderr is a herdr error body carrying code, for a failed run.
func herdrErrorStderr(t *testing.T, code string) string {
	t.Helper()
	return encodedText(t, map[string]any{"error": map[string]any{"code": code, "message": "boom"}, "id": 1})
}

func mustPressKeys(t *testing.T, tokens ...string) pressKeys {
	t.Helper()
	keys, err := pressKeysParse(tokens)
	if err != nil {
		t.Fatal(err)
	}
	return keys
}

// typeLiteral types literal as plain text, without stopping between writes.
func typeLiteral(m multiplexer, pane, literal string) *multiplexerError {
	_, err := m.typeLiteralUntil(pane, literal, func() bool { return false })
	return err
}

const (
	muxTestTmuxSocket   = "/tmp/tmux-1000/default"
	muxTestHerdrSession = "default"
)

func muxTestServer(backend string) string {
	switch backend {
	case "tmux":
		return muxTestTmuxSocket
	case "herdr":
		return muxTestHerdrSession
	default:
		panic("fixture backend")
	}
}

func muxAdapter(t *testing.T, backend string, runner commandRunner) multiplexer {
	t.Helper()
	mux, err := multiplexerForServer(backend, muxTestServer(backend), runner)
	if err != nil {
		t.Fatal(err)
	}
	return mux
}

func TestMultiplexer_MemberPaneNameJoinsTheMemberIdAndNameWithASpacedHyphen(t *testing.T) {
	t.Parallel()
	cases := []struct {
		memberID int64
		name     string
		want     string
	}{
		{12, "drafter", "12 - drafter"},
		{7, "lead reviewer", "7 - lead reviewer"},
		{3, "a - b", "3 - a - b"},
		{1000, " padded ", "1000 -  padded "},
	}
	for _, tc := range cases {
		if got := memberPaneName(tc.memberID, tc.name); got != tc.want {
			t.Fatalf("memberPaneName(%d, %q) = %q, want %q", tc.memberID, tc.name, got, tc.want)
		}
	}
}

func TestMultiplexer_PressAdaptersRequestOnlyTheOrderedKeysAndQualifyStdoutAcknowledgment(t *testing.T) {
	t.Parallel()
	keys := mustPressKeys(t, "A", "a", "+", "-", ";", "Space", "Escape", "Enter", "Tab", "Backspace", "Up", "F12", "Ctrl+A", "Shift+Tab")
	cases := []struct {
		backend string
		pane    string
		want    []string
	}{
		{backend: "tmux", pane: "%2", want: []string{
			"tmux", "-S", muxTestTmuxSocket, "send-keys", "-t", "%2",
			"0x41", "0x61", "0x2b", "0x2d", "0x3b", "Space", "Escape", "Enter", "Tab", "BSpace", "Up", "F12", "C-a", "BTab",
		}},
		{backend: "herdr", pane: "w1:p2", want: []string{
			"herdr", "--session", muxTestHerdrSession, "pane", "send-keys", "w1:p2",
			"A", "a", "plus", "minus", "semicolon", "space", "esc", "enter", "tab", "backspace", "up", "f12", "ctrl+a", "shift+tab",
		}},
	}
	for _, tc := range cases {
		t.Run(tc.backend, func(t *testing.T) {
			t.Parallel()
			runner := fakeRunnerWithBinary(t, tc.backend)
			runner.respond(" \n\t")
			if err := muxAdapter(t, tc.backend, runner).sendKeys(tc.pane, keys, newDeadline()); err != nil {
				t.Fatalf("%s: %v", tc.backend, err)
			}
			calls := runner.runArgvs()
			if len(calls) != 1 {
				t.Fatalf("%s: %d process calls, want 1", tc.backend, len(calls))
			}
			if tc.backend == "tmux" {
				for index := 6; index < 11; index++ {
					value := calls[0][index]
					if !strings.HasPrefix(value, "0x") || len(value) != 4 {
						t.Fatalf("%s: literal key %q is not a 0xHH byte", tc.backend, value)
					}
					calls[0][index] = strings.ToLower(value)
				}
			}
			assertCalls(t, tc.backend, calls, [][]string{tc.want})
			if len(runner.responses) != 0 {
				t.Fatalf("%s: unused responses %v", tc.backend, runner.responses)
			}
			unreadable := fakeRunnerWithBinary(t, tc.backend)
			unreadable.respond("unexpected acknowledgment")
			err := muxAdapter(t, tc.backend, unreadable).sendKeys(tc.pane, keys, newDeadline())
			if err == nil || !err.submissionStarted {
				t.Fatalf("%s: non-whitespace stdout leaves actual effects uncertain: %v", tc.backend, err)
			}
			if len(unreadable.runArgvs()) != 1 {
				t.Fatalf("%s: %d process calls, want 1", tc.backend, len(unreadable.runArgvs()))
			}
		})
	}
}

func muxTestPlacement(backend string) placement {
	pane := "w1:p2"
	if backend == "tmux" {
		pane = "%2"
	}
	return placement{
		backend:       backend,
		muxSession:    "main",
		muxWindowID:   some("@1"),
		muxWindowName: some("main"),
		windowOrdinal: 0,
		columnSide:    columnSideRight,
		muxPaneID:     some(pane),
		codingAgent:   "codex",
		createdAt:     "2026-10-27T10:33:00+00:00",
		muxServer:     muxTestServer(backend),
	}
}

func assertMultiplexerError(t *testing.T, name string, err *multiplexerError, message string) {
	t.Helper()
	if err == nil {
		t.Fatalf("%s: succeeded, want %q", name, message)
	}
	if err.Error() != message {
		t.Fatalf("%s: error %q, want %q", name, err.Error(), message)
	}
}

func TestMultiplexer_PressTargetValidationReadsOnlyTheRecordedTargetOnTheRecordedServer(t *testing.T) {
	t.Parallel()
	tmuxTarget := []string{"tmux", "-S", muxTestTmuxSocket, "display-message", "-p", "-t", "%2", "#{pane_id}|#{window_id}|#{session_name}"}
	valid := fakeRunnerWithBinary(t, "tmux")
	valid.respond("%2|@1|main\n")
	if err := validateKeyTarget(muxAdapter(t, "tmux", valid), muxTestPlacement("tmux"), newDeadline()); err != nil {
		t.Fatal(err)
	}
	assertCalls(t, "tmux valid", valid.runArgvs(), [][]string{tmuxTarget})

	moved := fakeRunnerWithBinary(t, "tmux")
	moved.respond("%2|@9|main\n")
	assertMultiplexerError(t, "tmux moved", validateKeyTarget(muxAdapter(t, "tmux", moved), muxTestPlacement("tmux"), newDeadline()), placementMismatch)
	assertCalls(t, "tmux moved", moved.runArgvs(), [][]string{tmuxTarget})

	herdrList := []string{"herdr", "--session", muxTestHerdrSession, "pane", "list"}
	pane := map[string]any{"workspace_id": "main", "tab_id": "@1", "pane_id": "w1:p2"}
	herdrValid := fakeRunnerWithBinary(t, "herdr")
	herdrValid.respond(herdrEnvelope(t, map[string]any{"panes": []any{pane}}))
	if err := validateKeyTarget(muxAdapter(t, "herdr", herdrValid), muxTestPlacement("herdr"), newDeadline()); err != nil {
		t.Fatal(err)
	}
	assertCalls(t, "herdr valid", herdrValid.runArgvs(), [][]string{herdrList})

	duplicate := fakeRunnerWithBinary(t, "herdr")
	duplicate.respond(herdrEnvelope(t, map[string]any{"panes": []any{pane, pane}}))
	assertMultiplexerError(t, "herdr duplicate", validateKeyTarget(muxAdapter(t, "herdr", duplicate), muxTestPlacement("herdr"), newDeadline()), placementMismatch)
	assertCalls(t, "herdr duplicate", duplicate.runArgvs(), [][]string{herdrList})
}

func TestMultiplexer_ARecordedServerNeedsOnlyTheBinaryAndRejectsAnUnsupportedBackend(t *testing.T) {
	t.Parallel()
	for _, backend := range []string{"tmux", "herdr"} {
		if err := muxAdapter(t, backend, fakeRunnerWithBinary(t, backend)).ensureAvailable(); err != nil {
			t.Fatalf("%s: %v", backend, err)
		}
		assertMultiplexerError(t, backend, muxAdapter(t, backend, fakeRunnerWithoutBinaries(t)).ensureAvailable(), backend+" binary not found on PATH")
	}
	mux, err := multiplexerForServer("screen", "s", fakeRunnerWithoutBinaries(t))
	if mux != nil {
		t.Fatal("an unsupported recorded backend builds no multiplexer")
	}
	assertMultiplexerError(t, "screen", err, "unsupported recorded multiplexer backend 'screen'")
}

func TestMultiplexer_ARecordedTmuxOrHerdrServerReadsTheRequestedPaneAndReportsAMissingOne(t *testing.T) {
	t.Parallel()
	tmuxRead := []string{"tmux", "-S", muxTestTmuxSocket, "display-message", "-p", "-t", "%2", "#{session_name}|#{window_id}|#{pane_id}"}
	for _, tc := range []struct {
		reply string
		found bool
	}{{"main|@1|%2\n", true}, {"||\n", false}, {"main|@1|%3\n", false}} {
		runner := fakeRunnerWithBinary(t, "tmux")
		runner.respond(tc.reply)
		context, err := muxAdapter(t, "tmux", runner).paneContext("%2")
		if err != nil {
			t.Fatalf("%q: %v", tc.reply, err)
		}
		if _, found := context.get(); found != tc.found {
			t.Fatalf("%q: found = %v, want %v", tc.reply, found, tc.found)
		}
		assertCalls(t, tc.reply, runner.runArgvs(), [][]string{tmuxRead})
	}

	runner := fakeRunnerWithBinary(t, "herdr")
	runner.respond(herdrEnvelope(t, map[string]any{"pane": map[string]any{"workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p2"}}))
	context, err := muxAdapter(t, "herdr", runner).paneContext("w1:p2")
	if err != nil {
		t.Fatal(err)
	}
	if want := some(multiplexerContext{session: "w1", windowID: "w1:t1", paneID: "w1:p2"}); context != want {
		t.Fatalf("herdr context %#v, want %#v", context, want)
	}
	assertCalls(t, "herdr current", runner.runArgvs(), [][]string{{"herdr", "--session", muxTestHerdrSession, "pane", "current", "--pane", "w1:p2"}})
	missing := fakeRunnerWithBinary(t, "herdr")
	missing.fail(failedRun(herdrErrorStderr(t, "pane_not_found")))
	context, err = muxAdapter(t, "herdr", missing).paneContext("w1:p9")
	if _, found := context.get(); err != nil || found {
		t.Fatalf("a missing herdr pane reads as none: %#v %v", context, err)
	}
}

// timeoutRecorder records each runFor timeout and refuses unbounded runs.
type timeoutRecorder struct {
	t        *testing.T
	timeouts []time.Duration
}

func (r *timeoutRecorder) binaryExists(string) bool {
	return true
}

func (r *timeoutRecorder) run(argv []string) (string, *runError) {
	r.t.Errorf("press bounds every process by the invocation deadline: %q", argv)
	return "", failedRun("unbounded run")
}

func (r *timeoutRecorder) runFor(_ []string, timeout time.Duration) (string, *runError) {
	r.timeouts = append(r.timeouts, timeout)
	return "", nil
}

func TestMultiplexer_PressDeadlineRunnerClipsEachCallToTheRemainingDeadlineAndRefusesExpiry(t *testing.T) {
	t.Parallel()
	recorder := &timeoutRecorder{t: t}
	bounded := deadlineRunnerWrap(recorder, deadlineUntil(time.Now().Add(2*time.Second)))
	if _, err := bounded.runFor([]string{"fixture"}, 7*time.Second); err != nil {
		t.Fatal(err)
	}
	if _, err := bounded.runFor([]string{"fixture"}, 500*time.Millisecond); err != nil {
		t.Fatal(err)
	}
	if len(recorder.timeouts) != 2 || recorder.timeouts[0] <= time.Second || recorder.timeouts[0] > 2*time.Second || recorder.timeouts[1] != 500*time.Millisecond {
		t.Fatalf("recorded timeouts %v, want about 2s then 500ms", recorder.timeouts)
	}
	expired := deadlineRunnerWrap(recorder, deadlineUntil(time.Now()))
	if _, err := expired.runFor([]string{"fixture"}, time.Second); err == nil {
		t.Fatal("expired work starts no underlying command")
	}
	if len(recorder.timeouts) != 2 {
		t.Fatalf("an expired deadline started a command: %v", recorder.timeouts)
	}
}

func muxEnv(pairs map[string]string) func(string) opt[string] {
	return func(name string) opt[string] {
		if value, ok := pairs[name]; ok {
			return some(value)
		}
		return none[string]()
	}
}

func TestMultiplexer_AutoDetectionResolvesTheSinglePresentBackendOrFails(t *testing.T) {
	t.Parallel()
	const ambiguous = "ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; unset the one that does not belong to this session"
	const absent = "no supported multiplexer detected: neither HERDR_ENV nor TMUX is set; run cafleet inside a tmux or herdr session"
	cases := []struct {
		name  string
		pairs map[string]string
		want  muxBackend
		fails string
	}{
		{name: "herdr", pairs: map[string]string{"HERDR_ENV": "1"}, want: muxBackendHerdr},
		{name: "tmux", pairs: map[string]string{"TMUX": "/tmp/tmux-1000/default,123,0"}, want: muxBackendTmux},
		{name: "empty values", pairs: map[string]string{"HERDR_ENV": "", "TMUX": ""}, fails: absent},
		{name: "both", pairs: map[string]string{"HERDR_ENV": "1", "TMUX": "/tmp/tmux"}, fails: ambiguous},
		{name: "neither", pairs: map[string]string{}, fails: absent},
	}
	for _, tc := range cases {
		backend, err := muxBackendDetect(muxEnv(tc.pairs))
		if tc.fails != "" {
			assertMultiplexerError(t, tc.name, err, tc.fails)
			continue
		}
		if err != nil || backend != tc.want {
			t.Fatalf("%s: detected %v (%v), want %v", tc.name, backend, err, tc.want)
		}
	}
}

func TestMultiplexer_TmuxCallerServerIsTheTmuxSocketPathBeforeItsLastTwoCommas(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "tmux")
	server, err := callerServer(muxBackendTmux, muxEnv(map[string]string{"TMUX": "/tmp/a,b/default,123,0"}), runner)
	if err != nil || server != "/tmp/a,b/default" {
		t.Fatalf("caller server %q (%v), want /tmp/a,b/default", server, err)
	}
	assertCalls(t, "tmux socket from TMUX", runner.runArgvs(), nil)
}

func TestMultiplexer_TmuxCallerServerWithoutTmuxAsksTheDefaultServerForItsSocket(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "tmux")
	runner.respond("/tmp/tmux-0/default\n")
	server, err := callerServer(muxBackendTmux, muxEnv(map[string]string{"TMUX": ""}), runner)
	if err != nil || server != "/tmp/tmux-0/default" {
		t.Fatalf("caller server %q (%v), want /tmp/tmux-0/default", server, err)
	}
	assertCalls(t, "default socket", runner.runArgvs(), [][]string{defaultTmuxSocket})
	for _, tc := range []struct {
		name  string
		reply fakeResponse
	}{
		{name: "no server running", reply: fakeResponse{err: failedRun("no server running on /tmp/tmux-0/default")}},
		{name: "empty reply", reply: fakeResponse{out: "\n"}},
	} {
		runner := fakeRunnerWithBinary(t, "tmux")
		runner.responses = append(runner.responses, tc.reply)
		_, err := callerServer(muxBackendTmux, muxEnv(map[string]string{}), runner)
		assertMultiplexerError(t, tc.name, err, "no tmux server found; pass --mux-server <socket path>")
		assertCalls(t, tc.name, runner.runArgvs(), [][]string{defaultTmuxSocket})
	}
}

func TestMultiplexer_HerdrCallerServerPrefersTheSessionThenTheMatchingSocketThenDefault(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "herdr")
	for _, tc := range []struct {
		pairs map[string]string
		want  string
	}{
		{pairs: map[string]string{"HERDR_SESSION": "work", "HERDR_SOCKET_PATH": "/run/other.sock"}, want: "work"},
		{pairs: map[string]string{"HERDR_SESSION": ""}, want: "default"},
	} {
		server, err := callerServer(muxBackendHerdr, muxEnv(tc.pairs), runner)
		if err != nil || server != tc.want {
			t.Fatalf("%v: caller server %q (%v), want %q", tc.pairs, server, err, tc.want)
		}
	}
	assertCalls(t, "session from the environment", runner.runArgvs(), nil)
	listing := encodedText(t, map[string]any{"sessions": []any{
		map[string]any{"name": "default", "status": "running", "socket_path": "/run/herdr.sock"},
		map[string]any{"name": "work", "status": "running", "socket_path": "/run/work.sock"},
	}})
	for _, tc := range []struct{ socket, want string }{{"/run/work.sock", "work"}, {"/run/gone.sock", "default"}} {
		runner := fakeRunnerWithBinary(t, "herdr")
		runner.respond(listing)
		server, err := callerServer(muxBackendHerdr, muxEnv(map[string]string{"HERDR_SOCKET_PATH": tc.socket}), runner)
		if err != nil || server != tc.want {
			t.Fatalf("%s: caller server %q (%v), want %q", tc.socket, server, err, tc.want)
		}
		assertCalls(t, tc.socket, runner.runArgvs(), [][]string{{"herdr", "session", "list", "--json"}})
	}
}

func TestMultiplexer_CaptureWindowDropsOnlyTheVisuallyBlankTailThenKeepsTheLastLines(t *testing.T) {
	t.Parallel()
	cases := []struct {
		raw   string
		lines int64
		want  string
	}{
		{"l1\nl2\nl3\nx\rY\n", 2, "l3\nx\rY"},
		{"drawn1\ndrawn2\n\n\n\n\n\n\n", 2, "drawn1\ndrawn2"},
		{"scrollback\nprompt>\ncomposer\n \n\t\n\n\n", 2, "prompt>\ncomposer"},
		{"a\n\nb\n\n\n", 3, "a\n\nb"},
		{"top\nkept \x1b[31mred\x1b[0m\n\x1b[39m\x1b[49m\n\x1b[2K \n\n", 2, "top\nkept \x1b[31mred\x1b[0m"},
		{"one\ntwo\x1b[0m\n\x1b[39m\x1b[49m\n \x1b[2K\n", 2, "one\ntwo\x1b[0m"},
		{"one\ntwo\nthree\n\n\n", 2, "two\nthree"},
		{"raw\nbuffer\n\n \n", 20, "raw\nbuffer"},
		{"\n \n\n", 2, ""},
	}
	for _, tc := range cases {
		if got := captureWindow(tc.raw, tc.lines); got != tc.want {
			t.Fatalf("captureWindow(%q, %d) = %q, want %q", tc.raw, tc.lines, got, tc.want)
		}
	}
}

func TestMultiplexer_ExpiredDeadlinesStartNoNativeCommandOrOptionalLayoutRestore(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithoutBinaries(t)
	mux := herdrMultiplexerForServer(runner, muxTestHerdrSession).bounded(deadlineUntil(time.Now()))
	if err := mux.closePane("w1:p2", "w1:t1"); err == nil {
		t.Fatal("an expired close starts no command and fails")
	}
	if err := mux.restoreAfterClose(); err != nil {
		t.Fatal(err)
	}
	assertCalls(t, "expired", runner.runArgvs(), nil)
}

func assertPressUsage(t *testing.T, tokens []string, message string) {
	t.Helper()
	_, err := pressKeysParse(tokens)
	if err == nil {
		t.Fatalf("%q is outside the common grammar", tokens)
	}
	failure := cafleetErrorOf(err)
	if failure.message() != message || failure.exitCode() != 2 {
		t.Fatalf("%q: error %q (exit %d), want the usage error %q", tokens, failure.message(), failure.exitCode(), message)
	}
}

func TestMultiplexer_PressKeysValidateTheCompleteCommonGrammarAndByteCountBounds(t *testing.T) {
	t.Parallel()
	keys := mustPressKeys(t, "A", "a", "+", "-", ";", "!", "~", "sPaCe", "ESCAPE", "enter", "Tab", "Backspace", "up", "down", "left", "right", "f1", "F12", "cTrL+A", "CTRL+Z", "sHiFt+tAb")
	want := []string{"A", "a", "+", "-", ";", "!", "~", "Space", "Escape", "Enter", "Tab", "Backspace", "Up", "Down", "Left", "Right", "F1", "F12", "Ctrl+a", "Ctrl+z", "Shift+Tab"}
	if got := keys.canonical(); !slices.Equal(got, want) {
		t.Fatalf("canonical keys %q, want %q", got, want)
	}
	if got := mustPressKeys(t, slices.Repeat([]string{"a"}, 64)...).canonical(); len(got) != 64 {
		t.Fatalf("64 keys parse to %d", len(got))
	}
	assertPressUsage(t, []string{}, "press requires between 1 and 64 keys")
	assertPressUsage(t, slices.Repeat([]string{"a"}, 65), "press requires between 1 and 64 keys")
	assertPressUsage(t, []string{"Enter", strings.Repeat("é", 17)}, "press key at position 2 exceeds 32 UTF-8 bytes")
	for _, invalid := range []string{"F0", "F13", "Ctrl+1", " ", "é"} {
		assertPressUsage(t, []string{"a", invalid}, fmt.Sprintf("invalid press key at position 2: %s", encodedText(t, invalid)))
	}
}
