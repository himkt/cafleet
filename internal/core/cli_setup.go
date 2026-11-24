package core

import (
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"slices"
	"strings"
	"sync"
	"time"
)

func setupRun(s settings, args setupArgs) error {
	if err := prepareDatabase(s); err != nil {
		return err
	}
	home, err := homeDir()
	if err != nil {
		return err
	}
	selected := make([]codingAgent, 0, len(args.codingAgent))
	for _, name := range args.codingAgent {
		selected = append(selected, codingAgentNamed(name))
	}
	return installSelected(selected, processEnv, home)
}

// codingAgentNamed is the backend the parser already validated name against.
func codingAgentNamed(name string) codingAgent {
	for _, agent := range codingAgentAll() {
		if agent.name() == name {
			return agent
		}
	}
	panic("the parser accepts only registered backend names, not " + name)
}

// pathParent is Rust's Path::parent of a relative or absolute path, with none for a bare name.
func pathParent(path string) opt[string] {
	index := strings.LastIndex(path, "/")
	switch {
	case index < 0:
		return none[string]()
	case index == 0:
		return some("/")
	default:
		return some(path[:index])
	}
}

func prepareDatabase(s settings) error {
	path, err := databasePath(s.databaseURL)
	if err != nil {
		return err
	}
	if path == "" {
		return appError("database URL has no file path")
	}
	if parent, set := pathParent(path).get(); set && parent != "" {
		if err := os.MkdirAll(parent, 0o777); err != nil {
			return appErrorf("cannot create %s: %s", parent, ioErrorText(err))
		}
	}
	if err := requireCreatableSchema(s.databaseURL); err != nil {
		return err
	}
	conn, err := connect(s.databaseURL)
	if err != nil {
		return err
	}
	return mergeCleanup(initializeOrReport(conn, s.databaseURL, path), conn.close())
}

// requireCreatableSchema refuses an existing database whose schema is neither missing nor
// current, before setup opens it for writing.
func requireCreatableSchema(databaseURL string) error {
	existing, err := openExisting(databaseURL, true)
	if err != nil {
		return err
	}
	conn, set := existing.get()
	if !set {
		return nil
	}
	schema := classifySelectedSchema(conn, databaseURL)
	var refusal error
	if schema.kind != schemaMissing && schema.kind != schemaCurrent {
		refusal = schema.requireCurrent()
	}
	return mergeCleanup(refusal, conn.close())
}

func initializeOrReport(conn *dbConn, databaseURL, path string) error {
	schema := classifySelectedSchema(conn, databaseURL)
	switch schema.kind {
	case schemaMissing:
		if err := initialize(conn, databaseURL); err != nil {
			return err
		}
		if err := classifySelectedSchema(conn, databaseURL).requireCurrent(); err != nil {
			return err
		}
		return writeOutput(fmt.Sprintf("Initialized database at %s.", path))
	case schemaCurrent:
		return writeOutput(fmt.Sprintf("Database already initialized at %s.", path))
	case schemaIncompatible, schemaUnreachable:
		return schema.requireCurrent()
	default:
		panic("invalid schemaStateKind")
	}
}

func installSelected(selected []codingAgent, env envLookup, home string) error {
	for _, agent := range codingAgentAll() {
		if len(selected) == 0 || slices.Contains(selected, agent) {
			paths, err := assetsAgentPaths(env, home, agent)
			if err != nil {
				return err
			}
			if err := installAgent(agent, paths, os.Stdout); err != nil {
				return err
			}
		}
	}
	return nil
}

type check struct {
	name   string
	status checkStatus
	detail string
}

func (c check) MarshalJSONTo(encoder *jsontext.Encoder) error {
	return json.MarshalEncode(encoder, jsonObject{{"name", c.name}, {"status", c.status.asStr()}, {"detail", c.detail}})
}

func newCheck(name string, status checkStatus, detail string) check {
	return check{name: name, status: status, detail: detail}
}

// checkFixable is a row that rerunning setup for its backend, the row's name prefix, fixes.
func checkFixable(name string, status checkStatus, detail string) check {
	backend, _, _ := strings.Cut(name, ".")
	return newCheck(name, status, fmt.Sprintf("%s; run 'cafleet setup --coding-agent %s'", detail, backend))
}

//sumtype:decl
type callerReading interface{ isCallerReading() }

