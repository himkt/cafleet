package core

import (
	"slices"
	"strings"
	"testing"
	"time"
)

const tmuxTestSocket = "/tmp/tmux-1000/default"

func tmuxOnServer(runner commandRunner) *tmuxMultiplexer {
	return tmuxMultiplexerForServer(runner, tmuxTestSocket)
}

// tmuxCall is `tmux -S <socket> <parts…>`.
func tmuxCall(parts ...string) []string {
	return append([]string{"tmux", "-S", tmuxTestSocket}, parts...)
}

type tmuxReply struct {
	out    string
	stderr string
	failed bool
}

func okReply(out string) tmuxReply {
	return tmuxReply{out: out}
}

func failedReply(stderr string) tmuxReply {
	return tmuxReply{stderr: stderr, failed: true}
}

func (f *fakeRunner) queue(replies ...tmuxReply) {
	for _, reply := range replies {
		if reply.failed {
			f.fail(failedRun(reply.stderr))
		} else {
			f.respond(reply.out)
		}
	}
}

func TestMultiplexerTmux_PaneProcessReadsThePidOfTheMatchingPaneOnly(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name   string
		reply  tmuxReply
		pid    opt[int64]
		detail string
		fails  bool
	}{
		{name: "matching pane", reply: okReply("%3|400625\n"), pid: some[int64](400625)},
		{name: "empty pane id and pid", reply: okReply("|\n")},
		{name: "empty pane id", reply: okReply("|400625\n")},
		{name: "other pane", reply: okReply("%4|400625\n")},
		{name: "pane id with the same prefix", reply: okReply("%30|400625\n")},
		{name: "non-integer pid", reply: okReply("%3|abc\n"), detail: "%3|abc", fails: true},
		{name: "empty pid", reply: okReply("%3|\n"), detail: "%3|", fails: true},
		{name: "zero pid", reply: okReply("%3|0\n"), detail: "%3|0", fails: true},
		{name: "negative pid", reply: okReply("%3|-5\n"), detail: "%3|-5", fails: true},
		{name: "pid with a suffix", reply: okReply("%3|12x\n"), detail: "%3|12x", fails: true},
		{name: "command failure", reply: failedReply("no server running on /tmp/tmux-1000/default"), fails: true},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "tmux")
		runner.queue(tc.reply)
		pid, err := tmuxOnServer(runner).paneProcess("%3")
		switch {
		case !tc.fails && (err != nil || pid != tc.pid):
			t.Fatalf("%s: pid %#v (%v), want %#v", tc.name, pid, err, tc.pid)
		case tc.fails && err == nil:
			t.Fatalf("%s: succeeded with %#v", tc.name, pid)
		case tc.fails && !strings.Contains(err.Error(), tc.detail):
			t.Fatalf("%s: error %q lacks %q", tc.name, err.Error(), tc.detail)
		case tc.fails && !tc.reply.failed && !strings.HasPrefix(err.Error(), "unexpected tmux display-message output: "):
			t.Fatalf("%s: error %q", tc.name, err.Error())
		}
		assertCalls(t, tc.name, runner.runArgvs(), [][]string{tmuxCall("display-message", "-p", "-t", "%3", "#{pane_id}|#{pane_pid}")})
	}
}

func TestMultiplexerTmux_CapturePaneRejectsNonPositiveLines(t *testing.T) {
	t.Parallel()
	_, err := tmuxOnServer(fakeRunnerWithBinary(t, "tmux")).capturePane("%5", 0)
	assertMultiplexerError(t, "zero lines", err, "capture_pane: lines must be positive, got 0")
}

func TestMultiplexerTmux_CapturePaneKeepsTheLastLinesAfterDroppingTheBlankTail(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "tmux")
	runner.respond("l1\nl2\nl3\nx\rY\n")
	captured, err := tmuxOnServer(runner).capturePane("%5", 2)
	if err != nil || captured != "l3\nx\rY" {
		t.Fatalf("captured %q (%v): the \\r survives and the trailing blank line is dropped before the slice", captured, err)
	}
	assertCalls(t, "capture over-fetches by the fixed 1000-line margin", runner.runArgvs(),
		[][]string{tmuxCall("capture-pane", "-p", "-e", "-t", "%5", "-S", "-1002")})
}

