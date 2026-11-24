package core

import (
	"encoding/json/jsontext"
	"slices"
	"strings"
	"testing"
	"time"
)

const observationOpened = "2026-11-21T12:04:08.000000+00:00"

func healthyObservation() observationHealth {
	return observationHealth{Visibility: visibilityNotVisible, Eligibility: inputEligibilityComposerEmpty{}}
}

func degradedObservation() observationHealth {
	return observationHealth{
		Visibility:       visibilityUnknown,
		Eligibility:      inputEligibilityHold{reason: gateReasonGap},
		PendingCount:     2,
		GapCount:         3,
		Degraded:         true,
		Diagnostic:       some("sink_unavailable"),
		WriterDiagnostic: some("database error: database is locked"),
		GapOpenSince:     some(observationOpened),
	}
}

func registryRuntime() jsonObject {
	return jsonObject{
		{"run_id", 37}, {"mode", "auto"}, {"status", "running"}, {"coverage", "tested"},
		{"child_pid", 4242}, {"child_start_identity", "child-start"},
	}
}

func showDiagnostics() []runnerDiagnosticEntry {
	return []runnerDiagnosticEntry{
		{source: runnerDiagnosticSourceEventWriter, message: "event writer diagnostic: database is locked", firstAt: "2026-11-21T12:04:00.000000+00:00", lastAt: "2026-11-21T12:05:00.000000+00:00", count: 4},
		{source: runnerDiagnosticSourceObservation, message: "observation gap opened: sink_unavailable", firstAt: observationOpened, lastAt: observationOpened, count: 1},
	}
}

type showCase struct {
	name        string
	probe       observationProbe
	probeName   string
	observation jsontext.Value
	err         jsontext.Value
	suffix      string
}

var showCases = []showCase{
	{"skipped", observationProbeSkipped{}, "skipped", jsontext.Value("null"), jsontext.Value("null"), " observation=-"},
	{"failed", observationProbeFailed{message: "cannot connect to the runner"}, "failed", jsontext.Value("null"), jsontext.Value(`"cannot connect to the runner"`), " observation=unavailable"},
	{"answered before the child started", observationProbeAnswered{health: none[observationHealth]()}, "answered", jsontext.Value("null"), jsontext.Value("null"), " observation=pending"},
	{"answered healthy", observationProbeAnswered{health: some(healthyObservation())}, "answered",
		jsontext.Value(`{"visibility":"not_visible","eligibility":"composer_empty","degraded":false,"diagnostic":null,"writer_diagnostic":null,"pending_count":0,"gap_count":0,"gap_open_since":null}`),
		jsontext.Value("null"), " observation=composer_empty"},
	{"answered degraded", observationProbeAnswered{health: some(degradedObservation())}, "answered",
		jsontext.Value(`{"visibility":"unknown","eligibility":"hold:gap","degraded":true,"diagnostic":"sink_unavailable","writer_diagnostic":"database error: database is locked","pending_count":2,"gap_count":3,"gap_open_since":"` + observationOpened + `"}`),
		jsontext.Value("null"), " observation=hold:gap degraded"},
}