// callerReadingSharedServer is row 1 refused before any multiplexer command.
type callerReadingSharedServer struct {
	label   string
	pid     int64
	message string
}

type callerReadingUnavailable struct{ cause error }

// callerReadingResolved is a resolved multiplexer; paneErr non-nil is Rust's Err pane reading.
type callerReadingResolved struct {
	name    string
	server  string
	pane    paneReading
	paneErr error
}

func (callerReadingSharedServer) isCallerReading() {}
func (callerReadingUnavailable) isCallerReading()  {}
func (callerReadingResolved) isCallerReading()     {}

//sumtype:decl
type paneReading interface{ isPaneReading() }

type paneReadingVerified struct {
	variable string
	context  multiplexerContext
}

// paneReadingUnverified carries the unverifiedMessage or unsetMessage body.
type paneReadingUnverified struct{ detail string }

func (paneReadingVerified) isPaneReading()   {}
func (paneReadingUnverified) isPaneReading() {}

// doctorInputs is everything doctor reads from outside the process; homeErr non-nil is Rust's
// Err home.
type doctorInputs struct {
	caller   func() callerReading
	settings settings
	probe    spawnProbe
	home     string
	homeErr  error
}

type report struct {
	checks []check
}

func (r report) hasErrors() bool {
	return slices.ContainsFunc(r.checks, func(c check) bool { return c.status == checkStatusError })
}

type doctorMode uint8

const (
	doctorModeText doctorMode = iota + 1
	doctorModeJSON
)

func doctor(s settings, args doctorArgs) error {
	return doctorWith(s, args, systemProcessTable{})
}

func doctorWith(s settings, args doctorArgs, table processTable) error {
	home, homeErr := homeDir()
	r := gather(doctorInputs{
		caller:   func() callerReading { return detectCaller(table) },
		settings: s,
		probe:    systemProbe{},
		home:     home,
		homeErr:  homeErr,
	})
	mode := doctorModeText
	if args.json {
		mode = doctorModeJSON
	}
	if err := writeOutput(render(r, mode)); err != nil {
		return err
	}
	if r.hasErrors() {
		return appError("doctor found errors")
	}
	return nil
}

// detectCaller is the fleet create verification without --pane, as doctor reports it.
func detectCaller(table processTable) callerReading {
	env, err := callerEnv()
	if err != nil {
		return callerReadingUnavailable{cause: err}
	}
	a := walk(table)
	if shared, set := sharedServerOf(a).get(); set {
		return callerReadingSharedServer{message: sharedServerMessage(shared), label: shared.label, pid: shared.pid}
	}
	mux, err := resolveEnvMux(env, systemRunner{})
	if err != nil {
		return callerReadingUnavailable{cause: err}
	}
	pane, paneErr := readCallerPane(mux, env, table, a)
	return callerReadingResolved{name: mux.backend.name(), server: mux.server, pane: pane, paneErr: paneErr}
}

func readCallerPane(mux callerMux, env map[string]string, table processTable, a ancestry) (paneReading, error) {
	pane, set := envPane(mux.backend, env).get()
	if !set {
		return paneReadingUnverified{detail: unsetMessage(mux.backend)}, nil
	}
	verdict, err := verifyPane(mux.mux, table, a, pane.pane)
	if err != nil {
		return nil, err
	}
	switch verdict := verdict.(type) {
	case verificationVerified:
		found, muxErr := mux.mux.paneContext(pane.pane)
		if muxErr != nil {
			return nil, muxErr.cafleet()
		}
		context, present := found.get()
		if !present {
			return nil, appErrorf("pane %s was not found on the %s server %s", pane.pane, mux.backend.name(), mux.server)
		}
		return paneReadingVerified{variable: pane.variable, context: context}, nil
	case verificationRefused:
		return paneReadingUnverified{detail: unverifiedMessage(pane, verdict.refusal, mux)}, nil
	default:
		panic("invalid verification")
	}
}

type memberProbe struct {
	run   runRecord
	probe observationProbe
}

