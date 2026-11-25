package core

import (
	"fmt"
	"slices"
	"testing"
)

const (
	callerTestTmuxSocket   = "/tmp/tmux-1000/default"
	callerTestHerdrSession = "work"
	callerTestUnreadable   = "cannot read /proc/412/stat: Permission denied (os error 13)"
	callerTestTmuxGuidance = "the tmux command prompt (the user presses the tmux prefix key in the Director pane, then types :display-message -d 0 \"#{pane_id} #{socket_path}\" and Enter; the status line shows the pane ID, then the server socket)"
	callerTestHerdrGuide   = "! env -u HERDR_PANE_ID herdr --session <session> pane current --current (typed by the user into the Director's composer while the Director pane is focused; <session> is the herdr session the user attached, default unless herdr was started with --session <name>, and herdr session list shows the running sessions; the pane ID is result.pane.pane_id and the server is <session>)"
	callerTestRerun        = "then run cafleet fleet create with --pane <pane-id> --mux-server <server>"
)

func callerTmuxMux(runner commandRunner) *tmuxMultiplexer {
	return tmuxMultiplexerForServer(runner, callerTestTmuxSocket)
}

func callerHerdrMux(runner commandRunner) *herdrMultiplexer {
	return herdrMultiplexerForServer(runner, callerTestHerdrSession)
}

// answerTmux queues the tmux `#{pane_id}|#{pane_pid}` reply for pane; none is tmux's
// missing-pane `|`.
func answerTmux(runner *fakeRunner, pane string, pid opt[int64]) {
	if value, ok := pid.get(); ok {
		runner.respond(fmt.Sprintf("%s|%d\n", pane, value))
		return
	}
	runner.respond("|\n")
}

// answerHerdr queues the herdr `pane process-info` reply; none is herdr's pane_not_found.
func answerHerdr(t *testing.T, runner *fakeRunner, pid opt[int64]) {
	t.Helper()
	if value, ok := pid.get(); ok {
		runner.respond(herdrEnvelope(t, map[string]any{"process_info": map[string]any{"shell_pid": value}}))
		return
	}
	runner.failCode(t, "pane_not_found")
}

type chainEntry struct {
	pid    int64
	parent int64
	argv   []string
}

func fixtureChain(own int64, chain ...chainEntry) *fixtureProcessTable {
	table := newFixtureProcessTable(own)
	for _, entry := range chain {
		table.process(entry.pid, entry.parent, entry.argv...)
	}
	return table
}

// plainChain is `30 → 20 → 10 → 1`: the caller, a shell and a multiplexer server.
func plainChain() *fixtureProcessTable {
	return fixtureChain(30,
		chainEntry{30, 20, []string{"cafleet", "fleet", "create"}},
		chainEntry{20, 10, []string{"/bin/sh"}},
		chainEntry{10, 1, []string{"tmux", "new-session"}},
	)
}

func ancestryEqual(a, b ancestry) bool {
	return a.end == b.end && slices.EqualFunc(a.entries, b.entries, func(x, y ancestryEntry) bool {
		return x.pid == y.pid && slices.Equal(x.argv, y.argv)
	})
}

func mustVerifyPane(t *testing.T, name string, m multiplexer, table *fixtureProcessTable, pane string) verification {
	t.Helper()
	decided, err := verifyPane(m, table, walk(table), pane)
	if err != nil {
		t.Fatalf("%s: %v", name, err)
	}
	return decided
}

func TestMultiplexerCaller_WalkTermination_WalkRecordsEachReadableAncestorAndNamesHowItEnded(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		table *fixtureProcessTable
		want  ancestry
	}{
		{name: "reaching pid 1 completes without reading it", table: plainChain(), want: ancestry{entries: []ancestryEntry{
			{pid: 30, argv: []string{"cafleet", "fleet", "create"}}, {pid: 20, argv: []string{"/bin/sh"}}, {pid: 10, argv: []string{"tmux", "new-session"}},
		}, end: walkEndComplete{}}},
		{name: "own pid 1 completes with no entries", table: newFixtureProcessTable(1), want: ancestry{end: walkEndComplete{}}},
		{name: "an absent entry stops at that pid", table: fixtureChain(30, chainEntry{30, 20, []string{"cafleet"}}),
			want: ancestry{entries: []ancestryEntry{{pid: 30, argv: []string{"cafleet"}}}, end: walkEndStopped{pid: 20}}},
		{name: "parent 0 records the entry and stops there", table: fixtureChain(30, chainEntry{30, 20, []string{"cafleet"}}, chainEntry{20, 0, []string{"init"}}),
			want: ancestry{entries: []ancestryEntry{{pid: 30, argv: []string{"cafleet"}}, {pid: 20, argv: []string{"init"}}}, end: walkEndStopped{pid: 20}}},
		{name: "a read error ends unreadable with the whole message", table: fixtureChain(30, chainEntry{30, 412, []string{"cafleet"}}).unreadable(412, callerTestUnreadable),
			want: ancestry{entries: []ancestryEntry{{pid: 30, argv: []string{"cafleet"}}}, end: walkEndUnreadable{cause: callerTestUnreadable}}},
	}
	for _, tc := range cases {
		if got := walk(tc.table); !ancestryEqual(got, tc.want) {
			t.Fatalf("%s: ancestry %#v, want %#v", tc.name, got, tc.want)
		}
	}
}

func TestMultiplexerCaller_WalkTermination_AChainLongerThan4096EntriesStopsAtTheLastRecordedPid(t *testing.T) {
	t.Parallel()
	const own = 100_000
	table := newFixtureProcessTable(own)
	for step := range int64(5000) {
		table.process(own-step, own-step-1, "sh")
	}
	got := walk(table)
	if len(got.entries) != 4096 || got.entries[0].pid != own || got.entries[4095].pid != own-4095 || got.end != walkEnd(walkEndStopped{pid: own - 4095}) {
		t.Fatalf("a capped walk recorded %d entries ending %#v", len(got.entries), got.end)
	}
}

