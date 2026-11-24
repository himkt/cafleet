package core

import (
	"maps"
	"slices"
	"strings"
	"testing"
	"time"
)

const (
	herdrTestSession      = "work"
	herdrTestCwd          = "/work/project"
	herdrTestTab          = "w1:t1"
	herdrTestTemporaryTab = "w1:t9"
	herdrTestCommandLine  = "/abs/cafleet __member-run --descriptor '/run/d j.json'"
)

func herdrOnServer(runner commandRunner) *herdrMultiplexer {
	return herdrMultiplexerForServer(runner, herdrTestSession)
}

// herdrCall is `herdr --session <session> <parts…>`.
func herdrCall(parts ...string) []string {
	return append([]string{"herdr", "--session", herdrTestSession}, parts...)
}

func herdrCommand() []string {
	return []string{"/abs/cafleet", "__member-run", "--descriptor", "/run/d j.json"}
}

func herdrColumn(side columnSide, columnPanes, otherPanes []string) paneTarget {
	return columnTarget{windowID: herdrTestTab, side: side, columnPanes: columnPanes, otherPanes: otherPanes}
}

func (f *fakeRunner) failCode(t *testing.T, code string) {
	t.Helper()
	f.fail(failedRun(herdrErrorStderr(t, code)))
}

func (f *fakeRunner) okEnvelope(t *testing.T) {
	t.Helper()
	f.respond(herdrEnvelope(t, map[string]any{}))
}

func layoutPane(id string, x, y, width, height int) map[string]any {
	return map[string]any{"pane_id": id, "rect": map[string]any{"x": x, "y": y, "width": width, "height": height}}
}

func layoutSplit(direction string, rect [4]int, ratio float64) map[string]any {
	return map[string]any{
		"direction": direction,
		"rect":      map[string]any{"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3]},
		"ratio":     ratio,
	}
}

func layoutReply(t *testing.T, panes, splits []any) string {
	t.Helper()
	return herdrEnvelope(t, map[string]any{"layout": map[string]any{
		"area":   map[string]any{"x": 0, "y": 0, "width": 120, "height": 60},
		"panes":  panes,
		"splits": splits,
	}})
}

// stacked is a full-width single column of ids, top to bottom.
func stacked(ids ...string) []any {
	height := 60 / len(ids)
	panes := make([]any, len(ids))
	for index, id := range ids {
		panes[index] = layoutPane(id, 0, index*height, 120, height)
	}
	return panes
}

func splitReply(t *testing.T, paneID string) string {
	t.Helper()
	return herdrEnvelope(t, map[string]any{"pane": map[string]any{"pane_id": paneID}})
}

// movedToNewTab is herdr 0.9.5's `pane move --new-tab` reply.
func movedToNewTab(t *testing.T, paneID, tab string) string {
	t.Helper()
	return herdrEnvelope(t, map[string]any{"type": "pane_move", "move_result": map[string]any{
		"changed":          true,
		"pane":             map[string]any{"pane_id": paneID, "tab_id": tab},
		"previous_tab_id":  herdrTestTab,
		"previous_pane_id": paneID,
		"created_tab":      map[string]any{"tab_id": tab, "focused": false, "label": "2"},
		"target_layout":    map[string]any{"tab_id": tab, "panes": []any{}, "splits": []any{}},
	}})
}

// movedInto is herdr 0.9.5's `pane move --tab <tab>` reply; closedTab is the source tab the
// move emptied.
func movedInto(t *testing.T, paneID, tab string, closedTab opt[string]) string {
	t.Helper()
	moveResult := map[string]any{
		"changed":       true,
		"pane":          map[string]any{"pane_id": paneID, "tab_id": tab},
		"target_layout": map[string]any{"tab_id": tab, "panes": []any{}, "splits": []any{}},
	}
	if closed, ok := closedTab.get(); ok {
		moveResult["closed_tab_id"] = closed
	}
	return herdrEnvelope(t, map[string]any{"type": "pane_move", "move_result": moveResult})
}

// respondToMoves queues one reply per `pane move` in calls, the last move emptying the
// temporary tab.
func respondToMoves(t *testing.T, runner *fakeRunner, calls [][]string) {
	t.Helper()
	var moves [][]string
	for _, call := range calls {
		if call[3] == "pane" && call[4] == "move" {
			moves = append(moves, call)
		}
	}
	for index, call := range moves {
		if call[6] == "--new-tab" {
			runner.respond(movedToNewTab(t, call[5], herdrTestTemporaryTab))
			continue
		}
		closed := none[string]()
		if index+1 == len(moves) {
			closed = some(herdrTestTemporaryTab)
		}
		runner.respond(movedInto(t, call[5], call[7], closed))
	}
}

func herdrSplitCall(target, direction string) []string {
	return herdrCall("pane", "split", target, "--direction", direction, "--no-focus", "--cwd", herdrTestCwd)
}

func herdrMoveNewTab(pane string) []string {
	return herdrCall("pane", "move", pane, "--new-tab", "--no-focus")
}

func herdrMoveInto(pane, tab, direction, target string) []string {
	return herdrCall("pane", "move", pane, "--tab", tab, "--split", direction, "--target-pane", target, "--no-focus")
}

func herdrResize(pane, direction, amount string) []string {
	return herdrCall("pane", "resize", "--pane", pane, "--direction", direction, "--amount", amount)
}

func herdrLayoutRead(pane string) []string {
	return herdrCall("pane", "layout", "--pane", pane)
}

func callsNamed(runner *fakeRunner, verb string) [][]string {
	var named [][]string
	for _, call := range runner.runArgvs() {
		if call[3] == "pane" && call[4] == verb {
			named = append(named, call)
		}
	}
	return named
}

// assertReadOptions checks a `pane read` call: its prefix, then its option pairs in any order.
func assertReadOptions(t *testing.T, call []string, pane string, options map[string]string) {
	t.Helper()
	prefix := herdrCall("pane", "read", pane)
	if len(call) != len(prefix)+2*len(options) || !slices.Equal(call[:len(prefix)], prefix) {
		t.Fatalf("read call %q, want %q with options %v", call, prefix, options)
	}
	got := map[string]string{}
	for index := len(prefix); index+1 < len(call); index += 2 {
		got[call[index]] = call[index+1]
	}
	if !maps.Equal(got, options) {
		t.Fatalf("read options %v, want %v", got, options)
	}
}

func TestMultiplexerHerdr_StructuredReadsRejectNonJsonAndMissingResult(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct{ reply, prefix string }{
		{reply: "not json", prefix: "herdr returned non-JSON output:"},
		{reply: `{"id":1}`, prefix: "herdr output missing 'result' object:"},
	} {
		runner := fakeRunnerWithBinary(t, "herdr")
		runner.respond(tc.reply)
		_, err := herdrOnServer(runner).paneContext("w1:p1")
		if err == nil || !strings.HasPrefix(err.Error(), tc.prefix) {
			t.Fatalf("%q: error %v, want prefix %q", tc.reply, err, tc.prefix)
		}
	}
}

func TestMultiplexerHerdr_PaneProcessReadsTheShellPidAndTreatsOnlyAMissingPaneAsNone(t *testing.T) {
	t.Parallel()
	const missing = "herdr pane process-info missing 'shell_pid' field"
	cases := []struct {
		name    string
		respond func(*fakeRunner)
		pid     opt[int64]
		fails   bool
		message string
	}{
		{name: "integer shell pid", respond: func(f *fakeRunner) {
			f.respond(herdrEnvelope(t, map[string]any{"process_info": map[string]any{"shell_pid": 407363}}))
		}, pid: some[int64](407363)},
		{name: "pane not found", respond: func(f *fakeRunner) { f.failCode(t, "pane_not_found") }},
		{name: "no shell pid", respond: func(f *fakeRunner) {
			f.respond(herdrEnvelope(t, map[string]any{"process_info": map[string]any{}}))
		}, fails: true, message: missing},
		{name: "string shell pid", respond: func(f *fakeRunner) {
			f.respond(herdrEnvelope(t, map[string]any{"process_info": map[string]any{"shell_pid": "407363"}}))
		}, fails: true, message: missing},
		{name: "null shell pid", respond: func(f *fakeRunner) {
			f.respond(herdrEnvelope(t, map[string]any{"process_info": map[string]any{"shell_pid": nil}}))
		}, fails: true, message: missing},
		{name: "no process info", respond: func(f *fakeRunner) { f.respond(herdrEnvelope(t, map[string]any{})) }, fails: true, message: missing},
		{name: "unavailable server", respond: func(f *fakeRunner) { f.failCode(t, "server_unavailable") }, fails: true},
		{name: "timeout", respond: func(f *fakeRunner) { f.fail(&runError{kind: runErrorTimeout}) }, fails: true},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "herdr")
		tc.respond(runner)
		pid, err := herdrOnServer(runner).paneProcess("w5:p1")
		switch {
		case !tc.fails && (err != nil || pid != tc.pid):
			t.Fatalf("%s: pid %#v (%v), want %#v", tc.name, pid, err, tc.pid)
		case tc.fails && err == nil:
			t.Fatalf("%s: succeeded with %#v", tc.name, pid)
		case tc.fails && tc.message != "" && err.Error() != tc.message:
			t.Fatalf("%s: error %q, want %q", tc.name, err.Error(), tc.message)
		}
		assertCalls(t, tc.name, runner.runArgvs(), [][]string{herdrCall("pane", "process-info", "--pane", "w5:p1")})
	}
}

