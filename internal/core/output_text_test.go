package core

import (
	"strings"
	"testing"
)

func TestOutputText_QuotedEscapesLikeRustStrDebug(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name  string
		input string
		want  string
	}{
		{name: "printable text and apostrophes", input: "plain 'quote' é ☺", want: `"plain 'quote' é ☺"`},
		{name: "named escapes", input: "\"\\\t\n\r\x00", want: `"\"\\\t\n\r\0"`},
		{name: "other controls", input: "\x01\x1f\x7f\u0085", want: `"\u{1}\u{1f}\u{7f}\u{85}"`},
		{name: "separators and format characters", input: "\u00a0\u2028\u2029\u3000\u200b\ufeff\u00ad\u200d\U000e0020", want: `"\u{a0}\u{2028}\u{2029}\u{3000}\u{200b}\u{feff}\u{ad}\u{200d}\u{e0020}"`},
		{name: "space stays raw", input: "a b", want: `"a b"`},
		{name: "grapheme extend after a base", input: "e\u0301", want: `"e\u{301}"`},
		{name: "grapheme extend alone", input: "\u0301", want: `"\u{301}"`},
		{name: "variation selector", input: "😀\ufe0f", want: `"😀\u{fe0f}"`},
		{name: "private use and unassigned", input: "\ue000\u0378\U0010ffff", want: `"\u{e000}\u{378}\u{10ffff}"`},
		{name: "empty", input: "", want: `""`},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := quoted(tc.input); got != tc.want {
				t.Fatalf("%s: quoted = %s, want %s", tc.name, got, tc.want)
			}
		})
	}
}

func TestOutputText_DeliveryText_MessageSentTextIsTheConfirmationLineThenTheEnvelope(t *testing.T) {
	t.Parallel()
	message := messageRecordFixture(9, 2, 3, "hello")
	if got, want := formatMessageSent(message), "Message sent.\n"+formatMessage(message); got != want {
		t.Fatalf("formatMessageSent = %q, want %q", got, want)
	}
}

func TestOutputText_UnicodeTruncationChangesOnlyHumanMessageBodies(t *testing.T) {
	t.Parallel()
	prefix := strings.Repeat("界", textBodyLimit-1)
	message := messageRecord{
		messageID:       5,
		fromMemberID:    2,
		toMemberID:      3,
		createdAt:       "created",
		statusTimestamp: "acked",
		status:          messageStatusCompleted,
		text:            prefix + "🙂a\nnext",
	}
	if got, want := formatMessage(message), "[5 | from:2 | acked]\n"+prefix+"🙂…"; got != want {
		t.Fatalf("truncated body = %q, want %q", got, want)
	}
	encodedText := mustEncodeJSON(t, message.text)
	if got := mustEncodeJSON(t, outputJSONMessage(message)); !strings.HasSuffix(got, `,"text":`+encodedText+`}`) {
		t.Fatalf("the JSON message keeps the complete text: %s", got)
	}
	message.text = strings.Repeat("界", textBodyLimit)
	if got, want := formatMessage(message), "[5 | from:2 | acked]\n"+message.text; got != want {
		t.Fatalf("an exact-limit body = %q, want %q", got, want)
	}
	if got := formatMessages(nil); got != "No messages found." {
		t.Fatalf("formatMessages(nil) = %q", got)
	}
	if got, want := formatMessages([]messageRecord{message, message}), formatMessage(message)+"\n\n"+formatMessage(message); got != want {
		t.Fatalf("formatMessages = %q, want %q", got, want)
	}
}

func textPlacementFixture(window opt[[2]string], pane opt[string]) placement {
	recorded := placement{
		backend:       "tmux",
		muxSession:    "main",
		muxWindowID:   none[string](),
		muxWindowName: none[string](),
		windowOrdinal: 0,
		columnSide:    columnSideRight,
		muxPaneID:     pane,
		codingAgent:   "claude",
		createdAt:     "created",
		muxServer:     "/tmp/tmux-1000/default",
	}
	if ids, ok := window.get(); ok {
		recorded.muxWindowID = some(ids[0])
		recorded.muxWindowName = some(ids[1])
	}
	return recorded
}