func TestMultiplexerCaller_WalkTermination_AParentCycleTerminatesAtTheCap(t *testing.T) {
	t.Parallel()
	got := walk(fixtureChain(50, chainEntry{50, 50, []string{"loop"}}))
	if len(got.entries) != 4096 || got.end != walkEnd(walkEndStopped{pid: 50}) {
		t.Fatalf("a parent cycle recorded %d entries ending %#v", len(got.entries), got.end)
	}
}

// callerDecision is the outcome of row 1 (sharedServerOf) and, when row 1 passes, rows 2–7
// (verifyPane).
type callerDecision struct {
	shared   opt[sharedServer]
	verified verification
}

func callerDecide(t *testing.T, name string, table *fixtureProcessTable, m multiplexer, pane string) callerDecision {
	t.Helper()
	walked := walk(table)
	if shared, ok := sharedServerOf(walked).get(); ok {
		return callerDecision{shared: some(shared)}
	}
	decided, err := verifyPane(m, table, walked, pane)
	if err != nil {
		t.Fatalf("%s: %v", name, err)
	}
	return callerDecision{verified: decided}
}

func callerShared(label string, pid int64) callerDecision {
	return callerDecision{shared: some(sharedServer{label: label, pid: pid})}
}

func TestMultiplexerCaller_DecisionTable_Row1RefusesAtTheNearestSharedServerAncestorAmongTheRecordedEntries(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		table *fixtureProcessTable
		want  opt[sharedServer]
	}{
		{name: "nearest shared-server ancestor", table: fixtureChain(30,
			chainEntry{30, 20, []string{"cafleet"}},
			chainEntry{20, 15, []string{"python", "-m", "hermes_cli.main", "gateway", "run"}},
			chainEntry{15, 10, []string{"/opt/bin/codex", "app-server", "--list"}},
			chainEntry{10, 1, []string{"tmux"}},
		), want: some(sharedServer{label: "hermes gateway run", pid: 20})},
		{name: "no shared-server ancestor", table: plainChain(), want: none[sharedServer]()},
		{name: "entries recorded before an unreadable end", table: fixtureChain(30,
			chainEntry{30, 20, []string{"cafleet"}},
			chainEntry{20, 412, []string{"codex", "app-server"}},
		).unreadable(412, callerTestUnreadable), want: some(sharedServer{label: "codex app-server", pid: 20})},
	}
	for _, tc := range cases {
		if got := sharedServerOf(walk(tc.table)); got != tc.want {
			t.Fatalf("%s: shared server %#v, want %#v", tc.name, got, tc.want)
		}
	}
}

func TestMultiplexerCaller_DecisionTable_Rows2To7DecideATmuxPaneInOrder(t *testing.T) {
	t.Parallel()
	unreadableAfterShell := func() *fixtureProcessTable {
		return fixtureChain(30, chainEntry{30, 20, []string{"cafleet"}}, chainEntry{20, 412, []string{"/bin/sh"}}).unreadable(412, callerTestUnreadable)
	}
	cases := []struct {
		name    string
		table   *fixtureProcessTable
		panePID opt[int64]
		want    verification
	}{
		{name: "row 2: a missing pane refuses before the namespace check", table: plainChain().nested(),
			want: verificationRefused{refusal: refusalPaneNotFound{}}},
		{name: "row 3: a nested namespace refuses even an ancestor pane", table: plainChain().nested(), panePID: some[int64](20),
			want: verificationRefused{refusal: refusalNestedPidNamespace{}}},
		{name: "row 4 precedes an unreadable walk end", table: unreadableAfterShell(), panePID: some[int64](20), want: verificationVerified{}},
		{name: "row 4 precedes a stopped walk end", table: fixtureChain(30, chainEntry{30, 20, []string{"cafleet"}}, chainEntry{20, 15, []string{"/bin/sh"}}),
			panePID: some[int64](20), want: verificationVerified{}},
		{name: "row 5: an unreadable walk refuses with its cause", table: fixtureChain(30, chainEntry{30, 412, []string{"cafleet"}}).unreadable(412, callerTestUnreadable),
			panePID: some[int64](99), want: verificationRefused{refusal: refusalProcessUnreadable{cause: callerTestUnreadable}}},
		{name: "row 6: a walk stopped at an absent entry refuses with its pid", table: fixtureChain(30, chainEntry{30, 20, []string{"cafleet"}}),
			panePID: some[int64](99), want: verificationRefused{refusal: refusalWalkStopped{pid: 20}}},
		{name: "row 6: a walk stopped at parent 0 refuses with its pid", table: fixtureChain(30, chainEntry{30, 20, []string{"cafleet"}}, chainEntry{20, 0, []string{"init"}}),
			panePID: some[int64](99), want: verificationRefused{refusal: refusalWalkStopped{pid: 20}}},
		{name: "row 7: a completed walk without the pane refuses with the pane pid", table: plainChain(), panePID: some[int64](99),
			want: verificationRefused{refusal: refusalPaneNotAncestor{panePID: 99}}},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "tmux")
		answerTmux(runner, "%3", tc.panePID)
		if got := mustVerifyPane(t, tc.name, callerTmuxMux(runner), tc.table, "%3"); got != tc.want {
			t.Fatalf("%s: verification %#v, want %#v", tc.name, got, tc.want)
		}
	}
}