func TestMultiplexerHerdr_CapturePaneKeepsCrlfRowsAndDropsTheCrlfBlankTail(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "herdr")
	runner.respond("scrollback\r\nprompt> ls\r\nstatus bar\r\n\r\n \r\n")
	captured, err := herdrOnServer(runner).capturePane("w1:p2", 2)
	if err != nil || captured != "prompt> ls\r\nstatus bar\r" {
		t.Fatalf("captured %q (%v)", captured, err)
	}
	calls := runner.runArgvs()
	if len(calls) != 1 {
		t.Fatalf("%d process calls, want 1", len(calls))
	}
	assertReadOptions(t, calls[0], "w1:p2", map[string]string{"--format": "ansi", "--source": "recent-unwrapped", "--lines": "1002"})
}

func TestMultiplexerHerdr_Spawn_AColumnWithMembersSplitsItsLiveBottomPaneThenRunsNamesAndEqualizes(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "herdr")
	runner.respond(layoutReply(t,
		[]any{layoutPane("w1:p1", 0, 0, 60, 60), layoutPane("w1:p3", 60, 30, 60, 30), layoutPane("w1:p2", 60, 0, 60, 30)},
		[]any{layoutSplit("right", [4]int{0, 0, 120, 60}, 0.5), layoutSplit("down", [4]int{60, 0, 60, 60}, 0.5)}))
	runner.respond(splitReply(t, "w1:p4"))
	runner.okEnvelope(t)
	runner.okEnvelope(t)
	runner.respond(layoutReply(t,
		[]any{layoutPane("w1:p1", 0, 0, 60, 60), layoutPane("w1:p2", 60, 0, 60, 30), layoutPane("w1:p3", 60, 30, 60, 15), layoutPane("w1:p4", 60, 45, 60, 15)},
		[]any{layoutSplit("right", [4]int{0, 0, 120, 60}, 0.5), layoutSplit("down", [4]int{60, 0, 60, 60}, 0.5), layoutSplit("down", [4]int{60, 30, 60, 30}, 0.5)}))
	runner.okEnvelope(t)
	spawned, err := herdrOnServer(runner).spawnMemberPane(herdrColumn(columnSideRight, []string{"w1:p2", "w1:p3"}, []string{"w1:p1"}), "drafter", herdrTestCwd, herdrCommand())
	if want := (spawnedPane{paneID: "w1:p4", windowID: herdrTestTab}); err != nil || spawned != want {
		t.Fatalf("spawned %#v (%v), want %#v", spawned, err, want)
	}
	assertCalls(t, "the bottom pane comes from the live layout, not the registry order", runner.runArgvs(), [][]string{
		herdrLayoutRead("w1:p2"),
		herdrSplitCall("w1:p3", "down"),
		herdrCall("pane", "run", "w1:p4", herdrTestCommandLine),
		herdrCall("pane", "rename", "w1:p4", "drafter"),
		herdrLayoutRead("w1:p4"),
		herdrResize("w1:p3", "up", "0.1667"),
	})
}

