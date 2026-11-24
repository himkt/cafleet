package core

import (
	"encoding/json/v2"
	"fmt"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"time"
)

const (
	hermesTestMember  int64 = 7
	hermesTestHome          = "/srv/hermes/profiles/work"
	hermesTestSession       = "local-ab12cd34"
	hermesTestBusy          = "☤ ❯ type to redirect this turn · Tab queues it for after"
	hermesTestIdle          = "❯ Turn these notes into a to-do list"
)

func hermesFrame(region string) string {
	border := strings.Repeat("─", 80)
	return border + "\n" + region + "\n" + border
}

// hermesFrameDraft is the composer region holding draft, wrapped at 78 characters, or the
// placeholder when draft is empty.
func hermesFrameDraft(draft string) string {
	if draft == "" {
		return hermesFrame("❯ \x1b[3mtype a message\x1b[23m")
	}
	var rows []string
	for _, line := range strings.Split(draft, "\n") {
		characters := []rune(line)
		if len(characters) == 0 {
			rows = append(rows, "")
			continue
		}
		for start := 0; start < len(characters); start += 78 {
			rows = append(rows, string(characters[start:min(start+78, len(characters))]))
		}
	}
	rows[0] = "❯ " + rows[0]
	return hermesFrame(strings.Join(rows, "\n"))
}

// hermesHost is a Hermes client pane: keys edit its composer, and Enter submits the composer text
// unless it arrives before the completion menu rendered (droppedEnters).
type hermesHost struct {
	screen         []string
	composer       string
	interruptStops bool
	reportsStop    bool
	printsSummary  bool
	exitsOnExit    bool
	exited         bool
	droppedEnters  int
	interrupts     int
	enters         int
	submitted      []string
	recorded       []string
	homes          []string
	registry       opt[string]
	live           [][2]uint64
	checked        [][2]uint64
	pauses         int
	origin         time.Time
}

func newWorkingHermesHost() *hermesHost {
	return &hermesHost{
		screen:         []string{"│  Session: ab12cd34", "❯ Run the long task", hermesTestBusy},
		interruptStops: true,
		reportsStop:    true,
		printsSummary:  true,
		exitsOnExit:    true,
		origin:         time.Now(),
	}
}

func (h *hermesHost) until() time.Time { return h.origin.Add(10 * time.Second) }

func (h *hermesHost) submit() {
	command := h.composer
	h.composer = ""
	switch {
	case command == "/stop" && h.reportsStop:
		h.screen = append(h.screen, "Stopping 1 background process(es)...", "✅ Stopped 1 process(es).")
	case command == "/exit":
		if h.printsSummary {
			h.screen = append(h.screen, "  hermes --resume "+hermesTestSession, "Session: "+hermesTestSession)
		}
		h.exited = h.exitsOnExit
	}
	h.submitted = append(h.submitted, command)
}

func (h *hermesHost) clientExited() (bool, error) { return h.exited, nil }

func (h *hermesHost) sendKeys(keys []string) error {
	if h.exited {
		panic("keys reach only a live client")
	}
	for _, key := range keys {
		switch key {
		case "Ctrl+C":
			h.interrupts++
			if h.interruptStops {
				h.screen = slices.DeleteFunc(h.screen, func(line string) bool { return line == hermesTestBusy })
				h.screen = append(h.screen, hermesTestIdle)
			}
		case "Escape":
		case "Backspace":
			if runes := []rune(h.composer); len(runes) > 0 {
				h.composer = string(runes[:len(runes)-1])
			}
		case "Enter":
			h.enters++
			if h.droppedEnters > 0 {
				h.droppedEnters--
			} else {
				h.submit()
			}
		default:
			if len([]rune(key)) != 1 {
				panic("unexpected key " + key)
			}
			h.composer += key
		}
	}
	return nil
}

func (h *hermesHost) capture() (string, error) {
	return strings.Join(h.screen, "\n") + "\n" + hermesFrameDraft(h.composer), nil
}

func (h *hermesHost) recordSessionID(sessionID string) error {
	h.recorded = append(h.recorded, sessionID)
	return nil
}

func (h *hermesHost) processesJSON(home string) (string, bool, error) {
	h.homes = append(h.homes, home)
	text, ok := h.registry.get()
	return text, ok, nil
}