func TestCliMember_ObservationSurface_MemberShowRuntimeAppendsTheProbeKeysForEachOutcome(t *testing.T) {
	t.Parallel()
	registry := mustJSON(t, registryRuntime())
	registryKeys := testJSONKeys(t, registry)
	for _, tc := range showCases {
		runtime := mustJSON(t, showRuntime(registryRuntime(), tc.probe, showDiagnostics()))
		if keys := testJSONKeys(t, runtime); !slices.Equal(keys, append(slices.Clone(registryKeys), "observation_probe", "observation", "observation_error", "diagnostics")) {
			t.Fatalf("%s: keys %q", tc.name, keys)
		}
		for _, key := range registryKeys {
			assertJSONEqual(t, jsonAt(t, runtime, key), jsonAt(t, registry, key), tc.name+": "+key)
		}
		assertJSONEqual(t, jsonAt(t, runtime, "observation_probe"), mustJSON(t, tc.probeName), tc.name)
		observation := jsonAt(t, runtime, "observation")
		assertJSONEqual(t, observation, tc.observation, tc.name)
		if string(tc.observation) != "null" {
			if keys := testJSONKeys(t, observation); !slices.Equal(keys, []string{"visibility", "eligibility", "degraded", "diagnostic", "writer_diagnostic", "pending_count", "gap_count", "gap_open_since"}) {
				t.Fatalf("%s: observation keys %q", tc.name, keys)
			}
		}
		assertJSONEqual(t, jsonAt(t, runtime, "observation_error"), tc.err, tc.name)
		diagnostics := jsonAt(t, runtime, "diagnostics")
		assertJSONEqual(t, diagnostics, jsontext.Value(`[
			{"source":"event_writer","message":"event writer diagnostic: database is locked","first_at":"2026-11-21T12:04:00.000000+00:00","last_at":"2026-11-21T12:05:00.000000+00:00","count":4},
			{"source":"observation","message":"observation gap opened: sink_unavailable","first_at":"`+observationOpened+`","last_at":"`+observationOpened+`","count":1}]`), tc.name)
		for _, row := range decodedArray(t, diagnostics) {
			if keys := testJSONKeys(t, row); !slices.Equal(keys, []string{"source", "message", "first_at", "last_at", "count"}) {
				t.Fatalf("%s: diagnostic keys %q", tc.name, keys)
			}
		}
	}
}

func TestCliMember_ObservationSurface_MemberShowTextSuffixNamesTheObservationStateAndDegradation(t *testing.T) {
	t.Parallel()
	for _, tc := range showCases {
		if got := observationSuffix(tc.probe); got != tc.suffix {
			t.Fatalf("%s: suffix %q, want %q", tc.name, got, tc.suffix)
		}
	}
}

func TestCliMember_ObservationSurface_MemberShowRuntimeWithoutDiagnosticsReportsAnEmptyList(t *testing.T) {
	t.Parallel()
	runtime := mustJSON(t, showRuntime(registryRuntime(), observationProbeSkipped{}, nil))
	assertJSONEqual(t, jsonAt(t, runtime, "diagnostics"), jsontext.Value("[]"), "diagnostics")
}

func TestCliMember_ObservationSurface_ProbeObservationSkipsUnprobedStatusesAndFailsWithoutARunner(t *testing.T) {
	t.Parallel()
	cases := []struct {
		status string
		probed bool
	}{
		{"starting", false}, {"prepared", false}, {"recovering", false}, {"exited", false}, {"failed", false},
		{"running", true}, {"degraded", true},
	}
	for _, tc := range cases {
		fixture := newBrokerRuntimeFixture(t)
		if _, err := fixture.conn.execute("UPDATE member_runs SET status=?1 WHERE run_id=?2", tc.status, fixture.run.runID); err != nil {
			t.Fatal(err)
		}
		run := mustRun(t, fixture.conn, fixture.run.runID)
		if want, err := runStatusTryFrom(tc.status); err != nil || run.status != want {
			t.Fatalf("%s: status %v %v", tc.status, run.status, err)
		}
		started := time.Now()
		probe := probeObservation(run, systemClock{})
		if elapsed := time.Since(started); elapsed >= 2*time.Second {
			t.Fatalf("%s: the probe is bounded by its 1 s deadline: %v", tc.status, elapsed)
		}
		switch probe := probe.(type) {
		case observationProbeSkipped:
			if tc.probed {
				t.Fatalf("%s: skipped", tc.status)
			}
		case observationProbeFailed:
			if !tc.probed || strings.TrimSpace(probe.message) == "" {
				t.Fatalf("%s: failed %q", tc.status, probe.message)
			}
		case observationProbeAnswered:
			t.Fatalf("%s: no runner answers in this fixture", tc.status)
		default:
			panic("invalid observationProbe")
		}
	}
}