func TestMultiplexerCaller_DecisionTable_Row2ReadsThePaneThroughTheMultiplexerOnItsServer(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "herdr")
	answerHerdr(t, runner, none[int64]())
	if got := mustVerifyPane(t, "row 2", callerHerdrMux(runner), plainChain(), "w1:p2"); got != verification(verificationRefused{refusal: refusalPaneNotFound{}}) {
		t.Fatalf("verification %#v, want pane not found", got)
	}
	assertCalls(t, "row 2", runner.runArgvs(), [][]string{{"herdr", "--session", callerTestHerdrSession, "pane", "process-info", "--pane", "w1:p2"}})
}

func TestMultiplexerCaller_DecisionTable_Row4AnAncestorPaneVerifies(t *testing.T) {
	t.Parallel()
	for _, backend := range []string{"tmux", "herdr"} {
		runner := fakeRunnerWithBinary(t, backend)
		var mux multiplexer
		pane := "%3"
		if backend == "tmux" {
			answerTmux(runner, pane, some[int64](20))
			mux = callerTmuxMux(runner)
		} else {
			pane = "w1:p2"
			answerHerdr(t, runner, some[int64](20))
			mux = callerHerdrMux(runner)
		}
		if got := mustVerifyPane(t, backend, mux, plainChain(), pane); got != verification(verificationVerified{}) {
			t.Fatalf("%s: verification %#v, want verified", backend, got)
		}
	}
}

func TestMultiplexerCaller_DecisionTable_APaneProcessCommandFailurePropagatesAsAnError(t *testing.T) {
	t.Parallel()
	table := plainChain()
	runner := fakeRunnerWithBinary(t, "tmux")
	runner.fail(failedRun("no server running on /tmp/tmux-1000/default"))
	if _, err := verifyPane(callerTmuxMux(runner), table, walk(table), "%3"); err == nil {
		t.Fatal("a pane process failure propagates")
	}
}

func TestMultiplexerCaller_DecisionTable_ANamespaceReadFailurePropagatesAsAnError(t *testing.T) {
	t.Parallel()
	const message = "cannot read /proc/self/status: Permission denied (os error 13)"
	table := plainChain().namespaceError(message)
	runner := fakeRunnerWithBinary(t, "tmux")
	answerTmux(runner, "%3", some[int64](20))
	_, err := verifyPane(callerTmuxMux(runner), table, walk(table), "%3")
	if err == nil || err.Error() != message {
		t.Fatalf("error %v, want %q", err, message)
	}
}

const (
	callerEvidenceTmuxServerPID  = 399808
	callerEvidenceHerdrServerPID = 3340413
	callerEvidenceOpencode       = "/root/.local/share/mise/installs/aqua-anomalyco-opencode/v2.2.5/opencode"
	callerEvidenceHermesPython   = "/home/linuxbrew/.linuxbrew/Cellar/hermes-agent/0.22.4_1/libexec/bin/python"
)

var (
	callerEvidenceCafleet       = []string{"cafleet", "…"}
	callerEvidenceCodexDaemon   = []string{"/root/.codex/packages/app-server-daemon/releases/0.167.1-x86_64-unknown-linux-musl/bin/codex", "app-server", "--list"}
	callerEvidenceCodexPaneA    = []string{"codex", "--enable", "daemon_auto_start", "--dangerously-bypass-approvals-and-sandbox", "<prompt>"}
	callerEvidenceTmuxServer    = []string{"tmux", "-L", "cafleet-probe", "-f", "/dev/null", "new-session", "-d", "-s", "probe", "-x", "220", "-y", "50", "-c", "/tmp/cafleet-env-probe/work"}
	callerEvidenceHerdrServer   = []string{"/root/.local/share/mise/installs/aqua-ogulcancelik-herdr/v0.9.5/herdr", "server"}
	callerEvidenceHermesGateway = []string{callerEvidenceHermesPython, "-m", "hermes_cli.main", "--profile", "default", "gateway", "run", "--quiet", "--idle-exit"}
	callerEvidenceHermesChat    = []string{callerEvidenceHermesPython, "/home/linuxbrew/.linuxbrew/Cellar/hermes-agent/0.22.4_1/libexec/bin/hermes", "chat", "--cli", "--yolo", "-q", "<prompt>"}
)

type callerEvidenceOutcome uint8

const (
	callerEvidenceSharedServer callerEvidenceOutcome = iota
	callerEvidenceVerified
	callerEvidenceNotAncestor
)

// callerEvidenceCase is a tmux capture: the chain stops below the tmux server, whose pid is every
// top parent.
type callerEvidenceCase struct {
	name    string
	ownPID  int64
	chain   []chainEntry
	pane    string
	panePID opt[int64]
	outcome callerEvidenceOutcome
	shared  sharedServer
}

func (c callerEvidenceCase) rowOne() bool {
	return c.outcome == callerEvidenceSharedServer
}

func (c callerEvidenceCase) want(t *testing.T, panePID opt[int64]) callerDecision {
	t.Helper()
	switch c.outcome {
	case callerEvidenceSharedServer:
		return callerDecision{shared: some(c.shared)}
	case callerEvidenceVerified:
		return callerDecision{verified: verificationVerified{}}
	case callerEvidenceNotAncestor:
		pid, ok := panePID.get()
		if !ok {
			t.Fatalf("%s: a row-7 case names its pane pid", c.name)
		}
		return callerDecision{verified: verificationRefused{refusal: refusalPaneNotAncestor{panePID: pid}}}
	default:
		panic("invalid callerEvidenceOutcome")
	}
}

func (c callerEvidenceCase) tmuxTable() *fixtureProcessTable {
	return fixtureChain(c.ownPID, c.chain...).process(callerEvidenceTmuxServerPID, 1, callerEvidenceTmuxServer...)
}

// callerEvidenceHerdrShell is the pane's shell between each agent and the herdr server; the shell
// is the pane pid.
func callerEvidenceHerdrShell(pid int64) int64 {
	return pid + 1
}

