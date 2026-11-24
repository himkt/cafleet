package core

import (
	"fmt"
	"os"
	"slices"
	"strings"
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