func TestMultiplexerHerdr_Spawn_TheDirectorWindowsFirstMemberSplitsTheLoneLeftPaneRightward(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "herdr")
	runner.respond(layoutReply(t, []any{layoutPane("w1:p1", 0, 0, 120, 60)}, []any{}))
	runner.respond(splitReply(t, "w1:p2"))
	runner.okEnvelope(t)
	runner.okEnvelope(t)
	runner.respond(layoutReply(t, []any{layoutPane("w1:p1", 0, 0, 60, 60), layoutPane("w1:p2", 60, 0, 60, 60)}, []any{layoutSplit("right", [4]int{0, 0, 120, 60}, 0.5)}))
	spawned, err := herdrOnServer(runner).spawnMemberPane(herdrColumn(columnSideRight, nil, []string{"w1:p1"}), "drafter", herdrTestCwd, herdrCommand())
	if err != nil || spawned.paneID != "w1:p2" {
		t.Fatalf("spawned %#v (%v)", spawned, err)
	}
	assertCalls(t, "first member", runner.runArgvs(), [][]string{
		herdrLayoutRead("w1:p1"),
		herdrSplitCall("w1:p1", "right"),
		herdrCall("pane", "run", "w1:p2", herdrTestCommandLine),
		herdrCall("pane", "rename", "w1:p2", "drafter"),
		herdrLayoutRead("w1:p2"),
	})
}

func rightReparentingCalls(left []string, newPane string) [][]string {
	calls := [][]string{
		herdrLayoutRead(left[0]),
		herdrSplitCall(left[0], "right"),
		herdrCall("pane", "run", newPane, herdrTestCommandLine),
		herdrCall("pane", "rename", newPane, "worker"),
		herdrMoveNewTab(left[1]),
	}
	for i := 2; i < len(left); i++ {
		calls = append(calls, herdrMoveInto(left[i], herdrTestTemporaryTab, "down", left[i-1]))
	}
	for i := 1; i < len(left); i++ {
		calls = append(calls, herdrMoveInto(left[i], herdrTestTab, "down", left[i-1]))
	}
	return append(calls, herdrLayoutRead(newPane))
}

func reversed(panes []any) []any {
	shuffled := slices.Clone(panes)
	slices.Reverse(shuffled)
	return shuffled
}

func TestMultiplexerHerdr_Spawn_AnEmptyRightColumnBesideAStackedLeftColumnRebuildsTheLeftChain(t *testing.T) {
	t.Parallel()
	for _, left := range [][]string{{"w1:p1", "w1:p2"}, {"w1:p1", "w1:p2", "w1:p3", "w1:p4"}} {
		runner := fakeRunnerWithBinary(t, "herdr")
		runner.respond(layoutReply(t, reversed(stacked(left...)), []any{}))
		runner.respond(splitReply(t, "w1:p9"))
		runner.okEnvelope(t)
		runner.okEnvelope(t)
		expected := rightReparentingCalls(left, "w1:p9")
		respondToMoves(t, runner, expected)
		runner.failCode(t, "layout_unavailable")
		spawned, err := herdrOnServer(runner).spawnMemberPane(herdrColumn(columnSideRight, nil, []string{left[0]}), "worker", herdrTestCwd, herdrCommand())
		if want := (spawnedPane{paneID: "w1:p9", windowID: herdrTestTab}); err != nil || spawned != want {
			t.Fatalf("k = %d: spawned %#v (%v), want %#v", len(left), spawned, err, want)
		}
		assertCalls(t, "left chain rebuild", runner.runArgvs(), expected)
	}
}

func leftReparentingCalls(right []string, newPane string) [][]string {
	calls := [][]string{
		herdrLayoutRead(right[0]),
		herdrSplitCall(right[0], "right"),
		herdrCall("pane", "run", newPane, herdrTestCommandLine),
		herdrCall("pane", "rename", newPane, "worker"),
		herdrMoveNewTab(right[0]),
	}
	for i := 1; i < len(right); i++ {
		calls = append(calls, herdrMoveInto(right[i], herdrTestTemporaryTab, "down", right[i-1]))
	}
	calls = append(calls, herdrMoveInto(right[0], herdrTestTab, "right", newPane))
	for i := 1; i < len(right); i++ {
		calls = append(calls, herdrMoveInto(right[i], herdrTestTab, "down", right[i-1]))
	}
	return append(calls, herdrLayoutRead(newPane))
}

func TestMultiplexerHerdr_Spawn_AnEmptyLeftColumnMovesTheRightChainBesideTheNewPane(t *testing.T) {
	t.Parallel()
	for _, right := range [][]string{{"w1:p5"}, {"w1:p5", "w1:p6", "w1:p7"}} {
		runner := fakeRunnerWithBinary(t, "herdr")
		runner.respond(layoutReply(t, reversed(stacked(right...)), []any{}))
		runner.respond(splitReply(t, "w1:p9"))
		runner.okEnvelope(t)
		runner.okEnvelope(t)
		expected := leftReparentingCalls(right, "w1:p9")
		respondToMoves(t, runner, expected)
		runner.failCode(t, "layout_unavailable")
		spawned, err := herdrOnServer(runner).spawnMemberPane(herdrColumn(columnSideLeft, nil, []string{right[0]}), "worker", herdrTestCwd, herdrCommand())
		if want := (spawnedPane{paneID: "w1:p9", windowID: herdrTestTab}); err != nil || spawned != want {
			t.Fatalf("m = %d: spawned %#v (%v), want %#v", len(right), spawned, err, want)
		}
		assertCalls(t, "right chain move", runner.runArgvs(), expected)
	}
}

func TestMultiplexerHerdr_Spawn_ANewWindowIsADetachedLabelledTabWhoseRootPaneIsTheMember(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "herdr")
	runner.respond(herdrEnvelope(t, map[string]any{"tab": map[string]any{"tab_id": "w1:t5", "label": "cafleet-3-1"}, "root_pane": map[string]any{"pane_id": "w1:p20"}}))
	runner.okEnvelope(t)
	runner.okEnvelope(t)
	runner.respond(layoutReply(t, []any{layoutPane("w1:p20", 0, 0, 120, 60)}, []any{}))
	spawned, err := herdrOnServer(runner).spawnMemberPane(paneTargetNewWindow{session: "w1", windowName: "cafleet-3-1"}, "tester", herdrTestCwd, herdrCommand())
	if want := (spawnedPane{paneID: "w1:p20", windowID: "w1:t5"}); err != nil || spawned != want {
		t.Fatalf("spawned %#v (%v), want %#v", spawned, err, want)
	}
	assertCalls(t, "new tab", runner.runArgvs(), [][]string{
		herdrCall("tab", "create", "--workspace", "w1", "--cwd", herdrTestCwd, "--label", "cafleet-3-1", "--no-focus"),
		herdrCall("pane", "run", "w1:p20", herdrTestCommandLine),
		herdrCall("pane", "rename", "w1:p20", "tester"),
		herdrLayoutRead("w1:p20"),
	})
}

