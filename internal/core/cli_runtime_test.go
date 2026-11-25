package core

import (
	"context"
	"slices"
	"strconv"
	"strings"
	"testing"
	"time"
)

func permissionInspectArgs() []string {
	return []string{"permission", "inspect", "17", "--event-id", "19", "--from-member-id", "23"}
}

func permissionRespondArgs() []string {
	return []string{
		"permission", "respond", "17", "--event-id", "19", "--from-member-id", "23",
		"--expected-run-id", "29", "--expected-owner-epoch", "3",
		"--request-token", strings.Repeat("a", 64), "--decision", "allow",
	}
}

// withArgument is args with the value of field replaced; field "interaction" names the
// positional interaction ID.
func withArgument(args []string, field, value string) []string {
	changed := slices.Clone(args)
	index := 2
	if field != "interaction" {
		index = slices.Index(changed, field) + 1
		if index == 0 {
			panic("the fixture contains " + field)
		}
	}
	changed[index] = value
	return changed
}

func assertAccepted(t *testing.T, args []string) cliCommand {
	t.Helper()
	return mustParse(t, strings.Join(args, " "), args...)
}

func assertRejected(t *testing.T, args []string) {
	t.Helper()
	if outcome := parseOutcome(t, strings.Join(args, " "), args...); outcome.exitCode() != 2 {
		t.Fatalf("%q exits %d, want a usage error: %s", args, outcome.exitCode(), outcome.text)
	}
}

func TestCliRuntime_PermissionEntry_PermissionCommandsAcceptPositiveIdsWithJsonAndRejectEveryOtherId(t *testing.T) {
	t.Parallel()
	fixtures := []struct {
		args   []string
		fields []string
	}{
		{permissionInspectArgs(), []string{"interaction", "--event-id", "--from-member-id"}},
		{permissionRespondArgs(), []string{"interaction", "--event-id", "--from-member-id", "--expected-run-id", "--expected-owner-epoch"}},
	}
	for _, fixture := range fixtures {
		assertAccepted(t, append(slices.Clone(fixture.args), "--json"))
		for _, field := range fixture.fields {
			assertRejected(t, withArgument(fixture.args, field, "0"))
		}
	}
	for _, command := range []string{"event", "input"} {
		assertAccepted(t, []string{command, "show", "481", "--json"})
		assertRejected(t, []string{command, "show", "0"})
	}
}

func permissionRespondChoice(choice string) []string {
	args := permissionRespondArgs()
	index := slices.Index(args, "--decision")
	args[index] = "--choice"
	args[index+1] = choice
	return args
}

func TestCliRuntime_PermissionEntry_PermissionRespondParsesExactlyOneValidDecisionOrChoice(t *testing.T) {
	t.Parallel()
	type answer struct {
		decision opt[string]
		choice   opt[string]
	}
	neither := permissionRespondArgs()
	index := slices.Index(neither, "--decision")
	neither = slices.Delete(neither, index, index+2)
	cases := []struct {
		name   string
		args   []string
		answer opt[answer]
	}{
		{"decision allow", withArgument(permissionRespondArgs(), "--decision", "allow"), some(answer{some("allow"), none[string]()})},
		{"decision deny", withArgument(permissionRespondArgs(), "--decision", "deny"), some(answer{some("deny"), none[string]()})},
	}
	for _, choice := range []string{"decline", "not-now", "dont-show-again", "a", strings.Repeat("a", 64)} {
		cases = append(cases, struct {
			name   string
			args   []string
			answer opt[answer]
		}{"choice " + choice, permissionRespondChoice(choice), some(answer{none[string](), some(choice)})})
	}
	for _, refused := range []struct {
		name string
		args []string
	}{
		{"decision then choice", append(permissionRespondArgs(), "--choice", "decline")},
		{"choice then decision", append(permissionRespondChoice("decline"), "--decision", "deny")},
		{"neither", neither},
		{"repeated choice", append(permissionRespondChoice("decline"), "--choice", "not-now")},
		{"choice outside the id shape", permissionRespondChoice("Decline")},
		{"uppercase request token", withArgument(permissionRespondArgs(), "--request-token", strings.Repeat("A", 64))},
		{"short request token", withArgument(permissionRespondArgs(), "--request-token", strings.Repeat("a", 63))},
		{"unknown decision", withArgument(permissionRespondArgs(), "--decision", "approve")},
	} {
		cases = append(cases, struct {
			name   string
			args   []string
			answer opt[answer]
		}{refused.name, refused.args, none[answer]()})
	}
	for _, tc := range cases {
		want, accepted := tc.answer.get()
		if !accepted {
			assertRejected(t, tc.args)
			continue
		}
		respond, ok := assertAccepted(t, tc.args).(permissionCommandRespond)
		if !ok {
			t.Fatalf("%s: a permission respond command", tc.name)
		}
		if got := (answer{respond.decision, respond.choice}); got != want {
			t.Fatalf("%s: answer = %+v, want %+v", tc.name, got, want)
		}
	}
}