func (c callerEvidenceCase) herdrTable() *fixtureProcessTable {
	table := newFixtureProcessTable(c.ownPID)
	for _, entry := range c.chain {
		if entry.parent != callerEvidenceTmuxServerPID {
			table.process(entry.pid, entry.parent, entry.argv...)
			continue
		}
		shell := callerEvidenceHerdrShell(entry.pid)
		table.process(entry.pid, shell, entry.argv...).process(shell, callerEvidenceHerdrServerPID, "/bin/sh")
	}
	return table.process(callerEvidenceHerdrServerPID, 1, callerEvidenceHerdrServer...)
}

func (c callerEvidenceCase) herdrPanePID() opt[int64] {
	pid, ok := c.panePID.get()
	if !ok {
		return c.panePID
	}
	for _, entry := range c.chain {
		if entry.pid == pid && entry.parent == callerEvidenceTmuxServerPID {
			return some(callerEvidenceHerdrShell(pid))
		}
	}
	return c.panePID
}

func (c callerEvidenceCase) herdrPane() string {
	return "w5:p" + c.pane[1:]
}

func callerEvidenceTmuxCases() []callerEvidenceCase {
	return []callerEvidenceCase{
		{name: "Codex daemon, pane A", ownPID: 401748, chain: []chainEntry{
			{401748, 401380, callerEvidenceCafleet},
			{401380, 400625, callerEvidenceCodexDaemon},
			{400625, callerEvidenceTmuxServerPID, callerEvidenceCodexPaneA},
		}, pane: "%3", panePID: some[int64](400625), outcome: callerEvidenceSharedServer, shared: sharedServer{label: "codex app-server", pid: 401380}},
		{name: "Codex daemon, pane C (pane A's variables)", ownPID: 402502, chain: []chainEntry{
			{402502, 401380, callerEvidenceCafleet},
			{401380, 400625, callerEvidenceCodexDaemon},
			{400625, callerEvidenceTmuxServerPID, callerEvidenceCodexPaneA},
		}, pane: "%3", panePID: some[int64](400625), outcome: callerEvidenceSharedServer, shared: sharedServer{label: "codex app-server", pid: 401380}},
		{name: "Codex pane C after pane A closed", ownPID: 402611, chain: []chainEntry{
			{402611, 401380, callerEvidenceCafleet},
			{401380, 1, callerEvidenceCodexDaemon},
		}, pane: "%3", panePID: none[int64](), outcome: callerEvidenceSharedServer, shared: sharedServer{label: "codex app-server", pid: 401380}},
		{name: "Codex --no-daemon", ownPID: 403233, chain: []chainEntry{
			{403233, 402835, callerEvidenceCafleet},
			{402835, callerEvidenceTmuxServerPID, []string{"codex", "--no-daemon", "--dangerously-bypass-approvals-and-sandbox", "<prompt>"}},
		}, pane: "%6", panePID: some[int64](402835), outcome: callerEvidenceVerified},
		{name: "Codex --approve-for-me", ownPID: 402101, chain: []chainEntry{
			{402101, 401813, callerEvidenceCafleet},
			{401813, callerEvidenceTmuxServerPID, []string{"codex", "--enable", "daemon_auto_start", "--approve-for-me", "<prompt>"}},
		}, pane: "%4", panePID: some[int64](401813), outcome: callerEvidenceVerified},
		{name: "Claude Code", ownPID: 403317, chain: []chainEntry{
			{403317, 403316, callerEvidenceCafleet},
			{403316, 403012, []string{"/bin/bash", "-c", "<script>"}},
			{403012, callerEvidenceTmuxServerPID, []string{"claude", "--allowedTools", "Bash(sh /tmp/cafleet-env-probe/detect.sh *)", "--", "<prompt>"}},
		}, pane: "%7", panePID: some[int64](403012), outcome: callerEvidenceVerified},
		{name: "pi", ownPID: 403373, chain: []chainEntry{
			{403373, 403148, callerEvidenceCafleet},
			{403148, callerEvidenceTmuxServerPID, []string{"pi", "<prompt>"}},
		}, pane: "%8", panePID: some[int64](403148), outcome: callerEvidenceVerified},
		{name: "OpenCode --standalone", ownPID: 403403, chain: []chainEntry{
			{403403, 403187, callerEvidenceCafleet},
			{403187, 403176, []string{callerEvidenceOpencode, "serve", "--stdio", "--port", "0"}},
			{403176, callerEvidenceTmuxServerPID, []string{"opencode", "--standalone", "--auto", "--prompt", "<prompt>"}},
		}, pane: "%9", panePID: some[int64](403176), outcome: callerEvidenceVerified},
		{name: "OpenCode service, first client", ownPID: 403443, chain: []chainEntry{
			{403443, 403264, callerEvidenceCafleet},
			{403264, 403252, []string{callerEvidenceOpencode, "serve", "--service"}},
			{403252, callerEvidenceTmuxServerPID, []string{"opencode", "--auto", "--prompt", "<prompt>"}},
		}, pane: "%10", panePID: some[int64](403252), outcome: callerEvidenceVerified},
		{name: "OpenCode service, second client", ownPID: 405295, chain: []chainEntry{
			{405295, 403264, callerEvidenceCafleet},
			{403264, 403252, []string{callerEvidenceOpencode, "serve", "--service"}},
			{403252, callerEvidenceTmuxServerPID, []string{"opencode", "--auto", "--prompt", "<prompt>"}},
		}, pane: "%13", panePID: some[int64](405228), outcome: callerEvidenceNotAncestor},
	}
}