const (
	tmuxTestCwd        = "/work/project"
	tmuxTestListFormat = "#{pane_id}|#{pane_left}"
)

func tmuxTestCommand() []string {
	return []string{"/abs/cafleet", "__member-run", "--descriptor", "/run/d.json"}
}

func tmuxColumn(side columnSide, columnPanes, otherPanes []string) paneTarget {
	return columnTarget{windowID: "@1", side: side, columnPanes: columnPanes, otherPanes: otherPanes}
}

func tmuxNewWindow() paneTarget {
	return paneTargetNewWindow{session: "main", windowName: "cafleet-3-1"}
}

func tmuxWithCommand(parts ...string) []string {
	return append(tmuxCall(parts...), tmuxTestCommand()...)
}

func tmuxSplitBelow(target string) []string {
	return tmuxWithCommand("split-window", "-v", "-d", "-t", target, "-P", "-F", "#{pane_id}")
}

func TestMultiplexerTmux_Spawn_SpawnMemberPaneSplitsNamesAndEqualizesEachTarget(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		target  paneTarget
		title   string
		replies []string
		spawned spawnedPane
		calls   [][]string
	}{
		{
			name: "a column with members splits below its last pane", target: tmuxColumn(columnSideRight, []string{"%2", "%5"}, []string{"%0"}), title: "drafter",
			replies: []string{"%9\n", "", "%0|0\n%2|61\n%5|61\n%9|61\n", ""}, spawned: spawnedPane{paneID: "%9", windowID: "@1"},
			calls: [][]string{
				tmuxSplitBelow("%5"),
				tmuxCall("select-pane", "-t", "%9", "-T", "drafter"),
				tmuxCall("list-panes", "-t", "@1", "-F", tmuxTestListFormat),
				tmuxCall("select-layout", "-E", "-t", "%2"),
			},
		},
		{
			name: "an empty right column takes a full-height right half and needs no layout", target: tmuxColumn(columnSideRight, nil, []string{"%0"}), title: "drafter",
			replies: []string{"%9\n", "", "%0|0\n%9|61\n"}, spawned: spawnedPane{paneID: "%9", windowID: "@1"},
			calls: [][]string{
				tmuxWithCommand("split-window", "-h", "-f", "-l", "50%", "-d", "-t", "%0", "-P", "-F", "#{pane_id}"),
				tmuxCall("select-pane", "-t", "%9", "-T", "drafter"),
				tmuxCall("list-panes", "-t", "@1", "-F", tmuxTestListFormat),
			},
		},
		{
			name: "an empty left column takes a full-height left half before the other column", target: tmuxColumn(columnSideLeft, nil, []string{"%3", "%4"}), title: "reviewer",
			replies: []string{"%9\n", "", "%9|0\n%3|61\n%4|61\n", ""}, spawned: spawnedPane{paneID: "%9", windowID: "@1"},
			calls: [][]string{
				tmuxWithCommand("split-window", "-h", "-f", "-b", "-l", "50%", "-d", "-t", "%3", "-P", "-F", "#{pane_id}"),
				tmuxCall("select-pane", "-t", "%9", "-T", "reviewer"),
				tmuxCall("list-panes", "-t", "@1", "-F", tmuxTestListFormat),
				tmuxCall("select-layout", "-E", "-t", "%3"),
			},
		},
		{
			name: "a new window opens detached and named in the session with the cwd", target: tmuxNewWindow(), title: "tester",
			replies: []string{"@6|%12\n", "", "%12|0\n"}, spawned: spawnedPane{paneID: "%12", windowID: "@6"},
			calls: [][]string{
				tmuxWithCommand("new-window", "-d", "-t", "main:", "-n", "cafleet-3-1", "-c", tmuxTestCwd, "-P", "-F", "#{window_id}|#{pane_id}"),
				tmuxCall("select-pane", "-t", "%12", "-T", "tester"),
				tmuxCall("list-panes", "-t", "@6", "-F", tmuxTestListFormat),
			},
		},
		{
			name: "equalization runs once per column of two or more panes", target: tmuxColumn(columnSideRight, []string{"%10", "%11"}, []string{"%7", "%8"}), title: "worker",
			replies: []string{"%9\n", "", "%7|0\n%8|0\n%9|61\n%10|61\n%11|61\n", "", ""}, spawned: spawnedPane{paneID: "%9", windowID: "@1"},
			calls: [][]string{
				tmuxSplitBelow("%11"),
				tmuxCall("select-pane", "-t", "%9", "-T", "worker"),
				tmuxCall("list-panes", "-t", "@1", "-F", tmuxTestListFormat),
				tmuxCall("select-layout", "-E", "-t", "%7"),
				tmuxCall("select-layout", "-E", "-t", "%9"),
			},
		},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "tmux")
		for _, reply := range tc.replies {
			runner.respond(reply)
		}
		spawned, err := tmuxOnServer(runner).spawnMemberPane(tc.target, tc.title, tmuxTestCwd, tmuxTestCommand())
		if err != nil || spawned != tc.spawned {
			t.Fatalf("%s: spawned %#v (%v), want %#v", tc.name, spawned, err, tc.spawned)
		}
		assertCalls(t, tc.name, runner.runArgvs(), tc.calls)
	}
}

