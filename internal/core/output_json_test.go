package core

import (
	"context"
	"encoding/json/jsontext"
	"fmt"
	"slices"
	"strings"
	"testing"
)

func mustEncodeJSON(t *testing.T, value any) string {
	t.Helper()
	encoded, err := encodeJSON(value)
	if err != nil {
		t.Fatal(err)
	}
	return string(encoded)
}

func jsonPlacementFixture(concrete bool) placement {
	recorded := placement{
		backend:       "tmux",
		muxSession:    "main",
		muxWindowID:   none[string](),
		muxWindowName: none[string](),
		windowOrdinal: 1,
		columnSide:    columnSideLeft,
		muxPaneID:     none[string](),
		codingAgent:   "claude",
		createdAt:     "2026-11-13T00:00:00+00:00",
		muxServer:     "/tmp/tmux-1000/default",
	}
	if concrete {
		recorded.muxWindowID = some("@6")
		recorded.muxWindowName = some("cafleet-3-1")
		recorded.muxPaneID = some("%12")
	}
	return recorded
}

func TestOutputJson_PlacementJson_APlacementOrdersItsWindowAndSlotKeysBeforeThePaneAndNullsPendingIds(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name     string
		concrete bool
		want     string
	}{
		{
			name:     "concrete placement",
			concrete: true,
			want:     `{"backend":"tmux","mux_session":"main","mux_window_id":"@6","mux_window_name":"cafleet-3-1","window_ordinal":1,"column_side":"left","mux_pane_id":"%12","coding_agent":"claude","created_at":"2026-11-13T00:00:00+00:00"}`,
		},
		{
			name: "pending placement",
			want: `{"backend":"tmux","mux_session":"main","mux_window_id":null,"mux_window_name":null,"window_ordinal":1,"column_side":"left","mux_pane_id":null,"coding_agent":"claude","created_at":"2026-11-13T00:00:00+00:00"}`,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := mustEncodeJSON(t, outputJSONPlacement(jsonPlacementFixture(tc.concrete))); got != tc.want {
				t.Fatalf("%s: placement JSON\n%s\nwant\n%s", tc.name, got, tc.want)
			}
		})
	}
}

func TestOutputJson_PlacementJson_MemberActivityNamesTheLatestConsumptionLastConsumedBeforeIdle(t *testing.T) {
	t.Parallel()
	row := memberActivity{
		member: memberRecord{
			memberID:  4,
			fleetID:   1,
			name:      "worker",
			status:    memberStatusActive,
			kind:      memberKindMember,
			placement: jsonPlacementFixture(true),
		},
		lastSent:     none[string](),
		lastRecv:     some("2026-11-13T00:00:01+00:00"),
		lastConsumed: some("2026-11-13T00:00:02+00:00"),
		idle:         some(int64(3)),
	}
	placementJSON := mustEncodeJSON(t, outputJSONPlacement(jsonPlacementFixture(true)))
	want := `{"member_id":4,"name":"worker","kind":"member","placement":` + placementJSON +
		`,"last_sent":null,"last_recv":"2026-11-13T00:00:01+00:00","last_consumed":"2026-11-13T00:00:02+00:00","idle":3}`
	if got := mustEncodeJSON(t, outputJSONMemberActivity(row)); got != want {
		t.Fatalf("member activity JSON\n%s\nwant\n%s", got, want)
	}
}

func TestOutputJson_PlacementJson_ColumnSidesRenderAsTheirStoredValues(t *testing.T) {
	t.Parallel()
	cases := []struct {
		side columnSide
		wire string
	}{
		{columnSideLeft, `"column_side":"left"`},
		{columnSideRight, `"column_side":"right"`},
	}
	for _, tc := range cases {
		recorded := jsonPlacementFixture(true)
		recorded.columnSide = tc.side
		if got := mustEncodeJSON(t, outputJSONPlacement(recorded)); !strings.Contains(got, tc.wire) {
			t.Fatalf("%s: placement JSON %s", tc.wire, got)
		}
	}
}

func deliveryFixture(status deliveryStatus, method opt[deliveryMethod], reason opt[string]) deliveryProjection {
	return deliveryProjection{Status: status, Method: method, Reason: reason}
}

func assertJSONKeyOrder(t *testing.T, document jsontext.Value, want []string, context string) {
	t.Helper()
	if got := testJSONKeys(t, document); !slices.Equal(got, want) {
		t.Fatalf("%s: keys %q, want %q", context, got, want)
	}
}