func TestMultiplexerCaller_EvidenceFixtures_EveryTmuxCaseReachesItsExpectedRow(t *testing.T) {
	t.Parallel()
	for _, tc := range callerEvidenceTmuxCases() {
		runner := fakeRunnerWithBinary(t, "tmux")
		if !tc.rowOne() {
			answerTmux(runner, tc.pane, tc.panePID)
		}
		if got, want := callerDecide(t, tc.name, tc.tmuxTable(), callerTmuxMux(runner), tc.pane), tc.want(t, tc.panePID); got != want {
			t.Fatalf("%s: decision %#v, want %#v", tc.name, got, want)
		}
		if tc.rowOne() && len(runner.runArgvs()) != 0 {
			t.Fatalf("%s: row 1 ran %q", tc.name, runner.runArgvs())
		}
	}
}

func TestMultiplexerCaller_EvidenceFixtures_EveryHerdrCaseWithThePaneShellReachesItsExpectedRow(t *testing.T) {
	t.Parallel()
	for _, tc := range callerEvidenceTmuxCases() {
		runner := fakeRunnerWithBinary(t, "herdr")
		if !tc.rowOne() {
			answerHerdr(t, runner, tc.herdrPanePID())
		}
		if got, want := callerDecide(t, tc.name, tc.herdrTable(), callerHerdrMux(runner), tc.herdrPane()), tc.want(t, tc.herdrPanePID()); got != want {
			t.Fatalf("%s: decision %#v, want %#v", tc.name, got, want)
		}
		if tc.rowOne() && len(runner.runArgvs()) != 0 {
			t.Fatalf("%s: row 1 ran %q", tc.name, runner.runArgvs())
		}
	}
}

func TestMultiplexerCaller_EvidenceFixtures_PaneCWouldPassRow4AloneBecausePaneAIsItsAncestor(t *testing.T) {
	t.Parallel()
	index := slices.IndexFunc(callerEvidenceTmuxCases(), func(tc callerEvidenceCase) bool {
		return tc.name == "Codex daemon, pane C (pane A's variables)"
	})
	if index < 0 {
		t.Fatal("the pane C fixture exists")
	}
	tc := callerEvidenceTmuxCases()[index]
	runner := fakeRunnerWithBinary(t, "tmux")
	answerTmux(runner, tc.pane, tc.panePID)
	if got := mustVerifyPane(t, tc.name, callerTmuxMux(runner), tc.tmuxTable(), tc.pane); got != verification(verificationVerified{}) {
		t.Fatalf("verification %#v, want verified", got)
	}
}

func TestMultiplexerCaller_EvidenceFixtures_TmuxHermesRefusesAtRow1AlthoughItsChatPaneIsAnAncestor(t *testing.T) {
	t.Parallel()
	table := fixtureChain(428897,
		chainEntry{428897, 428895, []string{"sh", "/tmp/cafleet-env-probe/capture.sh", "hermes"}},
		chainEntry{428895, 428596, []string{"/usr/bin/bash", "-c", "<script>"}},
		chainEntry{428596, 428571, callerEvidenceHermesGateway},
		chainEntry{428571, 428475, callerEvidenceHermesChat},
		chainEntry{428475, 1, callerEvidenceTmuxServer},
	)
	runner := fakeRunnerWithBinary(t, "tmux")
	if got, want := callerDecide(t, "tmux hermes", table, callerTmuxMux(runner), "%1"), callerShared("hermes gateway run", 428596); got != want {
		t.Fatalf("decision %#v, want %#v", got, want)
	}
	if len(runner.runArgvs()) != 0 {
		t.Fatalf("row 1 ran %q", runner.runArgvs())
	}
	answerTmux(runner, "%1", some[int64](428571))
	if got := mustVerifyPane(t, "row 4 alone", callerTmuxMux(runner), table, "%1"); got != verification(verificationVerified{}) {
		t.Fatalf("row 4 alone would accept the chat client pane: verification %#v", got)
	}
}

func TestMultiplexerCaller_EvidenceFixtures_HerdrHermesWithThePaneShellRefusesAtRow1(t *testing.T) {
	t.Parallel()
	table := fixtureChain(428897,
		chainEntry{428897, 428895, []string{"sh", "/tmp/cafleet-env-probe/capture.sh", "hermes"}},
		chainEntry{428895, 428596, []string{"/usr/bin/bash", "-c", "<script>"}},
		chainEntry{428596, 428571, callerEvidenceHermesGateway},
		chainEntry{428571, 428572, callerEvidenceHermesChat},
		chainEntry{428572, callerEvidenceHerdrServerPID, []string{"/bin/sh"}},
		chainEntry{callerEvidenceHerdrServerPID, 1, callerEvidenceHerdrServer},
	)
	runner := fakeRunnerWithBinary(t, "herdr")
	if got, want := callerDecide(t, "herdr hermes", table, callerHerdrMux(runner), "w5:p2"), callerShared("hermes gateway run", 428596); got != want {
		t.Fatalf("decision %#v, want %#v", got, want)
	}
	if len(runner.runArgvs()) != 0 {
		t.Fatalf("row 1 ran %q", runner.runArgvs())
	}
}

func TestMultiplexerCaller_EvidenceFixtures_HerdrCodexDaemonPaneARefusesAtRow1(t *testing.T) {
	t.Parallel()
	table := fixtureChain(408076,
		chainEntry{408076, 407596, callerEvidenceCafleet},
		chainEntry{407596, 407442, callerEvidenceCodexDaemon},
		chainEntry{407442, 407363, callerEvidenceCodexPaneA},
		chainEntry{407363, callerEvidenceHerdrServerPID, []string{"/bin/sh"}},
		chainEntry{callerEvidenceHerdrServerPID, 1, callerEvidenceHerdrServer},
	)
	runner := fakeRunnerWithBinary(t, "herdr")
	if got, want := callerDecide(t, "herdr codex daemon", table, callerHerdrMux(runner), "w5:p1"), callerShared("codex app-server", 407596); got != want {
		t.Fatalf("decision %#v, want %#v", got, want)
	}
	if len(runner.runArgvs()) != 0 {
		t.Fatalf("row 1 ran %q", runner.runArgvs())
	}
}

