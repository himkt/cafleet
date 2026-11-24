package core

import (
	"context"
	"encoding/json/jsontext"
	"math"
	"time"
)

func inspectionAssociation(ctx context.Context, conn *dbConn, run runRecord, input permissionInspectInput) (runtimeEventView, error) {
	if err := ensure(
		input.EventID > 0 && input.InteractionID > 0 && input.DirectorMemberID == run.directorMemberID,
		"permission inspection identity mismatch",
	); err != nil {
		return runtimeEventView{}, err
	}
	view, err := getEvent(ctx, conn, input.EventID)
	if err != nil {
		return runtimeEventView{}, err
	}
	messageID, messaged := view.event.messageID.get()
	interaction, interacted := view.interaction.get()
	if err := ensure(
		view.event.runID == run.runID &&
			view.event.interactionID == some(input.InteractionID) &&
			view.event.kind == runtimeEventKindInteractionOpened &&
			view.event.notificationState == notificationStateInbox &&
			messaged && messageID > 0 &&
			interacted &&
			interaction.runID == run.runID &&
			(interaction.classification == interactionClassPermissionRequest || interaction.classification == interactionClassQuestion) &&
			!interaction.closedAt.set,
		"permission inspection current event/interaction/message mismatch",
	); err != nil {
		return runtimeEventView{}, err
	}
	director, err := getMember(ctx, conn, run.directorMemberID, run.fleetID)
	if err != nil {
		return runtimeEventView{}, err
	}
	record, found := director.get()
	if !found {
		return runtimeEventView{}, runtimeError("permission Director is absent")
	}
	fleet, err := activeMemberFleet(ctx, conn, run.directorMemberID)
	if err != nil {
		return runtimeEventView{}, err
	}
	if err := ensure(record.kind == memberKindDirector && fleet == some(run.fleetID), "permission Director is inactive"); err != nil {
		return runtimeEventView{}, err
	}
	live, err := queryRow(conn,
		"SELECT EXISTS(SELECT 1 FROM fleets WHERE fleet_id=?1 AND director_member_id=?2 AND deleted_at IS NULL)",
		[]any{run.fleetID, run.directorMemberID},
		func(row sqlRow) (bool, error) { return row.bool(0) })
	if err != nil {
		return runtimeEventView{}, err
	}
	if err := ensure(live, "permission fleet/Director changed"); err != nil {
		return runtimeEventView{}, err
	}
	return view, nil
}

// interactionMissingEvidence is the operation-centred facts an inspected interaction lacks; a
// question lacks none of them.
func interactionMissingEvidence(classification interactionClass, evidence jsontext.Value) ([]missingEvidence, error) {
	if classification == interactionClassQuestion {
		if err := validateEvidence(evidence); err != nil {
			return nil, err
		}
		return []missingEvidence{}, nil
	}
	return inspectionMissingEvidence(evidence)
}

func inspectionMissingEvidence(evidence jsontext.Value) ([]missingEvidence, error) {
	if err := validateEvidence(evidence); err != nil {
		return nil, err
	}
	return []missingEvidence{
		missingEvidenceOperation,
		missingEvidenceArguments,
		missingEvidenceCwd,
		missingEvidenceEnvironment,
		missingEvidenceTargets,
		missingEvidenceEffects,
		missingEvidenceChoiceScope,
	}, nil
}

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