func TestOutputJson_DeliveryOutcomes_PromptAndExecOutcomesAppendDeliveryAndDispatchOnlyOnSubmission(t *testing.T) {
	t.Parallel()
	cases := []struct {
		inputID  opt[int64]
		delivery deliveryProjection
		want     string
	}{
		{some(int64(5)), deliveryFixture(deliveryStatusSubmitted, some(deliveryMethodProfile), none[string]()), `{"status":"submitted","method":"profile","reason":null}`},
		{some(int64(6)), deliveryFixture(deliveryStatusQueued, none[deliveryMethod](), some("interaction_open")), `{"status":"queued","method":null,"reason":"interaction_open"}`},
		{none[int64](), deliveryFixture(deliveryStatusHeld, none[deliveryMethod](), some("observer_unavailable")), `{"status":"held","method":null,"reason":"observer_unavailable"}`},
		{some(int64(7)), deliveryFixture(deliveryStatusUncertain, some(deliveryMethodFallback), some("submission_plan_unavailable")), `{"status":"uncertain","method":"fallback","reason":"submission_plan_unavailable"}`},
	}
	for _, tc := range cases {
		inputID := jsontext.Value("null")
		if id, ok := tc.inputID.get(); ok {
			inputID = mustJSON(t, id)
		}
		prompt := mustJSON(t, outputJSONMemberPrompt(7, "%2", "do it", tc.inputID, tc.delivery))
		assertJSONKeyOrder(t, prompt, []string{"member_id", "pane_id", "text", "input_id", "delivery"}, tc.want)
		assertJSONEqual(t, prompt, jsontext.Value(`{"member_id":7,"pane_id":"%2","text":"do it","input_id":`+string(inputID)+`,"delivery":`+tc.want+`}`), tc.want)
		exec := mustJSON(t, outputJSONMemberExec(7, "%2", "codex", "ls -l", tc.inputID, tc.delivery))
		assertJSONKeyOrder(t, exec, []string{"member_id", "pane_id", "coding_agent", "command", "dispatched", "input_id", "delivery"}, tc.want)
		assertJSONEqual(t, jsonAt(t, exec, "dispatched"), mustJSON(t, tc.delivery.Status == deliveryStatusSubmitted), tc.want)
		assertJSONEqual(t, jsonAt(t, exec, "delivery"), jsontext.Value(tc.want), tc.want)
	}
}

var runtimeJSONKeys = []string{
	"run_id", "resolved_mode", "status", "owner_epoch", "last_health_at", "coverage",
	"current_interaction_id", "unresolved_input_ids", "owner_kind", "owner_pid",
	"owner_start_identity", "runner_pid", "runner_start_identity", "child_pid", "child_start_identity",
}

var inputJSONKeys = []string{
	"input_id", "run_id", "operation_key", "kind", "payload_digest", "state", "created_at", "updated_at",
	"disposition", "evidence_ref", "delivery", "deferred_at", "defer_reason", "stop_cause",
}

func TestOutputJson_Runtime_RuntimePresentationExposesRecordedOwnerSnapshotAndUnresolvedInputs(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	event := fixture.open(t, owner)
	input := fixture.input(t, owner, "exec:uncertain", inputKindExec)
	queued := fixture.input(t, owner, "prompt:queued", inputKindPrompt)
	if _, err := fixture.conn.execute("UPDATE member_input_operations SET state='ready',deferred_at='2026-10-30T10:00:00Z',defer_reason='interaction_open' WHERE input_id=?1", queued.inputID); err != nil {
		t.Fatal(err)
	}
	zeroBytes := fixture.input(t, owner, "prompt:zero-bytes", inputKindPrompt)
	if _, err := fixture.conn.execute("UPDATE member_input_operations SET state='ready' WHERE input_id=?1", zeroBytes.inputID); err != nil {
		t.Fatal(err)
	}
	member := requireMember(t, fixture.conn, fixture.member, fixture.fleet)
	output := mustJSON(t, outputJSONMember(member))
	assertJSONKeyOrder(t, output, []string{"member_id", "name", "description", "status", "registered_at", "kind", "placement", "runtime"}, "member")
	runtime := jsonAt(t, output, "runtime")
	assertJSONKeyOrder(t, runtime, runtimeJSONKeys, "runtime")
	assertJSONEqual(t, jsonAt(t, runtime, "run_id"), mustJSON(t, owner.runID), "run_id")
	assertJSONEqual(t, jsonAt(t, runtime, "current_interaction_id"), mustJSON(t, eventInteractionID(t, event)), "current_interaction_id")
	assertJSONEqual(t, jsonAt(t, runtime, "unresolved_input_ids"), mustJSON(t, []int64{input.inputID, queued.inputID}),
		"a queued ready input is unresolved; a plain zero-byte ready input is not")
	assertJSONEqual(t, jsonAt(t, runtime, "owner_start_identity"), jsontext.Value(`"fixture-start-identity"`), "owner_start_identity")
	assertJSONEqual(t, jsonAt(t, runtime, "child_pid"), jsontext.Value("null"), "child_pid")
	assertJSONEqual(t, jsonAt(t, runtime, "child_start_identity"), jsontext.Value("null"), "child_start_identity")
	director := requireMember(t, fixture.conn, fixture.director, fixture.fleet)
	assertJSONEqual(t, jsonAt(t, mustJSON(t, outputJSONMember(director)), "runtime"), jsontext.Value("null"), "the Director has no runtime")
	want := fmt.Sprintf("%d worker active window=main\nRuntime: run=%d mode=never status=running coverage=unknown", fixture.member, owner.runID)
	if got := formatMemberDetail(member); got != want {
		t.Fatalf("formatMemberDetail = %q, want %q", got, want)
	}
	if _, err := fixture.conn.execute("UPDATE member_runs SET native_version='0.164.0',coverage='tested' WHERE run_id=?1", owner.runID); err != nil {
		t.Fatal(err)
	}
	member = requireMember(t, fixture.conn, fixture.member, fixture.fleet)
	runtime = jsonAt(t, mustJSON(t, outputJSONMember(member)), "runtime")
	assertJSONEqual(t, jsonAt(t, runtime, "coverage"), jsontext.Value(`"tested"`), "coverage")
	assertJSONKeyOrder(t, runtime, runtimeJSONKeys, "tested runtime")
	line := fmt.Sprintf("Runtime: run=%d mode=never status=running coverage=tested", owner.runID)
	if got, want := formatMemberDetail(member), fmt.Sprintf("%d worker active window=main\n%s", fixture.member, line); got != want {
		t.Fatalf("formatMemberDetail = %q, want %q", got, want)
	}
	view, err := getEvent(ctx, fixture.conn, event.eventID)
	if err != nil {
		t.Fatal(err)
	}
	assertJSONKeyOrder(t, jsonAt(t, mustJSON(t, outputJSONRuntimeEvent(view)), "run"), runtimeJSONKeys, "event run")
	if got := lines(formatRuntimeEvent(view)); len(got) < 2 || got[1] != line {
		t.Fatalf("formatRuntimeEvent lines = %q, want the runtime line %q", got, line)
	}
}

