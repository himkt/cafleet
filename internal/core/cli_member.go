package core

import (
	"context"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"
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

func memberRun(ctx context.Context, conn *dbConn, s settings, command memberCommand, d deadline) error {
	switch command := command.(type) {
	case memberCommandList:
		return memberList(ctx, conn, command.fleetID, command.json)
	case memberCommandShow:
		return memberShow(ctx, conn, command.memberID, command.json)
	case memberCommandPrompt:
		return cliMemberPrompt(ctx, conn, s, command.memberID, command.text, command.json, d)
	case memberCommandExec:
		text, resolved := command.body.command.get()
		if !resolved {
			panic("exec input was resolved before its deadline")
		}
		return cliMemberExec(ctx, conn, s, command.memberID, text, command.json, d)
	case memberCommandDelete:
		return cliMemberDelete(ctx, conn, s, command.memberID, command.json, d)
	case memberCommandCapture:
		return cliMemberCapture(ctx, conn, command.memberID, command.lines, command.ansi, command.json)
	case memberCommandCreate:
		return cliMemberCreate(ctx, conn, command.createArgs)
	case memberCommandPress:
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

func requirePane(member memberRecord, action string) (string, error) {
	pane, placed := member.placement.muxPaneID.get()
	if !placed {
		return "", appErrorf("member %d has no pane yet (pending placement) — nothing to %s.", member.memberID, action)
	}
	return pane, nil
}

func wrappedDeliveryRefusal(label string, memberID int64, result submissionResult) string {
	if result.Status == submissionStatusRejected && result.Stage == submissionStageZeroBytes {
		cause, caused := result.Reason.get()
		if !caused {
			panic("a pre-write rejection has its cause")
		}
		return fmt.Sprintf("%s was not dispatched to member %d: %s", label, memberID, cause)
	}
	var description string
	switch result.Status {
	case submissionStatusHeld:
		description = "held"
	case submissionStatusRejected:
		description = "rejected"
	case submissionStatusUncertain:
		description = "uncertain"
	case submissionStatusAccepted, submissionStatusQueued:
		panic("submitted and queued results are not refusals")
	default:
		panic("invalid submissionStatus")
	}
	reason := fmt.Sprintf("%s delivery to member %d is %s", label, memberID, description)
	if detail, set := result.Reason.get(); set {
		reason += ": " + detail
	}
	if delivery, delivered := result.Delivery.get(); delivered {
		if method, set := delivery.Method.get(); set {
			reason += " (method=" + method.asStr() + ")"
		}
	}
	return reason
}

func dispatchInput(ctx context.Context, conn *dbConn, s settings, member memberRecord, text string, kind inputKind, label string, d deadline) (opt[int64], deliveryProjection, error) {
	snapshot, recorded := member.runtime.get()
	if !recorded {
		panic("every ordinary member records its run")
	}
	run := snapshot.run
	random, err := randomHex32()
	if err != nil {
		return none[int64](), deliveryProjection{}, appError("cannot generate input operation key: " + cafleetErrorOf(err).message())
	}
	request := submitInput{
		OperationKey: fmt.Sprintf("%s:%d:%s", kind.asStr(), run.runID, random),
		InputKind:    kind,
		Literal:      text,
	}
	result, failure := submitWrappedInput(ctx, conn, s, run, member.placement, request, d)
	if failure != nil {
		return none[int64](), deliveryProjection{}, failure.asError()
	}
	if result.Status != submissionStatusAccepted && result.Status != submissionStatusQueued {
		scope := inputContext{
			runID:         some(run.runID),
			interactionID: snapshot.currentInteractionID,
			inputID:       result.InputID,
			operationKey:  some(request.OperationKey),
		}
		return none[int64](), deliveryProjection{}, scope.failure(appError(wrappedDeliveryRefusal(label, member.memberID, result))).asError()
	}
	delivery, delivered := result.Delivery.get()
	if !delivered {
		panic("validated submission replies carry their delivery")
	}
	return result.InputID, delivery, nil
}

type cliMemberDispatch struct {
	kind            inputKind
	command         string
	label           string
	directorRefusal string
}

type dispatchedInput struct {
	member   memberRecord
	pane     string
	inputID  opt[int64]
	delivery deliveryProjection
}

func dispatchToMember(ctx context.Context, conn *dbConn, s settings, memberID int64, text string, dispatch cliMemberDispatch, d deadline) (dispatchedInput, error) {
	member, err := withDatabaseBudget(ctx, conn, d, func(ctx context.Context, conn *dbConn) (memberRecord, error) {
		return activeMember(ctx, conn, memberID)
	})
	if err != nil {
		return dispatchedInput{}, err
	}
	if member.kind == memberKindDirector {
		return dispatchedInput{}, appError(dispatch.directorRefusal)
	}
	pane, err := requirePane(member, dispatch.command)
	if err != nil {
		return dispatchedInput{}, err
	}
	inputID, delivery, err := dispatchInput(ctx, conn, s, member, text, dispatch.kind, dispatch.label, d)
	if err != nil {
		return dispatchedInput{}, err
	}
	return dispatchedInput{member: member, pane: pane, inputID: inputID, delivery: delivery}, nil
}

func cliMemberPrompt(ctx context.Context, conn *dbConn, s settings, memberID int64, text string, json bool, d deadline) error {
	if !validInput(text) {
		return usageError("text must be one non-empty line without control characters.")
	}
	sent, err := dispatchToMember(ctx, conn, s, memberID, text, cliMemberDispatch{
		kind:            inputKindPrompt,
		command:         "prompt",
		label:           "prompt",
		directorRefusal: "cannot prompt the Director's own pane",
	}, d)
	if err != nil {
		return err
	}
	return emit(json, outputJSONMemberPrompt(memberID, sent.pane, text, sent.inputID, sent.delivery), func() string {
		return formatMemberPrompt(memberID, text, sent.member.name, sent.pane, sent.inputID, sent.delivery)
	})
}

func cliMemberExec(ctx context.Context, conn *dbConn, s settings, memberID int64, command string, json bool, d deadline) error {
	sent, err := dispatchToMember(ctx, conn, s, memberID, command, cliMemberDispatch{
		kind:            inputKindExec,
		command:         "exec",
		label:           "native command",
		directorRefusal: "cannot exec in the Director's own pane",
	}, d)
	if err != nil {
		return err
	}
	agent := sent.member.placement.codingAgent
	return emit(json, outputJSONMemberExec(memberID, sent.pane, agent, command, sent.inputID, sent.delivery), func() string {
		return formatMemberExec(memberID, sent.pane, sent.inputID, sent.delivery)
	})
}

func resolveCodingAgent(ctx context.Context, conn *dbConn, fleetID, directorID int64, explicit opt[string]) (codingAgent, error) {
	if name, set := explicit.get(); set {
		agent, known := codingAgentCodingAgent(name)
		if !known {
			panic("the parser accepts only bundled coding agents for --coding-agent")
		}
		return agent, nil
	}
	director, err := getMember(ctx, conn, directorID, fleetID)
	if err != nil {
		return 0, err
	}
	record, active := director.get()
	if !active {
		panic("the live fleet's root Director is an active member (INV2)")
	}
	agent, known := codingAgentCodingAgent(record.placement.codingAgent)
	if !known {
		return 0, invalidStoredValueError("member_placements.coding_agent", record.placement.codingAgent)
	}
	return agent, nil
}

// chosenSlot is the slot choice, the window list it was chosen from and the fleet's
// last_window_ordinal.
type chosenSlot struct {
	choice            slotChoice
	windows           []windowOccupancy
	lastWindowOrdinal int64
}

// chooseMemberSlot reads occupancy from the registry and each scanned window's live height,
// stopping at the first window with a free slot.
func chooseMemberSlot(ctx context.Context, conn *dbConn, mux multiplexer, fleetID int64, director placement) (chosenSlot, error) {
	directorPane, recorded := director.muxPaneID.get()
	if !recorded {
		panic("the root Director placement records its pane")
	}
	directorWindow := director.concreteWindowID()
	last, err := lastWindowOrdinal(ctx, conn, fleetID)
	if err != nil {
		return chosenSlot{}, err
	}
	occupied, err := concreteOccupancy(ctx, conn, fleetID)
	if err != nil {
		return chosenSlot{}, err
	}
	var directorMembers []string
	if len(occupied) > 0 && occupied[0].windowOrdinal == 0 {
		directorMembers = occupied[0].right
		occupied = occupied[1:]
	}
	height, muxErr := mux.windowHeight(directorWindow, []string{directorPane})
	if muxErr != nil {
		return chosenSlot{}, muxErr.cafleet()
	}
	windowName, named := director.muxWindowName.get()
	if !named {
		panic("a concrete placement records its window name")
	}
	windows := []windowOccupancy{{
		ordinal:      0,
		height:       height,
		windowID:     directorWindow,
		windowName:   windowName,
		right:        directorMembers,
		directorPane: some(directorPane),
	}}
	for _, window := range occupied {
		if _, free := chooseSlot(fleetID, windows, last).(slotChoiceExisting); free {
			break
		}
		height, muxErr := mux.windowHeight(window.muxWindowID, window.panes)
		if muxErr != nil {
			return chosenSlot{}, unreadableWindow(ctx, conn, fleetID, window, muxErr)
		}
		windows = append(windows, windowOccupancy{
			ordinal:    window.windowOrdinal,
			windowID:   window.muxWindowID,
			windowName: window.muxWindowName,
			height:     height,
			left:       window.left,
			right:      window.right,
		})
	}
	return chosenSlot{choice: chooseSlot(fleetID, windows, last), windows: windows, lastWindowOrdinal: last}, nil
}

func unreadableWindow(ctx context.Context, conn *dbConn, fleetID int64, window occupiedWindow, cause *multiplexerError) error {
	ids, err := windowMemberIDs(ctx, conn, fleetID, window.windowOrdinal)
	if err != nil {
		return err
	}
	members := make([]string, 0, len(ids))
	for _, id := range ids {
		members = append(members, strconv.FormatInt(id, 10))
	}
	return appErrorf("window %s (ordinal %d) of fleet %d cannot be read: %s. Its active members %s may have lost their panes; run 'cafleet member delete MEMBER_ID' for each, then retry.",
		window.muxWindowName, window.windowOrdinal, fleetID, strings.Join(lines(cause.Error()), "; "), strings.Join(members, ", "))
}

type slotTarget struct {
	pane                      paneTarget
	windowOrdinal             int64
	columnSide                columnSide
	windowName                string
	advancesWindowOrdinalFrom opt[int64]
}

func chosenSlotTarget(slot chosenSlot, session string) slotTarget {
	switch choice := slot.choice.(type) {
	case slotChoiceExisting:
		index := slices.IndexFunc(slot.windows, func(window windowOccupancy) bool { return window.ordinal == choice.ordinal })
		if index < 0 {
			panic("the chosen slot's window was scanned")
		}
		window := slot.windows[index]
		var columnPanes, otherPanes []string
		directorPane, hasDirector := window.directorPane.get()
		switch {
		case choice.side == columnSideRight && hasDirector:
			columnPanes, otherPanes = slices.Clone(window.right), []string{directorPane}
		case choice.side == columnSideRight:
			columnPanes, otherPanes = slices.Clone(window.right), slices.Clone(window.left)
		default:
			columnPanes, otherPanes = slices.Clone(window.left), slices.Clone(window.right)
		}
		return slotTarget{
			pane:          columnTarget{windowID: choice.windowID, side: choice.side, columnPanes: columnPanes, otherPanes: otherPanes},
			windowOrdinal: choice.ordinal,
			columnSide:    choice.side,
			windowName:    choice.windowName,
		}
	case slotChoiceNewWindow:
		return slotTarget{
			pane:                      paneTargetNewWindow{session: session, windowName: choice.windowName},
			windowOrdinal:             choice.ordinal,
			columnSide:                columnSideLeft,
			windowName:                choice.windowName,
			advancesWindowOrdinalFrom: some(slot.lastWindowOrdinal),
		}
	default:
		panic("invalid slotChoice")
	}
}

func cliMemberCreate(ctx context.Context, conn *dbConn, args createArgs) error {
	return createWithLayoutWait(ctx, conn, args, layoutLockWait)
}

type hermesProbe struct {
	home string
	err  error
}

func createWithLayoutWait(ctx context.Context, conn *dbConn, args createArgs, layoutWait time.Duration) error {
	fleetID, name := args.fleetID, args.name
	fleet, err := requireFleet(ctx, conn, fleetID)
	if err != nil {
		return err
	}
	if fleet.deletedAt.set {
		return appErrorf("fleet %d is deleted", fleetID)
	}
	directorID := fleet.directorMemberID
	backend, err := resolveCodingAgent(ctx, conn, fleetID, directorID, args.codingAgent)
	if err != nil {
		return err
	}
	agentName := backend.name()
	if err := backend.validateModel(args.model); err != nil {
		return err
	}
	if err := backend.validateEffort(args.effort); err != nil {
		return err
	}
	policy, err := backend.resolvePolicy(args.permissionMode)
	if err != nil {
		return err
	}
	catalog := profileCatalogBundled()
	promptBody, err := resolveBody(args.body.prompt, args.body.file)
	if err != nil {
		return err
	}
	if err := backend.validatePrompt(promptBody); err != nil {
		return err
	}
	databaseFile := conn.path()
	if databaseFile == "" {
		panic("the registry is a file database")
	}
	layout, acquired, err := layoutLockAcquireWithin("sqlite:///"+databaseFile, fleetID, deadlineUntil(time.Now().Add(layoutWait)))
	if err != nil {
		return err
	}
	if !acquired {
		return appErrorf("fleet %d pane layout remained busy for 60 seconds; member was not created.", fleetID)
	}
	defer layout.close()
	span, err := deferTerminationBegin()
	if err != nil {
		return err
	}
	defer span.close()
	until := time.Now().Add(10 * time.Second)

	director, err := directorPlacement(ctx, conn, fleetID)
	if err != nil {
		return err
	}
	mux, err := muxForPlacement(director, none[deadline]())
	if err != nil {
		return err
	}
	executable, err := backend.ensureAvailable(systemProbe{}, policy.resolvedMode)
	if err != nil {
		return err
	}
	slot, err := chooseMemberSlot(ctx, conn, mux.bounded(deadlineUntil(until)), fleetID, director)
	if err != nil {
		return err
	}
	target := chosenSlotTarget(slot, director.muxSession)
	var hermes opt[hermesCli]
	if backend == codingAgentHermes {
		home, err := systemProbe{}.homeDir()
		if err != nil {
			return err
		}
		base, err := backend.configDir(systemProbe{}.envVar, home)
		if err != nil {
			return err
		}
		hermes = some(newHermesCli(executable, base))
	}
	preflightStart := time.Now()
	probeDeadline := minTime(until, preflightStart.Add(2*time.Second))
	configPathDeadline := minTime(until, preflightStart.Add(5*time.Second))
	var homeHandle *scopedJoinHandle[hermesProbe]
	if cli, set := hermes.get(); set {
		homeHandle = scopedSpawn(func() hermesProbe {
			home, err := effectiveHome(cli, configPathDeadline)
			return hermesProbe{home: home, err: err}
		})
	}
	version, probeErr := backend.probeNativeVersion(executable, probeDeadline)
	hermesHome := none[string]()
	if homeHandle != nil {
		probed := joined(homeHandle)
		if probed.err != nil {
			return probed.err
		}
		hermesHome = some(probed.home)
	}
	native := codingAgentNativeLaunchIdentity(backend, version, probeErr, catalog)
	if warning, set := native.warning.get(); set {
		fmt.Fprintln(os.Stderr, warning)
	}
	identity, err := databaseIdentity(ctx, conn)
	if err != nil {
		return err
	}
	canonical, present := jsonLookup(identity, "canonical_path")
	canonicalPath, isString := jsonString(canonical)
	if !present || !isString {
		panic("canonical database path")
	}
	promptDatabaseURL, err := spawnDatabaseURL(canonicalPath)
	if err != nil {
		return err
	}
	citesDatabaseURL, err := promptCitesDatabaseURL(promptBody)
	if err != nil {
		return err
	}
	if promptDatabaseURL != defaultDatabase && !citesDatabaseURL {
		return appError("the fleet database is not the default; the spawn prompt must include {database_url} so the member passes --database-url")
	}
	forwardedConfig, err := backend.forwardedConfig(systemProbe{}.envVar)
	if err != nil {
		return err
	}
	nextRun, err := queryRow(conn, "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='member_runs'),0)+1", nil,
		func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		return err
	}
	if _, err := runtimePathsForRun(identity, nextRun); err != nil {
		return err
	}
	cwd, err := os.Getwd()
	if err != nil {
		return appError("cannot resolve the working directory for pane spawn: " + ioErrorText(err))
	}
	size := terminalSize{rows: 24, cols: 80}
	if _, err := getTermios(int(os.Stdin.Fd())); err == nil {
		if size, err = runtimeServiceTerminalSize(os.Stdin); err != nil {
			return err
		}
	}
	launchNonce, err := randomHex32()
	if err != nil {
		return appError("launch nonce unavailable: " + cafleetErrorOf(err).message())
	}
	pending := pendingMemberSpec{
		fleetID:     fleetID,
		name:        name,
		description: args.description,
		placement: newPlacement{
			backend:       mux.name(),
			muxSession:    director.muxSession,
			windowOrdinal: target.windowOrdinal,
			columnSide:    target.columnSide,
			codingAgent:   agentName,
			muxServer:     director.muxServer,
			pane:          none[placementPane](),
		},
		windowName:                target.windowName,
		advancesWindowOrdinalFrom: target.advancesWindowOrdinalFrom,
	}
	creator, err := processIdentityCurrent()
	if err != nil {
		return err
	}
	settings := runSettings{
		databaseIdentityJSON: identity,
		creator:              creator,
		launchNonce:          launchNonce,
		requestedMode:        policy.requestedMode,
		resolvedMode:         policy.resolvedMode,
		modeSource:           policy.modeSource,
		permissionArgvJSON:   jsonValue(policy.permissionArgv),
		nativeVersion:        native.nativeVersion,
		coverage:             native.coverage,
	}
	if !utf8.ValidString(cwd) {
		return appError("cannot resolve the working directory for pane spawn: " + lossyUTF8([]byte(cwd)) + " is not valid UTF-8")
	}
	host := newNativeLaunchHost(mux, target.pane, cwd, systemClock{})
	created, err := createRegistered(ctx, conn, pending, settings,
		func(ctx context.Context, conn *dbConn, pending pendingRegistration) (*runArtifacts, spawnSpec, error) {
			artifacts, err := runArtifactsCreateWithDeadline(ctx, conn, pending.run.runID, until)
			if err != nil {
				return nil, spawnSpec{}, err
			}
			if home, set := hermesHome.get(); set {
				if err := setNativeSession(ctx, conn, pending.run.runID, nativeSession{hermesHome: home, sessionID: none[string]()}); err != nil {
					return nil, spawnSpec{}, err
				}
			}
			prompt, err := substituteSpawnPlaceholders(promptBody, fleetID, pending.member.memberID, directorID, agentName, promptDatabaseURL)
			if err != nil {
				return nil, spawnSpec{}, err
			}
			argv, err := backend.launchArgv(executable, prompt, memberPaneName(pending.member.memberID, name), cwd, args.model, args.effort, &policy)
			if err != nil {
				return nil, spawnSpec{}, err
			}
			var environment [][2]string
			if forwarded, set := forwardedConfig.get(); set {
				environment = [][2]string{forwarded}
			}
			return artifacts, spawnSpec{argv: argv, cwd: cwd, environment: environment, size: size}, nil
		}, host, until)
	closeErr := host.close()
	if err != nil {
		return mergeCleanup(err, closeErr)
	}
	if closeErr != nil {
		return closeErr
	}
	runtime, err := brokerRuntimeSnapshot(ctx, conn, created.started.Identity.RunID)
	if err != nil {
		return err
	}
	result := jsonObject{
		{"member_id", created.run.memberID},
		{"name", name},
		{"registered_at", created.member.registeredAt},
		{"placement", outputJSONPlacement(created.placement)},
		{"runtime", outputJSONRuntimeSnapshot(runtime)},
	}
	return emit(args.json, result, func() string {
		return formatMember(created.member, created.placement) + "\n" + formatRuntimeSnapshot(runtime)
	})
}

func minTime(a, b time.Time) time.Time {
	if b.Before(a) {
		return b
	}
	return a
}

//sumtype:decl
type deleteAction interface{ isDeleteAction() }

type deleteActionDeregistered struct{ paneStatus string }

type deleteActionClose struct{}

func (deleteActionDeregistered) isDeleteAction() {}
func (deleteActionClose) isDeleteAction()        {}

func lockMemberDeletion(ctx context.Context, conn *dbConn, s settings, memberID int64, d deadline) (*lockedDeletion[memberRecord, deleteAction], error) {
	return lockDeletion(ctx, conn, s, d,
		func(ctx context.Context, conn *dbConn) (memberRecord, []memberPlacement, error) {
			member, err := activeMember(ctx, conn, memberID)
			if err != nil {
				return memberRecord{}, nil, err
			}
			var targets []memberPlacement
			if member.placement.muxPaneID.set {
				targets = []memberPlacement{{memberID: memberID, placement: member.placement}}
			}
			return member, targets, nil
		},
		func(ctx context.Context, conn *dbConn, snapshot *memberRecord, _ []*paneLock) (deleteAction, bool, error) {
			type revalidated struct {
				action deleteAction
				valid  bool
			}
			result, err := inTransaction(conn, transactionImmediate, func() (revalidated, error) {
				current, err := activeMember(ctx, conn, memberID)
				if err != nil {
					return revalidated{}, err
				}
				if !memberDeletionMatches(*snapshot, current) {
					return revalidated{}, nil
				}
				if current.kind == memberKindDirector {
					return revalidated{}, appError("cannot deregister the root Director; use 'cafleet fleet delete' instead")
				}
				if current.placement.muxPaneID.set {
					return revalidated{action: deleteActionClose{}, valid: true}, nil
				}
				changed, err := deregisterMemberRows(ctx, conn, memberID)
				if err != nil {
					return revalidated{}, err
				}
				if !changed {
					panic("the revalidated member remains active")
				}
				return revalidated{action: deleteActionDeregistered{paneStatus: pendingPaneStatus}, valid: true}, nil
			})
			return result.action, result.valid, err
		},
		func(int64) error {
			return appErrorf("member %d's pane input remained busy for 5 seconds; member was not deleted.", memberID)
		},
	)
}

func cliMemberDelete(ctx context.Context, conn *dbConn, s settings, memberID int64, json bool, d deadline) error {
	paneStatus, err := deleteMemberUnderLock(ctx, conn, s, memberID, d)
	if err != nil {
		return err
	}
	return emit(json, jsonObject{{"member_id", memberID}, {"pane_status", paneStatus}}, func() string {
		return formatMemberDelete(memberID, paneStatus)
	})
}

// deleteMemberUnderLock ends or deregisters the member and releases its pane lock before the
// outcome is printed.
func deleteMemberUnderLock(ctx context.Context, conn *dbConn, s settings, memberID int64, d deadline) (string, error) {
	locked, err := lockMemberDeletion(ctx, conn, s, memberID, d)
	if err != nil {
		return "", err
	}
	defer locked.close()
	switch action := locked.value.(type) {
	case deleteActionDeregistered:
		return action.paneStatus, nil
	case deleteActionClose:
		mux, err := muxForFleet(ctx, conn, locked.snapshot.fleetID, none[deadline]())
		if err != nil {
			return "", err
		}
		return closeMemberWithin(ctx, conn, s, memberID, locked, mux, d, layoutLockWait)
	default:
		panic("invalid deleteAction")
	}
}

// closeMemberWithin acquires the fleet layout lock within layoutWait after the held pane lock,
// then ends the member under both.
func closeMemberWithin(ctx context.Context, conn *dbConn, s settings, memberID int64, locked *lockedDeletion[memberRecord, deleteAction], mux multiplexer, d deadline, layoutWait time.Duration) (string, error) {
	if _, closing := locked.value.(deleteActionClose); !closing {
		panic("pane close requires a concrete input-locked placement")
	}
	if locked.snapshot.memberID != memberID {
		panic("the locked deletion snapshot is the deleted member")
	}
	layout, d, err := acquireDeletionLayout(s, locked.snapshot.fleetID, d, layoutWait, "member")
	if err != nil {
		return "", err
	}
	defer layout.close()
	snapshot, held, err := locked.snapshotAndPaneLock(s.databaseURL, locked.snapshot.placement)
	if err != nil {
		return "", err
	}
	return endMember(ctx, conn, s, snapshot, held, layout, newMultiplexerDeletionHost(mux), &d,
		func(context.Context, *dbConn) error { return nil })
}

func cliMemberCapture(ctx context.Context, conn *dbConn, memberID, lines int64, ansi, json bool) error {
	member, err := activeMember(ctx, conn, memberID)
	if err != nil {
		return err
	}
	paneID, err := requirePane(member, "capture")
	if err != nil {
		return err
	}
	mux, err := muxForFleet(ctx, conn, member.fleetID, none[deadline]())
	if err != nil {
		return err
	}
	raw, muxErr := mux.capturePane(paneID, lines)
	if muxErr != nil {
		return appError("capture failed: " + muxErr.Error())
	}
	capture := memberCapture{memberID: memberID, paneID: paneID, lines: lines, snapshot: captureSnapshotFromRaw(raw, ansi, time.Now())}
	return writeMemberCapture(os.Stdout, capture, json)
}

const defaultDatabase = "default"

// spawnDatabaseURL is the {database_url} value: default when the fleet database is the default
// database, otherwise its canonical URL.
func spawnDatabaseURL(canonicalPath string) (string, error) {
	defaultURL, err := defaultDatabaseURL()
	if err != nil {
		return "", err
	}
	defaultPath, err := databasePath(defaultURL)
	if err != nil {
		return "", err
	}
	if resolved, err := filepath.EvalSymlinks(defaultPath); err == nil {
		if absolute, err := filepath.Abs(resolved); err == nil && absolute == canonicalPath {
			return defaultDatabase, nil
		}
	}
	return "sqlite:///" + canonicalPath, nil
}

const braceHint = "Double literal braces ({{, }}) to keep them as text."

var spawnPlaceholders = []string{"fleet_id", "member_id", "director_member_id", "coding_agent", "database_url"}

// substituteSpawnPlaceholders is the spawn-placeholder mini-formatter of `member create`: Python
// brace grammar with {{ / }} escapes, the five placeholders, and the unknown-vs-malformed usage
// errors.
func substituteSpawnPlaceholders(body string, fleetID, memberID, directorMemberID int64, codingAgent, databaseURL string) (string, error) {
	return renderSpawnPlaceholders(body, func(key string) string {
		switch key {
		case "fleet_id":
			return strconv.FormatInt(fleetID, 10)
		case "member_id":
			return strconv.FormatInt(memberID, 10)
		case "director_member_id":
			return strconv.FormatInt(directorMemberID, 10)
		case "coding_agent":
			return codingAgent
		case "database_url":
			return databaseURL
		default:
			panic(key + " is a spawn placeholder")
		}
	})
}

func promptCitesDatabaseURL(body string) (bool, error) {
	cited := false
	_, err := renderSpawnPlaceholders(body, func(key string) string {
		cited = cited || key == "database_url"
		return ""
	})
	return cited, err
}

func renderSpawnPlaceholders(body string, value func(string) string) (string, error) {
	var out strings.Builder
	out.Grow(len(body))
	chars := []rune(body)
	for index := 0; index < len(chars); index++ {
		switch c := chars[index]; c {
		case '{':
			if index+1 < len(chars) && chars[index+1] == '{' {
				index++
				out.WriteRune('{')
				continue
			}
			var key strings.Builder
			for {
				index++
				if index >= len(chars) {
					return "", malformed("expected '}' before end of string")
				}
				if chars[index] == '}' {
					break
				}
				if chars[index] == '{' {
					return "", malformed("unexpected '{' in placeholder name")
				}
				key.WriteRune(chars[index])
			}
			if !slices.Contains(spawnPlaceholders, key.String()) {
				return "", usageError("Unknown placeholder '" + key.String() + "' in custom prompt. Supported placeholders: {fleet_id}, {member_id}, {director_member_id}, {coding_agent}, {database_url}. " + braceHint)
			}
			out.WriteString(value(key.String()))
		case '}':
			if index+1 < len(chars) && chars[index+1] == '}' {
				index++
				out.WriteRune('}')
			} else {
				return "", malformed("single '}' encountered outside a placeholder")
			}
		default:
			out.WriteRune(c)
		}
	}
	return out.String(), nil
}

func malformed(detail string) error {
	return usageError("Malformed custom prompt: " + detail + ". " + braceHint)
}

type captureSnapshot struct {
	content       string
	capturedAt    string
	contentSHA256 string
}

func captureSnapshotFromRaw(raw string, ansi bool, now time.Time) captureSnapshot {
	content := raw
	if !ansi {
		content = stripAnsi(raw)
	}
	return captureSnapshot{content: content, capturedAt: formatUTC(now), contentSHA256: sha256Hex([]byte(content))}
}

type memberCapture struct {
	memberID int64
	paneID   string
	lines    int64
	snapshot captureSnapshot
}

func writeMemberCapture(out io.Writer, capture memberCapture, json bool) error {
	var err error
	if json {
		_, err = io.WriteString(out, formatJSON(outputJSONMemberCapture(capture))+"\n")
	} else {
		_, err = io.WriteString(out, capture.snapshot.content)
	}
	return outputResult(err)
}
