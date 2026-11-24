package core

import (
	"fmt"
	"strings"
)

const maxAncestry = 4096

const tmuxGuidance = `the tmux command prompt (the user presses the tmux prefix key in the Director pane, then types :display-message -d 0 "#{pane_id} #{socket_path}" and Enter; the status line shows the pane ID, then the server socket)`

const herdrGuidance = "! env -u HERDR_PANE_ID herdr --session <session> pane current --current (typed by the user into the Director's composer while the Director pane is focused; <session> is the herdr session the user attached, default unless herdr was started with --session <name>, and herdr session list shows the running sessions; the pane ID is result.pane.pane_id and the server is <session>)"

const rerun = "then run cafleet fleet create with --pane <pane-id> --mux-server <server>"

type callerMux struct {
	backend muxBackend
	server  string
	mux     multiplexer
}

type callerPane struct {
	variable string
	pane     string
}

//sumtype:decl
type walkEnd interface {
	isWalkEnd()
}

type walkEndComplete struct{}

type walkEndStopped struct {
	pid int64
}

type walkEndUnreadable struct {
	cause string
}

func (walkEndComplete) isWalkEnd()   {}
func (walkEndStopped) isWalkEnd()    {}
func (walkEndUnreadable) isWalkEnd() {}

type ancestryEntry struct {
	pid  int64
	argv []string
}

type ancestry struct {
	// entries starts at the caller.
	entries []ancestryEntry
	end     walkEnd
}

type sharedServer struct {
	label string
	pid   int64
}

//sumtype:decl
type refusal interface {
	isRefusal()
}

type refusalPaneNotFound struct{}

type refusalPaneNotAncestor struct {
	panePID int64
}

type refusalWalkStopped struct {
	pid int64
}

type refusalNestedPidNamespace struct{}

type refusalProcessUnreadable struct {
	cause string
}

func (refusalPaneNotFound) isRefusal()       {}
func (refusalPaneNotAncestor) isRefusal()    {}
func (refusalWalkStopped) isRefusal()        {}
func (refusalNestedPidNamespace) isRefusal() {}
func (refusalProcessUnreadable) isRefusal()  {}

//sumtype:decl
type verification interface {
	isVerification()
}

type verificationVerified struct{}

type verificationRefused struct {
	refusal refusal
}

func (verificationVerified) isVerification() {}
func (verificationRefused) isVerification()  {}

// walk follows the parent chain from this process toward pid 1, at most maxAncestry entries.
func walk(table processTable) ancestry {
	var entries []ancestryEntry
	pid := table.ownPID()
	for {
		if pid == 1 {
			return ancestry{entries: entries, end: walkEndComplete{}}
		}
		if len(entries) == maxAncestry {
			return ancestry{entries: entries, end: walkEndStopped{pid: entries[len(entries)-1].pid}}
		}
		entry, present, err := table.entry(pid)
		if err != nil {
			return ancestry{entries: entries, end: walkEndUnreadable{cause: cafleetErrorOf(err).message()}}
		}
		if !present {
			return ancestry{entries: entries, end: walkEndStopped{pid: pid}}
		}
		entries = append(entries, ancestryEntry{pid: pid, argv: entry.argv})
		if entry.parent == 0 {
			return ancestry{entries: entries, end: walkEndStopped{pid: pid}}
		}
		pid = entry.parent
	}
}

func lookup(env map[string]string) func(string) opt[string] {
	return func(name string) opt[string] {
		if value, set := env[name]; set && value != "" {
			return some(value)
		}
		return none[string]()
	}
}

func available(backend muxBackend, server string, runner commandRunner) (callerMux, error) {
	mux := backend.forServer(server, runner)
	if err := mux.ensureAvailable(); err != nil {
		return callerMux{}, err.cafleet()
	}
	return callerMux{backend: backend, server: server, mux: mux}, nil
}

// resolveEnvMux runs backend detection, server resolution and availability, without --pane.
func resolveEnvMux(env map[string]string, runner commandRunner) (callerMux, error) {
	backend, err := muxBackendDetect(lookup(env))
	if err != nil {
		return callerMux{}, err.cafleet()
	}
	server, err := callerServer(backend, lookup(env), runner)
	if err != nil {
		return callerMux{}, err.cafleet()
	}
	return available(backend, server, runner)
}

func paneBackend(pane string) (muxBackend, error) {
	if digits, tmux := strings.CutPrefix(pane, "%"); tmux && digits != "" && !strings.ContainsFunc(digits, func(r rune) bool { return r < '0' || r > '9' }) {
		return muxBackendTmux, nil
	}
	if workspace, rest, found := strings.Cut(pane, ":"); found && !strings.HasPrefix(pane, "%") && workspace != "" && rest != "" && !strings.Contains(rest, ":") {
		return muxBackendHerdr, nil
	}
	return 0, appErrorf("--pane %s is neither a tmux pane ID (%%N) nor a herdr pane ID (<workspace>:<pane>)", pane)
}