// gather runs every check, the member probes overlapping the backend checks; rows keep their
// order.
func gather(inputs doctorInputs) report {
	backends := resolveBackends(inputs.probe, inputs.home, inputs.homeErr)
	checks := multiplexerRows(inputs.caller())
	databaseCheck, runs := doctorDatabase(inputs.settings)
	checks = append(checks, databaseCheck)
	probes := make([]memberProbe, len(runs))
	var wait sync.WaitGroup
	for index, run := range runs {
		wait.Go(func() {
			probes[index] = memberProbe{run: run, probe: probeObservation(run, systemClock{})}
		})
	}
	agents := agentChecks(backends)
	now := time.Now().UTC()
	databaseURL := none[string]()
	if inputs.settings.databaseURLExplicit {
		databaseURL = some(inputs.settings.databaseURL)
	}
	wait.Wait()
	for _, probed := range probes {
		checks = append(checks, memberRuntimeCheck(probed.run.memberID, probed.run, probed.probe, now, databaseURL))
	}
	return report{checks: append(checks, agents...)}
}

func render(r report, mode doctorMode) string {
	switch mode {
	case doctorModeJSON:
		encoded, err := encodeJSON(r.checks)
		if err != nil {
			panic("doctor checks serialize: " + err.Error())
		}
		return string(encoded)
	case doctorModeText:
		lines := make([]string, 0, len(r.checks))
		for _, c := range r.checks {
			lines = append(lines, fmt.Sprintf("%s %s: %s", c.status.asStr(), c.name, escape(c.detail)))
		}
		return strings.Join(lines, "\n")
	default:
		panic("invalid doctorMode")
	}
}

type statusDetail struct {
	status checkStatus
	detail string
}

// multiplexerRows is the multiplexer and multiplexer.caller_pane checks.
func multiplexerRows(reading callerReading) []check {
	var mux, pane statusDetail
	switch reading := reading.(type) {
	case callerReadingSharedServer:
		mux = statusDetail{checkStatusInfo, fmt.Sprintf("not checked: a shared %s server (pid %d) supplies the caller environment", reading.label, reading.pid)}
		pane = statusDetail{checkStatusWarn, reading.message}
	case callerReadingUnavailable:
		mux = statusDetail{checkStatusError, reading.cause.Error()}
		pane = statusDetail{checkStatusInfo, "unavailable: the multiplexer is unavailable"}
	case callerReadingResolved:
		mux = statusDetail{checkStatusOk, fmt.Sprintf("%s server=%s", reading.name, reading.server)}
		if reading.paneErr != nil {
			pane = statusDetail{checkStatusError, reading.paneErr.Error()}
			break
		}
		switch read := reading.pane.(type) {
		case paneReadingVerified:
			pane = statusDetail{checkStatusOk, fmt.Sprintf("%s=%s session=%s window=%s", read.variable, read.context.paneID, read.context.session, read.context.windowID)}
		case paneReadingUnverified:
			pane = statusDetail{checkStatusWarn, read.detail}
		default:
			panic("invalid paneReading")
		}
	default:
		panic("invalid callerReading")
	}
	return []check{
		newCheck("multiplexer", mux.status, mux.detail),
		newCheck("multiplexer.caller_pane", pane.status, pane.detail),
	}
}

func databaseRow(s settings, schema schemaState) check {
	if schema.kind == schemaCurrent {
		path, err := databasePath(s.databaseURL)
		if err != nil {
			panic("a current schema was opened through a valid database URL")
		}
		return newCheck("database", checkStatusOk, fmt.Sprintf("cafleet %s at %s", Version, path))
	}
	err := schema.requireCurrent()
	if err == nil {
		panic("a schema that is not current is unavailable")
	}
	return newCheck("database", checkStatusError, err.Error())
}

func escape(detail string) string {
	return strings.ReplaceAll(strings.ReplaceAll(detail, "\n", `\n`), "\r", `\r`)
}

// doctorDatabase is the Rust database: the database row, and the runs that get member rows when
// that row is ok.
func doctorDatabase(s settings) (check, []runRecord) {
	existing, err := openExisting(s.databaseURL, true)
	if err != nil {
		return databaseRow(s, schemaState{kind: schemaUnreachable, cause: cafleetErrorOf(err)}), nil
	}
	conn, set := existing.get()
	if !set {
		return databaseRow(s, schemaState{kind: schemaMissing}), nil
	}
	defer conn.close() //nolint:errcheck // a read-only connection; doctor reports through its rows
	row := databaseRow(s, classifySelectedSchema(conn, s.databaseURL))
	if row.status != checkStatusOk {
		return row, nil
	}
	runs, err := liveMemberRuns(context.Background(), conn)
	if err != nil {
		return newCheck("database", checkStatusError, err.Error()), nil
	}
	return row, runs
}