func TestMultiplexerTmux_Spawn_NamingAndEqualizationFailuresNeverFailTheSpawn(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name                 string
		rename, list, layout tmuxReply
	}{
		{name: "rename and layout fail", rename: failedReply("rename boom"), list: okReply("%0|0\n%2|61\n%9|61\n"), layout: failedReply("layout boom")},
		{name: "list fails", rename: okReply(""), list: failedReply("list boom"), layout: okReply("")},
		{name: "list is malformed", rename: okReply(""), list: okReply("garbage without separator\n"), layout: okReply("")},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "tmux")
		runner.queue(okReply("%9\n"), tc.rename, tc.list, tc.layout)
		spawned, err := tmuxOnServer(runner).spawnMemberPane(tmuxColumn(columnSideRight, []string{"%2"}, []string{"%0"}), "drafter", tmuxTestCwd, tmuxTestCommand())
		if err != nil || spawned.paneID != "%9" {
			t.Fatalf("%s: spawned %#v (%v)", tc.name, spawned, err)
		}
		for _, call := range runner.runArgvs() {
			if call[3] == "kill-pane" {
				t.Fatalf("%s: a best-effort failure closed the spawned pane", tc.name)
			}
		}
	}
}

func TestMultiplexerTmux_Spawn_AFailedOrMalformedCreationReplyLeavesPaneCleanupUnknown(t *testing.T) {
	t.Parallel()
	split := tmuxColumn(columnSideRight, []string{"%2"}, []string{"%0"})
	cases := []struct {
		name    string
		target  paneTarget
		reply   tmuxReply
		message string
	}{
		{name: "blank split reply", target: split, reply: okReply("   \n"), message: "pane ID unknown; pane cleanup unconfirmed"},
		{name: "failed split", target: split, reply: failedReply("split failed"), message: "pane ID unknown; pane cleanup unconfirmed"},
		{name: "empty new-window reply", target: tmuxNewWindow(), reply: okReply("\n"), message: "unexpected tmux new-window output: "},
		{name: "new-window reply without a pane", target: tmuxNewWindow(), reply: okReply("@6\n"), message: "unexpected tmux new-window output: \"@6"},
		{name: "new-window reply with an empty pane", target: tmuxNewWindow(), reply: okReply("@6|\n"), message: "unexpected tmux new-window output: \"@6|"},
		{name: "new-window reply without a window", target: tmuxNewWindow(), reply: okReply("|%12\n"), message: "unexpected tmux new-window output: \"|%12"},
		{name: "failed new-window", target: tmuxNewWindow(), reply: failedReply("no server running")},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "tmux")
		runner.queue(tc.reply)
		_, err := tmuxOnServer(runner).spawnMemberPane(tc.target, "drafter", tmuxTestCwd, tmuxTestCommand())
		if err == nil {
			t.Fatalf("%s: the spawn succeeded", tc.name)
		}
		if err.paneCleanup != some[paneCleanup](paneCleanupUnknown{}) {
			t.Fatalf("%s: pane cleanup %#v, want unknown", tc.name, err.paneCleanup)
		}
		if !strings.Contains(err.Error(), tc.message) {
			t.Fatalf("%s: error %q lacks %q", tc.name, err.Error(), tc.message)
		}
		if len(runner.runArgvs()) != 1 {
			t.Fatalf("%s: %d process calls, want 1", tc.name, len(runner.runArgvs()))
		}
	}
}

