package core

import (
	"math"
	"time"
)

func combineManualProgress[T any](progress T, progressErr, accounting error) (T, error) {
	switch {
	case progressErr == nil && accounting == nil:
		return progress, nil
	case progressErr != nil && accounting != nil:
		return progress, cafleetErrorOf(progressErr).withDiagnostic(cafleetErrorOf(accounting).message())
	case progressErr != nil:
		return progress, progressErr
	default:
		return progress, accounting
	}
}

// inputExpiration is an input deadline as an absolute monotonic-clock nanosecond reading, so it
// crosses a process boundary.
type inputExpiration struct {
	expiresAt uint64
}

func inputExpirationCapture(deadline time.Time, c clock) (inputExpiration, error) {
	sample, err := c.monotonicSample()
	if err != nil {
		return inputExpiration{}, err
	}
	if err := sample.validate(); err != nil {
		return inputExpiration{}, err
	}
	remaining := deadline.Sub(sample.after)
	if remaining <= 0 {
		return inputExpiration{}, deadlineExceededError()
	}
	remainingNs := uint64(remaining) //nolint:gosec // remaining is positive
	if remainingNs > math.MaxUint64-sample.monotonicNs {
		return inputExpiration{}, runtimeError("input expiration overflows monotonic nanoseconds")
	}
	return inputExpiration{expiresAt: sample.monotonicNs + remainingNs}, nil
}

func (e inputExpiration) receive(admittedAt time.Time, c clock) (time.Time, error) {
	sample, err := c.monotonicSample()
	if err != nil {
		return time.Time{}, err
	}
	if err := sample.validate(); err != nil {
		return time.Time{}, err
	}
	if e.expiresAt <= sample.monotonicNs {
		return time.Time{}, deadlineExceededError()
	}
	remainingNs := e.expiresAt - sample.monotonicNs
	if remainingNs > math.MaxInt64 {
		return time.Time{}, runtimeError("input expiration exceeds the local clock range")
	}
	expiration := sample.before.Add(time.Duration(remainingNs))
	admission := admittedAt.Add(5 * time.Second)
	deadline := expiration
	if admission.Before(deadline) {
		deadline = admission
	}
	if err := runtimeCheckDeadline(sample.after, deadline); err != nil {
		return time.Time{}, err
	}
	return deadline, nil
}

func (e inputExpiration) monotonicNs() uint64 {
	return e.expiresAt
}

func receiveInputExpiration(expiresAtMonoNs uint64, admittedAt time.Time, c clock) (time.Time, error) {
	return inputExpiration{expiresAt: expiresAtMonoNs}.receive(admittedAt, c)
}
