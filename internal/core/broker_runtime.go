package core

import (
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"os"
	"strings"
	"syscall"
	"unicode/utf8"
)

type modeSource uint8

const (
	modeSourceDefault modeSource = iota + 1
	modeSourceExplicit
)

func (m modeSource) asStr() string {
	switch m {
	case modeSourceDefault:
		return "default"
	case modeSourceExplicit:
		return "explicit"
	default:
		panic("invalid modeSource")
	}
}

func modeSourceTryFrom(value string) (modeSource, error) {
	switch value {
	case "default":
		return modeSourceDefault, nil
	case "explicit":
		return modeSourceExplicit, nil
	default:
		return 0, invalidStoredValueError("member_runs.mode_source", value)
	}
}

type coverage uint8

const (
	coverageTested coverage = iota + 1
	coverageUnknown
)

func (c coverage) asStr() string {
	switch c {
	case coverageTested:
		return "tested"
	case coverageUnknown:
		return "unknown"
	default:
		panic("invalid coverage")
	}
}

func coverageTryFrom(value string) (coverage, error) {
	switch value {
	case "tested":
		return coverageTested, nil
	case "unknown":
		return coverageUnknown, nil
	default:
		return 0, invalidStoredValueError("member_runs.coverage", value)
	}
}

type interactionClass uint8

const (
	interactionClassPermissionRequest interactionClass = iota + 1
	interactionClassQuestion
	interactionClassUnknownInteraction
)

func (c interactionClass) asStr() string {
	switch c {
	case interactionClassPermissionRequest:
		return "permission_request"
	case interactionClassQuestion:
		return "question"
	case interactionClassUnknownInteraction:
		return "unknown_interaction"
	default:
		panic("invalid interactionClass")
	}
}

func interactionClassTryFrom(value string) (interactionClass, error) {
	switch value {
	case "permission_request":
		return interactionClassPermissionRequest, nil
	case "question":
		return interactionClassQuestion, nil
	case "unknown_interaction":
		return interactionClassUnknownInteraction, nil
	default:
		return 0, invalidStoredValueError("member_interactions.classification", value)
	}
}

type cleanupState uint8

const (
	cleanupStateComplete cleanupState = iota + 1
	cleanupStateIncomplete
)

func (c cleanupState) asStr() string {
	switch c {
	case cleanupStateComplete:
		return "complete"
	case cleanupStateIncomplete:
		return "incomplete"
	default:
		panic("invalid cleanupState")
	}
}

func cleanupStateTryFrom(value string) (cleanupState, error) {
	switch value {
	case "complete":
		return cleanupStateComplete, nil
	case "incomplete":
		return cleanupStateIncomplete, nil
	default:
		return 0, invalidStoredValueError("member_runs.cleanup_state", value)
	}
}

type permissionDecision uint8

const (
	permissionDecisionAllow permissionDecision = iota + 1
	permissionDecisionDeny
)

func (d permissionDecision) asStr() string {
	switch d {
	case permissionDecisionAllow:
		return "allow"
	case permissionDecisionDeny:
		return "deny"
	default:
		panic("invalid permissionDecision")
	}
}

func permissionDecisionTryFrom(value string) (permissionDecision, error) {
	switch value {
	case "allow":
		return permissionDecisionAllow, nil
	case "deny":
		return permissionDecisionDeny, nil
	default:
		return 0, invalidStoredValueError("response.decision", value)
	}
}

