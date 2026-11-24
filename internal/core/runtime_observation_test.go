package core

import (
	"testing"
	"time"
)

func TestRuntimeObservation_InputEligibilityRendersComposerEmptyOrHoldWithItsReason(t *testing.T) {
	t.Parallel()
	cases := []struct {
		eligibility inputEligibility
		expected    string
	}{
		{inputEligibilityComposerEmpty{}, "composer_empty"},
		{inputEligibilityHold{reason: gateReasonCandidate}, "hold:candidate"},
		{inputEligibilityHold{reason: gateReasonInteraction}, "hold:interaction"},
		{inputEligibilityHold{reason: gateReasonUnknown}, "hold:unknown"},
		{inputEligibilityHold{reason: gateReasonObserverUnavailable}, "hold:observer_unavailable"},
		{inputEligibilityHold{reason: gateReasonGap}, "hold:gap"},
	}
	for _, tc := range cases {
		if got := tc.eligibility.rendering(); got != tc.expected {
			t.Fatalf("%s: rendering = %q", tc.expected, got)
		}
	}
}

const observationFixtureNow = "2026-10-26T16:40:00.000000+00:00"

// observationFixtureClock is a virtual clock whose wall time starts at observationFixtureNow.
func observationFixtureClock(t *testing.T) *testClock {
	t.Helper()
	origin, ok := parseLenient(observationFixtureNow)
	if !ok {
		t.Fatal("the fixture stamp is canonical")
	}
	return testClockStartingAt(time.Now()).withWallOrigin(origin)
}
