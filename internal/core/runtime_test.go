package core

import (
	"context"
	"encoding/json/jsontext"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

type runnerFixture struct {
	dir  string
	conn *dbConn
	url  string
	run  runRecord
}

func initializedConnAt(t *testing.T, path string) (*dbConn, string) {
	t.Helper()
	url := databaseURLFor(path)
	conn, err := connect(url)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { closeConnection(t, conn) })
	if err := initialize(conn, url); err != nil {
		t.Fatal(err)
	}
	return conn, url
}

func newRunnerFixture(t *testing.T) *runnerFixture {
	t.Helper()
	ctx := context.Background()
	dir := shortTempdir(t)
	conn, url := initializedConnAt(t, filepath.Join(dir, "broker.db"))
	fleet, director := createFleetFixture(t, conn, "runner-contract")
	placement := testPlacement(some("%2"))
	placement.codingAgent = "codex"
	member, err := registerMember(ctx, conn, fleet, "worker", "assignment", placement)
	if err != nil {
		t.Fatal(err)
	}
	process, err := processIdentityCurrent()
	if err != nil {
		t.Fatal(err)
	}
	identity, err := databaseIdentity(ctx, conn)
	if err != nil {
		t.Fatal(err)
	}
	paths, err := runtimePathsForRun(identity, 1)
	if err != nil {
		t.Fatal(err)
	}
	run, err := createRun(ctx, conn, newRun{
		memberID:             member.memberID,
		fleetID:              fleet,
		directorMemberID:     director,
		backend:              "codex",
		databaseIdentityJSON: identity,
		launchNonce:          strings.Repeat("a", 64),
		runtimePath:          paths.runDir,
		requestedMode:        none[string](),
		resolvedMode:         "approve-for-me",
		modeSource:           modeSourceDefault,
		permissionArgvJSON:   jsontext.Value(`["--approve-for-me"]`),
		nativeVersion:        none[string](),
		coverage:             coverageTested,
		ownerPID:             process.pid,
		ownerStartIdentity:   process.startIdentity,
	})
	if err != nil {
		t.Fatal(err)
	}
	return &runnerFixture{dir: dir, conn: conn, url: url, run: run}
}

func (f *runnerFixture) refresh(t *testing.T) {
	t.Helper()
	run, err := getRun(context.Background(), f.conn, f.run.runID)
	if err != nil {
		t.Fatal(err)
	}
	f.run = run
}

func (f *runnerFixture) artifacts(t *testing.T) *runArtifacts {
	t.Helper()
	artifacts, err := createRunArtifacts(context.Background(), f.conn, f.run.runID)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(artifacts.close)
	f.refresh(t)
	return artifacts
}

func (f *runnerFixture) spec() spawnSpec {
	return spawnSpec{
		argv: []string{"/usr/bin/env", "literal assignment 世界 '$x' `text`"},
		cwd:  f.dir,
		size: terminalSize{rows: 24, cols: 80},
	}
}

func (f *runnerFixture) databaseFile(t *testing.T) string {
	t.Helper()
	path, err := databasePath(f.url)
	if err != nil {
		t.Fatal(err)
	}
	return path
}

func fiveSecondsFromNow() time.Time {
	return systemClock{}.now().Add(5 * time.Second)
}

// runtimeVirtualLead is how far a runtime test's virtual clock starts ahead of real time. The
// database and lock budgets keep real time, so a deadline derived from the virtual clock lies in
// the real future however little virtual time a test spends.
const runtimeVirtualLead = time.Hour

func virtualClock() *testClock {
	return testClockStartingAt(time.Now().Add(runtimeVirtualLead))
}