func TestMultiplexerHerdr_Spawn_AFailedOrIncompleteCreationReplyLeavesPaneCleanupUnknown(t *testing.T) {
	t.Parallel()
	tab := paneTargetNewWindow{session: "w1", windowName: "cafleet-3-1"}
	split := herdrColumn(columnSideRight, nil, []string{"w1:p1"})
	missingField := func(message string) bool {
		return strings.HasPrefix(message, "herdr tab create missing '") && strings.Contains(message, "' field")
	}
	anyMessage := func(string) bool { return true }
	cases := []struct {
		name        string
		target      paneTarget
		readsLayout bool
		respond     func(*fakeRunner)
		message     func(string) bool
	}{
		{name: "tab create without a root pane", target: tab, respond: func(f *fakeRunner) {
			f.respond(herdrEnvelope(t, map[string]any{"tab": map[string]any{"tab_id": "w1:t5"}}))
		}, message: missingField},
		{name: "tab create without a tab", target: tab, respond: func(f *fakeRunner) {
			f.respond(herdrEnvelope(t, map[string]any{"root_pane": map[string]any{"pane_id": "w1:p20"}}))
		}, message: missingField},
		{name: "tab create with empty ids", target: tab, respond: func(f *fakeRunner) {
			f.respond(herdrEnvelope(t, map[string]any{"tab": map[string]any{}, "root_pane": map[string]any{}}))
		}, message: missingField},
		{name: "tab create timeout", target: tab, respond: func(f *fakeRunner) { f.fail(&runError{kind: runErrorTimeout}) }, message: anyMessage},
		{name: "tab create failure", target: tab, respond: func(f *fakeRunner) { f.failCode(t, "workspace_not_found") }, message: anyMessage},
		{name: "split without a pane id", target: split, readsLayout: true, respond: func(f *fakeRunner) {
			f.respond(herdrEnvelope(t, map[string]any{"pane": map[string]any{}}))
		}, message: anyMessage},
		{name: "split with a blank pane id", target: split, readsLayout: true, respond: func(f *fakeRunner) {
			f.respond(herdrEnvelope(t, map[string]any{"pane": map[string]any{"pane_id": " "}}))
		}, message: anyMessage},
		{name: "split timeout", target: split, readsLayout: true, respond: func(f *fakeRunner) { f.fail(&runError{kind: runErrorTimeout}) }, message: anyMessage},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "herdr")
		calls := 1
		if tc.readsLayout {
			runner.respond(layoutReply(t, []any{layoutPane("w1:p1", 0, 0, 120, 60)}, []any{}))
			calls = 2
		}
		tc.respond(runner)
		_, err := herdrOnServer(runner).spawnMemberPane(tc.target, "worker", herdrTestCwd, herdrCommand())
		if err == nil {
			t.Fatalf("%s: the spawn succeeded", tc.name)
		}
		if err.paneCleanup != some[paneCleanup](paneCleanupUnknown{}) {
			t.Fatalf("%s: pane cleanup %#v, want unknown", tc.name, err.paneCleanup)
		}
		if !tc.message(err.Error()) {
			t.Fatalf("%s: error %q", tc.name, err.Error())
		}
		if len(runner.runArgvs()) != calls {
			t.Fatalf("%s: %d process calls, want %d", tc.name, len(runner.runArgvs()), calls)
		}
	}
}

func TestMultiplexerHerdr_Spawn_AFailedPaneRunClosesOnlyTheNewPaneAndMovesNothing(t *testing.T) {
	t.Parallel()
	for _, closeFails := range []bool{false, true} {
		runner := fakeRunnerWithBinary(t, "herdr")
		runner.respond(layoutReply(t, stacked("w1:p5", "w1:p6"), []any{}))
		runner.respond(splitReply(t, "w1:p9"))
		runner.fail(failedRun("primary run failure"))
		if closeFails {
			runner.fail(failedRun("close failed"))
		} else {
			runner.respond("")
		}
		runner.failCode(t, "server_unavailable")
		runner.failCode(t, "server_unavailable")
		_, err := herdrOnServer(runner).spawnMemberPane(herdrColumn(columnSideLeft, nil, []string{"w1:p5"}), "worker", herdrTestCwd, herdrCommand())
		if err == nil || !strings.Contains(err.Error(), "primary run failure") {
			t.Fatalf("closeFails=%v: error %v lacks the primary failure", closeFails, err)
		}
		cleanup, ok := err.paneCleanup.get()
		attempted, isAttempted := cleanup.(paneCleanupAttempted)
		if _, cleanupFailed := attempted.err.get(); !ok || !isAttempted || attempted.paneID != "w1:p9" || cleanupFailed != closeFails {
			t.Fatalf("closeFails=%v: pane cleanup %#v", closeFails, err.paneCleanup)
		}
		assertCalls(t, "close", callsNamed(runner, "close"), [][]string{herdrCall("pane", "close", "w1:p9")})
		if len(callsNamed(runner, "move")) != 0 || len(callsNamed(runner, "rename")) != 0 {
			t.Fatalf("closeFails=%v: a failed run moved or renamed panes", closeFails)
		}
	}
}

func TestMultiplexerHerdr_Spawn_AFailedMoveStopsReParentingAndKeepsTheSpawn(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "herdr")
	runner.respond(layoutReply(t, stacked("w1:p5", "w1:p6", "w1:p7"), []any{}))
	runner.respond(splitReply(t, "w1:p9"))
	runner.okEnvelope(t)
	runner.okEnvelope(t)
	runner.respond(movedToNewTab(t, "w1:p5", herdrTestTemporaryTab))
	runner.fail(failedRun(encodedText(t, map[string]any{"error": map[string]any{"code": "target_pane_not_found", "message": "target pane w1:p5 not found"}, "id": 1})))
	runner.failCode(t, "layout_unavailable")
	spawned, err := herdrOnServer(runner).spawnMemberPane(herdrColumn(columnSideLeft, nil, []string{"w1:p5"}), "worker", herdrTestCwd, herdrCommand())
	if err != nil || spawned.paneID != "w1:p9" {
		t.Fatalf("spawned %#v (%v)", spawned, err)
	}
	assertCalls(t, "the remaining panes stay where they are", callsNamed(runner, "move"), [][]string{
		herdrMoveNewTab("w1:p5"),
		herdrMoveInto("w1:p6", herdrTestTemporaryTab, "down", "w1:p5"),
	})
	if len(callsNamed(runner, "close")) != 0 {
		t.Fatal("a failed move closed a pane")
	}
}

