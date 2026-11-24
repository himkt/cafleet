package core

import "testing"

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