func TestMultiplexerTmux_WindowReads_RenameWindowTargetsTheRecordedSocketAndConcreteWindow(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name  string
		reply tmuxReply
	}{
		{name: "rename succeeds", reply: okReply("")},
		{name: "rename fails", reply: failedReply("rename denied")},
	} {
		runner := fakeRunnerWithBinary(t, "tmux")
		runner.queue(tc.reply)
		err := tmuxOnServer(runner).renameWindow("@17", "cafleet-42-0")
		switch {
		case !tc.reply.failed && err != nil:
			t.Fatalf("%s: %v", tc.name, err)
		case tc.reply.failed && (err == nil || !strings.Contains(err.Error(), "rename denied")):
			t.Fatalf("%s: error %v lacks the cause", tc.name, err)
		}
		assertCalls(t, tc.name, runner.runArgvs(), [][]string{tmuxCall("rename-window", "-t", "@17", "cafleet-42-0")})
	}
}

// assertDisplayRead checks a display-message read: the expected value, or a failure whose text
// begins with the unexpected-output prefix exactly when unexpected is set.
func assertDisplayRead[T comparable](t *testing.T, name string, got T, err *multiplexerError, want opt[T], unexpected bool) {
	t.Helper()
	if value, ok := want.get(); ok {
		if err != nil || got != value {
			t.Fatalf("%s: read %#v (%v), want %#v", name, got, err, value)
		}
		return
	}
	if err == nil {
		t.Fatalf("%s: read %#v, want a failure", name, got)
	}
	if prefixed := strings.HasPrefix(err.Error(), "unexpected tmux display-message output: "); prefixed != unexpected {
		t.Fatalf("%s: error %q, unexpected-output prefix %v, want %v", name, err.Error(), prefixed, unexpected)
	}
}

func TestMultiplexerTmux_WindowReads_WindowNameReadsTheNamedWindowAndKeepsEverySeparatorAfterTheFirst(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name       string
		reply      tmuxReply
		want       opt[string]
		unexpected bool
	}{
		{name: "plain name", reply: okReply("@1|main\n"), want: some("main")},
		{name: "empty name", reply: okReply("@1|\n"), want: some("")},
		{name: "name with a separator", reply: okReply("@1|agents|left\n"), want: some("agents|left")},
		{name: "name with spaces", reply: okReply("@1| agents | left \n"), want: some(" agents | left ")},
		{name: "name ending in CR", reply: okReply("@1|agents\r\n"), want: some("agents\r")},
		{name: "name with a trailing blank line", reply: okReply("@1|agents\n\n"), want: some("agents\n")},
		{name: "name of CR LF", reply: okReply("@1|\r\n\n"), want: some("\r\n")},
		{name: "blank reply", reply: okReply("\n"), unexpected: true},
		{name: "empty reply", reply: okReply(""), unexpected: true},
		{name: "reply without a window", reply: okReply("main\n"), unexpected: true},
		{name: "other window", reply: okReply("@2|main\n"), unexpected: true},
		{name: "empty window id", reply: okReply("|main\n"), unexpected: true},
		{name: "command failure", reply: failedReply("no server running")},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "tmux")
		runner.queue(tc.reply)
		name, err := tmuxOnServer(runner).windowName("@1")
		assertDisplayRead(t, tc.name, name, err, tc.want, tc.unexpected)
		assertCalls(t, tc.name, runner.runArgvs(), [][]string{tmuxCall("display-message", "-p", "-t", "@1", "#{window_id}|#{window_name}")})
	}
}