func TestMultiplexerHerdr_Spawn_NamingAndEqualizationFailuresNeverFailTheSpawn(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "herdr")
	runner.respond(layoutReply(t, []any{layoutPane("w1:p1", 0, 0, 120, 60)}, []any{}))
	runner.respond(splitReply(t, "w1:p2"))
	runner.okEnvelope(t)
	runner.failCode(t, "pane_not_found")
	runner.failCode(t, "layout_unavailable")
	spawned, err := herdrOnServer(runner).spawnMemberPane(herdrColumn(columnSideRight, nil, []string{"w1:p1"}), "drafter", herdrTestCwd, herdrCommand())
	if err != nil || spawned.paneID != "w1:p2" {
		t.Fatalf("spawned %#v (%v)", spawned, err)
	}
	if len(callsNamed(runner, "close")) != 0 {
		t.Fatal("a best-effort failure closed the spawned pane")
	}
}

func TestMultiplexerHerdr_Spawn_PanesWithEqualYKeepTheirLayoutOrderWhenTheColumnIsSorted(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name   string
		order  []string
		bottom string
	}{
		{name: "p2 listed before p3", order: []string{"w1:p2", "w1:p3"}, bottom: "w1:p3"},
		{name: "p3 listed before p2", order: []string{"w1:p3", "w1:p2"}, bottom: "w1:p2"},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "herdr")
		panes := []any{layoutPane("w1:p1", 0, 0, 60, 60)}
		for _, id := range tc.order {
			panes = append(panes, layoutPane(id, 60, 0, 60, 30))
		}
		runner.respond(layoutReply(t, panes, []any{}))
		runner.respond(splitReply(t, "w1:p4"))
		runner.okEnvelope(t)
		runner.okEnvelope(t)
		runner.failCode(t, "layout_unavailable")
		if _, err := herdrOnServer(runner).spawnMemberPane(herdrColumn(columnSideRight, []string{"w1:p2", "w1:p3"}, []string{"w1:p1"}), "drafter", herdrTestCwd, herdrCommand()); err != nil {
			t.Fatalf("%s: %v", tc.name, err)
		}
		if calls := runner.runArgvs(); len(calls) < 2 || !slices.Equal(calls[1], herdrSplitCall(tc.bottom, "down")) {
			t.Fatalf("%s: the stable sort on rect.y makes %s the bottom pane (F13): %q", tc.name, tc.bottom, calls)
		}
	}
}

func twoColumnLayout(t *testing.T) string {
	t.Helper()
	return layoutReply(t,
		[]any{
			layoutPane("w1:pe", 60, 30, 60, 30),
			layoutPane("w1:pa", 0, 0, 60, 20),
			layoutPane("w1:pc", 0, 40, 60, 20),
			layoutPane("w1:pd", 60, 0, 60, 30),
			layoutPane("w1:pb", 0, 20, 60, 20),
		},
		[]any{
			layoutSplit("right", [4]int{0, 0, 120, 60}, 0.5),
			layoutSplit("down", [4]int{0, 20, 60, 40}, 0.5),
			layoutSplit("down", [4]int{0, 0, 60, 60}, 0.5),
			layoutSplit("down", [4]int{60, 0, 60, 60}, 0.7),
		})
}

func sortedCalls(calls [][]string) [][]string {
	sorted := slices.Clone(calls)
	slices.SortFunc(sorted, func(a, b []string) int { return strings.Compare(strings.Join(a, "\x00"), strings.Join(b, "\x00")) })
	return sorted
}

func TestMultiplexerHerdr_Equalize_RestorationEqualizesEveryColumnOfTheRecordedTab(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "herdr")
	runner.respond("")
	mux := herdrOnServer(runner)
	if err := mux.closePane("w1:px", herdrTestTab); err != nil {
		t.Fatal(err)
	}
	assertCalls(t, "the close reads no pre-close tab", runner.runArgvs(), [][]string{herdrCall("pane", "close", "w1:px")})
	runner.respond(herdrEnvelope(t, map[string]any{"panes": []any{
		map[string]any{"pane_id": "w2:p1", "tab_id": "w2:t1"},
		map[string]any{"pane_id": "w1:pd", "tab_id": herdrTestTab},
		map[string]any{"pane_id": "w1:pa", "tab_id": herdrTestTab},
	}}))
	runner.respond(twoColumnLayout(t))
	runner.succeed(2)
	if err := mux.restoreAfterClose(); err != nil {
		t.Fatal(err)
	}
	calls := runner.runArgvs()
	assertCalls(t, "restoration reads", calls[1:3], [][]string{herdrCall("pane", "list"), herdrLayoutRead("w1:pd")})
	assertCalls(t, "resizes", sortedCalls(calls[3:]), sortedCalls([][]string{herdrResize("w1:pb", "up", "0.1667"), herdrResize("w1:pe", "up", "0.2")}))
}

func TestMultiplexerHerdr_Equalize_AColumnWithoutItsOwnDownChainIsLeftUntouched(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "herdr")
	runner.respond("")
	mux := herdrOnServer(runner)
	if err := mux.closePane("w1:px", herdrTestTab); err != nil {
		t.Fatal(err)
	}
	runner.respond(herdrEnvelope(t, map[string]any{"panes": []any{map[string]any{"pane_id": "w1:pa", "tab_id": herdrTestTab}}}))
	runner.respond(layoutReply(t,
		[]any{layoutPane("w1:pa", 0, 0, 60, 60), layoutPane("w1:pd", 60, 0, 60, 30), layoutPane("w1:pe", 60, 30, 60, 30)},
		[]any{layoutSplit("right", [4]int{0, 0, 120, 60}, 0.5), layoutSplit("down", [4]int{60, 0, 120, 60}, 0.7)}))
	if err := mux.restoreAfterClose(); err != nil {
		t.Fatal(err)
	}
	if len(runner.runArgvs()) != 3 {
		t.Fatalf("a split of another width never matches: %q", runner.runArgvs())
	}
}