func (v modeSource) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *modeSource) UnmarshalText(text []byte) error {
	parsed, err := modeSourceTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

func (v coverage) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *coverage) UnmarshalText(text []byte) error {
	parsed, err := coverageTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

func (v interactionClass) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *interactionClass) UnmarshalText(text []byte) error {
	parsed, err := interactionClassTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

func (v cleanupState) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *cleanupState) UnmarshalText(text []byte) error {
	parsed, err := cleanupStateTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

func (v permissionDecision) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *permissionDecision) UnmarshalText(text []byte) error {
	parsed, err := permissionDecisionTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type runStatus uint8

const (
	runStatusStarting runStatus = iota + 1
	runStatusPrepared
	runStatusRunning
	runStatusDegraded
	runStatusRecovering
	runStatusExited
	runStatusFailed
	runStatusLost
)

func (v runStatus) asStr() string {
	switch v {
	case runStatusStarting:
		return "starting"
	case runStatusPrepared:
		return "prepared"
	case runStatusRunning:
		return "running"
	case runStatusDegraded:
		return "degraded"
	case runStatusRecovering:
		return "recovering"
	case runStatusExited:
		return "exited"
	case runStatusFailed:
		return "failed"
	case runStatusLost:
		return "lost"
	default:
		panic("invalid runStatus")
	}
}

func runStatusTryFrom(value string) (runStatus, error) {
	switch value {
	case "starting":
		return runStatusStarting, nil
	case "prepared":
		return runStatusPrepared, nil
	case "running":
		return runStatusRunning, nil
	case "degraded":
		return runStatusDegraded, nil
	case "recovering":
		return runStatusRecovering, nil
	case "exited":
		return runStatusExited, nil
	case "failed":
		return runStatusFailed, nil
	case "lost":
		return runStatusLost, nil
	default:
		return 0, invalidStoredValueError("member_runs.status", value)
	}
}

func (v runStatus) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *runStatus) UnmarshalText(text []byte) error {
	parsed, err := runStatusTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type ownerKind uint8

const (
	ownerKindCreator ownerKind = iota + 1
	ownerKindRunner
	ownerKindReconciler
)

func (v ownerKind) asStr() string {
	switch v {
	case ownerKindCreator:
		return "creator"
	case ownerKindRunner:
		return "runner"
	case ownerKindReconciler:
		return "reconciler"
	default:
		panic("invalid ownerKind")
	}
}

func ownerKindTryFrom(value string) (ownerKind, error) {
	switch value {
	case "creator":
		return ownerKindCreator, nil
	case "runner":
		return ownerKindRunner, nil
	case "reconciler":
		return ownerKindReconciler, nil
	default:
		return 0, invalidStoredValueError("member_runs.owner_kind", value)
	}
}

func (v ownerKind) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *ownerKind) UnmarshalText(text []byte) error {
	parsed, err := ownerKindTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type interactionCloseReason uint8

const (
	interactionCloseReasonNotVisible interactionCloseReason = iota + 1
	interactionCloseReasonSuperseded
	interactionCloseReasonProcessExited
	interactionCloseReasonMonitorLost
)

func (v interactionCloseReason) asStr() string {
	switch v {
	case interactionCloseReasonNotVisible:
		return "not_visible"
	case interactionCloseReasonSuperseded:
		return "superseded"
	case interactionCloseReasonProcessExited:
		return "process_exited"
	case interactionCloseReasonMonitorLost:
		return "monitor_lost"
	default:
		panic("invalid interactionCloseReason")
	}
}

func interactionCloseReasonTryFrom(value string) (interactionCloseReason, error) {
	switch value {
	case "not_visible":
		return interactionCloseReasonNotVisible, nil
	case "superseded":
		return interactionCloseReasonSuperseded, nil
	case "process_exited":
		return interactionCloseReasonProcessExited, nil
	case "monitor_lost":
		return interactionCloseReasonMonitorLost, nil
	default:
		return 0, invalidStoredValueError("member_interactions.close_reason", value)
	}
}

func (v interactionCloseReason) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *interactionCloseReason) UnmarshalText(text []byte) error {
	parsed, err := interactionCloseReasonTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type runtimeEventKind uint8

const (
	runtimeEventKindCoverageUnknown runtimeEventKind = iota + 1
	runtimeEventKindInteractionOpened
	runtimeEventKindInteractionClosed
	runtimeEventKindInputDeliveryResult
	runtimeEventKindInputDeliveryStalled
	runtimeEventKindRunnerDegraded
	runtimeEventKindRunnerLost
	runtimeEventKindRunnerExited
)

func (v runtimeEventKind) asStr() string {
	switch v {
	case runtimeEventKindCoverageUnknown:
		return "coverage_unknown"
	case runtimeEventKindInteractionOpened:
		return "interaction_opened"
	case runtimeEventKindInteractionClosed:
		return "interaction_closed"
	case runtimeEventKindInputDeliveryResult:
		return "input_delivery_result"
	case runtimeEventKindInputDeliveryStalled:
		return "input_delivery_stalled"
	case runtimeEventKindRunnerDegraded:
		return "runner_degraded"
	case runtimeEventKindRunnerLost:
		return "runner_lost"
	case runtimeEventKindRunnerExited:
		return "runner_exited"
	default:
		panic("invalid runtimeEventKind")
	}
}

func runtimeEventKindTryFrom(value string) (runtimeEventKind, error) {
	switch value {
	case "coverage_unknown":
		return runtimeEventKindCoverageUnknown, nil
	case "interaction_opened":
		return runtimeEventKindInteractionOpened, nil
	case "interaction_closed":
		return runtimeEventKindInteractionClosed, nil
	case "input_delivery_result":
		return runtimeEventKindInputDeliveryResult, nil
	case "input_delivery_stalled":
		return runtimeEventKindInputDeliveryStalled, nil
	case "runner_degraded":
		return runtimeEventKindRunnerDegraded, nil
	case "runner_lost":
		return runtimeEventKindRunnerLost, nil
	case "runner_exited":
		return runtimeEventKindRunnerExited, nil
	default:
		return 0, invalidStoredValueError("member_runtime_events.kind", value)
	}
}

func (v runtimeEventKind) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *runtimeEventKind) UnmarshalText(text []byte) error {
	parsed, err := runtimeEventKindTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type runtimeCauseKind uint8

const (
	runtimeCauseKindCoverageUnknown runtimeCauseKind = iota + 1
	runtimeCauseKindNativeExit
	runtimeCauseKindStoragePressure
	runtimeCauseKindStorageRecovered
	runtimeCauseKindObserverError
	runtimeCauseKindMonitorLost
	runtimeCauseKindRuntimePathMissing
	runtimeCauseKindEventRejected
)

func (v runtimeCauseKind) asStr() string {
	switch v {
	case runtimeCauseKindCoverageUnknown:
		return "coverage_unknown"
	case runtimeCauseKindNativeExit:
		return "native_exit"
	case runtimeCauseKindStoragePressure:
		return "storage_pressure"
	case runtimeCauseKindStorageRecovered:
		return "storage_recovered"
	case runtimeCauseKindObserverError:
		return "observer_error"
	case runtimeCauseKindMonitorLost:
		return "monitor_lost"
	case runtimeCauseKindRuntimePathMissing:
		return "runtime_path_missing"
	case runtimeCauseKindEventRejected:
		return "event_rejected"
	default:
		panic("invalid runtimeCauseKind")
	}
}

func runtimeCauseKindTryFrom(value string) (runtimeCauseKind, error) {
	switch value {
	case "coverage_unknown":
		return runtimeCauseKindCoverageUnknown, nil
	case "native_exit":
		return runtimeCauseKindNativeExit, nil
	case "storage_pressure":
		return runtimeCauseKindStoragePressure, nil
	case "storage_recovered":
		return runtimeCauseKindStorageRecovered, nil
	case "observer_error":
		return runtimeCauseKindObserverError, nil
	case "monitor_lost":
		return runtimeCauseKindMonitorLost, nil
	case "runtime_path_missing":
		return runtimeCauseKindRuntimePathMissing, nil
	case "event_rejected":
		return runtimeCauseKindEventRejected, nil
	default:
		return 0, invalidStoredValueError("cause.kind", value)
	}
}

func (v runtimeCauseKind) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *runtimeCauseKind) UnmarshalText(text []byte) error {
	parsed, err := runtimeCauseKindTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type notificationState uint8

const (
	notificationStateInbox notificationState = iota + 1
	notificationStateArchived
)

func (v notificationState) asStr() string {
	switch v {
	case notificationStateInbox:
		return "inbox"
	case notificationStateArchived:
		return "archived"
	default:
		panic("invalid notificationState")
	}
}

func notificationStateTryFrom(value string) (notificationState, error) {
	switch value {
	case "inbox":
		return notificationStateInbox, nil
	case "archived":
		return notificationStateArchived, nil
	default:
		return 0, invalidStoredValueError("member_runtime_events.notification_state", value)
	}
}

func (v notificationState) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *notificationState) UnmarshalText(text []byte) error {
	parsed, err := notificationStateTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type archiveReason uint8

const (
	archiveReasonMemberInactive archiveReason = iota + 1
	archiveReasonFleetDeleted
	archiveReasonDirectorInactive
	archiveReasonHeadNotReady
	archiveReasonDuplicateDelivery
)

func (v archiveReason) asStr() string {
	switch v {
	case archiveReasonMemberInactive:
		return "member_inactive"
	case archiveReasonFleetDeleted:
		return "fleet_deleted"
	case archiveReasonDirectorInactive:
		return "director_inactive"
	case archiveReasonHeadNotReady:
		return "head_not_ready"
	case archiveReasonDuplicateDelivery:
		return "duplicate_delivery"
	default:
		panic("invalid archiveReason")
	}
}

func archiveReasonTryFrom(value string) (archiveReason, error) {
	switch value {
	case "member_inactive":
		return archiveReasonMemberInactive, nil
	case "fleet_deleted":
		return archiveReasonFleetDeleted, nil
	case "director_inactive":
		return archiveReasonDirectorInactive, nil
	case "head_not_ready":
		return archiveReasonHeadNotReady, nil
	case "duplicate_delivery":
		return archiveReasonDuplicateDelivery, nil
	default:
		return 0, invalidStoredValueError("member_runtime_events.archive_reason", value)
	}
}

func (v archiveReason) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *archiveReason) UnmarshalText(text []byte) error {
	parsed, err := archiveReasonTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type runnerDiagnosticSource uint8

const (
	runnerDiagnosticSourceEventWriter runnerDiagnosticSource = iota + 1
	runnerDiagnosticSourceObservation
	runnerDiagnosticSourceDeferredInput
	runnerDiagnosticSourceQueueHousekeeping
	runnerDiagnosticSourcePreviewSweep
	runnerDiagnosticSourceDirectorDelivery
	runnerDiagnosticSourceInputResult
	runnerDiagnosticSourcePermissionAnswer
	runnerDiagnosticSourceShutdown
)

func (v runnerDiagnosticSource) asStr() string {
	switch v {
	case runnerDiagnosticSourceEventWriter:
		return "event_writer"
	case runnerDiagnosticSourceObservation:
		return "observation"
	case runnerDiagnosticSourceDeferredInput:
		return "deferred_input"
	case runnerDiagnosticSourceQueueHousekeeping:
		return "queue_housekeeping"
	case runnerDiagnosticSourcePreviewSweep:
		return "preview_sweep"
	case runnerDiagnosticSourceDirectorDelivery:
		return "director_delivery"
	case runnerDiagnosticSourceInputResult:
		return "input_result"
	case runnerDiagnosticSourcePermissionAnswer:
		return "permission_answer"
	case runnerDiagnosticSourceShutdown:
		return "shutdown"
	default:
		panic("invalid runnerDiagnosticSource")
	}
}

func runnerDiagnosticSourceTryFrom(value string) (runnerDiagnosticSource, error) {
	switch value {
	case "event_writer":
		return runnerDiagnosticSourceEventWriter, nil
	case "observation":
		return runnerDiagnosticSourceObservation, nil
	case "deferred_input":
		return runnerDiagnosticSourceDeferredInput, nil
	case "queue_housekeeping":
		return runnerDiagnosticSourceQueueHousekeeping, nil
	case "preview_sweep":
		return runnerDiagnosticSourcePreviewSweep, nil
	case "director_delivery":
		return runnerDiagnosticSourceDirectorDelivery, nil
	case "input_result":
		return runnerDiagnosticSourceInputResult, nil
	case "permission_answer":
		return runnerDiagnosticSourcePermissionAnswer, nil
	case "shutdown":
		return runnerDiagnosticSourceShutdown, nil
	default:
		return 0, invalidStoredValueError("member_runner_diagnostics.source", value)
	}
}

func (v runnerDiagnosticSource) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *runnerDiagnosticSource) UnmarshalText(text []byte) error {
	parsed, err := runnerDiagnosticSourceTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type inputKind uint8

const (
	inputKindPrompt inputKind = iota + 1
	inputKindExec
)

func (v inputKind) asStr() string {
	switch v {
	case inputKindPrompt:
		return "prompt"
	case inputKindExec:
		return "exec"
	default:
		panic("invalid inputKind")
	}
}

func inputKindTryFrom(value string) (inputKind, error) {
	switch value {
	case "prompt":
		return inputKindPrompt, nil
	case "exec":
		return inputKindExec, nil
	default:
		return 0, invalidStoredValueError("member_input_operations.kind", value)
	}
}

func (v inputKind) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *inputKind) UnmarshalText(text []byte) error {
	parsed, err := inputKindTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type inputState uint8

const (
	inputStateReady inputState = iota + 1
	inputStateInProgress
	inputStateAccepted
	inputStateUncertain
	inputStateSettled
)

func (v inputState) asStr() string {
	switch v {
	case inputStateReady:
		return "ready"
	case inputStateInProgress:
		return "in_progress"
	case inputStateAccepted:
		return "accepted"
	case inputStateUncertain:
		return "uncertain"
	case inputStateSettled:
		return "settled"
	default:
		panic("invalid inputState")
	}
}

func inputStateTryFrom(value string) (inputState, error) {
	switch value {
	case "ready":
		return inputStateReady, nil
	case "in_progress":
		return inputStateInProgress, nil
	case "accepted":
		return inputStateAccepted, nil
	case "uncertain":
		return inputStateUncertain, nil
	case "settled":
		return inputStateSettled, nil
	default:
		return 0, invalidStoredValueError("member_input_operations.state", value)
	}
}

func (v inputState) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *inputState) UnmarshalText(text []byte) error {
	parsed, err := inputStateTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type inputDisposition uint8

const (
	inputDispositionSubmitted inputDisposition = iota + 1
	inputDispositionNotSubmitted
	inputDispositionAbandonedUncertain
)

func (v inputDisposition) asStr() string {
	switch v {
	case inputDispositionSubmitted:
		return "submitted"
	case inputDispositionNotSubmitted:
		return "not_submitted"
	case inputDispositionAbandonedUncertain:
		return "abandoned_uncertain"
	default:
		panic("invalid inputDisposition")
	}
}

func inputDispositionTryFrom(value string) (inputDisposition, error) {
	switch value {
	case "submitted":
		return inputDispositionSubmitted, nil
	case "not_submitted":
		return inputDispositionNotSubmitted, nil
	case "abandoned_uncertain":
		return inputDispositionAbandonedUncertain, nil
	default:
		return 0, invalidStoredValueError("member_input_operations.disposition", value)
	}
}

func (v inputDisposition) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *inputDisposition) UnmarshalText(text []byte) error {
	parsed, err := inputDispositionTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type stopCause uint8

const (
	stopCauseInteractionCandidate stopCause = iota + 1
	stopCauseInteractionOpen
	stopCauseDeadlineExpired
	stopCauseLiteralMismatch
	stopCauseSubmitUnconfirmed
	stopCauseShortWrite
	stopCauseWriteFailed
	stopCauseRunnerStopped
)

func (v stopCause) asStr() string {
	switch v {
	case stopCauseInteractionCandidate:
		return "interaction_candidate"
	case stopCauseInteractionOpen:
		return "interaction_open"
	case stopCauseDeadlineExpired:
		return "deadline_expired"
	case stopCauseLiteralMismatch:
		return "literal_mismatch"
	case stopCauseSubmitUnconfirmed:
		return "submit_unconfirmed"
	case stopCauseShortWrite:
		return "short_write"
	case stopCauseWriteFailed:
		return "write_failed"
	case stopCauseRunnerStopped:
		return "runner_stopped"
	default:
		panic("invalid stopCause")
	}
}

func stopCauseTryFrom(value string) (stopCause, error) {
	switch value {
	case "interaction_candidate":
		return stopCauseInteractionCandidate, nil
	case "interaction_open":
		return stopCauseInteractionOpen, nil
	case "deadline_expired":
		return stopCauseDeadlineExpired, nil
	case "literal_mismatch":
		return stopCauseLiteralMismatch, nil
	case "submit_unconfirmed":
		return stopCauseSubmitUnconfirmed, nil
	case "short_write":
		return stopCauseShortWrite, nil
	case "write_failed":
		return stopCauseWriteFailed, nil
	case "runner_stopped":
		return stopCauseRunnerStopped, nil
	default:
		return 0, invalidStoredValueError("member_input_operations.stop_cause", value)
	}
}

func (v stopCause) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *stopCause) UnmarshalText(text []byte) error {
	parsed, err := stopCauseTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type deliveryStatus uint8

const (
	deliveryStatusSubmitted deliveryStatus = iota + 1
	deliveryStatusQueued
	deliveryStatusHeld
	deliveryStatusUncertain
	deliveryStatusNotSubmitted
	deliveryStatusNotApplicable
)

func (v deliveryStatus) asStr() string {
	switch v {
	case deliveryStatusSubmitted:
		return "submitted"
	case deliveryStatusQueued:
		return "queued"
	case deliveryStatusHeld:
		return "held"
	case deliveryStatusUncertain:
		return "uncertain"
	case deliveryStatusNotSubmitted:
		return "not_submitted"
	case deliveryStatusNotApplicable:
		return "not_applicable"
	default:
		panic("invalid deliveryStatus")
	}
}

func deliveryStatusTryFrom(value string) (deliveryStatus, error) {
	switch value {
	case "submitted":
		return deliveryStatusSubmitted, nil
	case "queued":
		return deliveryStatusQueued, nil
	case "held":
		return deliveryStatusHeld, nil
	case "uncertain":
		return deliveryStatusUncertain, nil
	case "not_submitted":
		return deliveryStatusNotSubmitted, nil
	case "not_applicable":
		return deliveryStatusNotApplicable, nil
	default:
		return 0, invalidStoredValueError("delivery.status", value)
	}
}

func (v deliveryStatus) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *deliveryStatus) UnmarshalText(text []byte) error {
	parsed, err := deliveryStatusTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

type deliveryMethod uint8

const (
	deliveryMethodProfile deliveryMethod = iota + 1
	deliveryMethodFallback
)

func (v deliveryMethod) asStr() string {
	switch v {
	case deliveryMethodProfile:
		return "profile"
	case deliveryMethodFallback:
		return "fallback"
	default:
		panic("invalid deliveryMethod")
	}
}

func deliveryMethodTryFrom(value string) (deliveryMethod, error) {
	switch value {
	case "profile":
		return deliveryMethodProfile, nil
	case "fallback":
		return deliveryMethodFallback, nil
	default:
		return 0, invalidStoredValueError("member_input_operations.delivery_method", value)
	}
}

func (v deliveryMethod) MarshalText() ([]byte, error) { return []byte(v.asStr()), nil }

func (v *deliveryMethod) UnmarshalText(text []byte) error {
	parsed, err := deliveryMethodTryFrom(string(text))
	if err != nil {
		return err
	}
	*v = parsed
	return nil
}

func brokerRuntimeInvalid(field, description string) *cafleetError {
	return invalidStoredValueError(field, description)
}

func brokerRuntimeRequire(condition bool, field, description string) error {
	if condition {
		return nil
	}
	return brokerRuntimeInvalid(field, description)
}

func brokerRuntimeNonblank(value, field string) error {
	return brokerRuntimeRequire(strings.TrimSpace(value) != "", field, "required nonempty value")
}

func rfc3339(value, field string) error {
	if _, ok := parseRFC3339(value); !ok {
		return brokerRuntimeInvalid(field, "RFC 3339 timestamp required")
	}
	return nil
}

func brokerRuntimeHex(value string, length int, field string) error {
	valid := len(value) == length && strings.Trim(value, "0123456789abcdefABCDEF") == ""
	return brokerRuntimeRequire(valid, field, "invalid hexadecimal identity")
}

// jsonColumn reads the named column as a JSON object, or an array when array holds.
func jsonColumn(row sqlRow, field string, array bool) (jsontext.Value, error) {
	text, err := row.text(row.columnIndex(field))
	if err != nil {
		return nil, err
	}
	value := jsontext.Value(text)
	want := jsontext.Kind('{')
	if array {
		want = '['
	}
	if !value.IsValid(jsontext.AllowDuplicateNames(true)) || value.Kind() != want {
		return nil, brokerRuntimeInvalid(field, "invalid JSON shape")
	}
	return value, nil
}

// optionalJSON reads the named nullable column like jsonColumn; nil means NULL.
func optionalJSON(row sqlRow, field string, array bool) (jsontext.Value, error) {
	present, err := row.optText(row.columnIndex(field))
	if err != nil {
		return nil, err
	}
	if _, set := present.get(); !set {
		return nil, nil
	}
	return jsonColumn(row, field, array)
}

func databaseIdentity(_ context.Context, conn *dbConn) (jsontext.Value, error) {
	canonical, err := canonicalizePath(conn.path())
	if err != nil {
		return nil, appError("cannot identify runtime database: " + ioErrorText(err))
	}
	info, err := os.Stat(canonical)
	if err != nil {
		return nil, appError("cannot inspect runtime database: " + ioErrorText(err))
	}
	instance, err := queryRow(conn, "SELECT instance_id FROM runtime_database_identity WHERE singleton_id=1",
		nil, func(row sqlRow) (string, error) { return row.text(0) })
	if err != nil {
		return nil, err
	}
	if err := brokerRuntimeHex(instance, 32, "runtime_database_identity.instance_id"); err != nil {
		return nil, err
	}
	if !utf8.ValidString(canonical) {
		return nil, brokerRuntimeInvalid("database_identity.canonical_path", "UTF-8 path required")
	}
	stat, ok := info.Sys().(*syscall.Stat_t)
	if !ok {
		panic("a file's metadata on this platform is a syscall.Stat_t")
	}
	encoded, err := encodeJSON(jsonObject{
		{"canonical_path", canonical},
		{"instance_id", instance},
		{"device", uint64(stat.Dev)}, //nolint:gosec // dev_t is a device number, never negative
		{"inode", stat.Ino},
	})
	if err != nil {
		return nil, err
	}
	return encoded, nil
}

func mapRun(row sqlRow) (runRecord, error) {
	var run runRecord
	var err error
	column := row.columnIndex
	if run.runID, err = row.int64(column("run_id")); err != nil {
		return runRecord{}, err
	}
	if run.memberID, err = row.int64(column("member_id")); err != nil {
		return runRecord{}, err
	}
	if run.fleetID, err = row.int64(column("fleet_id")); err != nil {
		return runRecord{}, err
	}
	if run.directorMemberID, err = row.int64(column("director_member_id")); err != nil {
		return runRecord{}, err
	}
	if run.backend, err = row.text(column("backend")); err != nil {
		return runRecord{}, err
	}
	if run.databaseIdentityJSON, err = jsonColumn(row, "database_identity_json", false); err != nil {
		return runRecord{}, err
	}
	if run.launchNonce, err = row.text(column("launch_nonce")); err != nil {
		return runRecord{}, err
	}
	if run.runtimePath, err = row.text(column("runtime_path")); err != nil {
		return runRecord{}, err
	}
	if run.requestedMode, err = row.optText(column("requested_mode")); err != nil {
		return runRecord{}, err
	}
	if run.resolvedMode, err = row.text(column("resolved_mode")); err != nil {
		return runRecord{}, err
	}
	if run.modeSource, err = rowStoredEnum(row, column("mode_source"), modeSourceTryFrom); err != nil {
		return runRecord{}, err
	}
	if run.permissionArgvJSON, err = jsonColumn(row, "permission_argv_json", true); err != nil {
		return runRecord{}, err
	}
	if run.coverage, err = rowStoredEnum(row, column("coverage"), coverageTryFrom); err != nil {
		return runRecord{}, err
	}
	if run.status, err = rowStoredEnum(row, column("status"), runStatusTryFrom); err != nil {
		return runRecord{}, err
	}
	if run.nativeVersion, err = row.optText(column("native_version")); err != nil {
		return runRecord{}, err
	}
	if run.ownerEpoch, err = row.int64(column("owner_epoch")); err != nil {
		return runRecord{}, err
	}
	if run.ownerKind, err = rowStoredEnum(row, column("owner_kind"), ownerKindTryFrom); err != nil {
		return runRecord{}, err
	}
	if run.ownerPID, err = row.int64(column("owner_pid")); err != nil {
		return runRecord{}, err
	}
	if run.ownerStartIdentity, err = row.text(column("owner_start_identity")); err != nil {
		return runRecord{}, err
	}
	if run.cleanupState, err = rowStoredEnum(row, column("cleanup_state"), cleanupStateTryFrom); err != nil {
		return runRecord{}, err
	}
	if run.runtimeDirIdentityJSON, err = optionalJSON(row, "runtime_dir_identity_json", false); err != nil {
		return runRecord{}, err
	}
	if run.runLockIdentityJSON, err = optionalJSON(row, "run_lock_identity_json", false); err != nil {
		return runRecord{}, err
	}
	if run.runnerPID, err = row.optInt64(column("runner_pid")); err != nil {
		return runRecord{}, err
	}
	if run.runnerStartIdentity, err = row.optText(column("runner_start_identity")); err != nil {
		return runRecord{}, err
	}
	if run.childPID, err = row.optInt64(column("child_pid")); err != nil {
		return runRecord{}, err
	}
	if run.childStartIdentity, err = row.optText(column("child_start_identity")); err != nil {
		return runRecord{}, err
	}
	if run.startedAt, err = row.optText(column("started_at")); err != nil {
		return runRecord{}, err
	}
	if run.lastHealthAt, err = row.optText(column("last_health_at")); err != nil {
		return runRecord{}, err
	}
	if run.endedAt, err = row.optText(column("ended_at")); err != nil {
		return runRecord{}, err
	}
	if run.exitCode, err = row.optInt64(column("exit_code")); err != nil {
		return runRecord{}, err
	}
	if run.cleanupDiagnosticsJSON, err = optionalJSON(row, "cleanup_diagnostics_json", true); err != nil {
		return runRecord{}, err
	}
	recovery, err := row.optText(column("recovery_origin_status"))
	if err != nil {
		return runRecord{}, err
	}
	run.recoveryOriginStatus = none[runStatus]()
	if text, set := recovery.get(); set {
		status, err := runStatusTryFrom(text)
		if err != nil {
			return runRecord{}, err
		}
		run.recoveryOriginStatus = some(status)
	}
	return run, nil
}

func getRun(_ context.Context, conn *dbConn, runID int64) (runRecord, error) {
	rows, err := queryRows(conn, "SELECT * FROM member_runs WHERE run_id=?1", []any{runID}, mapRun)
	if err != nil {
		return runRecord{}, err
	}
	if len(rows) == 0 {
		return runRecord{}, appError(fmt.Sprintf("Run %d not found", runID))
	}
	run := rows[0]
	if err := validateIdentity(run.databaseIdentityJSON); err != nil {
		return runRecord{}, err
	}
	var argv []jsontext.Value
	if err := json.Unmarshal(run.permissionArgvJSON, &argv, jsontext.AllowDuplicateNames(true)); err != nil {
		panic("a decoded permission argv is a JSON array: " + err.Error())
	}
	allStrings := true
	for _, argument := range argv {
		allStrings = allStrings && argument.Kind() == '"'
	}
	if err := brokerRuntimeRequire(allStrings, "member_runs.permission_argv_json", "string arguments required"); err != nil {
		return runRecord{}, err
	}
	return run, nil
}

func validateIdentity(identity jsontext.Value) error {
	var object map[string]jsontext.Value
	if identity.Kind() != '{' || json.Unmarshal(identity, &object, jsontext.AllowDuplicateNames(true)) != nil {
		return brokerRuntimeInvalid("database_identity_json", "object required")
	}
	var path string
	if raw, present := object["canonical_path"]; !present || raw.Kind() != '"' || json.Unmarshal(raw, &path) != nil {
		return brokerRuntimeInvalid("database_identity_json", "canonical_path required")
	}
	if err := brokerRuntimeRequire(strings.HasPrefix(path, "/"), "database_identity_json", "absolute canonical path required"); err != nil {
		return err
	}
	var instance string
	if raw, present := object["instance_id"]; !present || raw.Kind() != '"' || json.Unmarshal(raw, &instance) != nil {
		return brokerRuntimeInvalid("database_identity_json", "instance_id required")
	}
	if err := brokerRuntimeHex(instance, 32, "database_identity_json"); err != nil {
		return err
	}
	for _, field := range []string{"device", "inode"} {
		var number uint64
		raw, present := object[field]
		valid := present && raw.Kind() == '0' && json.Unmarshal(raw, &number) == nil
		if err := brokerRuntimeRequire(valid, "database_identity_json", "filesystem identity required"); err != nil {
			return err
		}
	}
	return nil
}

func historicalOwnership(ctx context.Context, conn *dbConn, run runRecord) (opt[archiveReason], error) {
	present := func(memberID int64) (fleetMembership, error) {
		membership, err := memberFleetStatus(ctx, conn, memberID)
		if err != nil {
			return fleetMembership{}, err
		}
		found, set := membership.get()
		if !set {
			return fleetMembership{}, appError("database error: Query returned no rows")
		}
		return found, nil
	}
	member, err := present(run.memberID)
	if err != nil {
		return none[archiveReason](), err
	}
	director, err := present(run.directorMemberID)
	if err != nil {
		return none[archiveReason](), err
	}
	found, err := getFleet(ctx, conn, run.fleetID)
	if err != nil {
		return none[archiveReason](), err
	}
	fleet, set := found.get()
	if !set {
		return none[archiveReason](), brokerRuntimeInvalid("member_runs.fleet_id", "fleet missing")
	}
	consistent := member.fleetID == run.fleetID && director.fleetID == run.fleetID &&
		fleet.directorMemberID == run.directorMemberID && run.memberID != run.directorMemberID
	if err := brokerRuntimeRequire(consistent, "member_runs.ownership", "member/fleet/Director mismatch"); err != nil {
		return none[archiveReason](), err
	}
	switch {
	case fleet.deletedAt != none[string]():
		return some(archiveReasonFleetDeleted), nil
	case member.status != memberStatusActive:
		return some(archiveReasonMemberInactive), nil
	case director.status != memberStatusActive:
		return some(archiveReasonDirectorInactive), nil
	default:
		return none[archiveReason](), nil
	}
}

func validateOwner(ctx context.Context, conn *dbConn, owner runOwner) (runRecord, error) {
	run, err := getRun(ctx, conn, owner.runID)
	if err != nil {
		return runRecord{}, err
	}
	if err := brokerRuntimeRequire(run.owner().equal(owner), "member_runs.owner", "stale ownership token"); err != nil {
		return runRecord{}, err
	}
	identity, err := databaseIdentity(ctx, conn)
	if err != nil {
		return runRecord{}, err
	}
	if err := brokerRuntimeRequire(jsonEqual(identity, run.databaseIdentityJSON), "member_runs.database_identity_json", "database identity changed"); err != nil {
		return runRecord{}, err
	}
	if _, err := historicalOwnership(ctx, conn, run); err != nil {
		return runRecord{}, err
	}
	return run, nil
}

// ownedTransaction runs operation in one immediate transaction whose owner validation passed,
// within the inherited deadline.
func ownedTransaction[T any](ctx context.Context, conn *dbConn, owner runOwner, operation func(context.Context, *dbConn, runRecord) (T, error)) (T, error) {
	d := deadlineInheritedOrNew(ctx)
	return withDatabaseBudget(ctx, conn, d, func(ctx context.Context, conn *dbConn) (T, error) {
		return inTransaction(conn, transactionImmediate, func() (T, error) {
			var zero T
			run, err := validateOwner(ctx, conn, owner)
			if err != nil {
				return zero, err
			}
			result, err := operation(ctx, conn, run)
			if err != nil {
				return zero, err
			}
			if _, err := d.remaining(); err != nil {
				return zero, err
			}
			return result, nil
		})
	})
}

func insertRun(ctx context.Context, conn *dbConn, run newRun) (runRecord, error) {
	if err := validateIdentity(run.databaseIdentityJSON); err != nil {
		return runRecord{}, err
	}
	if err := brokerRuntimeNonblank(run.ownerStartIdentity, "owner_start_identity"); err != nil {
		return runRecord{}, err
	}
	if err := brokerRuntimeNonblank(run.resolvedMode, "resolved_mode"); err != nil {
		return runRecord{}, err
	}
	if err := brokerRuntimeHex(run.launchNonce, 64, "launch_nonce"); err != nil {
		return runRecord{}, err
	}
	if err := brokerRuntimeRequire(strings.HasPrefix(run.runtimePath, "/"), "runtime_path", "absolute path required"); err != nil {
		return runRecord{}, err
	}
	if err := brokerRuntimeRequire(run.permissionArgvJSON.Kind() == '[', "permission_argv_json", "array required"); err != nil {
		return runRecord{}, err
	}
	identity, err := databaseIdentity(ctx, conn)
	if err != nil {
		return runRecord{}, err
	}
	if err := brokerRuntimeRequire(jsonEqual(identity, run.databaseIdentityJSON), "database_identity_json", "database identity mismatch"); err != nil {
		return runRecord{}, err
	}
	fleet, err := requireLiveFleet(ctx, conn, run.fleetID)
	if err != nil {
		return runRecord{}, err
	}
	memberFleet, err := activeMemberFleet(ctx, conn, run.memberID)
	if err != nil {
		return runRecord{}, err
	}
	consistent := fleet.directorMemberID == run.directorMemberID && run.memberID != run.directorMemberID &&
		memberFleet == some(run.fleetID)
	if err := brokerRuntimeRequire(consistent, "member_runs.ownership", "member/fleet/Director mismatch"); err != nil {
		return runRecord{}, err
	}
	if _, err := conn.execute(
		`INSERT INTO member_runs(member_id,fleet_id,director_member_id,backend,database_identity_json,
             launch_nonce,runtime_path,requested_mode,resolved_mode,mode_source,permission_argv_json,
             coverage,status,native_version,owner_epoch,
             owner_kind,owner_pid,owner_start_identity,cleanup_state)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'starting',?13,1,'creator',?14,?15,'incomplete')`,
		run.memberID, run.fleetID, run.directorMemberID, run.backend, jsonText(run.databaseIdentityJSON),
		run.launchNonce, run.runtimePath, run.requestedMode, run.resolvedMode, run.modeSource.asStr(),
		jsonText(run.permissionArgvJSON), run.coverage.asStr(),
		run.nativeVersion, run.ownerPID, run.ownerStartIdentity); err != nil {
		return runRecord{}, err
	}
	return getRun(ctx, conn, conn.raw.LastInsertRowID())
}

func brokerRuntimeSnapshot(ctx context.Context, conn *dbConn, runID int64) (runtimeSnapshot, error) {
	run, err := getRun(ctx, conn, runID)
	if err != nil {
		return runtimeSnapshot{}, err
	}
	current, err := queryRowOptional(conn, "SELECT interaction_id FROM member_interactions WHERE run_id=?1 AND closed_at IS NULL",
		[]any{runID}, func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		return runtimeSnapshot{}, err
	}
	unresolved, err := queryRows(conn,
		"SELECT input_id FROM member_input_operations WHERE run_id=?1 AND "+unresolvedInput+" ORDER BY input_id",
		[]any{runID}, func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		return runtimeSnapshot{}, err
	}
	return runtimeSnapshot{run: run, currentInteractionID: current, unresolvedInputIDs: unresolved}, nil
}

func memberRuntime(ctx context.Context, conn *dbConn, memberID int64) (opt[*runtimeSnapshot], error) {
	id, err := queryRowOptional(conn, "SELECT run_id FROM member_runs WHERE member_id=?1 ORDER BY run_id DESC LIMIT 1",
		[]any{memberID}, func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		return none[*runtimeSnapshot](), err
	}
	runID, set := id.get()
	if !set {
		return none[*runtimeSnapshot](), nil
	}
	snapshot, err := brokerRuntimeSnapshot(ctx, conn, runID)
	if err != nil {
		return none[*runtimeSnapshot](), err
	}
	return some(&snapshot), nil
}

// liveMemberRuns is the latest run of every active ordinary member whose run is not terminal, in
// member-ID order.
func liveMemberRuns(ctx context.Context, conn *dbConn) ([]runRecord, error) {
	runIDs, err := queryRows(conn,
		`SELECT (SELECT MAX(r.run_id) FROM member_runs r WHERE r.member_id=m.member_id)
             FROM members m JOIN fleets f ON f.fleet_id=m.fleet_id
             WHERE m.status='active' AND f.director_member_id IS NOT m.member_id
               AND EXISTS(SELECT 1 FROM member_runs r WHERE r.member_id=m.member_id)
             ORDER BY m.member_id`,
		nil, func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		return nil, err
	}
	runs := make([]runRecord, 0, len(runIDs))
	for _, runID := range runIDs {
		run, err := getRun(ctx, conn, runID)
		if err != nil {
			return nil, err
		}
		if !run.status.terminal() {
			runs = append(runs, run)
		}
	}
	return runs, nil
}

// runnerDiagnosticEntry is one deduplicated runner diagnostic: its source, sanitized message,
// first and last times and repeat count.
type runnerDiagnosticEntry struct {
	source  runnerDiagnosticSource
	message string
	firstAt string
	lastAt  string
	count   int64
}

const (
	runnerDiagnosticRowsPerRun  = 256
	latestRunnerDiagnosticCount = 5
)

func upsertRunnerDiagnostic(conn *dbConn, runID int64, entry runnerDiagnosticEntry) error {
	_, err := conn.execute(
		`INSERT INTO member_runner_diagnostics(run_id,source,message,first_at,last_at,count)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(run_id,source,message) DO UPDATE SET last_at=excluded.last_at,count=count+excluded.count`,
		runID, entry.source.asStr(), entry.message, entry.firstAt, entry.lastAt, entry.count)
	return err
}

// writeRunnerDiagnostics stores entries, one overflow row stamped at for dropped entries, and
// prunes the run to its newest rows.
func writeRunnerDiagnostics(conn *dbConn, runID int64, entries []runnerDiagnosticEntry, dropped uint64, at string) error {
	for _, entry := range entries {
		if err := upsertRunnerDiagnostic(conn, runID, entry); err != nil {
			return err
		}
	}
	if dropped > 0 {
		overflow := runnerDiagnosticEntry{
			source:  runnerDiagnosticSourceEventWriter,
			message: fmt.Sprintf("diagnostic buffer overflow: %d entries dropped", dropped),
			firstAt: at,
			lastAt:  at,
			count:   1,
		}
		if err := upsertRunnerDiagnostic(conn, runID, overflow); err != nil {
			return err
		}
	}
	_, err := conn.execute(
		`DELETE FROM member_runner_diagnostics WHERE run_id=?1 AND diagnostic_id NOT IN (
             SELECT diagnostic_id FROM member_runner_diagnostics WHERE run_id=?1
             ORDER BY last_at DESC, diagnostic_id DESC LIMIT ?2)`,
		runID, int64(runnerDiagnosticRowsPerRun))
	return err
}

// persistRunnerDiagnostics writes runner diagnostics in one ownership-validated immediate
// transaction of the live owner.
func persistRunnerDiagnostics(ctx context.Context, conn *dbConn, owner runOwner, entries []runnerDiagnosticEntry, dropped uint64, at string) error {
	_, err := ownedTransaction(ctx, conn, owner, func(_ context.Context, conn *dbConn, run runRecord) (struct{}, error) {
		return struct{}{}, writeRunnerDiagnostics(conn, run.runID, entries, dropped, at)
	})
	return err
}

// latestRunnerDiagnostics is the run's newest runner diagnostics, newest last_at then
// diagnostic_id first.
func latestRunnerDiagnostics(_ context.Context, conn *dbConn, runID int64) ([]runnerDiagnosticEntry, error) {
	return queryRows(conn,
		`SELECT source,message,first_at,last_at,count FROM member_runner_diagnostics
         WHERE run_id=?1 ORDER BY last_at DESC, diagnostic_id DESC LIMIT ?2`,
		[]any{runID, int64(latestRunnerDiagnosticCount)}, func(row sqlRow) (runnerDiagnosticEntry, error) {
			var entry runnerDiagnosticEntry
			var err error
			if entry.source, err = rowStoredEnum(row, 0, runnerDiagnosticSourceTryFrom); err != nil {
				return runnerDiagnosticEntry{}, err
			}
			if entry.message, err = row.text(1); err != nil {
				return runnerDiagnosticEntry{}, err
			}
			if entry.firstAt, err = row.text(2); err != nil {
				return runnerDiagnosticEntry{}, err
			}
			if entry.lastAt, err = row.text(3); err != nil {
				return runnerDiagnosticEntry{}, err
			}
			if entry.count, err = row.int64(4); err != nil {
				return runnerDiagnosticEntry{}, err
			}
			return entry, nil
		})
}

func (c stopCause) isCheckStop() bool {
	switch c {
	case stopCauseInteractionCandidate, stopCauseInteractionOpen, stopCauseDeadlineExpired,
		stopCauseLiteralMismatch, stopCauseSubmitUnconfirmed:
		return true
	case stopCauseShortWrite, stopCauseWriteFailed, stopCauseRunnerStopped:
		return false
	default:
		panic("invalid stopCause")
	}
}

func (s runStatus) terminal() bool {
	switch s {
	case runStatusExited, runStatusFailed, runStatusLost:
		return true
	case runStatusStarting, runStatusPrepared, runStatusRunning, runStatusDegraded, runStatusRecovering:
		return false
	default:
		panic("invalid runStatus")
	}
}

type newRun struct {
	memberID             int64
	fleetID              int64
	directorMemberID     int64
	backend              string
	databaseIdentityJSON jsontext.Value
	launchNonce          string
	runtimePath          string
	requestedMode        opt[string]
	resolvedMode         string
	modeSource           modeSource
	permissionArgvJSON   jsontext.Value
	nativeVersion        opt[string]
	coverage             coverage
	ownerPID             int64
	ownerStartIdentity   string
}

type runRecord struct {
	runID                  int64
	memberID               int64
	fleetID                int64
	directorMemberID       int64
	backend                string
	databaseIdentityJSON   jsontext.Value
	launchNonce            string
	runtimePath            string
	requestedMode          opt[string]
	resolvedMode           string
	modeSource             modeSource
	permissionArgvJSON     jsontext.Value
	coverage               coverage
	status                 runStatus
	nativeVersion          opt[string]
	ownerEpoch             int64
	ownerKind              ownerKind
	ownerPID               int64
	ownerStartIdentity     string
	cleanupState           cleanupState
	runtimeDirIdentityJSON jsontext.Value
	runLockIdentityJSON    jsontext.Value
	runnerPID              opt[int64]
	runnerStartIdentity    opt[string]
	childPID               opt[int64]
	childStartIdentity     opt[string]
	startedAt              opt[string]
	lastHealthAt           opt[string]
	endedAt                opt[string]
	exitCode               opt[int64]
	cleanupDiagnosticsJSON jsontext.Value
	recoveryOriginStatus   opt[runStatus]
}

func (r runRecord) owner() runOwner {
	return runOwner{
		runID:                r.runID,
		databaseIdentityJSON: r.databaseIdentityJSON,
		ownerEpoch:           r.ownerEpoch,
		ownerKind:            r.ownerKind,
		ownerPID:             r.ownerPID,
		ownerStartIdentity:   r.ownerStartIdentity,
	}
}

func (r runRecord) equal(other runRecord) bool {
	return r.runID == other.runID && r.memberID == other.memberID && r.fleetID == other.fleetID &&
		r.directorMemberID == other.directorMemberID && r.backend == other.backend &&
		jsonEqual(r.databaseIdentityJSON, other.databaseIdentityJSON) && r.launchNonce == other.launchNonce &&
		r.runtimePath == other.runtimePath && r.requestedMode == other.requestedMode &&
		r.resolvedMode == other.resolvedMode && r.modeSource == other.modeSource &&
		jsonEqual(r.permissionArgvJSON, other.permissionArgvJSON) && r.coverage == other.coverage &&
		r.status == other.status && r.nativeVersion == other.nativeVersion && r.ownerEpoch == other.ownerEpoch &&
		r.ownerKind == other.ownerKind && r.ownerPID == other.ownerPID &&
		r.ownerStartIdentity == other.ownerStartIdentity && r.cleanupState == other.cleanupState &&
		jsonOptionalEqual(r.runtimeDirIdentityJSON, other.runtimeDirIdentityJSON) &&
		jsonOptionalEqual(r.runLockIdentityJSON, other.runLockIdentityJSON) &&
		r.runnerPID == other.runnerPID && r.runnerStartIdentity == other.runnerStartIdentity &&
		r.childPID == other.childPID && r.childStartIdentity == other.childStartIdentity &&
		r.startedAt == other.startedAt && r.lastHealthAt == other.lastHealthAt && r.endedAt == other.endedAt &&
		r.exitCode == other.exitCode &&
		jsonOptionalEqual(r.cleanupDiagnosticsJSON, other.cleanupDiagnosticsJSON) &&
		r.recoveryOriginStatus == other.recoveryOriginStatus
}

func (r runRecord) ownerProcess() processIdentity {
	return processIdentity{pid: r.ownerPID, startIdentity: r.ownerStartIdentity}
}

func (r runRecord) childProcess() (opt[processIdentity], error) {
	pid, hasPID := r.childPID.get()
	start, hasStart := r.childStartIdentity.get()
	switch {
	case !hasPID && !hasStart:
		return none[processIdentity](), nil
	case hasPID && hasStart:
		return some(processIdentity{pid: pid, startIdentity: start}), nil
	default:
		return none[processIdentity](), appError("recorded child generation is incomplete")
	}
}

func (r runRecord) isSettled() bool {
	return r.status.terminal() && r.cleanupState == cleanupStateComplete
}

type runOwner struct {
	runID                int64
	databaseIdentityJSON jsontext.Value
	ownerEpoch           int64
	ownerKind            ownerKind
	ownerPID             int64
	ownerStartIdentity   string
}

func (t observedTransition) equal(other observedTransition) bool {
	return t.RunID == other.RunID && t.EventSeq == other.EventSeq && t.EpisodeSeq == other.EpisodeSeq &&
		t.Kind == other.Kind && t.Classification == other.Classification && t.ObservedAt == other.ObservedAt &&
		jsonEqual(t.EvidenceJSON, other.EvidenceJSON) && t.CloseReason == other.CloseReason &&
		jsonOptionalEqual(t.CauseJSON, other.CauseJSON)
}

func (o runOwner) equal(other runOwner) bool {
	return o.runID == other.runID && jsonEqual(o.databaseIdentityJSON, other.databaseIdentityJSON) &&
		o.ownerEpoch == other.ownerEpoch && o.ownerKind == other.ownerKind &&
		o.ownerPID == other.ownerPID && o.ownerStartIdentity == other.ownerStartIdentity
}

type runtimeSnapshot struct {
	run                  runRecord
	currentInteractionID opt[int64]
	unresolvedInputIDs   []int64
}

type interactionRecord struct {
	interactionID  int64
	runID          int64
	episodeSeq     int64
	classification interactionClass
	evidenceDigest string
	openedAt       string
	evidenceJSON   jsontext.Value
	closedAt       opt[string]
	closeReason    opt[interactionCloseReason]
}

type observedTransition struct {
	RunID          int64                       `json:"run_id"`
	EventSeq       int64                       `json:"event_seq"`
	EpisodeSeq     opt[int64]                  `json:"episode_seq"`
	Kind           runtimeEventKind            `json:"kind"`
	Classification opt[interactionClass]       `json:"classification"`
	ObservedAt     string                      `json:"observed_at"`
	EvidenceJSON   jsontext.Value              `json:"evidence_json"`
	CloseReason    opt[interactionCloseReason] `json:"close_reason"`
	CauseJSON      jsontext.Value              `json:"cause_json"`
}

type persistenceMode uint8

const (
	persistenceModeLive persistenceMode = iota + 1
	persistenceModeRecovery
)

type runtimeEventRecord struct {
	eventID           int64
	runID             int64
	eventSeq          int64
	interactionID     opt[int64]
	kind              runtimeEventKind
	observedAt        string
	payloadJSON       jsontext.Value
	notificationState notificationState
	messageID         opt[int64]
	archiveReason     opt[archiveReason]
}

type runtimeEventView struct {
	event       runtimeEventRecord
	run         runtimeSnapshot
	interaction opt[*interactionRecord]
}

type newInput struct {
	operationKey  string
	kind          inputKind
	payloadDigest string
}

type inputOperation struct {
	inputID        int64
	runID          int64
	operationKey   string
	kind           inputKind
	payloadDigest  string
	state          inputState
	createdAt      string
	updatedAt      string
	disposition    opt[inputDisposition]
	evidenceRef    opt[string]
	deliveryMethod opt[deliveryMethod]
	deliveryReason opt[string]
	deferredAt     opt[string]
	deferReason    opt[string]
	stopCause      opt[stopCause]
}

type deliveryProjection struct {
	Status deliveryStatus      `json:"status"`
	Method opt[deliveryMethod] `json:"method"`
	Reason opt[string]         `json:"reason"`
}

func (d deliveryProjection) toJSON() jsonObject {
	method := none[string]()
	if value, set := d.Method.get(); set {
		method = some(value.asStr())
	}
	return jsonObject{{"status", d.Status.asStr()}, {"method", method}, {"reason", d.Reason}}
}

type permissionReceiptIdentity struct {
	databaseIdentityJSON jsontext.Value
	runID                int64
}

func permissionReceiptIdentityForRun(run runRecord) permissionReceiptIdentity {
	return permissionReceiptIdentity{databaseIdentityJSON: run.databaseIdentityJSON, runID: run.runID}
}

func (i inputOperation) sameIdentity(other inputOperation) bool {
	return i.inputID == other.inputID && i.runID == other.runID && i.operationKey == other.operationKey &&
		i.kind == other.kind && i.payloadDigest == other.payloadDigest && i.createdAt == other.createdAt
}