func textActivityRow(memberID int64, name string, kind memberKind, recorded placement, idle int64) memberActivity {
	return memberActivity{
		member: memberRecord{
			memberID:     memberID,
			fleetID:      3,
			name:         name,
			description:  "assignment",
			status:       memberStatusActive,
			kind:         kind,
			registeredAt: "registered",
			placement:    recorded,
		},
		lastSent:     none[string](),
		lastRecv:     none[string](),
		lastConsumed: none[string](),
		idle:         some(idle),
	}
}

func TestOutputText_RosterDistinguishesAbsentPendingAndConcretePlacement(t *testing.T) {
	t.Parallel()
	pending := textPlacementFixture(none[[2]string](), none[string]())
	pending.muxSession = "s"
	pending.codingAgent = "codex"
	row := textActivityRow(7, "worker", memberKindMember, pending, 60)
	row.member.fleetID = 1
	if got := formatMemberList(nil); got != "0 members." {
		t.Fatalf("formatMemberList(nil) = %q", got)
	}
	if got := formatMemberList([]memberActivity{row}); !strings.Contains(got, "codex     (pending)      (pending)  1m") {
		t.Fatalf("pending roster = %q", got)
	}
	if got := formatMemberDetail(row.member); got != "7 worker active window=(pending)" {
		t.Fatalf("pending detail = %q", got)
	}
	row.member.placement.muxPaneID = some("%9")
	row.member.placement.muxWindowID = some("@1")
	row.member.placement.muxWindowName = some("main")
	if got := formatMemberList([]memberActivity{row}); !strings.Contains(got, "codex     main           %9") {
		t.Fatalf("concrete roster = %q", got)
	}
	if got := formatMemberDetail(row.member); got != "7 worker active window=main" {
		t.Fatalf("concrete detail = %q", got)
	}
	idles := []struct {
		idle opt[int64]
		want string
	}{
		{none[int64](), "-"},
		{some(int64(0)), "0s"},
		{some(int64(59)), "59s"},
		{some(int64(60)), "1m"},
		{some(int64(3599)), "59m"},
		{some(int64(3600)), "1h"},
	}
	for _, tc := range idles {
		if got := formatIdle(tc.idle); got != tc.want {
			t.Fatalf("formatIdle(%v) = %q, want %q", tc.idle, got, tc.want)
		}
	}
}

func TestOutputText_WindowText_MemberTextShowsEachWindowOrPendingBesideThePaneAndStatus(t *testing.T) {
	t.Parallel()
	concrete := textPlacementFixture(some([2]string{"@6", "cafleet-3-1"}), some("%12"))
	pending := textPlacementFixture(none[[2]string](), none[string]())
	registered := registeredMember{memberID: 9, name: "drafter", registeredAt: "registered"}
	list := formatMemberList([]memberActivity{
		textActivityRow(8, "director", memberKindDirector, textPlacementFixture(some([2]string{"@1", "main"}), some("%3")), 12),
		textActivityRow(9, "drafter", memberKindMember, concrete, 5),
	})
	pendingList := lines(formatMemberList([]memberActivity{textActivityRow(10, "tester", memberKindMember, pending, 0)}))
	cases := []struct {
		name     string
		rendered string
		want     string
	}{
		{
			name:     "member list with concrete windows",
			rendered: list,
			want: strings.Join([]string{
				"2 members:",
				"  member_id  name           kind      backend   window         pane_id  idle",
				"  8          director       director  claude    main           %3       12s",
				"  9          drafter        member    claude    cafleet-3-1    %12      5s",
			}, "\n"),
		},
		{
			name:     "member list row with a pending window and pane",
			rendered: pendingList[2],
			want:     "  10         tester         member    claude    (pending)      (pending)  0s",
		},
		{name: "member create with a concrete placement", rendered: formatMember(registered, concrete), want: "9 drafter backend=claude pane=%12 window=cafleet-3-1"},
		{name: "member create with a pending placement", rendered: formatMember(registered, pending), want: "9 drafter backend=claude pane=(pending) window=(pending)"},
		{name: "member show with a concrete window", rendered: formatMemberDetail(textActivityRow(9, "drafter", memberKindMember, concrete, 5).member), want: "9 drafter active window=cafleet-3-1"},
		{name: "member show with a pending window", rendered: formatMemberDetail(textActivityRow(10, "tester", memberKindMember, pending, 0).member), want: "10 tester active window=(pending)"},
	}
	for _, tc := range cases {
		if tc.rendered != tc.want {
			t.Fatalf("%s:\n%q\nwant\n%q", tc.name, tc.rendered, tc.want)
		}
	}
}