func assertRequiredArguments(t *testing.T, args []string, flags []string) {
	t.Helper()
	assertRejected(t, slices.Delete(slices.Clone(args), 2, 3))
	for _, flag := range flags {
		index := slices.Index(args, flag)
		if index < 0 {
			t.Fatalf("the fixture contains %s", flag)
		}
		assertRejected(t, slices.Delete(slices.Clone(args), index, index+2))
	}
	assertAccepted(t, append(slices.Clone(args), "--json"))
}

func TestCliRuntime_PermissionEntry_PermissionInspectRequiresItsInteractionEventAndDirector(t *testing.T) {
	t.Parallel()
	assertRequiredArguments(t, permissionInspectArgs(), []string{"--event-id", "--from-member-id"})
	assertRequiredArguments(t, permissionRespondArgs(), []string{"--event-id", "--from-member-id", "--expected-run-id", "--expected-owner-epoch", "--request-token"})
}

func TestCliRuntime_PositiveIdAcceptsOnlyPositiveI64Values(t *testing.T) {
	t.Parallel()
	for _, value := range []string{"1", "9223372036854775807"} {
		if err := checkPositiveID(value); err != nil {
			t.Fatalf("%s: %v", value, err)
		}
	}
	for _, value := range []string{"0", "-1", "unknown", "1.0", "9223372036854775808"} {
		if err := checkPositiveID(value); err == nil || err.Error() != "a positive assigned ID is required" {
			t.Fatalf("%s: checkPositiveID = %v", value, err)
		}
	}
}

func TestCliRuntime_InputReconcileRequiresAnExactDispositionNonemptyEvidenceAndAPositiveId(t *testing.T) {
	t.Parallel()
	reconcile := func(id string, disposition, evidence opt[string]) []string {
		args := []string{"input", "reconcile", id, "--from-member-id", "1"}
		if value, ok := disposition.get(); ok {
			args = append(args, "--disposition", value)
		}
		if value, ok := evidence.get(); ok {
			args = append(args, "--evidence-ref", value)
		}
		return args
	}
	evidence := some("audit:inspected-receipt")
	cases := []struct {
		name     string
		args     []string
		accepted bool
		want     disposition
	}{
		{"submitted", reconcile("17", some("submitted"), evidence), true, dispositionSubmitted},
		{"not submitted", reconcile("17", some("not-submitted"), evidence), true, dispositionNotSubmitted},
		{"abandoned uncertain", reconcile("17", some("abandoned-uncertain"), evidence), true, dispositionAbandonedUncertain},
		{"missing evidence", reconcile("17", some("submitted"), none[string]()), false, 0},
		{"missing disposition", reconcile("17", none[string](), some("audit:missing-choice")), false, 0},
		{"unknown disposition", reconcile("17", some("approved"), some("audit:x")), false, 0},
		{"blank evidence", reconcile("17", some("submitted"), some("   ")), false, 0},
		{"zero input id", reconcile("0", some("submitted"), some("audit:x")), false, 0},
	}
	for _, tc := range cases {
		if !tc.accepted {
			assertRejected(t, tc.args)
			continue
		}
		parsed, ok := assertAccepted(t, append(slices.Clone(tc.args), "--json")).(inputCommandReconcile)
		if !ok || parsed.inputID != 17 || parsed.disposition != tc.want || parsed.evidenceRef != "audit:inspected-receipt" || !parsed.json {
			t.Fatalf("%s: parsed %+v", tc.name, parsed)
		}
	}
}