func TestMultiplexerTmux_WindowReads_WindowHeightReadsTheWindowByIdAndIgnoresTheAnchors(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name       string
		reply      tmuxReply
		want       opt[int64]
		unexpected bool
	}{
		{name: "positive height", reply: okReply("120\n"), want: some[int64](120)},
		{name: "blank reply", reply: okReply("\n"), unexpected: true},
		{name: "empty reply", reply: okReply(""), unexpected: true},
		{name: "non-integer", reply: okReply("abc\n"), unexpected: true},
		{name: "zero", reply: okReply("0\n"), unexpected: true},
		{name: "negative", reply: okReply("-3\n"), unexpected: true},
		{name: "integer with a suffix", reply: okReply("12x\n"), unexpected: true},
		{name: "fraction", reply: okReply("4.5\n"), unexpected: true},
		{name: "command failure", reply: failedReply("no server running")},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "tmux")
		runner.queue(tc.reply)
		height, err := tmuxOnServer(runner).windowHeight("@4", []string{"%5", "%6"})
		assertDisplayRead(t, tc.name, height, err, tc.want, tc.unexpected)
		assertCalls(t, tc.name, runner.runArgvs(), [][]string{tmuxCall("display-message", "-p", "-t", "@4", "#{window_height}")})
	}
}

func TestMultiplexerTmux_Close_KillPaneClosesThenEqualizesTheWindowAndToleratesOnlyAGonePane(t *testing.T) {
	t.Parallel()
	kill := func(pane string) []string { return tmuxCall("kill-pane", "-t", pane) }
	list := func(window string) []string { return tmuxCall("list-panes", "-t", window, "-F", tmuxTestListFormat) }
	layout := func(pane string) []string { return tmuxCall("select-layout", "-E", "-t", pane) }
	cases := []struct {
		name    string
		pane    string
		window  string
		replies []tmuxReply
		closes  bool
		calls   [][]string
	}{
		{name: "close then equalize every column", pane: "%7", window: "@1",
			replies: []tmuxReply{okReply(""), okReply("%0|0\n%2|61\n%3|61\n"), okReply("")}, closes: true,
			calls: [][]string{kill("%7"), list("@1"), layout("%2")}},
		{name: "a gone pane still restores the window", pane: "%5", window: "@1",
			replies: []tmuxReply{failedReply("No Such Pane: %5"), okReply("%0|0\n")}, closes: true,
			calls: [][]string{kill("%5"), list("@1")}},
		{name: "another close failure is refused before restoration", pane: "%5", window: "@1",
			replies: []tmuxReply{failedReply("server exited unexpectedly")},
			calls:   [][]string{kill("%5")}},
		{name: "a failed list leaves the close successful", pane: "%7", window: "@1",
			replies: []tmuxReply{okReply(""), failedReply("server exited unexpectedly")}, closes: true,
			calls: [][]string{kill("%7"), list("@1")}},
		{name: "a failed layout leaves the close successful", pane: "%7", window: "@1",
			replies: []tmuxReply{okReply(""), okReply("%0|0\n%2|61\n%3|61\n"), failedReply("layout boom")}, closes: true,
			calls: [][]string{kill("%7"), list("@1"), layout("%2")}},
		{name: "the last pane of a window leaves nothing to restore", pane: "%12", window: "@6",
			replies: []tmuxReply{okReply(""), failedReply("can't find window: @6")}, closes: true,
			calls: [][]string{kill("%12"), list("@6")}},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "tmux")
		runner.queue(tc.replies...)
		err := killPane(tmuxOnServer(runner), tc.pane, tc.window)
		if (err == nil) != tc.closes {
			t.Fatalf("%s: kill result %v, want closed=%v", tc.name, err, tc.closes)
		}
		assertCalls(t, tc.name, runner.runArgvs(), tc.calls)
	}
}