func TestMultiplexerHerdr_Close_KillPaneClosesThenRestoresALonePaneToFullWidthBestEffort(t *testing.T) {
	t.Parallel()
	closeCall := func(pane string) []string { return herdrCall("pane", "close", pane) }
	listCall := herdrCall("pane", "list")
	loneInTab := func(f *fakeRunner) {
		f.respond(herdrEnvelope(t, map[string]any{"panes": []any{map[string]any{"pane_id": "w1:p1", "tab_id": herdrTestTab}}}))
	}
	cases := []struct {
		name    string
		pane    string
		tab     string
		respond func(*fakeRunner)
		calls   [][]string
	}{
		{name: "the last pane of a tab leaves nothing to restore", pane: "w1:p20", tab: "w1:t5", respond: func(f *fakeRunner) {
			f.respond("")
			f.respond(herdrEnvelope(t, map[string]any{"panes": []any{map[string]any{"pane_id": "w1:p1", "tab_id": "w1:t0"}}}))
		}, calls: [][]string{closeCall("w1:p20"), listCall}},
		{name: "a lone pane with one residual right split is resized to full width", pane: "w1:p2", tab: herdrTestTab, respond: func(f *fakeRunner) {
			f.respond("")
			loneInTab(f)
			f.respond(layoutReply(t, []any{layoutPane("w1:p1", 0, 0, 72, 60)}, []any{layoutSplit("right", [4]int{0, 0, 120, 60}, 0.6)}))
			f.respond("")
		}, calls: [][]string{closeCall("w1:p2"), listCall, herdrLayoutRead("w1:p1"), herdrResize("w1:p1", "right", "0.4")}},
		{name: "a lone pane without residual splits needs no resize", pane: "w1:p2", tab: herdrTestTab, respond: func(f *fakeRunner) {
			f.respond("")
			loneInTab(f)
			f.respond(layoutReply(t, []any{layoutPane("w1:p1", 0, 0, 120, 60)}, []any{}))
		}, calls: [][]string{closeCall("w1:p2"), listCall, herdrLayoutRead("w1:p1")}},
		{name: "a gone pane and a failed restoration still succeed", pane: "w1:p2", tab: herdrTestTab, respond: func(f *fakeRunner) {
			f.failCode(t, "pane_not_found")
			f.failCode(t, "server_unavailable")
		}, calls: [][]string{closeCall("w1:p2"), listCall}},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "herdr")
		tc.respond(runner)
		if err := killPane(herdrOnServer(runner), tc.pane, tc.tab); err != nil {
			t.Fatalf("%s: %v", tc.name, err)
		}
		assertCalls(t, tc.name, runner.runArgvs(), tc.calls)
	}
}

func TestMultiplexerHerdr_Close_ACloseFailureOtherThanPaneGoneFails(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name  string
		close func(*herdrMultiplexer) *multiplexerError
	}{
		{name: "kill pane", close: func(m *herdrMultiplexer) *multiplexerError { return killPane(m, "w1:p2", herdrTestTab) }},
		{name: "close pane", close: func(m *herdrMultiplexer) *multiplexerError { return m.closePane("w1:p2", herdrTestTab) }},
	} {
		runner := fakeRunnerWithBinary(t, "herdr")
		runner.failCode(t, "server_unavailable")
		if err := tc.close(herdrOnServer(runner)); err == nil {
			t.Fatalf("%s: an unavailable server close succeeded", tc.name)
		}
		if len(runner.runArgvs()) != 1 {
			t.Fatalf("%s: %d process calls, want 1", tc.name, len(runner.runArgvs()))
		}
	}
}

func TestMultiplexerHerdr_Close_RestorationWithoutAPriorCloseDoesNothing(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithoutBinaries(t)
	if err := herdrOnServer(runner).restoreAfterClose(); err != nil {
		t.Fatal(err)
	}
	assertCalls(t, "no prior close", runner.runArgvs(), nil)
}

func heightReply(t *testing.T, value int) string {
	t.Helper()
	return herdrEnvelope(t, map[string]any{"layout": map[string]any{
		"area":   map[string]any{"x": 0, "y": 0, "width": 200, "height": value},
		"panes":  []any{},
		"splits": []any{},
	}})
}

func TestMultiplexerHerdr_WindowReads_WindowHeightReadsTheLayoutAreaOfTheFirstReadableAnchor(t *testing.T) {
	t.Parallel()
	lastAnchorError := func(message string) bool {
		return strings.Contains(message, "pane_not_found") && strings.Contains(message, "w1:p3") && !strings.Contains(message, "server_unavailable")
	}
	missingArea := func(message string) bool { return message == "herdr pane layout missing 'area' field" }
	cases := []struct {
		name    string
		anchors []string
		respond func(*fakeRunner)
		reads   int
		height  opt[int64]
		check   func(string) bool
	}{
		{name: "first anchor", anchors: []string{"w1:p2", "w1:p3"}, respond: func(f *fakeRunner) { f.respond(heightReply(t, 57)) }, reads: 1, height: some[int64](57)},
		{name: "failed anchors are skipped", anchors: []string{"w1:p2", "w1:p3", "w1:p4"}, respond: func(f *fakeRunner) {
			f.failCode(t, "pane_not_found")
			f.failCode(t, "server_unavailable")
			f.respond(heightReply(t, 122))
		}, reads: 3, height: some[int64](122)},
		{name: "every anchor fails with the last error", anchors: []string{"w1:p2", "w1:p3"}, respond: func(f *fakeRunner) {
			f.failCode(t, "server_unavailable")
			f.failCode(t, "pane_not_found")
		}, reads: 2, check: lastAnchorError},
		{name: "a layout without an area fails", anchors: []string{"w1:p2"}, respond: func(f *fakeRunner) {
			f.respond(herdrEnvelope(t, map[string]any{"layout": map[string]any{"panes": []any{}, "splits": []any{}}}))
		}, reads: 1, check: missingArea},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "herdr")
		tc.respond(runner)
		height, err := herdrOnServer(runner).windowHeight(herdrTestTab, tc.anchors)
		if want, ok := tc.height.get(); ok {
			if err != nil || height != want {
				t.Fatalf("%s: height %d (%v), want %d", tc.name, height, err, want)
			}
		} else if err == nil || !tc.check(err.Error()) {
			t.Fatalf("%s: error %v", tc.name, err)
		}
		var reads [][]string
		for _, anchor := range tc.anchors[:tc.reads] {
			reads = append(reads, herdrLayoutRead(anchor))
		}
		assertCalls(t, tc.name, runner.runArgvs(), reads)
	}
}

