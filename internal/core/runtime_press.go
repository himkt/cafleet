package core

import (
	"context"
	"fmt"
)

type keyAcknowledgment struct {
	memberID int64
	paneID   string
	keys     []string
}

// pressEffectError adds the delivery diagnostic to a failed press: uncertain once a key
// process started, otherwise not dispatched.
func pressEffectError(memberID int64, cause error, started bool) error {
	diagnostic := fmt.Sprintf("keys were not dispatched to member %d.", memberID)
	if started {
		diagnostic = fmt.Sprintf("key delivery to member %d is uncertain; any prefix may have been handled. Inspect the pane before an explicit retry.", memberID)
	}
	return cafleetErrorOf(cause).withDiagnostic(diagnostic)
}

// pressMemberWithMux sends keys to member's pane through mux under its pane lock, inside a
// termination-deferral span, after confirming the recorded placement.
func pressMemberWithMux(ctx context.Context, conn *dbConn, s settings, member memberRecord, keys pressKeys, d deadline, mux multiplexer, c clock) (keyAcknowledgment, error) {
	started := false
	acknowledgment, err := pressLocked(ctx, conn, s, member, keys, d, mux, c, &started)
	if err != nil {
		return keyAcknowledgment{}, pressEffectError(member.memberID, err, started)
	}
	return acknowledgment, nil
}

func pressLocked(ctx context.Context, conn *dbConn, s settings, member memberRecord, keys pressKeys, d deadline, mux multiplexer, c clock, started *bool) (keyAcknowledgment, error) {
	memberID := member.memberID
	if member.kind == memberKindDirector {
		return keyAcknowledgment{}, appError("cannot press keys in the Director's own pane")
	}
	p := member.placement
	paneID, concrete := p.muxPaneID.get()
	if !concrete {
		return keyAcknowledgment{}, appErrorf("member %d has no pane yet (pending placement) — nothing to press.", memberID)
	}
	span, err := deferTerminationBegin()
	if err != nil {
		return keyAcknowledgment{}, err
	}
	defer span.close()
	locked, acquired, err := lockMemberWithReservation(ctx, conn, s, member, d)
	if err != nil {
		return keyAcknowledgment{}, err
	}
	if !acquired {
		return keyAcknowledgment{}, appErrorf("member %d's pane input remained busy", memberID)
	}
	defer locked.close()
	if err := validateKeyTarget(mux, p, d); err != nil {
		return keyAcknowledgment{}, err.cafleet()
	}
	if _, err := d.remainingAt(c.now()); err != nil {
		return keyAcknowledgment{}, err
	}
	current, err := withDatabaseBudget(ctx, conn, d, func(ctx context.Context, conn *dbConn) (memberRecord, error) {
		return activeMember(ctx, conn, memberID)
	})
	if err != nil {
		return keyAcknowledgment{}, err
	}
	if current.fleetID != member.fleetID || current.kind != member.kind || current.placement != member.placement {
		return keyAcknowledgment{}, appErrorf("member %d placement changed before key dispatch", memberID)
	}
	if _, err := locked.paneLock(s.databaseURL); err != nil {
		return keyAcknowledgment{}, err
	}
	if _, err := d.remainingAt(c.now()); err != nil {
		return keyAcknowledgment{}, err
	}
	if err := mux.sendKeys(paneID, keys, d); err != nil {
		*started = err.submissionStarted
		return keyAcknowledgment{}, err.cafleet()
	}
	return keyAcknowledgment{memberID: memberID, paneID: paneID, keys: keys.canonical()}, nil
}