func TestMultiplexerCaller_EvidenceFixtures_HerdrRealDirectorSessionVerifiesItsPaneShell(t *testing.T) {
	t.Parallel()
	table := fixtureChain(410391,
		chainEntry{410391, 410390, callerEvidenceCafleet},
		chainEntry{410390, 363045, []string{"/bin/bash", "-c", "<script>"}},
		chainEntry{363045, 362690, []string{"claude"}},
		chainEntry{362690, 362679, []string{"zsh"}},
		chainEntry{362679, callerEvidenceHerdrServerPID, []string{"/bin/sh"}},
		chainEntry{callerEvidenceHerdrServerPID, 1, callerEvidenceHerdrServer},
	)
	runner := fakeRunnerWithBinary(t, "herdr")
	answerHerdr(t, runner, some[int64](362679))
	if got, want := callerDecide(t, "herdr Director", table, callerHerdrMux(runner), "w4:p3N"), (callerDecision{verified: verificationVerified{}}); got != want {
		t.Fatalf("decision %#v, want %#v", got, want)
	}
}

func TestMultiplexerCaller_PaneForm_EachPaneIdFormSelectsItsBackend(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name, pane, binary, server string
		backend                    muxBackend
	}{
		{name: "percent and digits", pane: "%12", binary: "tmux", server: callerTestTmuxSocket, backend: muxBackendTmux},
		{name: "workspace colon pane", pane: "w4:p3N", binary: "herdr", server: callerTestHerdrSession, backend: muxBackendHerdr},
	}
	for _, tc := range cases {
		caller, err := resolvePaneMux(tc.pane, some(tc.server), map[string]string{}, fakeRunnerWithBinary(t, tc.binary))
		if err != nil || caller.backend != tc.backend || caller.server != tc.server {
			t.Fatalf("%s: caller %v %q (%v)", tc.name, caller.backend, caller.server, err)
		}
	}
}

func TestMultiplexerCaller_PaneForm_AnyOtherFormFailsBeforeServerResolutionOrAvailability(t *testing.T) {
	t.Parallel()
	for _, pane := range []string{"12", "%", "%1a", "a:b:c", ":p1", "w4:", "%1:2"} {
		runner := fakeRunnerWithoutBinaries(t)
		_, err := resolvePaneMux(pane, none[string](), map[string]string{"TMUX": "/tmp/tmux-1000/default,123,0"}, runner)
		want := fmt.Sprintf("--pane %s is neither a tmux pane ID (%%N) nor a herdr pane ID (<workspace>:<pane>)", pane)
		if err == nil || err.Error() != want || cafleetErrorOf(err).exitCode() != 1 {
			t.Fatalf("%q: error %v, want the application error %q", pane, err, want)
		}
		assertCalls(t, pane, runner.runArgvs(), nil)
	}
}

var callerTestAmbiguous = map[string]string{"HERDR_ENV": "1", "HERDR_SESSION": "stale", "TMUX": "/tmp/stale/default,1,0"}

func TestMultiplexerCaller_PaneServerSelection_MuxServerWinsOverEveryEnvironmentAndTargetsThatServer(t *testing.T) {
	t.Parallel()
	for _, pairs := range []map[string]string{callerTestAmbiguous, {"TMUX": "/tmp/other/default,1,0"}, {}} {
		runner := fakeRunnerWithBinary(t, "tmux")
		caller, err := resolvePaneMux("%5", some("/tmp/tmux-0/default"), pairs, runner)
		if err != nil || caller.backend != muxBackendTmux || caller.server != "/tmp/tmux-0/default" {
			t.Fatalf("%v: caller %v %q (%v)", pairs, caller.backend, caller.server, err)
		}
		assertCalls(t, "resolution", runner.runArgvs(), nil)
		answerTmux(runner, "%5", some[int64](77))
		if pid, err := caller.mux.paneProcess("%5"); err != nil || pid != some[int64](77) {
			t.Fatalf("%v: pane process %#v (%v)", pairs, pid, err)
		}
		assertCalls(t, "targeted server", runner.runArgvs(), [][]string{{"tmux", "-S", "/tmp/tmux-0/default", "display-message", "-p", "-t", "%5", "#{pane_id}|#{pane_pid}"}})
	}
	runner := fakeRunnerWithBinary(t, "herdr")
	caller, err := resolvePaneMux("w4:p1", some("main"), callerTestAmbiguous, runner)
	if err != nil || caller.backend != muxBackendHerdr || caller.server != "main" {
		t.Fatalf("herdr caller %v %q (%v)", caller.backend, caller.server, err)
	}
	assertCalls(t, "herdr resolution", runner.runArgvs(), nil)
}