func TestOutputText_FleetDeletionReportsEveryOrdinaryMemberOutcomeInTextAndJson(t *testing.T) {
	t.Parallel()
	deleted := deletedFleet{
		fleetID:           4,
		deregisteredCount: 3,
		members: []deletedMember{
			{memberID: 6, paneStatus: "%7 (killed)"},
			{memberID: 8, paneStatus: "(pending — no pane)"},
		},
	}
	if got, want := formatFleetDelete(deleted), "Deleted fleet 4. Deregistered 3 members.\n  member 6: %7 (killed)\n  member 8: (pending — no pane)"; got != want {
		t.Fatalf("formatFleetDelete = %q, want %q", got, want)
	}
	want := `{"fleet_id":4,"deregistered_count":3,"members":[{"member_id":6,"pane_status":"%7 (killed)"},{"member_id":8,"pane_status":"(pending — no pane)"}]}`
	if got := mustEncodeJSON(t, outputJSONDeletedFleet(deleted)); got != want {
		t.Fatalf("deleted fleet JSON\n%s\nwant\n%s", got, want)
	}
	deleted.members = nil
	if got := formatFleetDelete(deleted); got != "Deleted fleet 4. Deregistered 3 members." {
		t.Fatalf("formatFleetDelete without members = %q", got)
	}
}

func TestOutputText_DeliveryText_DeliveryTextSubmittedPromptAndExecKeepTheirSuccessLinesThenDeliveryAndInput(t *testing.T) {
	t.Parallel()
	profile := deliveryFixture(deliveryStatusSubmitted, some(deliveryMethodProfile), none[string]())
	if got, want := formatMemberPrompt(7, "do it", "worker", "%2", some(int64(5)), profile),
		"Sent prompt \"do it\" to member worker (%2).\nDelivery: status=submitted method=profile reason=-\nInput: 5"; got != want {
		t.Fatalf("formatMemberPrompt = %q, want %q", got, want)
	}
	fallback := deliveryFixture(deliveryStatusSubmitted, some(deliveryMethodFallback), some("submission_plan_unavailable"))
	if got, want := formatMemberExec(7, "%2", some(int64(6)), fallback),
		"Dispatched native command to member 7 (%2). Inspect the pane for its result.\nDelivery: status=submitted method=fallback reason=submission_plan_unavailable\nInput: 6"; got != want {
		t.Fatalf("formatMemberExec = %q, want %q", got, want)
	}
}