func TestMultiplexerTmux_Close_ClosePaneClosesOnlyAndRestorationEqualizesTheRecordedWindowLater(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "tmux")
	runner.succeed(1)
	mux := tmuxOnServer(runner)
	if err := mux.closePane("%7", "@4"); err != nil {
		t.Fatal(err)
	}
	assertCalls(t, "close", runner.runArgvs(), [][]string{tmuxCall("kill-pane", "-t", "%7")})
	runner.respond("%20|0\n%21|0\n%22|61\n")
	runner.succeed(1)
	if err := mux.restoreAfterClose(); err != nil {
		t.Fatal(err)
	}
	assertCalls(t, "restore", runner.runArgvs()[1:], [][]string{
		tmuxCall("list-panes", "-t", "@4", "-F", tmuxTestListFormat),
		tmuxCall("select-layout", "-E", "-t", "%20"),
	})
}

func TestMultiplexerTmux_Close_RestoringAClosedWindowDoesNothingMore(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "tmux")
	runner.succeed(1)
	mux := tmuxOnServer(runner)
	if err := mux.closePane("%12", "@6"); err != nil {
		t.Fatal(err)
	}
	runner.fail(failedRun("can't find window: @6"))
	if err := mux.restoreAfterClose(); err != nil {
		t.Fatal(err)
	}
	if len(runner.runArgvs()) != 2 {
		t.Fatalf("%d process calls, want 2", len(runner.runArgvs()))
	}
}

func TestMultiplexerTmux_Close_ClosePaneToleratesAPaneGoneFailureAndRefusesOthers(t *testing.T) {
	t.Parallel()
	gone := fakeRunnerWithBinary(t, "tmux")
	gone.fail(failedRun("can't find pane: %7"))
	if err := tmuxOnServer(gone).closePane("%7", "@4"); err != nil {
		t.Fatalf("a gone pane closes: %v", err)
	}
	other := fakeRunnerWithBinary(t, "tmux")
	other.fail(failedRun("server exited unexpectedly"))
	if err := tmuxOnServer(other).closePane("%7", "@4"); err == nil {
		t.Fatal("another close failure is refused")
	}
}

const (
	tmuxTestPane     = "%5"
	tmuxTestMetadata = "#{pane_height} #{pane_width} #{cursor_y} #{cursor_x} #{alternate_on}"
)

func tmuxMetadataCall() []string {
	return tmuxCall("display-message", "-p", "-t", tmuxTestPane, tmuxTestMetadata)
}

func tmuxBodyCall() []string {
	return tmuxCall("capture-pane", "-p", "-e", "-N", "-t", tmuxTestPane)
}

func tmuxLiteralCall(chunk string) []string {
	return tmuxCall("send-keys", "-t", tmuxTestPane, "-l", "--", chunk)
}

func TestMultiplexerTmux_DirectorPane_CaptureScreenReadsMetadataThenTheBodyThenTheSameMetadata(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name      string
		metadata  string
		alternate bool
	}{
		{name: "primary screen", metadata: "24 80 23 2 0\n"},
		{name: "alternate screen", metadata: "24 80 23 2 1\n", alternate: true},
	} {
		body := "\x1b[1mtitle\x1b[0m\n› \n"
		runner := fakeRunnerWithBinary(t, "tmux")
		for _, reply := range []string{tc.metadata, body, tc.metadata} {
			runner.respond(reply)
		}
		capture, err := tmuxOnServer(runner).captureScreen(tmuxTestPane)
		want := screenCapture{rows: 24, cols: 80, cursor: some([2]uint16{23, 2}), alternateScreen: some(tc.alternate), body: body}
		if err != nil || capture != want {
			t.Fatalf("%s: capture %#v (%v), want %#v", tc.name, capture, err, want)
		}
		assertCalls(t, tc.name, runner.runArgvs(), [][]string{tmuxMetadataCall(), tmuxBodyCall(), tmuxMetadataCall()})
	}
}

func TestMultiplexerTmux_DirectorPane_CaptureScreenRetriesTheWholeCaptureWhenTheMetadataLineChanges(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "tmux")
	for _, reply := range []string{"24 80 23 2 0\n", "stale body\n", "30 100 29 4 0\n", "30 100 29 4 0\n", "settled body\n", "30 100 29 4 0\n"} {
		runner.respond(reply)
	}
	capture, err := tmuxOnServer(runner).captureScreen(tmuxTestPane)
	if err != nil || capture.rows != 30 || capture.cols != 100 || capture.cursor != some([2]uint16{29, 4}) || capture.body != "settled body\n" {
		t.Fatalf("capture %#v (%v), want the settled 30x100 screen", capture, err)
	}
	assertCalls(t, "retry", runner.runArgvs(), [][]string{
		tmuxMetadataCall(), tmuxBodyCall(), tmuxMetadataCall(), tmuxMetadataCall(), tmuxBodyCall(), tmuxMetadataCall(),
	})
}