func (h *hermesHost) registeredProcessRunning(pid uint32, hostStartTime uint64) (bool, error) {
	entry := [2]uint64{uint64(pid), hostStartTime}
	h.checked = append(h.checked, entry)
	return slices.Contains(h.live, entry), nil
}

func (h *hermesHost) pause() { h.pauses++ }

func (h *hermesHost) now() time.Time {
	return h.origin.Add(time.Duration(h.pauses) * 100 * time.Millisecond)
}

func hermesSession(sessionID opt[string]) nativeSession {
	return nativeSession{hermesHome: hermesTestHome, sessionID: sessionID}
}

type hermesRegistryEntry struct {
	pid   uint32
	start uint64
	owner string
}

func hermesRegistry(entries ...hermesRegistryEntry) opt[string] {
	array := ja()
	for _, e := range entries {
		array.push(jo("session_id", fmt.Sprintf("proc_%d", e.pid), "pid", uint64(e.pid), "host_start_time", e.start, "owner_task_id", e.owner, "session_key", "tui"))
	}
	return some(testJSONText(array))
}

func (h *hermesHost) settled(recorded opt[string]) (settlement, error) {
	session := hermesSession(recorded)
	return codingAgentHermesSettle(hermesTestMember, &session, h, h.until())
}

func assertSettlementVerified(t testing.TB, label string, outcome settlement, err error) {
	t.Helper()
	if err != nil || outcome != settlementVerified {
		t.Fatalf("%s: verified, got %v %v", label, outcome, err)
	}
}

func assertUnverified(t testing.TB, err error, cause string) {
	t.Helper()
	if err == nil {
		t.Fatalf("an unverified settlement: %s", cause)
	}
	failure := cafleetErrorOf(err)
	want := fmt.Sprintf("hermes work for member %d could not be verified as stopped: %s; member was not deleted.", hermesTestMember, cause)
	if failure.kind != errorApp || failure.exitCode() != 1 || failure.message() != want {
		t.Fatalf("got %q (exit %d), want %q", failure.message(), failure.exitCode(), want)
	}
}

func TestCodingAgentHermes_Cli_ConfigPathRunsUnderTheResolvedBaseAndTrimsItsStdout(t *testing.T) {
	t.Parallel()
	dir := t.TempDir()
	executable := executableStub(t, dir, "hermes", "[ \"$*\" = 'config path' ] || exit 9\nprintf '  %s/config.yaml  \\n' \"$HERMES_HOME\"")
	base := filepath.Join(dir, "hermes home")
	hermes := newHermesCli(executable, base)
	path, err := runStub(t, func() (string, error) { return hermes.configPath(time.Now().Add(5 * time.Second)) })
	if err != nil || path != base+"/config.yaml" {
		t.Fatalf("config path %q, %v", path, err)
	}
}

type clearingEvent struct {
	keys    []string
	pause   bool
	capture bool
}

// clearingHost records every key batch, pause and capture of a settlement over hermesHost, with a
// simulated clock.
type clearingHost struct {
	inner           *hermesHost
	trace           []clearingEvent
	elapsed         time.Duration
	sendCost        time.Duration
	frozen          bool
	changingColours bool
	captures        int
	firstEmptyAt    opt[time.Duration]
}

func newClearingHost(draft string) *clearingHost {
	inner := newWorkingHermesHost()
	inner.screen = []string{hermesTestIdle}
	inner.composer = draft
	return &clearingHost{inner: inner, sendCost: 50 * time.Millisecond}
}

func (h *clearingHost) clearBatches() [][]string {
	var batches [][]string
	for _, event := range h.trace {
		if slices.Contains(event.keys, "Backspace") {
			batches = append(batches, event.keys)
		}
	}
	return batches
}

func (h *clearingHost) settle(until time.Time) (settlement, error) {
	session := hermesSession(none[string]())
	return codingAgentHermesSettle(hermesTestMember, &session, h, until)
}

func (h *clearingHost) clientExited() (bool, error) { return h.inner.clientExited() }

func (h *clearingHost) sendKeys(keys []string) error {
	h.trace = append(h.trace, clearingEvent{keys: slices.Clone(keys)})
	h.elapsed += h.sendCost
	if h.frozen {
		return h.inner.sendKeys(slices.DeleteFunc(slices.Clone(keys), func(key string) bool { return key == "Backspace" }))
	}
	return h.inner.sendKeys(keys)
}