const doctorGapErrorAge = 30 * time.Second

func memberRuntimeCheck(memberID int64, run runRecord, probe observationProbe, now time.Time, databaseURL opt[string]) check {
	runID := run.runID
	hint := func() string { return recoveryHint(memberID, databaseURL) }
	var result statusDetail
	switch probe := probe.(type) {
	case observationProbeSkipped:
		result = statusDetail{checkStatusInfo, fmt.Sprintf("run=%d status=%s: not probed", runID, run.status.asStr())}
	case observationProbeFailed:
		result = statusDetail{checkStatusError, fmt.Sprintf("run=%d health probe failed: %s; %s", runID, probe.message, hint())}
	case observationProbeAnswered:
		health, answered := probe.health.get()
		if !answered {
			result = statusDetail{checkStatusInfo, fmt.Sprintf("run=%d observation=pending", runID)}
			break
		}
		observation := fmt.Sprintf("run=%d observation=%s", runID, health.Eligibility.rendering())
		if !health.Degraded {
			result = statusDetail{checkStatusOk, observation}
			break
		}
		diagnostic, set := health.Diagnostic.get()
		if !set {
			panic("validated degraded health names its diagnostic")
		}
		degraded := observation + " diagnostic=" + diagnostic
		if since, open := health.GapOpenSince.get(); open && gapAge(since, now) > doctorGapErrorAge {
			result = statusDetail{checkStatusError, fmt.Sprintf("%s gap_open_since=%s; %s", degraded, since, hint())}
		} else {
			result = statusDetail{checkStatusWarn, degraded}
		}
	default:
		panic("invalid observationProbe")
	}
	return newCheck(fmt.Sprintf("member.%d.runtime", memberID), result.status, result.detail)
}

func gapAge(since string, now time.Time) time.Duration {
	parsed, ok := parseLenient(since)
	if !ok {
		panic("validated health carries an RFC 3339 gap start")
	}
	return max(now.Sub(parsed), 0)
}

func recoveryHint(memberID int64, databaseURL opt[string]) string {
	suffix := ""
	if url, set := databaseURL.get(); set {
		suffix = " --database-url " + url
	}
	return fmt.Sprintf("inspect with 'cafleet member show %d --json%s'; if it does not recover, run 'cafleet member delete %d%s' and create a replacement member", memberID, suffix, memberID, suffix)
}

// doctorBackend is one backend's configuration base, skills directory and executable, each
// resolved once; a non-nil error replaces its value.
type doctorBackend struct {
	agent      codingAgent
	paths      string
	pathsErr   error
	skills     string
	skillsErr  error
	executable opt[string]
}

func resolveBackends(probe spawnProbe, home string, homeErr error) []doctorBackend {
	backends := make([]doctorBackend, 0, len(codingAgentAll()))
	for _, agent := range codingAgentAll() {
		backend := doctorBackend{agent: agent, executable: probe.resolveBinary(agent.name())}
		if homeErr != nil {
			backend.pathsErr = homeErr
		} else {
			backend.paths, backend.pathsErr = agent.configDir(probe.envVar, home)
		}
		switch {
		case backend.pathsErr != nil:
			backend.skillsErr = backend.pathsErr
		case homeErr != nil:
			backend.skillsErr = homeErr
		default:
			backend.skills = skillsDir(agent, backend.paths, home)
		}
		backends = append(backends, backend)
	}
	return backends
}

// unavailable is the info row of a check whose backend configuration failed or whose executable
// is absent.
func unavailable(name string, pathsErr error, executable bool) opt[check] {
	switch {
	case pathsErr != nil:
		return some(newCheck(name, checkStatusInfo, "unavailable: configuration failed: "+pathsErr.Error()))
	case !executable:
		return some(newCheck(name, checkStatusInfo, "unavailable: executable is absent"))
	default:
		return none[check]()
	}
}