func TestMultiplexerCaller_PaneServerSelection_WithoutMuxServerTheDerivedBackendEnvironmentOrDefaultServerApplies(t *testing.T) {
	t.Parallel()
	herdrEnv := map[string]string{"HERDR_ENV": "1", "HERDR_SESSION": "work"}
	tmuxEnv := map[string]string{"TMUX": "/tmp/tmux-1000/default,123,0"}
	cases := []struct {
		name, pane string
		pairs      map[string]string
		binary     string
		reply      opt[string]
		backend    muxBackend
		server     string
	}{
		{"a tmux environment names the caller server", "%5", tmuxEnv, "tmux", none[string](), muxBackendTmux, "/tmp/tmux-1000/default"},
		{"a herdr environment names the caller session", "w4:p1", herdrEnv, "herdr", none[string](), muxBackendHerdr, "work"},
		{"a tmux pane in an ambiguous environment asks the default server", "%5", callerTestAmbiguous, "tmux", some("/tmp/tmux-0/default\n"), muxBackendTmux, "/tmp/tmux-0/default"},
		{"a tmux pane in a herdr environment asks the default server", "%5", herdrEnv, "tmux", some("/tmp/tmux-0/default\n"), muxBackendTmux, "/tmp/tmux-0/default"},
		{"a tmux pane without an environment asks the default server", "%5", map[string]string{}, "tmux", some("/tmp/tmux-0/default\n"), muxBackendTmux, "/tmp/tmux-0/default"},
		{"a herdr pane in an ambiguous environment uses the default session", "w4:p1", callerTestAmbiguous, "herdr", none[string](), muxBackendHerdr, "default"},
		{"a herdr pane in a tmux environment uses the default session", "w4:p1", tmuxEnv, "herdr", none[string](), muxBackendHerdr, "default"},
		{"a herdr pane without an environment uses the default session", "w4:p1", map[string]string{}, "herdr", none[string](), muxBackendHerdr, "default"},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, tc.binary)
		var asked [][]string
		if reply, ok := tc.reply.get(); ok {
			runner.respond(reply)
			asked = [][]string{defaultTmuxSocket}
		}
		caller, err := resolvePaneMux(tc.pane, none[string](), tc.pairs, runner)
		if err != nil || caller.backend != tc.backend || caller.server != tc.server {
			t.Fatalf("%s: caller %v %q (%v), want %v %q", tc.name, caller.backend, caller.server, err, tc.backend, tc.server)
		}
		assertCalls(t, tc.name, runner.runArgvs(), asked)
	}
}

func TestMultiplexerCaller_PaneServerSelection_AnUnresolvedDefaultServerOrAMissingBinaryFailsResolution(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name      string
		muxServer opt[string]
		reply     opt[fakeResponse]
		message   string
		asked     bool
	}{
		{name: "empty default server reply", reply: some(fakeResponse{out: "\n"}), message: "no tmux server found; pass --mux-server <socket path>", asked: true},
		{name: "failed default server reply", reply: some(fakeResponse{err: failedRun("no server running on /tmp/tmux-0/default")}), message: "no tmux server found; pass --mux-server <socket path>", asked: true},
		{name: "missing binary", muxServer: some(callerTestTmuxSocket), message: "tmux binary not found on PATH"},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithoutBinaries(t)
		if reply, ok := tc.reply.get(); ok {
			runner = fakeRunnerWithBinary(t, "tmux")
			runner.responses = append(runner.responses, reply)
		}
		pairs := map[string]string{}
		var calls [][]string
		if tc.asked {
			pairs = callerTestAmbiguous
			calls = [][]string{defaultTmuxSocket}
		}
		_, err := resolvePaneMux("%5", tc.muxServer, pairs, runner)
		if err == nil || err.Error() != tc.message {
			t.Fatalf("%s: error %v, want %q", tc.name, err, tc.message)
		}
		assertCalls(t, tc.name, runner.runArgvs(), calls)
	}
}

func TestMultiplexerCaller_EnvResolution_EnvPaneReadsTheBackendVariableAndTreatsEmptyAsUnset(t *testing.T) {
	t.Parallel()
	pairs := map[string]string{"TMUX_PANE": "%3", "HERDR_PANE_ID": "w5:p1"}
	if got := envPane(muxBackendTmux, pairs); got != some(callerPane{variable: "TMUX_PANE", pane: "%3"}) {
		t.Fatalf("tmux env pane %#v", got)
	}
	if got := envPane(muxBackendHerdr, pairs); got != some(callerPane{variable: "HERDR_PANE_ID", pane: "w5:p1"}) {
		t.Fatalf("herdr env pane %#v", got)
	}
	empty := map[string]string{"TMUX_PANE": "", "HERDR_PANE_ID": ""}
	for _, tc := range []struct {
		backend muxBackend
		pairs   map[string]string
	}{
		{muxBackendTmux, empty}, {muxBackendHerdr, empty}, {muxBackendTmux, map[string]string{"HERDR_PANE_ID": "w5:p1"}},
	} {
		if got, set := envPane(tc.backend, tc.pairs).get(); set {
			t.Fatalf("%v %v: env pane %#v, want unset", tc.backend, tc.pairs, got)
		}
	}
}

func TestMultiplexerCaller_EnvResolution_ResolveEnvMuxDetectsTheBackendAndItsCallerServer(t *testing.T) {
	t.Parallel()
	tmuxRunner := fakeRunnerWithBinary(t, "tmux")
	caller, err := resolveEnvMux(map[string]string{"TMUX": "/tmp/tmux-1000/default,123,0", "TMUX_PANE": "%3"}, tmuxRunner)
	if err != nil || caller.backend != muxBackendTmux || caller.server != "/tmp/tmux-1000/default" {
		t.Fatalf("tmux caller %v %q (%v)", caller.backend, caller.server, err)
	}
	herdrRunner := fakeRunnerWithBinary(t, "herdr")
	caller, err = resolveEnvMux(map[string]string{"HERDR_ENV": "1", "HERDR_SESSION": "work"}, herdrRunner)
	if err != nil || caller.backend != muxBackendHerdr || caller.server != "work" {
		t.Fatalf("herdr caller %v %q (%v)", caller.backend, caller.server, err)
	}
	assertCalls(t, "herdr resolution", herdrRunner.runArgvs(), nil)
}