func (h *clearingHost) capture() (string, error) {
	h.trace = append(h.trace, clearingEvent{capture: true})
	h.captures++
	if _, seen := h.firstEmptyAt.get(); h.inner.composer == "" && !seen {
		h.firstEmptyAt = some(h.elapsed)
	}
	region := hermesFrameDraft(h.inner.composer)
	if h.changingColours {
		region = fmt.Sprintf("\x1b[3%dm%s\x1b[0m", h.captures%8, region)
	}
	return strings.Join(h.inner.screen, "\n") + "\n" + region, nil
}

func (h *clearingHost) recordSessionID(sessionID string) error {
	return h.inner.recordSessionID(sessionID)
}

func (h *clearingHost) processesJSON(home string) (string, bool, error) {
	return h.inner.processesJSON(home)
}

func (h *clearingHost) registeredProcessRunning(pid uint32, start uint64) (bool, error) {
	return h.inner.registeredProcessRunning(pid, start)
}

func (h *clearingHost) pause() {
	h.trace = append(h.trace, clearingEvent{pause: true})
	h.elapsed += 200 * time.Millisecond
}

func (h *clearingHost) now() time.Time { return h.inner.origin.Add(h.elapsed) }

func TestCodingAgentHermes_Settlement_ComposerReliability_ComposerRegionUsesSgrToDistinguishPlaceholdersFromTypedText(t *testing.T) {
	t.Parallel()
	cases := []struct {
		row   string
		empty bool
	}{
		{"❯ ", true},
		{"\x1b[1m❯ \x1b[0m", true},
		{"\x1b[1m❯ \x1b[0m\x1b[3;2mhint\x1b[0m", true},
		{"❯ \x1b[3mhint\x1b[23m", true},
		{"❯ \x1b[3mhint\x1b[23m   ", true},
		{"❯ typed", false},
		{"❯ \x1b[3mhint\x1b[23mx", false},
		{"❯ \x1b[3mhint\x1b[0mx", false},
		{"❯ \x1b[38;5;3mtyped\x1b[0m", false},
		{"❯ \x1b[38;2;3;3;3mtyped\x1b[0m", false},
	}
	for _, c := range cases {
		if got := composerEmpty(hermesFrame(c.row)); got != c.empty {
			t.Fatalf("%q: empty %v, want %v", c.row, got, c.empty)
		}
	}
}

func TestCodingAgentHermes_Settlement_ComposerReliability_ComposerRegionWithMultipleRowsIncludingBlankRowsIsNonempty(t *testing.T) {
	t.Parallel()
	for _, region := range []string{"❯ \n", "❯ \n ", "❯ wrapped\nliteral", "❯ \x1b[3mhint\x1b[23m\n"} {
		if composerEmpty(hermesFrame(region)) {
			t.Fatalf("%q is nonempty", region)
		}
	}
	if composerEmpty("❯ \x1b[3mhint\x1b[23m") {
		t.Fatal("an unframed prompt row is no live composer")
	}
}

func TestCodingAgentHermes_Settlement_ComposerReliability_ComposerRegionSelectsTheLiveBordersInsteadOfHistoricalPromptRows(t *testing.T) {
	t.Parallel()
	history := "❯ earlier submitted prompt\n❯ typed history"
	if !composerEmpty(history + "\n" + hermesFrame("❯ \x1b[3mhint\x1b[23m")) {
		t.Fatal("history above the live placeholder")
	}
	if composerEmpty(hermesFrame("❯ ") + "\n" + hermesFrame("❯ retained draft")) {
		t.Fatal("the lower frame holds a retained draft")
	}
	titled := "─ Composer " + strings.Repeat("─", 69) + "\n❯ \x1b[3mhint\x1b[23m\n" + strings.Repeat("─", 80)
	if !composerEmpty(titled) {
		t.Fatal("a titled top border")
	}
}

func TestCodingAgentHermes_Settlement_ComposerReliability_MeasuredLongHermesComposersRemainNonempty(t *testing.T) {
	t.Parallel()
	for _, id := range []string{
		"hermes-2026.9.24-composer-prompt-paste-2057-literal-tapped-v2-tmux-120-15",
		"hermes-2026.9.24-composer-prompt-paste-2057-literal-tapped-v2-herdr-119-15",
		"hermes-2026.9.24-composer-prompt-clear-backspace-rounds-v3-8192-input-pending-tapped-v2-tmux-120-56",
		"hermes-2026.9.24-composer-prompt-clear-backspace-rounds-v3-8192-input-pending-tapped-v2-herdr-119-53",
	} {
		if composerEmpty(strings.Join(fixtureLines(id), "\n")) {
			t.Fatalf("%s: a long composer is nonempty", id)
		}
	}
}