// resolvePaneMux is the backend an explicit --pane ID form selects, with its server and
// availability.
func resolvePaneMux(pane string, muxServer opt[string], env map[string]string, runner commandRunner) (callerMux, error) {
	backend, err := paneBackend(pane)
	if err != nil {
		return callerMux{}, err
	}
	server, explicit := muxServer.get()
	if !explicit {
		var resolveErr *multiplexerError
		if detected, detectErr := muxBackendDetect(lookup(env)); detectErr == nil && detected == backend {
			server, resolveErr = callerServer(backend, lookup(env), runner)
		} else if backend == muxBackendTmux {
			server, resolveErr = tmuxSocketReply(runner, defaultTmuxSocket)
		} else {
			server = defaultHerdrSession
		}
		if resolveErr != nil {
			return callerMux{}, resolveErr.cafleet()
		}
	}
	return available(backend, server, runner)
}

func paneVariable(backend muxBackend) string {
	switch backend {
	case muxBackendTmux:
		return "TMUX_PANE"
	case muxBackendHerdr:
		return "HERDR_PANE_ID"
	default:
		panic("invalid muxBackend")
	}
}

// envPane is the non-empty TMUX_PANE / HERDR_PANE_ID for backend.
func envPane(backend muxBackend, env map[string]string) opt[callerPane] {
	variable := paneVariable(backend)
	pane, set := lookup(env)(variable).get()
	if !set {
		return none[callerPane]()
	}
	return some(callerPane{variable: variable, pane: pane})
}

// verifyPane checks rows 2-7 for an env pane after sharedServerOf found no shared server.
func verifyPane(m multiplexer, table processTable, a ancestry, pane string) (verification, error) {
	process, err := m.paneProcess(pane)
	if err != nil {
		return nil, err.cafleet()
	}
	panePID, exists := process.get()
	if !exists {
		return verificationRefused{refusal: refusalPaneNotFound{}}, nil
	}
	nested, nestErr := table.nestedPIDNamespace()
	if nestErr != nil {
		return nil, nestErr
	}
	if nested {
		return verificationRefused{refusal: refusalNestedPidNamespace{}}, nil
	}
	for _, entry := range a.entries {
		if entry.pid == panePID {
			return verificationVerified{}, nil
		}
	}
	switch end := a.end.(type) {
	case walkEndUnreadable:
		return verificationRefused{refusal: refusalProcessUnreadable(end)}, nil
	case walkEndStopped:
		return verificationRefused{refusal: refusalWalkStopped(end)}, nil
	case walkEndComplete:
		return verificationRefused{refusal: refusalPaneNotAncestor{panePID: panePID}}, nil
	default:
		panic("invalid walkEnd")
	}
}

// identificationGuidance is the user-run command for one multiplexer.
func identificationGuidance(backend muxBackend) string {
	switch backend {
	case muxBackendTmux:
		return tmuxGuidance
	case muxBackendHerdr:
		return herdrGuidance
	default:
		panic("invalid muxBackend")
	}
}

// sharedServerMessage is the shared-server refusal body, without the `cafleet fleet create `
// prefix.
// sharedServerOf is the Rust caller::shared_server, renamed because it collides with the
// sharedServer type: the first ancestor whose argv matches a backend's shared-server signature.
func sharedServerOf(a ancestry) opt[sharedServer] {
	for _, entry := range a.entries {
		for _, agent := range codingAgentAll() {
			for _, signature := range agent.sharedServerSignatures() {
				if signature.matches(entry.argv) {
					return some(sharedServer{label: signature.label(agent), pid: entry.pid})
				}
			}
		}
	}
	return none[sharedServer]()
}

func sharedServerMessage(shared sharedServer) string {
	return fmt.Sprintf("cannot verify this session's pane: a shared %s server (pid %d) runs this command, so its multiplexer environment can belong to another pane; ask the user to read the Director pane ID and server with %s in tmux, or with %s in herdr, %s", shared.label, shared.pid, tmuxGuidance, herdrGuidance, rerun)
}

func unverifiedMessage(caller callerPane, r refusal, mux callerMux) string {
	var cause string
	switch r := r.(type) {
	case refusalPaneNotFound:
		cause = fmt.Sprintf("the pane does not exist on the %s server %s", mux.backend.name(), mux.server)
	case refusalPaneNotAncestor:
		cause = fmt.Sprintf("the pane's process %d is not an ancestor of this command", r.panePID)
	case refusalWalkStopped:
		cause = fmt.Sprintf("the process ancestry stopped at pid %d before reaching pid 1", r.pid)
	case refusalNestedPidNamespace:
		cause = "this command runs in a nested PID namespace"
	case refusalProcessUnreadable:
		cause = "the process ancestry could not be read: " + r.cause
	default:
		panic("invalid refusal")
	}
	return fmt.Sprintf("cannot verify that %s=%s is this session's pane: %s; ask the user to read the Director pane ID and server with %s, %s", caller.variable, caller.pane, cause, identificationGuidance(mux.backend), rerun)
}

func unsetMessage(backend muxBackend) string {
	return fmt.Sprintf("cannot identify its %s pane because %s is not set; ask the user to read the Director pane ID and server with %s, %s", backend.name(), paneVariable(backend), identificationGuidance(backend), rerun)
}