func TestMultiplexerCaller_EnvResolution_ResolveEnvMuxKeepsTheDetectionAndAvailabilityMessages(t *testing.T) {
	t.Parallel()
	_, err := resolveEnvMux(map[string]string{"HERDR_ENV": "1", "TMUX": "/tmp/tmux"}, fakeRunnerWithBinary(t, "tmux"))
	const ambiguous = "ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; unset the one that does not belong to this session"
	if err == nil || err.Error() != ambiguous {
		t.Fatalf("ambiguous error %v", err)
	}
	_, err = resolveEnvMux(map[string]string{"TMUX": "/tmp/tmux-1000/default,123,0"}, fakeRunnerWithoutBinaries(t))
	if err == nil || err.Error() != "tmux binary not found on PATH" {
		t.Fatalf("missing binary error %v", err)
	}
}

func refusalCallerMux(t *testing.T, backend muxBackend, server string) callerMux {
	t.Helper()
	return callerMux{backend: backend, server: server, mux: backend.forServer(server, fakeRunnerWithoutBinaries(t))}
}

func TestMultiplexerCaller_RefusalMessages_IdentificationGuidanceNamesTheUserRunCommandPerMultiplexer(t *testing.T) {
	t.Parallel()
	if identificationGuidance(muxBackendTmux) != callerTestTmuxGuidance || identificationGuidance(muxBackendHerdr) != callerTestHerdrGuide {
		t.Fatalf("guidance %q / %q", identificationGuidance(muxBackendTmux), identificationGuidance(muxBackendHerdr))
	}
}

func TestMultiplexerCaller_RefusalMessages_SharedServerMessageNamesBothGuidances(t *testing.T) {
	t.Parallel()
	got := "cafleet fleet create " + sharedServerMessage(sharedServer{label: "codex app-server", pid: 407596})
	want := "cafleet fleet create cannot verify this session's pane: a shared codex app-server server (pid 407596) runs this command, so its multiplexer environment can belong to another pane; ask the user to read the Director pane ID and server with the tmux command prompt (the user presses the tmux prefix key in the Director pane, then types :display-message -d 0 \"#{pane_id} #{socket_path}\" and Enter; the status line shows the pane ID, then the server socket) in tmux, or with ! env -u HERDR_PANE_ID herdr --session <session> pane current --current (typed by the user into the Director's composer while the Director pane is focused; <session> is the herdr session the user attached, default unless herdr was started with --session <name>, and herdr session list shows the running sessions; the pane ID is result.pane.pane_id and the server is <session>) in herdr, then run cafleet fleet create with --pane <pane-id> --mux-server <server>"
	if got != want {
		t.Fatalf("message\n%q\nwant\n%q", got, want)
	}
	got = sharedServerMessage(sharedServer{label: "hermes gateway run", pid: 403471})
	want = "cannot verify this session's pane: a shared hermes gateway run server (pid 403471) runs this command, so its multiplexer environment can belong to another pane; ask the user to read the Director pane ID and server with " +
		callerTestTmuxGuidance + " in tmux, or with " + callerTestHerdrGuide + " in herdr, " + callerTestRerun
	if got != want {
		t.Fatalf("message\n%q\nwant\n%q", got, want)
	}
}

func TestMultiplexerCaller_RefusalMessages_UnverifiedMessageStatesEachCauseOnBothMultiplexers(t *testing.T) {
	t.Parallel()
	causes := func(backend, server string) []struct {
		refusal refusal
		cause   string
	} {
		return []struct {
			refusal refusal
			cause   string
		}{
			{refusalPaneNotFound{}, fmt.Sprintf("the pane does not exist on the %s server %s", backend, server)},
			{refusalPaneNotAncestor{panePID: 405228}, "the pane's process 405228 is not an ancestor of this command"},
			{refusalWalkStopped{pid: 20}, "the process ancestry stopped at pid 20 before reaching pid 1"},
			{refusalNestedPidNamespace{}, "this command runs in a nested PID namespace"},
			{refusalProcessUnreadable{cause: callerTestUnreadable}, "the process ancestry could not be read: " + callerTestUnreadable},
		}
	}
	for _, tc := range []struct {
		backend                          muxBackend
		server, variable, pane, guidance string
	}{
		{muxBackendTmux, callerTestTmuxSocket, "TMUX_PANE", "%3", callerTestTmuxGuidance},
		{muxBackendHerdr, callerTestHerdrSession, "HERDR_PANE_ID", "w5:p1", callerTestHerdrGuide},
	} {
		mux := refusalCallerMux(t, tc.backend, tc.server)
		caller := callerPane{variable: tc.variable, pane: tc.pane}
		for _, cause := range causes(tc.backend.name(), tc.server) {
			want := fmt.Sprintf("cannot verify that %s=%s is this session's pane: %s; ask the user to read the Director pane ID and server with %s, %s", tc.variable, tc.pane, cause.cause, tc.guidance, callerTestRerun)
			if got := unverifiedMessage(caller, cause.refusal, mux); got != want {
				t.Fatalf("%#v: message\n%q\nwant\n%q", cause.refusal, got, want)
			}
		}
	}
}

func TestMultiplexerCaller_RefusalMessages_UnsetMessageNamesTheBackendVariableAndItsGuidance(t *testing.T) {
	t.Parallel()
	cases := []struct {
		backend muxBackend
		want    string
	}{
		{muxBackendTmux, "cannot identify its tmux pane because TMUX_PANE is not set; ask the user to read the Director pane ID and server with " + callerTestTmuxGuidance + ", " + callerTestRerun},
		{muxBackendHerdr, "cannot identify its herdr pane because HERDR_PANE_ID is not set; ask the user to read the Director pane ID and server with " + callerTestHerdrGuide + ", " + callerTestRerun},
	}
	for _, tc := range cases {
		if got := unsetMessage(tc.backend); got != tc.want {
			t.Fatalf("%v: message\n%q\nwant\n%q", tc.backend, got, tc.want)
		}
	}
}