func TestCodingAgentHermes_Settlement_ComposerReliability_HermesClearLoopUsesSixteenKeyBatchesThenOnePausePerRound(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name   string
		draft  string
		rounds int
	}{
		{"2057 ASCII", strings.Repeat("x", 2057), 3},
		{"8192 ASCII", strings.Repeat("x", 8192), 9},
		{"8192 wide", strings.Repeat("あ", 8192), 9},
		{"two lines", strings.Repeat("x", 4095) + "\n" + strings.Repeat("y", 4096), 9},
	}
	for _, c := range cases {
		host := newClearingHost(c.draft)
		outcome, err := host.settle(host.inner.origin.Add(20 * time.Second))
		assertSettlementVerified(t, c.name, outcome, err)
		if !slices.Equal(host.inner.submitted, []string{"/stop", "/exit"}) {
			t.Fatalf("%s: submitted %q", c.name, host.inner.submitted)
		}
		batches := host.clearBatches()
		if len(batches) != c.rounds*16 {
			t.Fatalf("%s: %d clear batches, want %d", c.name, len(batches), c.rounds*16)
		}
		for start := 0; start < len(batches); start += 16 {
			for index, batch := range batches[start : start+16] {
				first := "Backspace"
				if index == 0 {
					first = "Escape"
				}
				if len(batch) != 64 || batch[0] != first || slices.ContainsFunc(batch[1:], func(key string) bool { return key != "Backspace" }) {
					t.Fatalf("%s: round batch %d is %d keys starting %q", c.name, index, len(batch), batch[0])
				}
			}
		}
		for start, event := range host.trace {
			if len(event.keys) == 0 || event.keys[0] != "Escape" || !slices.Contains(event.keys, "Backspace") {
				continue
			}
			for _, batch := range host.trace[start : start+16] {
				if batch.keys == nil {
					t.Fatalf("%s: a round is 16 key batches", c.name)
				}
			}
			if !host.trace[start+16].pause || !host.trace[start+17].capture {
				t.Fatalf("%s: a round ends with one pause, then a capture", c.name)
			}
		}
		if at, ok := host.firstEmptyAt.get(); !ok || at > 10*time.Second {
			t.Fatalf("%s: a clear round observes the empty composer, at %v", c.name, at)
		}
	}
}

func TestCodingAgentHermes_Settlement_ComposerReliability_HermesClearLoopComparesStrippedRegionProgress(t *testing.T) {
	t.Parallel()
	host := newClearingHost("retained\nmultiline")
	host.frozen = true
	host.changingColours = true
	_, err := host.settle(host.inner.origin.Add(20 * time.Second))
	assertUnverified(t, err, "the Hermes composer still holds text after clearing")
	if len(host.clearBatches()) != 16 || len(host.inner.submitted) != 0 || host.inner.composer != "retained\nmultiline" {
		t.Fatalf("%d batches, submitted %q, composer %q", len(host.clearBatches()), host.inner.submitted, host.inner.composer)
	}
}

func TestCodingAgentHermes_Settlement_ComposerReliability_HermesClearLoopStopsAtTheSettlementBudgetBeforeTypingCommands(t *testing.T) {
	t.Parallel()
	host := newClearingHost(strings.Repeat("x", 8192))
	until := host.inner.origin.Add(time.Second)
	if _, err := host.settle(until); err == nil {
		t.Fatal("the budget ends the clear loop")
	}
	if host.now().Before(until) || len(host.inner.submitted) != 0 || host.inner.composer == "" {
		t.Fatalf("now %v, submitted %q, composer %d bytes", host.now().Sub(until), host.inner.submitted, len(host.inner.composer))
	}
}

func TestCodingAgentHermes_Settlement_ComposerReliability_HermesClearLoopObservesAnEmptyComposerWithoutBackspaceBatches(t *testing.T) {
	t.Parallel()
	host := newClearingHost("")
	outcome, err := host.settle(host.inner.origin.Add(20 * time.Second))
	assertSettlementVerified(t, "an empty composer", outcome, err)
	if len(host.clearBatches()) != 0 || !slices.Equal(host.inner.submitted, []string{"/stop", "/exit"}) {
		t.Fatalf("%d batches, submitted %q", len(host.clearBatches()), host.inner.submitted)
	}
}