func TestMultiplexerHerdr_WindowReads_WindowNameReadsTheStringLabelOfTheRequestedTab(t *testing.T) {
	t.Parallel()
	labelled := func(label any) map[string]any { return map[string]any{"tab_id": herdrTestTab, "label": label} }
	cases := []struct {
		name    string
		tab     map[string]any
		code    string
		label   opt[string]
		message string
	}{
		{name: "plain label", tab: labelled("agents"), label: some("agents")},
		{name: "empty label", tab: labelled(""), label: some("")},
		{name: "label with separators and CR LF", tab: labelled(" agents | left \r\n"), label: some(" agents | left \r\n")},
		{name: "fleet label", tab: labelled("cafleet-42-0"), label: some("cafleet-42-0")},
		{name: "no label", tab: map[string]any{"tab_id": herdrTestTab}, message: "herdr tab get missing 'label' field"},
		{name: "missing tab", code: "tab_not_found", message: "tab_not_found"},
		{name: "other tab", tab: map[string]any{"tab_id": "other", "label": "cafleet-42-0"}},
		{name: "no tab id", tab: map[string]any{"label": "cafleet-42-0"}},
		{name: "null tab id", tab: map[string]any{"tab_id": nil, "label": "cafleet-42-0"}},
		{name: "integer tab id", tab: map[string]any{"tab_id": 1, "label": "cafleet-42-0"}},
		{name: "null label", tab: labelled(nil)},
		{name: "integer label", tab: labelled(1)},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "herdr")
		if tc.code != "" {
			runner.failCode(t, tc.code)
		} else {
			runner.respond(herdrEnvelope(t, map[string]any{"tab": tc.tab}))
		}
		label, err := herdrOnServer(runner).windowName(herdrTestTab)
		if want, ok := tc.label.get(); ok {
			if err != nil || label != want {
				t.Fatalf("%s: label %q (%v), want %q", tc.name, label, err, want)
			}
		} else if err == nil || !strings.Contains(err.Error(), tc.message) {
			t.Fatalf("%s: error %v lacks %q", tc.name, err, tc.message)
		}
		assertCalls(t, tc.name, runner.runArgvs(), [][]string{herdrCall("tab", "get", herdrTestTab)})
	}
}

func TestMultiplexerHerdr_WindowReads_RenameWindowTargetsTheRecordedServerAndConcreteTab(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		respond func(*fakeRunner)
		renamed bool
		message string
	}{
		{name: "renamed", respond: func(f *fakeRunner) { f.okEnvelope(t) }, renamed: true},
		{name: "refused", respond: func(f *fakeRunner) { f.failCode(t, "rename denied") }, message: "rename denied"},
		{name: "non-JSON reply", respond: func(f *fakeRunner) { f.respond("not json") }},
		{name: "reply without a result", respond: func(f *fakeRunner) { f.respond(`{"id":1}`) }},
		{name: "null result", respond: func(f *fakeRunner) { f.respond(`{"result":null}`) }},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "herdr")
		tc.respond(runner)
		err := herdrOnServer(runner).renameWindow(herdrTestTab, "cafleet-42-0")
		switch {
		case tc.renamed && err != nil:
			t.Fatalf("%s: %v", tc.name, err)
		case !tc.renamed && (err == nil || !strings.Contains(err.Error(), tc.message)):
			t.Fatalf("%s: error %v lacks %q", tc.name, err, tc.message)
		}
		assertCalls(t, tc.name, runner.runArgvs(), [][]string{herdrCall("tab", "rename", herdrTestTab, "cafleet-42-0")})
	}
}

const herdrTestPane = "w1:p2"

// snapshotReply is an `api snapshot` reply in herdr 0.9.5's shape: layouts[].panes[] carry
// focused, pane_id and rect, and only the panes[] entries carry scroll.
func snapshotReply(t *testing.T, layouts, panes any) string {
	t.Helper()
	return herdrEnvelope(t, map[string]any{"snapshot": map[string]any{"layouts": layouts, "panes": panes, "version": 1}})
}

func snapshotLayoutPane(id string, width, height int) map[string]any {
	return map[string]any{"focused": false, "pane_id": id, "rect": map[string]any{"height": height, "width": width, "x": 0, "y": 0}}
}

func snapshotPaneState(id string, viewportRows int) map[string]any {
	return map[string]any{
		"agent_status": "unknown",
		"focused":      false,
		"pane_id":      id,
		"scroll":       map[string]any{"max_offset_from_bottom": 0, "offset_from_bottom": 0, "viewport_rows": viewportRows},
		"workspace_id": "w2",
	}
}

func directorLayouts() []any {
	return []any{
		map[string]any{"panes": []any{snapshotLayoutPane("w1:p1", 60, 40)}, "tab_id": "w1:t1"},
		map[string]any{"panes": []any{snapshotLayoutPane("w2:p1", 120, 40), snapshotLayoutPane(herdrTestPane, 97, 33)}, "tab_id": "w2:t1"},
	}
}

// directorPanes holds the pane between two others with their own viewport rows, so only a
// join by pane_id reads viewportRows.
func directorPanes(viewportRows int) []any {
	return []any{snapshotPaneState("w1:p1", 40), snapshotPaneState(herdrTestPane, viewportRows), snapshotPaneState("w2:p1", 41)}
}

func TestMultiplexerHerdr_DirectorPane_CaptureScreenTakesTheWidthAndRowsFromTheSnapshotAndTheBodyFromTheVisibleRead(t *testing.T) {
	t.Parallel()
	body := "› \x1b[2mAsk anything\x1b[0m\r\n\r\nstatus line\r\n"
	runner := fakeRunnerWithBinary(t, "herdr")
	runner.respond(snapshotReply(t, directorLayouts(), directorPanes(3)))
	runner.respond(body)
	capture, err := herdrOnServer(runner).captureScreen(herdrTestPane)
	want := screenCapture{rows: 3, cols: 97, body: body}
	if err != nil || capture != want {
		t.Fatalf("capture %#v (%v), want %#v with no cursor or alternate-screen flag", capture, err, want)
	}
	calls := runner.runArgvs()
	if len(calls) != 2 || !slices.Equal(calls[0], herdrCall("api", "snapshot")) {
		t.Fatalf("calls %q", calls)
	}
	assertReadOptions(t, calls[1], herdrTestPane, map[string]string{"--source": "visible", "--format": "ansi"})
}

func TestMultiplexerHerdr_DirectorPane_CaptureScreenKeepsTheViewportRowsWhenTheVisibleReadDropsTrailingBlankRows(t *testing.T) {
	t.Parallel()
	body := "\r\nfirst row\r\n\r\nlast drawn row\r\n"
	runner := fakeRunnerWithBinary(t, "herdr")
	runner.respond(snapshotReply(t, directorLayouts(), directorPanes(30)))
	runner.respond(body)
	capture, err := herdrOnServer(runner).captureScreen(herdrTestPane)
	if err != nil || capture.rows != 30 || capture.cols != 97 || capture.body != body {
		t.Fatalf("capture %#v (%v): the viewport keeps its rows and blank rows keep their positions", capture, err)
	}
}

