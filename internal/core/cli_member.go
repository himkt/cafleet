package core

import (
	"context"
	"time"
)

// prepareInput resolves an exec command's native command line before the command's deadline
// starts.
func prepareInput(command memberCommand) (memberCommand, error) {
	exec, ok := command.(memberCommandExec)
	if !ok {
		return command, nil
	}
	resolved, err := resolveCommandBody(exec.body.command, exec.body.file)
	if err != nil {
		return nil, err
	}
	exec.body = execBodyArgs{command: some(resolved), file: none[string]()}
	return exec, nil
}

func memberRun(ctx context.Context, conn *dbConn, command memberCommand) error {
	switch command := command.(type) {
	case memberCommandList:
		return memberList(ctx, conn, command.fleetID, command.json)
	case memberCommandShow:
		return memberShow(ctx, conn, command.memberID, command.json)
	case memberCommandCreate, memberCommandDelete, memberCommandPrompt,
		memberCommandExec, memberCommandPress, memberCommandCapture:
		return commandNotYetPorted()
	default:
		panic("invalid memberCommand")
	}
}

// memberShow prints a member; a member with a runtime adds the runner's observation probe and its
// latest runner diagnostics.
func memberShow(ctx context.Context, conn *dbConn, memberID int64, json bool) error {
	member, err := activeMember(ctx, conn, memberID)
	if err != nil {
		return err
	}
	value := outputJSONMember(member)
	snapshot, set := member.runtime.get()
	if !set {
		return emit(json, value, func() string { return formatMemberDetail(member) })
	}
	diagnostics, err := latestRunnerDiagnostics(ctx, conn, snapshot.run.runID)
	if err != nil {
		return err
	}
	probe := probeObservation(snapshot.run, systemClock{})
	value = value.with("runtime", showRuntime(outputJSONRuntimeSnapshot(*snapshot), probe, diagnostics))
	return emit(json, value, func() string { return formatMemberDetail(member) + observationSuffix(probe) })
}

const observationProbeDeadline = time.Second

//sumtype:decl
type observationProbe interface {
	isObservationProbe()
	asStr() string
}

type observationProbeSkipped struct{}

type observationProbeAnswered struct{ health opt[observationHealth] }

type observationProbeFailed struct{ message string }

func (observationProbeSkipped) isObservationProbe()  {}
func (observationProbeAnswered) isObservationProbe() {}
func (observationProbeFailed) isObservationProbe()   {}

func (observationProbeSkipped) asStr() string  { return "skipped" }
func (observationProbeAnswered) asStr() string { return "answered" }
func (observationProbeFailed) asStr() string   { return "failed" }

func probeObservation(run runRecord, c clock) observationProbe {
	if run.status != runStatusRunning && run.status != runStatusDegraded {
		return observationProbeSkipped{}
	}
	snapshot, err := probeHealth(run, c.now().Add(observationProbeDeadline), c)
	if err != nil {
		return observationProbeFailed{message: cafleetErrorOf(err).message()}
	}
	return observationProbeAnswered{health: snapshot.Observation}
}

// showRuntime is a member's runtime JSON with the probe outcome and the run's latest runner
// diagnostics appended.
func showRuntime(runtime jsonObject, probe observationProbe, diagnostics []runnerDiagnosticEntry) jsonObject {
	var observation, observationError any
	switch probe := probe.(type) {
	case observationProbeAnswered:
		if health, set := probe.health.get(); set {
			observation = observationJSON(health)
		}
	case observationProbeFailed:
		observationError = probe.message
	case observationProbeSkipped:
	default:
		panic("invalid observationProbe")
	}
	rows := make([]jsonObject, 0, len(diagnostics))
	for _, entry := range diagnostics {
		rows = append(rows, diagnosticJSON(entry))
	}
	return runtime.
		with("observation_probe", probe.asStr()).
		with("observation", observation).
		with("observation_error", observationError).
		with("diagnostics", rows)
}

func observationJSON(health observationHealth) jsonObject {
	return jsonObject{
		{"visibility", health.Visibility.asStr()},
		{"eligibility", health.Eligibility.rendering()},
		{"degraded", health.Degraded},
		{"diagnostic", health.Diagnostic},
		{"writer_diagnostic", health.WriterDiagnostic},
		{"pending_count", health.PendingCount},
		{"gap_count", health.GapCount},
		{"gap_open_since", health.GapOpenSince},
	}
}

func diagnosticJSON(entry runnerDiagnosticEntry) jsonObject {
	return jsonObject{
		{"source", entry.source.asStr()},
		{"message", entry.message},
		{"first_at", entry.firstAt},
		{"last_at", entry.lastAt},
		{"count", entry.count},
	}
}

func observationSuffix(probe observationProbe) string {
	switch probe := probe.(type) {
	case observationProbeSkipped:
		return " observation=-"
	case observationProbeFailed:
		return " observation=unavailable"
	case observationProbeAnswered:
		health, set := probe.health.get()
		if !set {
			return " observation=pending"
		}
		degraded := ""
		if health.Degraded {
			degraded = " degraded"
		}
		return " observation=" + health.Eligibility.rendering() + degraded
	default:
		panic("invalid observationProbe")
	}
}

func memberList(ctx context.Context, conn *dbConn, fleetID int64, json bool) error {
	members, err := listMembers(ctx, conn, fleetID)
	if err != nil {
		return err
	}
	payload := make([]jsonObject, 0, len(members))
	for _, member := range members {
		payload = append(payload, outputJSONMemberActivity(member))
	}
	return emit(json, payload, func() string { return formatMemberList(members) })
}