func TestCodingAgentHermes_Settlement_SettlementInterruptsARunningTurnStopsAndExitsThenVerifiesItsOwnProcesses(t *testing.T) {
	t.Parallel()
	host := newWorkingHermesHost()
	host.registry = hermesRegistry(hermesRegistryEntry{4242, 900, hermesTestSession}, hermesRegistryEntry{5151, 901, "local-ffff0000"})
	host.live = [][2]uint64{{5151, 901}}
	outcome, err := host.settled(none[string]())
	assertSettlementVerified(t, "a running turn", outcome, err)
	if host.interrupts != 1 || !slices.Equal(host.submitted, []string{"/stop", "/exit"}) || !slices.Equal(host.recorded, []string{hermesTestSession}) ||
		!slices.Equal(host.homes, []string{hermesTestHome}) {
		t.Fatalf("interrupts %d, submitted %q, recorded %q, homes %q", host.interrupts, host.submitted, host.recorded, host.homes)
	}
	if !slices.Equal(host.checked, [][2]uint64{{4242, 900}}) {
		t.Fatalf("only the session's own entries are checked, got %v", host.checked)
	}
}

func TestCodingAgentHermes_Settlement_SettlementOfAnIdleClientSendsNoInterruptAndAnAbsentRegistryVerifies(t *testing.T) {
	t.Parallel()
	host := newWorkingHermesHost()
	host.screen = []string{"❯ Run the long task", hermesTestIdle}
	outcome, err := host.settled(none[string]())
	assertSettlementVerified(t, "an idle client", outcome, err)
	if host.interrupts != 0 || !slices.Equal(host.submitted, []string{"/stop", "/exit"}) || len(host.checked) != 0 {
		t.Fatalf("interrupts %d, submitted %q, checked %v", host.interrupts, host.submitted, host.checked)
	}
}

func TestCodingAgentHermes_Settlement_SettlementClearsALeftoverDraftBeforeTypingEachCommand(t *testing.T) {
	t.Parallel()
	host := newWorkingHermesHost()
	host.screen = []string{hermesTestIdle}
	host.composer = "a half-typed draft the operator left"
	outcome, err := host.settled(none[string]())
	assertSettlementVerified(t, "a leftover draft", outcome, err)
	if !slices.Equal(host.submitted, []string{"/stop", "/exit"}) {
		t.Fatalf("submitted %q", host.submitted)
	}
}

func TestCodingAgentHermes_Settlement_SettlementPressesEnterAgainWhenTheFirstOneArrivesBeforeTheMenu(t *testing.T) {
	t.Parallel()
	host := newWorkingHermesHost()
	host.droppedEnters = 1
	outcome, err := host.settled(none[string]())
	assertSettlementVerified(t, "one dropped Enter", outcome, err)
	if !slices.Equal(host.submitted, []string{"/stop", "/exit"}) || host.enters != 3 {
		t.Fatalf("one dropped Enter, then one per command: submitted %q, enters %d", host.submitted, host.enters)
	}
	host = newWorkingHermesHost()
	host.droppedEnters = int(^uint(0) >> 1)
	_, err = host.settled(none[string]())
	assertUnverified(t, err, "Hermes did not report /stop")
	if host.enters != hermesMaxSubmits || len(host.submitted) != 0 || host.now().Before(host.until()) {
		t.Fatalf("Enter is retried at most %d times: enters %d, submitted %q", hermesMaxSubmits, host.enters, host.submitted)
	}
}

func TestCodingAgentHermes_Settlement_SettlementOfAClientThatAlreadyExitedWithoutASessionReportsItAndChecksNothing(t *testing.T) {
	t.Parallel()
	host := newWorkingHermesHost()
	host.exited = true
	host.registry = hermesRegistry(hermesRegistryEntry{4242, 900, hermesTestSession})
	host.live = [][2]uint64{{4242, 900}}
	outcome, err := host.settled(none[string]())
	if err != nil || outcome != settlementClientAlreadyExited {
		t.Fatalf("an exited client: %v %v", outcome, err)
	}
	if host.interrupts != 0 || host.enters != 0 || host.composer != "" || len(host.recorded) != 0 || len(host.homes) != 0 || len(host.checked) != 0 {
		t.Fatalf("interrupts %d, enters %d, recorded %q, homes %q, checked %v", host.interrupts, host.enters, host.recorded, host.homes, host.checked)
	}
	want := fmt.Sprintf("warning: hermes client for member %d had already exited; its session work may still be running in the Hermes gateway", hermesTestMember)
	if got := alreadyExitedWarning(hermesTestMember); got != want {
		t.Fatalf("warning %q", got)
	}
}