func TestMultiplexerTmux_DirectorPane_CaptureScreenFailsWhenTheGeometryNeverSettlesWithinTheDeadline(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "tmux")
	for round := range 200 {
		runner.respond("24 80 23 2 0\n")
		runner.respond("body\n")
		runner.respond("24 80 23 " + string(rune('3'+round%7)) + " 0\n")
	}
	started := time.Now()
	_, err := tmuxOnServer(runner).bounded(deadlineUntil(started.Add(300 * time.Millisecond))).captureScreen(tmuxTestPane)
	if err == nil {
		t.Fatal("unstable geometry is a capture failure")
	}
	if elapsed := time.Since(started); elapsed >= 3*time.Second {
		t.Fatalf("the retries stop at the deadline: %v", elapsed)
	}
}

func TestMultiplexerTmux_DirectorPane_CaptureScreenFailsOnAMalformedMetadataLineOrAFailedCommand(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name     string
		metadata tmuxReply
	}{
		{name: "non-numeric field", metadata: okReply("24 80 x 2 0\n")},
		{name: "missing field", metadata: okReply("24 80 23 2\n")},
		{name: "extra field", metadata: okReply("24 80 23 2 0 9\n")},
		{name: "empty line", metadata: okReply("\n")},
		{name: "missing pane", metadata: failedReply("can't find pane: %5")},
		{name: "server error", metadata: failedReply("server exited unexpectedly")},
	} {
		runner := fakeRunnerWithBinary(t, "tmux")
		runner.queue(tc.metadata, okReply("body\n"), tc.metadata)
		if _, err := tmuxOnServer(runner).captureScreen(tmuxTestPane); err == nil {
			t.Fatalf("%s is a capture failure", tc.name)
		}
	}
}

func TestMultiplexerTmux_DirectorPane_TypeLiteralSendsLiteralChunksOfAtMost256CharactersInOrder(t *testing.T) {
	t.Parallel()
	literal := strings.Repeat("あ", 300) + "-; tail"
	characters := []rune(literal)
	chunks := []string{string(characters[:256]), string(characters[256:])}
	runner := fakeRunnerWithBinary(t, "tmux")
	runner.succeed(len(chunks))
	if err := typeLiteral(tmuxOnServer(runner), tmuxTestPane, literal); err != nil {
		t.Fatal(err)
	}
	assertCalls(t, "chunks", runner.runArgvs(), [][]string{tmuxLiteralCall(chunks[0]), tmuxLiteralCall(chunks[1])})
}

func TestMultiplexerTmux_DirectorPane_TypeLiteralSendsAShortLiteralInOneProcess(t *testing.T) {
	t.Parallel()
	const literal = "[cafleet msg 7 from 3 2026-11-23T10:00:00+00:00] hi"
	runner := fakeRunnerWithBinary(t, "tmux")
	runner.succeed(1)
	if err := typeLiteral(tmuxOnServer(runner), tmuxTestPane, literal); err != nil {
		t.Fatal(err)
	}
	assertCalls(t, "short literal", runner.runArgvs(), [][]string{tmuxLiteralCall(literal)})
}

func TestMultiplexerTmux_DirectorPane_PressSubmitSendsTheSubmitBytesAsHexKeys(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		submit []byte
		hex    []string
	}{
		{submit: []byte("\r"), hex: []string{"0d"}},
		{submit: []byte("\x1b\r"), hex: []string{"1b", "0d"}},
	} {
		runner := fakeRunnerWithBinary(t, "tmux")
		runner.succeed(1)
		if err := tmuxOnServer(runner).pressSubmit(tmuxTestPane, tc.submit); err != nil {
			t.Fatal(err)
		}
		assertCalls(t, strings.Join(tc.hex, " "), runner.runArgvs(), [][]string{slices.Concat(tmuxCall("send-keys", "-t", tmuxTestPane, "-H"), tc.hex)})
	}
}