func TestMultiplexerHerdr_DirectorPane_CaptureScreenFailsForAPaneAbsentFromTheSnapshotOrAnUnreadableReply(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		respond func(*fakeRunner)
	}{
		{name: "pane absent from the layouts", respond: func(f *fakeRunner) {
			f.respond(snapshotReply(t, []any{map[string]any{"panes": []any{snapshotLayoutPane("w1:p1", 60, 40)}}}, directorPanes(30)))
		}},
		{name: "pane absent from the snapshot panes", respond: func(f *fakeRunner) {
			f.respond(snapshotReply(t, directorLayouts(), []any{snapshotPaneState("w1:p1", 40), snapshotPaneState("w2:p1", 41)}))
		}},
		{name: "no snapshot panes", respond: func(f *fakeRunner) {
			f.respond(herdrEnvelope(t, map[string]any{"snapshot": map[string]any{"layouts": directorLayouts()}}))
		}},
		{name: "no viewport rows", respond: func(f *fakeRunner) {
			f.respond(snapshotReply(t, directorLayouts(), []any{map[string]any{"pane_id": herdrTestPane, "scroll": map[string]any{"offset_from_bottom": 0}}}))
		}},
		{name: "scroll only on the layout entry", respond: func(f *fakeRunner) {
			f.respond(snapshotReply(t,
				[]any{map[string]any{"panes": []any{map[string]any{
					"focused": true, "pane_id": herdrTestPane,
					"rect":   map[string]any{"height": 30, "width": 97, "x": 0, "y": 0},
					"scroll": map[string]any{"viewport_rows": 30},
				}}}},
				[]any{map[string]any{"pane_id": herdrTestPane}}))
		}},
		{name: "non-JSON snapshot", respond: func(f *fakeRunner) { f.respond("not json") }},
		{name: "missing layouts", respond: func(f *fakeRunner) { f.respond(herdrEnvelope(t, map[string]any{"snapshot": map[string]any{}})) }},
		{name: "server error", respond: func(f *fakeRunner) { f.failCode(t, "server_unavailable") }},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "herdr")
		tc.respond(runner)
		runner.respond("body\r\n")
		if _, err := herdrOnServer(runner).captureScreen(herdrTestPane); err == nil {
			t.Fatalf("%s is a capture failure", tc.name)
		}
	}
}

func TestMultiplexerHerdr_DirectorPane_TypeLiteralSendsTheWholeLiteralInOneSendText(t *testing.T) {
	t.Parallel()
	for _, literal := range []string{"[cafleet msg 7 from 3 2026-11-23T10:00:00+00:00] hi", strings.Repeat("あ", 600)} {
		runner := fakeRunnerWithBinary(t, "herdr")
		runner.succeed(1)
		if err := typeLiteral(herdrOnServer(runner), herdrTestPane, literal); err != nil {
			t.Fatal(err)
		}
		assertCalls(t, "send-text", runner.runArgvs(), [][]string{herdrCall("pane", "send-text", herdrTestPane, literal)})
	}
}

func TestMultiplexerHerdr_DirectorPane_PressSubmitMapsTheCarriageReturnToTheEnterKey(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "herdr")
	runner.succeed(1)
	if err := herdrOnServer(runner).pressSubmit(herdrTestPane, []byte("\r")); err != nil {
		t.Fatal(err)
	}
	assertCalls(t, "enter", runner.runArgvs(), [][]string{herdrCall("pane", "send-keys", herdrTestPane, "enter")})
}

func TestMultiplexerHerdr_DirectorPane_PressSubmitRefusesBytesWithoutAKeyMappingBeforeAnyProcess(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "herdr")
	err := herdrOnServer(runner).pressSubmit(herdrTestPane, []byte{0xff})
	if err == nil || err.submissionStarted {
		t.Fatalf("an unmapped submit byte: %v", err)
	}
	assertCalls(t, "unmapped", runner.runArgvs(), nil)
}

func TestMultiplexerHerdr_DirectorPane_WriteFailuresReportWhetherAProcessMayHaveWritten(t *testing.T) {
	t.Parallel()
	literal := func(m multiplexer) *multiplexerError { return typeLiteral(m, herdrTestPane, "short") }
	submit := func(m multiplexer) *multiplexerError { return m.pressSubmit(herdrTestPane, []byte("\r")) }
	notStarted := &runError{kind: runErrorNotStarted, detail: "spawn failed"}
	cases := []struct {
		name    string
		write   func(multiplexer) *multiplexerError
		respond func(*fakeRunner)
		written bool
	}{
		{name: "send-text failed", write: literal, respond: func(f *fakeRunner) { f.failCode(t, "server_unavailable") }, written: true},
		{name: "send-text timed out", write: literal, respond: func(f *fakeRunner) { f.fail(&runError{kind: runErrorTimeout}) }, written: true},
		{name: "send-text unreadable acknowledgment", write: literal, respond: func(f *fakeRunner) { f.respond("unexpected output") }, written: true},
		{name: "send-text not started", write: literal, respond: func(f *fakeRunner) { f.fail(notStarted) }},
		{name: "send-text binary missing", write: literal, respond: func(f *fakeRunner) { f.fail(&runError{kind: runErrorBinaryNotFound, detail: "herdr"}) }},
		{name: "send-keys failed", write: submit, respond: func(f *fakeRunner) { f.failCode(t, "server_unavailable") }, written: true},
		{name: "send-keys not started", write: submit, respond: func(f *fakeRunner) { f.fail(notStarted) }},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "herdr")
		tc.respond(runner)
		err := tc.write(herdrOnServer(runner))
		if err == nil || err.submissionStarted != tc.written {
			t.Fatalf("%s: error %v, want submissionStarted=%v", tc.name, err, tc.written)
		}
	}
}

func TestMultiplexerHerdr_DirectorPane_AnExpiredDeadlineWritesNoTextAndNoKey(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "herdr")
	mux := herdrOnServer(runner).bounded(deadlineUntil(time.Now().Add(-time.Millisecond)))
	if err := typeLiteral(mux, herdrTestPane, "short"); err == nil || err.submissionStarted {
		t.Fatalf("an expired text write: %v", err)
	}
	if err := mux.pressSubmit(herdrTestPane, []byte("\r")); err == nil || err.submissionStarted {
		t.Fatalf("an expired submit key: %v", err)
	}
	assertCalls(t, "expired", runner.runArgvs(), nil)
}