func TestCodingAgentHermes_Settlement_OneMemberSettlementGetsTwentySecondsFromItsStart(t *testing.T) {
	t.Parallel()
	before := time.Now()
	at := settlementDeadline().instant()
	after := time.Now()
	if at.Before(before.Add(20*time.Second)) || at.After(after.Add(20*time.Second)) {
		t.Fatalf("%v after the call", at.Sub(after))
	}
}

func TestCodingAgentHermes_Settlement_SettlementFailsWhileARegisteredProcessOfTheSessionRunsAndARetryVerifiesIt(t *testing.T) {
	t.Parallel()
	host := newWorkingHermesHost()
	host.registry = hermesRegistry(hermesRegistryEntry{4242, 900, hermesTestSession}, hermesRegistryEntry{4343, 905, hermesTestSession})
	host.live = [][2]uint64{{4242, 900}, {4343, 905}}
	_, err := host.settled(none[string]())
	assertUnverified(t, err, "background processes of session "+hermesTestSession+" are still running: 4242, 4343")
	if !slices.Equal(host.recorded, []string{hermesTestSession}) || !host.exited {
		t.Fatalf("the verified exit records the session: recorded %q, exited %v", host.recorded, host.exited)
	}
	enters := host.enters
	host.live = nil
	outcome, err := host.settled(some(hermesTestSession))
	assertSettlementVerified(t, "a retry", outcome, err)
	if host.enters != enters || !slices.Equal(host.submitted, []string{"/stop", "/exit"}) || !slices.Equal(host.recorded, []string{hermesTestSession}) {
		t.Fatalf("a retry sends no keys to the exited client: enters %d→%d, submitted %q", enters, host.enters, host.submitted)
	}
}

func TestCodingAgentHermes_Settlement_SettlementReportsEachUnansweredControlWithinTheDeadline(t *testing.T) {
	t.Parallel()
	cases := []struct {
		cause  string
		change func(*hermesHost)
	}{
		{"the Hermes turn did not stop after Ctrl-C", func(h *hermesHost) { h.interruptStops = false }},
		{"Hermes did not report /stop", func(h *hermesHost) { h.reportsStop = false }},
		{"Hermes printed no exit summary after /exit", func(h *hermesHost) { h.printsSummary, h.exitsOnExit = false, false }},
		{"the Hermes client did not exit after /exit", func(h *hermesHost) { h.exitsOnExit = false }},
	}
	for _, c := range cases {
		host := newWorkingHermesHost()
		c.change(host)
		_, err := host.settled(none[string]())
		assertUnverified(t, err, c.cause)
		if host.now().Before(host.until()) || !host.now().Before(host.until().Add(200*time.Millisecond)) || len(host.recorded) != 0 {
			t.Fatalf("%s: %d pauses, recorded %q", c.cause, host.pauses, host.recorded)
		}
	}
}

func TestCodingAgentHermes_Settlement_SettlementWaitsForANewStopReportRatherThanOneAlreadyOnScreen(t *testing.T) {
	t.Parallel()
	host := newWorkingHermesHost()
	host.screen = []string{"No running background processes.", hermesTestIdle}
	host.reportsStop = false
	_, err := host.settled(none[string]())
	assertUnverified(t, err, "Hermes did not report /stop")
}

func TestCodingAgentHermes_Settlement_SettlementRejectsARegistryThatIsNotAHermesProcessList(t *testing.T) {
	t.Parallel()
	prefix := fmt.Sprintf("hermes work for member %d could not be verified as stopped: /srv/hermes/profiles/work/processes.json is not a Hermes process registry: ", hermesTestMember)
	for _, text := range []string{"{}", `[{"pid":1}]`, "not json"} {
		host := newWorkingHermesHost()
		host.registry = some(text)
		_, err := host.settled(none[string]())
		if err == nil {
			t.Fatalf("%s: an invalid registry fails settlement", text)
		}
		message := cafleetErrorOf(err).message()
		if !strings.HasPrefix(message, prefix) || !strings.HasSuffix(message, "; member was not deleted.") {
			t.Fatalf("%s: %q", text, message)
		}
	}
}