func TestOutputJson_Runtime_EventAndInputReadWrappersFollowTheApprovedOrderAndCompactText(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	event := fixture.open(t, owner)
	view, err := getEvent(ctx, fixture.conn, event.eventID)
	if err != nil {
		t.Fatal(err)
	}
	output := mustJSON(t, outputJSONRuntimeEvent(view))
	assertJSONKeyOrder(t, output, []string{"event", "notification_state", "message_id", "archive_reason", "run", "interaction"}, "event")
	assertJSONKeyOrder(t, jsonAt(t, output, "interaction"), []string{"interaction_id", "run_id", "episode_seq", "classification", "evidence_digest", "opened_at", "evidence_json", "closed_at", "close_reason"}, "interaction")
	want := fmt.Sprintf("Event: %d run=%d seq=1 kind=interaction_opened notification=inbox message=%d\nRuntime: run=%d mode=never status=running coverage=unknown\nInteraction: %d classification=permission_request closed_at=- close_reason=-",
		event.eventID, owner.runID, eventMessageID(t, event), owner.runID, eventInteractionID(t, event))
	if got := formatRuntimeEvent(view); got != want {
		t.Fatalf("formatRuntimeEvent = %q, want %q", got, want)
	}
	input := fixture.input(t, owner, "exec:lost-output", inputKindExec)
	assertJSONKeyOrder(t, jsonAt(t, mustJSON(t, outputJSONInputOperation(input)), "input"), inputJSONKeys, "input")
	if got, want := formatInputOperation(input), fmt.Sprintf("Input: %d run=%d kind=exec state=in_progress disposition=-", input.inputID, owner.runID); got != want {
		t.Fatalf("formatInputOperation = %q, want %q", got, want)
	}
}

func TestOutputJson_Runtime_InputPresentationReportsTheRecordedStopCauseAndKeepsTheTextLine(t *testing.T) {
	t.Parallel()
	ctx := context.Background()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	caused := fixture.input(t, owner, "prompt:caused", inputKindPrompt)
	caused = mustInput(t)(recordInputUncertain(ctx, fixture.conn, owner, caused.inputID, stopCauseShortWrite))
	stopCauseOf := func(input inputOperation) jsontext.Value {
		return jsonAt(t, mustJSON(t, outputJSONInputOperation(input)), "input", "stop_cause")
	}
	assertJSONEqual(t, stopCauseOf(caused), jsontext.Value(`"short_write"`), "the recorded cause")
	if got, want := formatInputOperation(caused), fmt.Sprintf("Input: %d run=%d kind=prompt state=uncertain disposition=-", caused.inputID, owner.runID); got != want {
		t.Fatalf("formatInputOperation = %q, want %q", got, want)
	}
	settled := mustInput(t)(settleInput(ctx, fixture.conn, owner, caused.inputID, inputStateUncertain, inputDispositionAbandonedUncertain, "audit:inspected"))
	assertJSONEqual(t, stopCauseOf(settled), jsontext.Value(`"short_write"`), "reconciliation keeps the recorded cause")
	plain := fixture.input(t, owner, "prompt:plain", inputKindPrompt)
	assertJSONEqual(t, stopCauseOf(plain), jsontext.Value("null"), "a plain receipt")
}