func TestOutputText_DeliveryText_DeliveryTextQueuedPromptAndExecPrintQueueLinesRetryAndTheReceiptPointer(t *testing.T) {
	t.Parallel()
	queued := deliveryFixture(deliveryStatusQueued, none[deliveryMethod](), some("interaction_open"))
	cases := []struct {
		text, first string
	}{
		{formatMemberPrompt(7, "do it", "worker", "%2", some(int64(8)), queued), "Queued prompt for member 7 (%2)."},
		{formatMemberExec(7, "%2", some(int64(8)), queued), "Queued native command for member 7 (%2)."},
	}
	for _, tc := range cases {
		if got := lines(tc.text); len(got) == 0 || got[0] != tc.first {
			t.Fatalf("first line of %q, want %q", tc.text, tc.first)
		}
		if strings.Contains(tc.text, "Sent prompt") || strings.Contains(tc.text, "Dispatched") {
			t.Fatalf("a queued result keeps no success line: %q", tc.text)
		}
		for _, part := range []string{"\nDelivery: status=queued method=- reason=interaction_open", "\nInput: 8", "retry", "cafleet input show 8 --json"} {
			if !strings.Contains(tc.text, part) {
				t.Fatalf("%q lacks %q", tc.text, part)
			}
		}
	}
}

func TestOutputText_QuotedText_MemberPromptTextQuotesEachScalarClassPerTheQuotedTextDefinition(t *testing.T) {
	t.Parallel()
	submitted := deliveryFixture(deliveryStatusSubmitted, some(deliveryMethodProfile), none[string]())
	cases := []struct {
		name, scalar, quoted string
	}{
		{"double quote", `"`, `"\""`},
		{"backslash", `\`, `"\\"`},
		{"tab", "\t", `"\t"`},
		{"line feed", "\n", `"\n"`},
		{"carriage return", "\r", `"\r"`},
		{"nul", "\x00", `"\0"`},
		{"Cc bell", "\x07", `"\u{7}"`},
		{"Cc escape", "\x1b", `"\u{1b}"`},
		{"Cc delete", "\x7f", `"\u{7f}"`},
		{"Cc next line", "\xc2\x85", `"\u{85}"`},
		{"Cf soft hyphen", "\xc2\xad", `"\u{ad}"`},
		{"Cf zero width space", "\xe2\x80\x8b", `"\u{200b}"`},
		{"Cf byte order mark", "\xef\xbb\xbf", `"\u{feff}"`},
		{"Co private use", "\xee\x80\x80", `"\u{e000}"`},
		{"Cn noncharacter", "\xef\xbf\xbf", `"\u{ffff}"`},
		{"Cn plane 16 noncharacter", "\xf4\x8f\xbf\xbf", `"\u{10ffff}"`},
		{"Zl line separator", "\xe2\x80\xa8", `"\u{2028}"`},
		{"Zp paragraph separator", "\xe2\x80\xa9", `"\u{2029}"`},
		{"Zs no-break space", "\xc2\xa0", `"\u{a0}"`},
		{"Zs ideographic space", "\xe3\x80\x80", `"\u{3000}"`},
		{"Grapheme_Extend alone", "\xcc\x81", `"\u{301}"`},
		{"Grapheme_Extend after a base", "e\xcc\x81", `"e\u{301}"`},
		{"Grapheme_Extend enclosing mark", "\xe2\x83\x9d", `"\u{20dd}"`},
		{"Grapheme_Extend zero width non-joiner", "\xe2\x80\x8c", `"\u{200c}"`},
		{"space", " ", `" "`},
		{"single quote", "'", `"'"`},
		{"solidus", "/", `"/"`},
		{"latin small e with acute", "\xc3\xa9", "\"\xc3\xa9\""},
		{"cjk ideographs", "\xe6\x97\xa5\xe6\x9c\xac", "\"\xe6\x97\xa5\xe6\x9c\xac\""},
		{"euro sign", "\xe2\x82\xac", "\"\xe2\x82\xac\""},
		{"emoji", "\xf0\x9f\x92\xa9", "\"\xf0\x9f\x92\xa9\""},
	}
	for _, tc := range cases {
		text := formatMemberPrompt(7, tc.scalar, "worker", "%2", none[int64](), submitted)
		first, _, _ := strings.Cut(text, "\n")
		if want := "Sent prompt " + tc.quoted + " to member worker (%2)."; first != want {
			t.Fatalf("%s: first line %q, want %q", tc.name, first, want)
		}
	}
}