func TestCodingAgentHermes_Settlement_NativeSessionRecordsTheHomeAndAddsTheSessionIdOnceVerified(t *testing.T) {
	t.Parallel()
	created, err := json.Marshal(hermesSession(none[string]()))
	if err != nil || string(created) != `{"hermes_home":"`+hermesTestHome+`"}` {
		t.Fatalf("created %s, %v", created, err)
	}
	stored := `{"hermes_home":"` + hermesTestHome + `","session_id":"` + hermesTestSession + `"}`
	verified, err := json.Marshal(hermesSession(some(hermesTestSession)))
	if err != nil || string(verified) != stored {
		t.Fatalf("verified %s, %v", verified, err)
	}
	var decoded nativeSession
	if err := json.Unmarshal([]byte(stored), &decoded); err != nil || decoded != hermesSession(some(hermesTestSession)) {
		t.Fatalf("decoded %+v, %v", decoded, err)
	}
	if err := json.Unmarshal([]byte(`{"hermes_home":"`+hermesTestHome+`","pid":1}`), &decoded); err == nil {
		t.Fatal("an unknown member is refused")
	}
}

func resolvedHermesHome(t *testing.T, body string) (string, error) {
	t.Helper()
	path := executableStub(t, t.TempDir(), "hermes", body)
	hermes := newHermesCli(path, "/srv/hermes")
	return runStub(t, func() (string, error) { return effectiveHome(hermes, time.Now().Add(2*time.Second)) })
}

func TestCodingAgentHermes_Settlement_EffectiveHomeIsTheDirectoryOfTheConfigPathHermesReportsForTheBase(t *testing.T) {
	t.Parallel()
	home, err := resolvedHermesHome(t, "[ \"$*\" = 'config path' ] && [ \"$HERMES_HOME\" = /srv/hermes ] || exit 3\nprintf '/srv/hermes/profiles/work/config.yaml\\n'")
	if err != nil || home != hermesTestHome {
		t.Fatalf("home %q, %v", home, err)
	}
	if _, err := homeOfConfigPath("config.yaml"); err == nil || cafleetErrorOf(err).message() != "printed no absolute file path: config.yaml" {
		t.Fatalf("a relative config path: %v", err)
	}
	_, err = resolvedHermesHome(t, "echo 'no profile' >&2\nexit 4")
	if err == nil {
		t.Fatal("a failed config path fails")
	}
	failure := cafleetErrorOf(err)
	if failure.exitCode() != 1 || !strings.HasPrefix(failure.message(), "cannot read hermes configuration through 'hermes config path': ") ||
		!strings.Contains(failure.message(), "no profile") {
		t.Fatalf("exit %d: %q", failure.exitCode(), failure.message())
	}
}

func TestCodingAgentHermes_Settlement_SettlementScreenPatternsAcceptOnlyAsciiDigitsAndAsciiWhitespace(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name        string
		screen      string
		stopReports int
		session     opt[string]
	}{
		{"ascii stop count", "Stopped 12 process(es).", 1, none[string]()},
		{"arabic-indic stop count", "Stopped \U00000661\U00000662 process(es).", 0, none[string]()},
		{"empty registry report", "No running background processes.", 1, none[string]()},
		{"ascii space and tab around the session", "Session: \tlocal-ab12 \t\r\n", 0, some("local-ab12")},
		{"no-break space before the session", "Session:\U000000a0local-ab12\n", 0, none[string]()},
		{"ideographic space before the session", "Session:\U00003000local-ab12\n", 0, none[string]()},
		{"ideographic space after the session", "Session: local-ab12\U00003000\n", 0, none[string]()},
	}
	for _, c := range cases {
		if got := stopReports(c.screen); got != c.stopReports {
			t.Fatalf("%s: %d stop reports, want %d", c.name, got, c.stopReports)
		}
		got := none[string]()
		if session, found := exitSummarySession(c.screen); found {
			got = some(session)
		}
		if got != c.session {
			t.Fatalf("%s: session %v, want %v", c.name, got, c.session)
		}
	}
}