func agentChecks(backends []doctorBackend) []check {
	var checks []check
	for _, backend := range backends {
		agent := backend.agent
		name := agent.name()
		_, executable := backend.executable.get()
		if backend.pathsErr != nil {
			checks = append(checks, newCheck(name+".configuration", checkStatusError, backend.pathsErr.Error()))
		} else {
			checks = append(checks, newCheck(name+".configuration", checkStatusOk, backend.paths))
		}
		if executable {
			checks = append(checks, newCheck(name+".executable", checkStatusOk, name+" is on PATH"))
		} else {
			checks = append(checks, newCheck(name+".executable", checkStatusInfo, name+" is optional and absent from PATH"))
		}
		if agent == codingAgentOpencode {
			policy, err := agent.resolvePolicy(none[string]())
			if err != nil {
				panic("registered backend has a valid default policy: " + err.Error())
			}
			for _, mode := range agent.policyChoices() {
				checks = append(checks, presetCheck(agent, mode, mode == policy.resolvedMode, backend.paths, backend.pathsErr, executable))
			}
			checks = append(checks, opencodeExtensionCheck(backend.paths, backend.pathsErr, executable))
		}
		if agent == codingAgentPi {
			checks = append(checks, piExtensionCheck(backend.paths, backend.pathsErr, executable))
		}
		checks = append(checks, skillsCheck(name, backend.skills, backend.skillsErr, executable))
	}
	return checks
}

// skillsCheck compares every embedded skill file with its installed copy under the backend's
// skills dir.
func skillsCheck(name, dir string, dirErr error, executable bool) check {
	checkName := name + ".skills"
	if row, set := unavailable(checkName, dirErr, executable).get(); set {
		return row
	}
	files := skillFiles(dir)
	absent := func(target string) bool {
		_, err := os.Lstat(target)
		return errors.Is(err, fs.ErrNotExist)
	}
	if !slices.ContainsFunc(files, func(file embeddedFile) bool { return !absent(file.path) }) {
		return checkFixable(checkName, checkStatusWarn, "skills not installed at "+dir)
	}
	for _, file := range files {
		if cause, set := installedFileProblem(file.path, file.data).get(); set {
			return checkFixable(checkName, checkStatusError, file.path+": "+cause)
		}
	}
	return newCheck(checkName, checkStatusOk, dir)
}

func presetCheck(agent codingAgent, mode string, isDefault bool, base string, baseErr error, executable bool) check {
	name := agent.name()
	checkName := name + ".preset." + mode
	if isDefault {
		checkName = name + ".preset"
	}
	if row, set := unavailable(checkName, baseErr, executable).get(); set {
		return row
	}
	target, set := agent.requiredPreset(base, mode).get()
	if !set {
		panic("OpenCode has a required preset for every launch policy")
	}
	if _, err := os.ReadFile(target); err != nil {
		return checkFixable(checkName, checkStatusError, target+": "+ioErrorText(err))
	}
	return newCheck(checkName, checkStatusOk, target)
}

// extensionCheck compares installed extension files with the embedded copies, then checks the
// executable record.
func extensionCheck(name string, files []embeddedFile, record, okDetail string) check {
	for _, file := range files {
		if cause, set := installedFileProblem(file.path, file.data).get(); set {
			return checkFixable(name, checkStatusError, file.path+": "+cause)
		}
	}
	if cause, set := executableRecordProblem(record).get(); set {
		return checkFixable(name, checkStatusError, record+": "+cause)
	}
	return newCheck(name, checkStatusOk, okDetail)
}

func extensionFiles(root, prefix string) []embeddedFile {
	embedded := embeddedExtension(prefix)
	files := make([]embeddedFile, 0, len(embedded))
	for _, file := range embedded {
		files = append(files, embeddedFile{path: joinPath(root, file.path), data: file.data})
	}
	return files
}

func opencodeExtensionCheck(base string, baseErr error, executable bool) check {
	return fileExtensionCheck("opencode.extension", base, baseErr, executable, opencodePluginsDir, opencodeExtensionPrefix, opencodeExecutableRecord)
}

func piExtensionCheck(base string, baseErr error, executable bool) check {
	return fileExtensionCheck("pi.extension", base, baseErr, executable, piExtensionsDir, piExtensionPrefix, piExecutableRecord)
}

// fileExtensionCheck checks an extension installed as embedded files beside one executable record
// under its base.
func fileExtensionCheck(name, base string, baseErr error, executable bool, extensionDir func(string) string, prefix, record string) check {
	if row, set := unavailable(name, baseErr, executable).get(); set {
		return row
	}
	dir := extensionDir(base)
	files := extensionFiles(dir, prefix)
	targets := make([]string, 0, len(files))
	for _, file := range files {
		targets = append(targets, file.path)
	}
	return extensionCheck(name, files, joinPath(dir, record), strings.Join(targets, ", "))
}