func TestMultiplexerTmux_DirectorPane_TypeLiteralFailuresReportWhetherAProcessMayHaveWritten(t *testing.T) {
	t.Parallel()
	long := strings.Repeat("a", 300)
	cases := []struct {
		name    string
		literal string
		replies []fakeResponse
		written bool
		calls   int
	}{
		{name: "spawn failure before the first process", literal: "short", replies: []fakeResponse{{err: &runError{kind: runErrorNotStarted, detail: "spawn failed"}}}, calls: 1},
		{name: "missing binary", literal: "short", replies: []fakeResponse{{err: &runError{kind: runErrorBinaryNotFound, detail: "tmux"}}}, calls: 1},
		{name: "first process failed", literal: "short", replies: []fakeResponse{{err: failedRun("server exited unexpectedly")}}, written: true, calls: 1},
		{name: "first process timed out", literal: "short", replies: []fakeResponse{{err: &runError{kind: runErrorTimeout}}}, written: true, calls: 1},
		{name: "unreadable acknowledgment", literal: "short", replies: []fakeResponse{{out: "unexpected output"}}, written: true, calls: 1},
		{name: "a later chunk not started", literal: long, replies: []fakeResponse{{}, {err: &runError{kind: runErrorNotStarted, detail: "spawn failed"}}}, written: true, calls: 2},
	}
	for _, tc := range cases {
		runner := fakeRunnerWithBinary(t, "tmux")
		runner.responses = append(runner.responses, tc.replies...)
		err := typeLiteral(tmuxOnServer(runner), tmuxTestPane, tc.literal)
		if err == nil || err.submissionStarted != tc.written {
			t.Fatalf("%s: error %v, want submissionStarted=%v", tc.name, err, tc.written)
		}
		if len(runner.runArgvs()) != tc.calls {
			t.Fatalf("%s: %d process calls, want %d", tc.name, len(runner.runArgvs()), tc.calls)
		}
	}
}

func TestMultiplexerTmux_DirectorPane_AnExpiredDeadlineWritesNoLiteralAndNoSubmitKey(t *testing.T) {
	t.Parallel()
	runner := fakeRunnerWithBinary(t, "tmux")
	mux := tmuxOnServer(runner).bounded(deadlineUntil(time.Now().Add(-time.Millisecond)))
	if err := typeLiteral(mux, tmuxTestPane, "short"); err == nil || err.submissionStarted {
		t.Fatalf("an expired literal write: %v", err)
	}
	if err := mux.pressSubmit(tmuxTestPane, []byte("\r")); err == nil || err.submissionStarted {
		t.Fatalf("an expired submit key: %v", err)
	}
	assertCalls(t, "expired", runner.runArgvs(), nil)
}

func TestMultiplexerTmux_DirectorPane_PressSubmitFailuresReportWhetherTheKeyMayHaveBeenWritten(t *testing.T) {
	t.Parallel()
	for _, tc := range []struct {
		name    string
		reply   fakeResponse
		written bool
	}{
		{name: "process failed", reply: fakeResponse{err: failedRun("server exited unexpectedly")}, written: true},
		{name: "process timed out", reply: fakeResponse{err: &runError{kind: runErrorTimeout}}, written: true},
		{name: "unreadable acknowledgment", reply: fakeResponse{out: "unexpected output"}, written: true},
		{name: "process not started", reply: fakeResponse{err: &runError{kind: runErrorNotStarted, detail: "spawn failed"}}},
	} {
		runner := fakeRunnerWithBinary(t, "tmux")
		runner.responses = append(runner.responses, tc.reply)
		err := tmuxOnServer(runner).pressSubmit(tmuxTestPane, []byte("\r"))
		if err == nil || err.submissionStarted != tc.written {
			t.Fatalf("%s: error %v, want submissionStarted=%v", tc.name, err, tc.written)
		}
	}
}