func TestCliRuntime_EventAndInputShowPrintTheStoredRecordAsJsonOrText(t *testing.T) {
	t.Parallel()
	fixture := newBrokerRuntimeFixture(t)
	owner := fixture.activate(t)
	event := fixture.open(t, owner)
	input := fixture.input(t, owner, "exec:shown", inputKindExec)
	view := readEvent(t, fixture.conn, event.eventID)
	eventID := strconv.FormatInt(event.eventID, 10)
	inputID := strconv.FormatInt(input.inputID, 10)
	cases := []struct {
		name string
		args []string
		want string
	}{
		{"event show --json", []string{"event", "show", eventID, "--json"}, mustEncodeJSON(t, outputJSONRuntimeEvent(view)) + "\n"},
		{"event show", []string{"event", "show", eventID}, formatRuntimeEvent(view) + "\n"},
		{"input show --json", []string{"input", "show", inputID, "--json"}, mustEncodeJSON(t, outputJSONInputOperation(input)) + "\n"},
		{"input show", []string{"input", "show", inputID}, formatInputOperation(input) + "\n"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			outcome := runCliChild(t, append([]string{"--database-url", fixture.url}, tc.args...)...)
			if outcome.code != 0 || outcome.stdout != tc.want || outcome.stderr != "" {
				t.Fatalf("%s: exit %d stdout %q stderr %q, want %q", tc.name, outcome.code, outcome.stdout, outcome.stderr, tc.want)
			}
		})
	}
}

func TestCliRuntime_EventAndInputShowReportAnUnknownRecord(t *testing.T) {
	t.Parallel()
	fixture := newBrokerRuntimeFixture(t)
	cases := []struct {
		name string
		args []string
		want string
	}{
		{"event show", []string{"event", "show", "999"}, "Error: Event 999 not found\n"},
		{"input show", []string{"input", "show", "999", "--json"}, "Error: Input 999 not found\n"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			outcome := runCliChild(t, append([]string{"--database-url", fixture.url}, tc.args...)...)
			if outcome.code != 1 || outcome.stdout != "" || outcome.stderr != tc.want {
				t.Fatalf("%s: exit %d stdout %q stderr %q, want exit 1 and %q", tc.name, outcome.code, outcome.stdout, outcome.stderr, tc.want)
			}
		})
	}
}

func TestCliRuntime_LiveReceiptCliReconcilesRealExecHistoryAndPreservesRepeatEvidence(t *testing.T) {
	t.Parallel()
	withInputService(t, func(f *serviceFixture, outer time.Time) {
		ctx := context.Background()
		s, run, p := serviceTarget(t, f)
		original, err := reserveInput(ctx, f.fixture.conn, run.owner(), newInput{
			operationKey: "exec:cli-receipt-history", kind: inputKindExec, payloadDigest: strings.Repeat("d", 64),
		})
		if err != nil {
			t.Fatal(err)
		}
		if _, err := recordInputUncertain(ctx, f.fixture.conn, run.owner(), original.inputID, stopCauseWriteFailed); err != nil {
			t.Fatal(err)
		}
		until := earliestTime(outer, time.Now().Add(2*time.Second))
		reconcile := func(disposition disposition, evidence string) error {
			return inputRun(ctx, f.fixture.conn, s, inputCommandReconcile{
				inputID: original.inputID, directorMemberID: run.directorMemberID,
				disposition: disposition, evidenceRef: evidence, json: true,
			}, deadlineUntil(until))
		}
		if err := reconcile(dispositionAbandonedUncertain, "fixture:cli-owned-resolution"); err != nil {
			t.Fatal(err)
		}
		settled := storedInput(t, f.fixture.conn, original.inputID)
		if settled.state != inputStateSettled || settled.disposition != some(inputDispositionAbandonedUncertain) ||
			settled.evidenceRef != some("fixture:cli-owned-resolution") {
			t.Fatalf("settled receipt %+v", settled)
		}
		held, acquired, err := paneLockTryAcquire(s.databaseURL, p)
		if err != nil || !acquired {
			t.Fatalf("the pane lock is free after reconciliation: %v %v", acquired, err)
		}
		changes := queryInt64(t, f.fixture.conn, "SELECT total_changes()")
		if err := reconcile(dispositionAbandonedUncertain, "fixture:replacement-evidence"); err != nil {
			t.Fatalf("a repeated disposition is accepted: %v", err)
		}
		if err := reconcile(dispositionSubmitted, "fixture:conflicting-cli-disposition"); err == nil {
			t.Fatal("a conflicting disposition is refused")
		}
		if after := queryInt64(t, f.fixture.conn, "SELECT total_changes()"); after != changes {
			t.Fatalf("receipt-only repeats changed %d rows", after-changes)
		}
		if got := storedInput(t, f.fixture.conn, original.inputID); got != settled {
			t.Fatalf("receipt %+v, want the first settlement %+v", got, settled)
		}
		if err := held.validateBinding(s.databaseURL, p); err != nil {
			t.Fatal(err)
		}
		held.close()
		inputServiceBarrier(t, f, outer)
		if lines := recordLines(t, f); !slices.Equal(lines, []string{"barrier"}) {
			t.Fatalf("receipt-only CLI work sends no native input: %q", lines)
		}
	})
}
