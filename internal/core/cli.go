package core

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"slices"
	"strings"
	"syscall"
	"unicode"
	"unicode/utf8"
)

const cliRootAbout = "CAFleet — CLI for the message broker and member registry."

const cliDatabaseURLHelp = "SQLite database URL `sqlite:///<path>` [default: the running version's database under ~/.local/share/cafleet]"

const cliJSONHelp = "Output in JSON format"

const cliDirectorHelp = "The root Director issuing this Director-only command"

// codingAgentNames lists the backend registry's names in registry order.
func codingAgentNames() []string {
	names := make([]string, 0, len(codingAgentAll()))
	for _, agent := range codingAgentAll() {
		names = append(names, agent.name())
	}
	return names
}

// permissionDecisionValues lists the stored permission decisions in declaration order.
var permissionDecisionValues = []string{"allow", "deny"}

func validChoiceID(value string) bool {
	if len(value) > 64 || value == "" {
		return false
	}
	for part := range strings.SplitSeq(value, "-") {
		if part == "" {
			return false
		}
		for i := range len(part) {
			if b := part[i]; (b < 'a' || b > 'z') && (b < '0' || b > '9') {
				return false
			}
		}
	}
	return true
}

func validateRequestToken(value string) error {
	valid := len(value) == 64
	for i := 0; valid && i < len(value); i++ {
		b := value[i]
		valid = (b >= '0' && b <= '9') || (b >= 'a' && b <= 'f')
	}
	if !valid {
		return usageError("a lowercase 64-hex request token is required")
	}
	return nil
}

// cliArgs is a successfully parsed invocation: the deepest `--database-url` and the command.
type cliArgs struct {
	databaseURL opt[string]
	command     cliCommand
}

// cliCommand is one parsed leaf command.
//
//sumtype:decl
type cliCommand interface {
	isCliCommand()
}

type memberRunCommand struct {
	descriptor string
}

type setupArgs struct {
	codingAgent []string
}

type doctorArgs struct {
	json bool
}

//sumtype:decl
type fleetCommand interface {
	cliCommand
	isFleetCommand()
}

type fleetCommandCreate struct {
	name        string
	codingAgent string
	pane        opt[string]
	muxServer   opt[string]
	json        bool
}

type fleetCommandList struct {
	json bool
}

type fleetCommandShow struct {
	fleetID int64
	json    bool
}

type fleetCommandDelete struct {
	fleetID int64
	json    bool
}

//sumtype:decl
type memberCommand interface {
	cliCommand
	isMemberCommand()
}

// promptArgs holds exactly one of prompt and file.
type promptArgs struct {
	prompt opt[string]
	file   opt[string]
}

type createArgs struct {
	fleetID        int64
	name           string
	description    string
	codingAgent    opt[string]
	model          opt[string]
	effort         opt[string]
	permissionMode opt[string]
	body           promptArgs
	json           bool
}

type memberCommandCreate struct {
	createArgs
}

type memberCommandDelete struct {
	memberID int64
	json     bool
}

type memberCommandShow struct {
	memberID int64
	json     bool
}

type memberCommandList struct {
	fleetID int64
	json    bool
}

type memberCommandPrompt struct {
	memberID         int64
	directorMemberID int64
	text             string
	json             bool
}

// execBodyArgs holds exactly one of command and file.
type execBodyArgs struct {
	command opt[string]
	file    opt[string]
}

type memberCommandExec struct {
	memberID         int64
	directorMemberID int64
	body             execBodyArgs
	json             bool
}

type memberCommandPress struct {
	memberID         int64
	directorMemberID int64
	keys             []string
	json             bool
}

type memberCommandCapture struct {
	memberID int64
	lines    int64
	ansi     bool
	json     bool
}

//sumtype:decl
type messageCommand interface {
	cliCommand
	isMessageCommand()
}

// bodyArgs holds exactly one of text and file.
type bodyArgs struct {
	text opt[string]
	file opt[string]
}

type messageCommandSend struct {
	fromMemberID int64
	toMemberID   int64
	body         bodyArgs
	json         bool
}

type messageCommandPoll struct {
	memberID int64
	json     bool
}

type messageCommandShow struct {
	messageID int64
	json      bool
}

//sumtype:decl
type eventCommand interface {
	cliCommand
	isEventCommand()
}

type eventCommandShow struct {
	eventID int64
	json    bool
}

type disposition uint8

const (
	dispositionSubmitted disposition = iota + 1
	dispositionNotSubmitted
	dispositionAbandonedUncertain
)

var dispositionValues = []string{"submitted", "not-submitted", "abandoned-uncertain"}

func dispositionFrom(value string) disposition {
	switch value {
	case "submitted":
		return dispositionSubmitted
	case "not-submitted":
		return dispositionNotSubmitted
	case "abandoned-uncertain":
		return dispositionAbandonedUncertain
	default:
		panic("the parser admits only disposition values: " + value)
	}
}

//sumtype:decl
type inputCommand interface {
	cliCommand
	isInputCommand()
}

type inputCommandShow struct {
	inputID int64
	json    bool
}

type inputCommandReconcile struct {
	inputID          int64
	directorMemberID int64
	disposition      disposition
	evidenceRef      string
	json             bool
}

//sumtype:decl
type permissionCommand interface {
	cliCommand
	isPermissionCommand()
}

type permissionCommandInspect struct {
	interactionID    int64
	eventID          int64
	directorMemberID int64
	json             bool
}

// permissionCommandRespond holds exactly one of decision and choice.
type permissionCommandRespond struct {
	interactionID      int64
	eventID            int64
	directorMemberID   int64
	expectedRunID      int64
	expectedOwnerEpoch int64
	requestToken       string
	decision           opt[string]
	choice             opt[string]
	json               bool
}

func (memberRunCommand) isCliCommand()         {}
func (setupArgs) isCliCommand()                {}
func (doctorArgs) isCliCommand()               {}
func (fleetCommandCreate) isCliCommand()       {}
func (fleetCommandList) isCliCommand()         {}
func (fleetCommandShow) isCliCommand()         {}
func (fleetCommandDelete) isCliCommand()       {}
func (memberCommandCreate) isCliCommand()      {}
func (memberCommandDelete) isCliCommand()      {}
func (memberCommandShow) isCliCommand()        {}
func (memberCommandList) isCliCommand()        {}
func (memberCommandPrompt) isCliCommand()      {}
func (memberCommandExec) isCliCommand()        {}
func (memberCommandPress) isCliCommand()       {}
func (memberCommandCapture) isCliCommand()     {}
func (messageCommandSend) isCliCommand()       {}
func (messageCommandPoll) isCliCommand()       {}
func (messageCommandShow) isCliCommand()       {}
func (eventCommandShow) isCliCommand()         {}
func (inputCommandShow) isCliCommand()         {}
func (inputCommandReconcile) isCliCommand()    {}
func (permissionCommandInspect) isCliCommand() {}
func (permissionCommandRespond) isCliCommand() {}

func (fleetCommandCreate) isFleetCommand() {}
func (fleetCommandList) isFleetCommand()   {}
func (fleetCommandShow) isFleetCommand()   {}
func (fleetCommandDelete) isFleetCommand() {}

func (memberCommandCreate) isMemberCommand()  {}
func (memberCommandDelete) isMemberCommand()  {}
func (memberCommandShow) isMemberCommand()    {}
func (memberCommandList) isMemberCommand()    {}
func (memberCommandPrompt) isMemberCommand()  {}
func (memberCommandExec) isMemberCommand()    {}
func (memberCommandPress) isMemberCommand()   {}
func (memberCommandCapture) isMemberCommand() {}

func (messageCommandSend) isMessageCommand() {}
func (messageCommandPoll) isMessageCommand() {}
func (messageCommandShow) isMessageCommand() {}

func (eventCommandShow) isEventCommand() {}

func (inputCommandShow) isInputCommand()      {}
func (inputCommandReconcile) isInputCommand() {}

func (permissionCommandInspect) isPermissionCommand() {}
func (permissionCommandRespond) isPermissionCommand() {}

// cliValueCheck validates one raw argument value; a failure is a usage error whose message is
// the reason the parse error reports.
type cliValueCheck func(value string) error

type cliOption struct {
	long string
	// value is the value placeholder; a flag has none.
	value string
	help  string
	// multiple options take one or more values per occurrence and may repeat.
	multiple     bool
	required     bool
	choices      []string
	defaultValue string
	requires     string
	check        cliValueCheck
}

type cliPositional struct {
	name     string
	help     string
	required bool
	// maxValues bounds the values the positional takes; a positional above one is variadic.
	maxValues int
	check     cliValueCheck
}

// cliArgGroup admits at most one of its members, named by option long name or positional name.
type cliArgGroup struct {
	members  []string
	required bool
}

// cliSpec is one command: a group with subcommands, or a leaf with arguments and a builder.
type cliSpec struct {
	name          string
	about         string
	hidden        bool
	subcommands   []*cliSpec
	options       []cliOption
	positionals   []cliPositional
	groups        []cliArgGroup
	overrideUsage string
	build         func(cliMatches) cliCommand
}

var cliDatabaseURLOption = cliOption{long: "database-url", value: "URL", help: cliDatabaseURLHelp}

func jsonFlag() cliOption {
	return cliOption{long: "json", help: cliJSONHelp}
}

func directorOption(check cliValueCheck) cliOption {
	return cliOption{long: "from-member-id", value: "DIRECTOR_MEMBER_ID", help: cliDirectorHelp, required: true, check: check}
}

func cliRoot() *cliSpec {
	return &cliSpec{
		name:  "cafleet",
		about: cliRootAbout,
		subcommands: []*cliSpec{
			{
				name:   "__member-run",
				hidden: true,
				options: []cliOption{
					{long: "descriptor", value: "ABSOLUTE_PATH", required: true},
				},
				build: func(m cliMatches) cliCommand {
					return memberRunCommand{descriptor: m.value("descriptor")}
				},
			},
			{
				name:  "setup",
				about: "Initialize the version's database and install embedded coding-agent skills/presets",
				options: []cliOption{
					{long: "coding-agent", value: "AGENT", multiple: true, choices: codingAgentNames(),
						help: "Install selected backends' assets (repeatable; default: every backend)"},
				},
				build: func(m cliMatches) cliCommand {
					return setupArgs{codingAgent: m.values["coding-agent"]}
				},
			},
			{
				name:    "doctor",
				about:   "Report independent environment, database and backend checks",
				options: []cliOption{jsonFlag()},
				build: func(m cliMatches) cliCommand {
					return doctorArgs{json: m.flag("json")}
				},
			},
			cliFleetSpec(),
			cliMemberSpec(),
			cliMessageSpec(),
			cliEventSpec(),
			cliInputSpec(),
			cliPermissionSpec(),
		},
	}
}

func cliFleetSpec() *cliSpec {
	bareJSON := cliOption{long: "json"}
	return &cliSpec{
		name:  "fleet",
		about: "Fleet lifecycle",
		subcommands: []*cliSpec{
			{
				name:  "create",
				about: "Register the calling Director in a new fleet",
				options: []cliOption{
					{long: "name", value: "NAME", required: true, help: "Fleet name"},
					{long: "coding-agent", value: "CODING_AGENT", required: true, choices: codingAgentNames(),
						help: "Director's coding agent"},
					{long: "pane", value: "PANE_ID",
						help: "The Director's pane; its tmux %N or herdr <workspace>:<pane> form selects the multiplexer [default: the caller's TMUX_PANE or HERDR_PANE_ID, verified against the caller's process ancestry]"},
					{long: "mux-server", value: "SERVER", requires: "pane",
						help: "The Director's tmux socket path or herdr session [default: the caller's server on the pane's multiplexer]"},
					jsonFlag(),
				},
				build: func(m cliMatches) cliCommand {
					return fleetCommandCreate{
						name:        m.value("name"),
						codingAgent: m.value("coding-agent"),
						pane:        m.optional("pane"),
						muxServer:   m.optional("mux-server"),
						json:        m.flag("json"),
					}
				},
			},
			{
				name:    "list",
				about:   "List non-deleted fleets",
				options: []cliOption{bareJSON},
				build: func(m cliMatches) cliCommand {
					return fleetCommandList{json: m.flag("json")}
				},
			},
			{
				name:        "show",
				about:       "Show one fleet",
				positionals: []cliPositional{{name: "FLEET_ID", required: true, maxValues: 1, check: checkI64}},
				options:     []cliOption{bareJSON},
				build: func(m cliMatches) cliCommand {
					return fleetCommandShow{fleetID: m.int64("FLEET_ID"), json: m.flag("json")}
				},
			},
			{
				name:        "delete",
				about:       "End a fleet's ordinary members, then deregister the fleet",
				positionals: []cliPositional{{name: "FLEET_ID", required: true, maxValues: 1, check: checkI64}},
				options:     []cliOption{bareJSON},
				build: func(m cliMatches) cliCommand {
					return fleetCommandDelete{fleetID: m.int64("FLEET_ID"), json: m.flag("json")}
				},
			},
		},
	}
}

func cliMemberSpec() *cliSpec {
	memberID := func(help string) cliPositional {
		return cliPositional{name: "MEMBER_ID", help: help, required: true, maxValues: 1, check: checkI64}
	}
	return &cliSpec{
		name:  "member",
		about: "Member lifecycle and pane interaction",
		subcommands: []*cliSpec{
			{
				name:  "create",
				about: "Register a member and spawn its coding-agent pane",
				positionals: []cliPositional{
					{name: "PROMPT", maxValues: 1,
						help: "Inline spawn prompt (backend-neutral template). Exactly one of PROMPT / --file"},
				},
				options: []cliOption{
					{long: "fleet-id", value: "INT", required: true, check: checkI64, help: "The fleet the new member joins"},
					{long: "name", value: "NAME", required: true, help: "Display name"},
					{long: "description", value: "DESCRIPTION", required: true, help: "One-sentence purpose"},
					{long: "coding-agent", value: "CODING_AGENT", choices: codingAgentNames(),
						help: "Backend binary to spawn / record [default: inherits the Director's backend]"},
					{long: "model", value: "MODEL", help: "Model passed to the backend binary"},
					{long: "effort", value: "EFFORT", help: "Reasoning-effort level, validated against the resolved backend's levels"},
					{long: "permission-mode", value: "MODE", help: codingAgentPermissionModeHelp()},
					{long: "file", value: "PATH", help: "UTF-8 file whose contents are the spawn prompt (`-` = stdin)"},
					jsonFlag(),
				},
				groups: []cliArgGroup{{members: []string{"PROMPT", "file"}, required: true}},
				build: func(m cliMatches) cliCommand {
					return memberCommandCreate{createArgs{
						fleetID:        m.int64("fleet-id"),
						name:           m.value("name"),
						description:    m.value("description"),
						codingAgent:    m.optional("coding-agent"),
						model:          m.optional("model"),
						effort:         m.optional("effort"),
						permissionMode: m.optional("permission-mode"),
						body:           promptArgs{prompt: m.optional("PROMPT"), file: m.optional("file")},
						json:           m.flag("json"),
					}}
				},
			},
			{
				name:        "delete",
				about:       "Tear down a member's pane (when one exists) and deregister it",
				positionals: []cliPositional{memberID("The member to delete")},
				options:     []cliOption{jsonFlag()},
				build: func(m cliMatches) cliCommand {
					return memberCommandDelete{memberID: m.int64("MEMBER_ID"), json: m.flag("json")}
				},
			},
			{
				name:        "show",
				about:       "Show one member's detail",
				positionals: []cliPositional{memberID("The member to show")},
				options:     []cliOption{jsonFlag()},
				build: func(m cliMatches) cliCommand {
					return memberCommandShow{memberID: m.int64("MEMBER_ID"), json: m.flag("json")}
				},
			},
			{
				name:  "list",
				about: "List every active registry entry of the fleet",
				positionals: []cliPositional{
					{name: "FLEET_ID", help: "The fleet whose roster is listed", required: true, maxValues: 1, check: checkI64},
				},
				options: []cliOption{jsonFlag()},
				build: func(m cliMatches) cliCommand {
					return memberCommandList{fleetID: m.int64("FLEET_ID"), json: m.flag("json")}
				},
			},
			{
				name:  "prompt",
				about: "Keystroke a prompt into a member's pane",
				positionals: []cliPositional{
					memberID("The target member"),
					{name: "TEXT", help: "Single line of text to dispatch", required: true, maxValues: 1},
				},
				options: []cliOption{directorOption(checkPositiveI64), jsonFlag()},
				build: func(m cliMatches) cliCommand {
					return memberCommandPrompt{
						memberID:         m.int64("MEMBER_ID"),
						directorMemberID: m.int64("from-member-id"),
						text:             m.value("TEXT"),
						json:             m.flag("json"),
					}
				},
			},
			{
				name:          "exec",
				about:         "Submit a native shell command through the member's runner, which uses its measured submission plan, delivery fallback or queue",
				overrideUsage: "cafleet member exec [OPTIONS] <MEMBER_ID> <COMMAND|--file <PATH>>",
				positionals: []cliPositional{
					memberID("The target member"),
					{name: "COMMAND", help: "The command to run. Exactly one of COMMAND / --file", maxValues: 1},
				},
				options: []cliOption{
					directorOption(checkPositiveI64),
					{long: "file", value: "PATH", help: "UTF-8 file whose contents are the command (`-` = stdin)"},
					jsonFlag(),
				},
				groups: []cliArgGroup{{members: []string{"COMMAND", "file"}, required: true}},
				build: func(m cliMatches) cliCommand {
					return memberCommandExec{
						memberID:         m.int64("MEMBER_ID"),
						directorMemberID: m.int64("from-member-id"),
						body:             execBodyArgs{command: m.optional("COMMAND"), file: m.optional("file")},
						json:             m.flag("json"),
					}
				},
			},
			{
				name:  "press",
				about: "Send an ordered vector of explicit native keys to a member pane",
				positionals: []cliPositional{
					{name: "MEMBER_ID", help: "The target member", required: true, maxValues: 1, check: checkPositiveI64},
					{name: "KEY", help: "One to 64 keys sent in order, with no implicit Escape or Enter", required: true, maxValues: 64},
				},
				options: []cliOption{directorOption(checkPositiveI64), jsonFlag()},
				build: func(m cliMatches) cliCommand {
					return memberCommandPress{
						memberID:         m.int64("MEMBER_ID"),
						directorMemberID: m.int64("from-member-id"),
						keys:             m.values["KEY"],
						json:             m.flag("json"),
					}
				},
			},
			{
				name:        "capture",
				about:       "Capture the tail of a member's pane",
				positionals: []cliPositional{memberID("The target member")},
				options: []cliOption{
					{long: "lines", value: "LINES", defaultValue: "20", check: checkI64, help: "Number of trailing lines to capture"},
					{long: "ansi", help: "Emit the raw capture, ANSI escapes preserved"},
					jsonFlag(),
				},
				build: func(m cliMatches) cliCommand {
					return memberCommandCapture{
						memberID: m.int64("MEMBER_ID"),
						lines:    m.int64("lines"),
						ansi:     m.flag("ansi"),
						json:     m.flag("json"),
					}
				},
			},
		},
	}
}

func cliMessageSpec() *cliSpec {
	return &cliSpec{
		name:  "message",
		about: "Durable point-to-point messages",
		subcommands: []*cliSpec{
			{
				name:  "send",
				about: "Send a unicast message",
				positionals: []cliPositional{
					{name: "TEXT", help: "Inline message body. Exactly one of TEXT / --file", maxValues: 1},
				},
				options: []cliOption{
					{long: "from-member-id", value: "FROM_MEMBER_ID", required: true, check: checkI64, help: "Sender's member ID"},
					{long: "to-member-id", value: "TO_MEMBER_ID", required: true, check: checkI64, help: "Recipient member ID"},
					{long: "file", value: "PATH", help: "UTF-8 file carrying the body (`-` = stdin)"},
					jsonFlag(),
				},
				groups: []cliArgGroup{{members: []string{"TEXT", "file"}, required: true}},
				build: func(m cliMatches) cliCommand {
					return messageCommandSend{
						fromMemberID: m.int64("from-member-id"),
						toMemberID:   m.int64("to-member-id"),
						body:         bodyArgs{text: m.optional("TEXT"), file: m.optional("file")},
						json:         m.flag("json"),
					}
				},
			},
			{
				name:  "poll",
				about: "Fetch and consume incoming messages",
				positionals: []cliPositional{
					{name: "MEMBER_ID", help: "Recipient whose inbox is fetched", required: true, maxValues: 1, check: checkI64},
				},
				options: []cliOption{jsonFlag()},
				build: func(m cliMatches) cliCommand {
					return messageCommandPoll{memberID: m.int64("MEMBER_ID"), json: m.flag("json")}
				},
			},
			{
				name:  "show",
				about: "Show one message",
				positionals: []cliPositional{
					{name: "MESSAGE_ID", help: "Message to fetch", required: true, maxValues: 1, check: checkI64},
				},
				options: []cliOption{jsonFlag()},
				build: func(m cliMatches) cliCommand {
					return messageCommandShow{messageID: m.int64("MESSAGE_ID"), json: m.flag("json")}
				},
			},
		},
	}
}

func cliEventSpec() *cliSpec {
	return &cliSpec{
		name:  "event",
		about: "Read a durable runtime event and current state",
		subcommands: []*cliSpec{
			{
				name:  "show",
				about: "Read a durable runtime event and its current run/interaction state",
				positionals: []cliPositional{
					{name: "EVENT_ID", help: "The runtime event to read", required: true, maxValues: 1, check: checkPositiveID},
				},
				options: []cliOption{jsonFlag()},
				build: func(m cliMatches) cliCommand {
					return eventCommandShow{eventID: m.int64("EVENT_ID"), json: m.flag("json")}
				},
			},
		},
	}
}

func cliInputSpec() *cliSpec {
	return &cliSpec{
		name:  "input",
		about: "Read / settle a durable input receipt",
		subcommands: []*cliSpec{
			{
				name:  "show",
				about: "Read a durable programmatic-input receipt",
				positionals: []cliPositional{
					{name: "INPUT_ID", help: "The programmatic-input receipt to read", required: true, maxValues: 1, check: checkPositiveID},
				},
				options: []cliOption{jsonFlag()},
				build: func(m cliMatches) cliCommand {
					return inputCommandShow{inputID: m.int64("INPUT_ID"), json: m.flag("json")}
				},
			},
			{
				name:  "reconcile",
				about: "Record an evidence-based input disposition under synchronized ownership",
				positionals: []cliPositional{
					{name: "INPUT_ID", help: "The programmatic-input receipt to settle", required: true, maxValues: 1, check: checkPositiveID},
				},
				options: []cliOption{
					directorOption(checkPositiveI64),
					{long: "disposition", value: "DISPOSITION", required: true, choices: dispositionValues,
						help: "The observed input disposition to record"},
					{long: "evidence-ref", value: "EVIDENCE_REF", required: true, check: checkNonblank("a nonempty evidence reference is required"),
						help: "Reference to the Director's recorded evidence for the disposition"},
					jsonFlag(),
				},
				build: func(m cliMatches) cliCommand {
					return inputCommandReconcile{
						inputID:          m.int64("INPUT_ID"),
						directorMemberID: m.int64("from-member-id"),
						disposition:      dispositionFrom(m.value("disposition")),
						evidenceRef:      m.value("evidence-ref"),
						json:             m.flag("json"),
					}
				},
			},
		},
	}
}

func cliPermissionSpec() *cliSpec {
	interaction := cliPositional{name: "INTERACTION_ID", help: "The interaction carrying the request", required: true, maxValues: 1, check: checkPositiveID}
	eventID := cliOption{long: "event-id", value: "EVENT_ID", required: true, check: checkPositiveID, help: "The runtime event delivered to the Director's inbox"}
	director := cliOption{long: "from-member-id", value: "DIRECTOR_MEMBER_ID", required: true, check: checkPositiveID, help: "The pinned Director's member ID"}
	return &cliSpec{
		name:  "permission",
		about: "Inspect / answer the current native permission request",
		subcommands: []*cliSpec{
			{
				name:        "inspect",
				about:       "Read the current native permission request and its answer capability",
				positionals: []cliPositional{interaction},
				options:     []cliOption{eventID, director, jsonFlag()},
				build: func(m cliMatches) cliCommand {
					return permissionCommandInspect{
						interactionID:    m.int64("INTERACTION_ID"),
						eventID:          m.int64("event-id"),
						directorMemberID: m.int64("from-member-id"),
						json:             m.flag("json"),
					}
				},
			},
			{
				name:        "respond",
				about:       "Answer the inspected native permission request with a semantic decision, or the inspected question with a tested choice",
				positionals: []cliPositional{interaction},
				options: []cliOption{
					eventID,
					director,
					{long: "expected-run-id", value: "EXPECTED_RUN_ID", required: true, check: checkPositiveID, help: "The run ID reported by inspection"},
					{long: "expected-owner-epoch", value: "EXPECTED_OWNER_EPOCH", required: true, check: checkPositiveID, help: "The owner epoch reported by inspection"},
					{long: "request-token", value: "REQUEST_TOKEN", required: true, check: validateRequestToken, help: "The request token reported by inspection"},
					{long: "decision", value: "DECISION", choices: permissionDecisionValues, help: "The semantic answer to a permission request"},
					{long: "choice", value: "CHOICE", check: checkChoiceID, help: "The tested choice that answers a question"},
					jsonFlag(),
				},
				groups: []cliArgGroup{{members: []string{"decision", "choice"}, required: true}},
				build: func(m cliMatches) cliCommand {
					return permissionCommandRespond{
						interactionID:      m.int64("INTERACTION_ID"),
						eventID:            m.int64("event-id"),
						directorMemberID:   m.int64("from-member-id"),
						expectedRunID:      m.int64("expected-run-id"),
						expectedOwnerEpoch: m.int64("expected-owner-epoch"),
						requestToken:       m.value("request-token"),
						decision:           m.optional("decision"),
						choice:             m.optional("choice"),
						json:               m.flag("json"),
					}
				},
			},
		},
	}
}

// parseI64 parses a decimal i64 as Rust's `str::parse` does, reporting its error texts.
func parseI64(value string) (int64, error) {
	if value == "" {
		return 0, usageError("cannot parse integer from empty string")
	}
	digits, negative := value, false
	switch value[0] {
	case '+', '-':
		if len(value) == 1 {
			return 0, usageError("invalid digit found in string")
		}
		digits, negative = value[1:], value[0] == '-'
	}
	var result int64
	for i := range len(digits) {
		digit := int64(digits[i]) - '0'
		if digit < 0 || digit > 9 {
			return 0, usageError("invalid digit found in string")
		}
		if negative {
			if result < (minInt64+digit)/10 {
				return 0, usageError("number too small to fit in target type")
			}
			result = result*10 - digit
		} else {
			if result > (maxInt64-digit)/10 {
				return 0, usageError("number too large to fit in target type")
			}
			result = result*10 + digit
		}
	}
	return result, nil
}

const (
	maxInt64 = int64(^uint64(0) >> 1)
	minInt64 = -maxInt64 - 1
)

func checkI64(value string) error {
	_, err := parseI64(value)
	return err
}

func checkPositiveI64(value string) error {
	id, err := parseI64(value)
	if err != nil {
		return err
	}
	if id < 1 {
		return usageErrorf("%d is not in 1..%d", id, maxInt64)
	}
	return nil
}

func checkPositiveID(value string) error {
	if id, err := parseI64(value); err != nil || id < 1 {
		return usageError("a positive assigned ID is required")
	}
	return nil
}

func checkChoiceID(value string) error {
	if !validChoiceID(value) {
		return usageError("a choice id is 1-64 bytes of lowercase hyphen-separated words")
	}
	return nil
}

func checkNonblank(message string) cliValueCheck {
	return func(value string) error {
		if strings.TrimSpace(value) == "" {
			return usageError(message)
		}
		return nil
	}
}

// cliMatches holds the raw values of the arguments one leaf parse accepted, keyed by option
// long name or positional name.
type cliMatches struct {
	values map[string][]string
	// seen records each argument's first appearance, for conflict ordering.
	seen []string
}

func (m cliMatches) has(id string) bool {
	_, ok := m.values[id]
	return ok
}

func (m cliMatches) flag(id string) bool {
	return m.has(id)
}

func (m cliMatches) value(id string) string {
	values, ok := m.values[id]
	if !ok {
		panic("the parser requires argument " + id)
	}
	return values[0]
}

func (m cliMatches) optional(id string) opt[string] {
	if values, ok := m.values[id]; ok {
		return some(values[0])
	}
	return none[string]()
}

func (m cliMatches) int64(id string) int64 {
	parsed, err := parseI64(m.value(id))
	if err != nil {
		panic("the parser validated integer argument " + id)
	}
	return parsed
}

type cliParseKind uint8

const (
	cliParseHelp cliParseKind = iota + 1
	cliParseVersion
	// A group invoked without a subcommand prints its help to stderr.
	cliParseMissingSubcommand
	cliParseUnknownArgument
	cliParseUnknownSubcommand
	cliParseMissingValue
	cliParseUnexpectedValue
	cliParseInvalidValue
	cliParseRepeatedArgument
	cliParseConflict
	cliParseMissingRequired
)

// cliParseExit is a parse that ends the invocation before dispatch: help or version output, or
// a parse error.
type cliParseExit struct {
	kind cliParseKind
	text string
}

func (e *cliParseExit) exitCode() int {
	switch e.kind {
	case cliParseHelp, cliParseVersion:
		return 0
	case cliParseMissingSubcommand,
		cliParseUnknownArgument,
		cliParseUnknownSubcommand,
		cliParseMissingValue,
		cliParseUnexpectedValue,
		cliParseInvalidValue,
		cliParseRepeatedArgument,
		cliParseConflict,
		cliParseMissingRequired:
		return 2
	default:
		panic("invalid cliParseKind")
	}
}

func (e *cliParseExit) write() {
	stream := os.Stderr
	if e.exitCode() == 0 {
		stream = os.Stdout
	}
	// The exit code of help, version and parse-error output does not depend on whether the
	// write succeeded, as with the Rust binary.
	_, _ = fmt.Fprintln(stream, e.text)
}

// cliArgsParseFrom parses argv, whose first element is the program name.
func cliArgsParseFrom(argv []string) (cliArgs, *cliParseExit) {
	if len(argv) == 0 {
		panic("argv carries the program name")
	}
	parser := cliParser{}
	command, exit := parser.group(cliRoot(), []string{"cafleet"}, argv[1:])
	if exit != nil {
		return cliArgs{}, exit
	}
	return cliArgs{databaseURL: parser.databaseURL, command: command}, nil
}

type cliParser struct {
	databaseURL opt[string]
}

func (p *cliParser) group(spec *cliSpec, path []string, args []string) (cliCommand, *cliParseExit) {
	usage := cliUsage(spec, path)
	levelURL := false
	for i := 0; i < len(args); i++ {
		arg := args[i]
		switch {
		case arg == "--":
			if i+1 == len(args) {
				return nil, &cliParseExit{kind: cliParseMissingSubcommand, text: cliHelp(spec, path)}
			}
			return nil, cliError(cliParseUnknownArgument, usage, "unexpected argument '%s' found", args[i+1])
		case strings.HasPrefix(arg, "--"):
			name, value, attached := strings.Cut(arg[2:], "=")
			switch {
			case name == "help":
				return nil, &cliParseExit{kind: cliParseHelp, text: cliHelp(spec, path)}
			case name == "version" && len(path) == 1:
				return nil, &cliParseExit{kind: cliParseVersion, text: "cafleet " + Version}
			case name == "database-url":
				if levelURL {
					return nil, cliError(cliParseRepeatedArgument, usage, "the argument '%s' cannot be used multiple times", cliOptionLabel(cliDatabaseURLOption))
				}
				if !attached {
					if i+1 == len(args) || strings.HasPrefix(args[i+1], "-") {
						return nil, cliMissingValue(cliDatabaseURLOption, usage)
					}
					i++
					value = args[i]
				}
				levelURL = true
				p.databaseURL = some(value)
			default:
				return nil, cliError(cliParseUnknownArgument, usage, "unexpected argument '--%s' found", name)
			}
		case strings.HasPrefix(arg, "-") && arg != "-":
			if exit := cliShortFlags(spec, path, arg, len(path) == 1); exit != nil {
				return nil, exit
			}
		case arg == "help":
			return nil, cliHelpCommand(spec, path, args[i+1:])
		default:
			child := spec.subcommand(arg)
			if child == nil {
				return nil, cliError(cliParseUnknownSubcommand, usage, "unrecognized subcommand '%s'", arg)
			}
			childPath := append(slices.Clone(path), child.name)
			if len(child.subcommands) > 0 {
				return p.group(child, childPath, args[i+1:])
			}
			return p.leaf(child, childPath, args[i+1:])
		}
	}
	return nil, &cliParseExit{kind: cliParseMissingSubcommand, text: cliHelp(spec, path)}
}

func (s *cliSpec) subcommand(name string) *cliSpec {
	for _, child := range s.subcommands {
		if child.name == name {
			return child
		}
	}
	return nil
}

// cliShortFlags handles a short-flag cluster: `-h` prints help, `-V` prints the root version,
// and any other short flag is unknown.
func cliShortFlags(spec *cliSpec, path []string, arg string, root bool) *cliParseExit {
	for _, flag := range arg[1:] {
		switch {
		case flag == 'h':
			return &cliParseExit{kind: cliParseHelp, text: cliHelp(spec, path)}
		case flag == 'V' && root:
			return &cliParseExit{kind: cliParseVersion, text: "cafleet " + Version}
		default:
			return cliError(cliParseUnknownArgument, cliUsage(spec, path), "unexpected argument '-%c' found", flag)
		}
	}
	panic("a short-flag cluster has at least one flag")
}

// cliHelpCommand renders `help [COMMAND]...` below spec.
func cliHelpCommand(spec *cliSpec, path []string, names []string) *cliParseExit {
	for _, name := range names {
		if name == "help" && len(spec.subcommands) > 0 {
			return &cliParseExit{kind: cliParseHelp, text: cliHelpOfHelp(path)}
		}
		child := spec.subcommand(name)
		if child == nil {
			return cliError(cliParseUnknownSubcommand, cliUsage(spec, path), "unrecognized subcommand '%s'", name)
		}
		spec, path = child, append(slices.Clone(path), child.name)
	}
	return &cliParseExit{kind: cliParseHelp, text: cliHelp(spec, path)}
}

func cliHelpOfHelp(path []string) string {
	return "Print this message or the help of the given subcommand(s)\n\nUsage: " + strings.Join(path, " ") +
		" help [COMMAND]...\n\nArguments:\n  [COMMAND]...  Print help for the subcommand(s)"
}

func (s *cliSpec) option(long string) (cliOption, bool) {
	if long == cliDatabaseURLOption.long {
		return cliDatabaseURLOption, true
	}
	for _, option := range s.options {
		if option.long == long {
			return option, true
		}
	}
	return cliOption{}, false
}

func (s *cliSpec) positional(name string) (cliPositional, bool) {
	for _, positional := range s.positionals {
		if positional.name == name {
			return positional, true
		}
	}
	return cliPositional{}, false
}

// leaf parses a leaf command's arguments in order: unknown arguments, missing and unexpected
// values, repeats and invalid values as each token is met; then conflicts; then missing
// required arguments.
func (p *cliParser) leaf(spec *cliSpec, path []string, args []string) (cliCommand, *cliParseExit) {
	usage := cliUsage(spec, path)
	matches := cliMatches{values: map[string][]string{}}
	record := func(id string, value string) {
		if _, ok := matches.values[id]; !ok {
			matches.seen = append(matches.seen, id)
		}
		matches.values[id] = append(matches.values[id], value)
	}
	// Positional values wait in a batch, one contiguous run for one positional, and are
	// validated when the run ends: at a recognized option, at the next positional, or at the end
	// of the arguments. An unknown argument is reported first.
	batchSlot, batch := 0, []string(nil)
	resolveBatch := func() *cliParseExit {
		if len(batch) == 0 {
			return nil
		}
		positional := spec.positionals[batchSlot]
		label := cliPositionalLabel(positional)
		for _, value := range batch {
			if exit := cliCheckValue(positional.check, nil, value, label, usage); exit != nil {
				return exit
			}
		}
		if len(batch) > positional.maxValues {
			return cliError(cliParseUnexpectedValue, usage, "unexpected value '%s' for '%s' found; no more were expected", batch[len(batch)-1], label)
		}
		for _, value := range batch {
			record(positional.name, value)
		}
		batch = nil
		return nil
	}
	taken := func(slot int) int {
		count := len(matches.values[spec.positionals[slot].name])
		if slot == batchSlot {
			count += len(batch)
		}
		return count
	}
	slot := 0
	escaped := false
	for i := 0; i < len(args); i++ {
		arg := args[i]
		switch {
		case escaped || arg == "-" || !strings.HasPrefix(arg, "-"):
			for slot < len(spec.positionals) && spec.positionals[slot].maxValues == 1 && taken(slot) == 1 {
				slot++
			}
			if slot == len(spec.positionals) {
				return nil, cliError(cliParseUnknownArgument, usage, "unexpected argument '%s' found", arg)
			}
			if slot != batchSlot {
				if exit := resolveBatch(); exit != nil {
					return nil, exit
				}
				batchSlot = slot
			}
			batch = append(batch, arg)
		case arg == "--":
			escaped = true
		case strings.HasPrefix(arg, "--"):
			name, value, attached := strings.Cut(arg[2:], "=")
			option, ok := spec.option(name)
			if !ok && name != "help" {
				return nil, cliError(cliParseUnknownArgument, usage, "unexpected argument '--%s' found", name)
			}
			if exit := resolveBatch(); exit != nil {
				return nil, exit
			}
			if name == "help" {
				return nil, &cliParseExit{kind: cliParseHelp, text: cliHelp(spec, path)}
			}
			if option.value == "" {
				if attached {
					return nil, cliError(cliParseUnexpectedValue, usage, "unexpected value '%s' for '--%s' found; no more were expected", value, name)
				}
				if matches.has(name) {
					return nil, cliError(cliParseRepeatedArgument, usage, "the argument '--%s' cannot be used multiple times", name)
				}
				record(name, "")
				continue
			}
			values := []string{value}
			if !attached {
				next := i + 1
				if next == len(args) || strings.HasPrefix(args[next], "--") {
					return nil, cliMissingValue(option, usage)
				}
				if args[next] != "-" && strings.HasPrefix(args[next], "-") {
					// A short-flag token is never a value; the next iteration rejects it.
					continue
				}
				values = []string{args[next]}
				i = next
				for option.multiple && i+1 < len(args) && (args[i+1] == "-" || !strings.HasPrefix(args[i+1], "-")) {
					i++
					values = append(values, args[i])
				}
			}
			if matches.has(name) && !option.multiple {
				return nil, cliError(cliParseRepeatedArgument, usage, "the argument '%s' cannot be used multiple times", cliOptionLabel(option))
			}
			for _, value := range values {
				if exit := cliCheckValue(option.check, option.choices, value, cliOptionLabel(option), usage); exit != nil {
					return nil, exit
				}
				if name == cliDatabaseURLOption.long {
					p.databaseURL = some(value)
				}
				record(name, value)
			}
		default:
			if arg[1] == 'h' {
				if exit := resolveBatch(); exit != nil {
					return nil, exit
				}
			}
			if exit := cliShortFlags(spec, path, arg, false); exit != nil {
				return nil, exit
			}
		}
	}
	if exit := resolveBatch(); exit != nil {
		return nil, exit
	}
	if exit := spec.checkConflicts(matches, usage); exit != nil {
		return nil, exit
	}
	if missing := spec.missingRequired(matches); len(missing) > 0 {
		return nil, cliError(cliParseMissingRequired, usage, "the following required arguments were not provided:\n  %s", strings.Join(missing, "\n  "))
	}
	for _, option := range spec.options {
		if option.defaultValue != "" && !matches.has(option.long) {
			matches.values[option.long] = []string{option.defaultValue}
		}
	}
	return spec.build(matches), nil
}

func cliCheckValue(check cliValueCheck, choices []string, value, label, usage string) *cliParseExit {
	if len(choices) > 0 && !slices.Contains(choices, value) {
		return cliError(cliParseInvalidValue, usage, "invalid value '%s' for '%s'\n  [possible values: %s]", value, label, strings.Join(choices, ", "))
	}
	if check == nil {
		return nil
	}
	if err := check(value); err != nil {
		return cliError(cliParseInvalidValue, usage, "invalid value '%s' for '%s': %s", value, label, cafleetErrorOf(err).message())
	}
	return nil
}

func cliMissingValue(option cliOption, usage string) *cliParseExit {
	message := fmt.Sprintf("a value is required for '%s' but none was supplied", cliOptionLabel(option))
	if len(option.choices) > 0 {
		message += "\n  [possible values: " + strings.Join(option.choices, ", ") + "]"
	}
	return cliError(cliParseMissingValue, usage, "%s", message)
}

func (s *cliSpec) checkConflicts(matches cliMatches, usage string) *cliParseExit {
	for _, group := range s.groups {
		var present []string
		for _, id := range matches.seen {
			if slices.Contains(group.members, id) {
				present = append(present, id)
			}
		}
		if len(present) > 1 {
			return cliError(cliParseConflict, usage, "the argument '%s' cannot be used with '%s'", s.argLabel(present[0]), s.argLabel(present[1]))
		}
	}
	return nil
}

// missingRequired lists the absent required arguments in usage order: options, the options an
// argument present requires, groups, then positionals.
func (s *cliSpec) missingRequired(matches cliMatches) []string {
	var missing []string
	for _, option := range s.options {
		if option.required && !matches.has(option.long) {
			missing = append(missing, cliOptionLabel(option))
		}
	}
	for _, option := range s.options {
		if option.requires != "" && matches.has(option.long) && !matches.has(option.requires) {
			required, ok := s.option(option.requires)
			if !ok {
				panic("a required companion option is declared: " + option.requires)
			}
			missing = append(missing, cliOptionLabel(required))
		}
	}
	for _, group := range s.groups {
		if group.required && !slices.ContainsFunc(group.members, matches.has) {
			missing = append(missing, s.groupLabel(group))
		}
	}
	for _, positional := range s.positionals {
		if positional.required && !matches.has(positional.name) {
			missing = append(missing, cliPositionalLabel(positional))
		}
	}
	return missing
}

func cliError(kind cliParseKind, usage, format string, args ...any) *cliParseExit {
	return &cliParseExit{
		kind: kind,
		text: "error: " + fmt.Sprintf(format, args...) + "\n\nUsage: " + usage + "\n\nFor more information, try '--help'.",
	}
}

func cliOptionLabel(option cliOption) string {
	switch {
	case option.value == "":
		return "--" + option.long
	case option.multiple:
		return "--" + option.long + " <" + option.value + ">..."
	default:
		return "--" + option.long + " <" + option.value + ">"
	}
}

func cliPositionalLabel(positional cliPositional) string {
	label := "[" + positional.name + "]"
	if positional.required {
		label = "<" + positional.name + ">"
	}
	if positional.maxValues > 1 {
		label += "..."
	}
	return label
}

func (s *cliSpec) argLabel(id string) string {
	if option, ok := s.option(id); ok {
		return cliOptionLabel(option)
	}
	positional, ok := s.positional(id)
	if !ok {
		panic("an argument id names a declared argument: " + id)
	}
	return cliPositionalLabel(positional)
}

func (s *cliSpec) groupLabel(group cliArgGroup) string {
	parts := make([]string, len(group.members))
	for i, id := range group.members {
		if option, ok := s.option(id); ok {
			parts[i] = cliOptionLabel(option)
		} else {
			parts[i] = id
		}
	}
	return "<" + strings.Join(parts, "|") + ">"
}

func (s *cliSpec) inGroup(id string) bool {
	return slices.ContainsFunc(s.groups, func(group cliArgGroup) bool {
		return slices.Contains(group.members, id)
	})
}

func cliUsage(spec *cliSpec, path []string) string {
	if spec.overrideUsage != "" {
		return spec.overrideUsage
	}
	parts := []string{strings.Join(path, " "), "[OPTIONS]"}
	if len(spec.subcommands) > 0 {
		return strings.Join(append(parts, "<COMMAND>"), " ")
	}
	for _, option := range spec.options {
		if option.required && !spec.inGroup(option.long) {
			parts = append(parts, cliOptionLabel(option))
		}
	}
	for _, group := range spec.groups {
		if group.required {
			parts = append(parts, spec.groupLabel(group))
		}
	}
	for _, positional := range spec.positionals {
		if positional.required && !spec.inGroup(positional.name) {
			parts = append(parts, cliPositionalLabel(positional))
		}
	}
	return strings.Join(parts, " ")
}

// cliHelpRow is one help entry: an optional short-flag column, the argument spec and its help.
type cliHelpRow struct {
	short string
	spec  string
	help  string
}

// Help rows move their help to the next line when the widest spec takes over 40% of this
// width and some help does not fit beside it.
const cliHelpWidth = 100

func cliHelpSection(title string, rows []cliHelpRow, shortColumn bool) string {
	longest := 0
	for _, row := range rows {
		longest = max(longest, utf8.RuneCountInString(row.spec))
	}
	taken := longest + 8
	nextLine := false
	for _, row := range rows {
		if taken <= cliHelpWidth && float64(taken)/cliHelpWidth > 0.40 && utf8.RuneCountInString(row.help) > cliHelpWidth-taken {
			nextLine = true
		}
	}
	var out strings.Builder
	out.WriteString(title + ":\n")
	for i, row := range rows {
		if i > 0 {
			out.WriteByte('\n')
		}
		out.WriteString("  ")
		if shortColumn {
			out.WriteString(cliShortColumn(row.short))
		}
		out.WriteString(row.spec)
		if nextLine {
			if row.help != "" {
				out.WriteString("\n          " + row.help)
			}
			continue
		}
		out.WriteString(strings.Repeat(" ", longest-utf8.RuneCountInString(row.spec)+2) + row.help)
	}
	return out.String()
}

func cliShortColumn(short string) string {
	if short == "" {
		return "    "
	}
	return "-" + short + ", "
}

func cliOptionHelp(option cliOption) string {
	extras := []string{}
	if option.help != "" {
		extras = append(extras, option.help)
	}
	if option.defaultValue != "" {
		extras = append(extras, "[default: "+option.defaultValue+"]")
	}
	if len(option.choices) > 0 {
		extras = append(extras, "[possible values: "+strings.Join(option.choices, ", ")+"]")
	}
	return strings.Join(extras, " ")
}

// cliHelpOptions orders a leaf's options for help: by declaration position with the global
// database URL at position zero, ties by long name, then help.
func cliHelpOptions(spec *cliSpec) []cliHelpRow {
	type keyed struct {
		position int
		option   cliOption
	}
	entries := []keyed{{0, cliDatabaseURLOption}}
	for i, option := range spec.options {
		entries = append(entries, keyed{i, option})
	}
	slices.SortStableFunc(entries, func(a, b keyed) int {
		if a.position != b.position {
			return a.position - b.position
		}
		return strings.Compare(a.option.long, b.option.long)
	})
	rows := make([]cliHelpRow, 0, len(entries)+1)
	for _, entry := range entries {
		rows = append(rows, cliHelpRow{spec: cliOptionLabel(entry.option), help: cliOptionHelp(entry.option)})
	}
	return append(rows, cliHelpRow{short: "h", spec: "--help", help: "Print help"})
}

func cliHelp(spec *cliSpec, path []string) string {
	var sections []string
	if spec.about != "" {
		sections = append(sections, spec.about)
	}
	sections = append(sections, "Usage: "+cliUsage(spec, path))
	options := []cliHelpRow{
		{spec: cliOptionLabel(cliDatabaseURLOption), help: cliDatabaseURLHelp},
		{short: "h", spec: "--help", help: "Print help"},
	}
	if len(spec.subcommands) > 0 {
		var commands []cliHelpRow
		for _, child := range spec.subcommands {
			if !child.hidden {
				commands = append(commands, cliHelpRow{spec: child.name, help: child.about})
			}
		}
		commands = append(commands, cliHelpRow{spec: "help", help: "Print this message or the help of the given subcommand(s)"})
		sections = append(sections, cliHelpSection("Commands", commands, false))
		if len(path) == 1 {
			options = append(options, cliHelpRow{short: "V", spec: "--version", help: "Print version"})
		}
	} else {
		if len(spec.positionals) > 0 {
			positionals := make([]cliHelpRow, len(spec.positionals))
			for i, positional := range spec.positionals {
				positionals[i] = cliHelpRow{spec: cliPositionalLabel(positional), help: positional.help}
			}
			sections = append(sections, cliHelpSection("Arguments", positionals, false))
		}
		options = cliHelpOptions(spec)
	}
	sections = append(sections, cliHelpSection("Options", options, true))
	return strings.Join(sections, "\n\n")
}

// directorOnly names a Director-only command: its name, the query from its subject to the
// subject's fleet, the subject id and its `--from-member-id`.
type directorOnly struct {
	name             string
	fleetQuery       string
	subject          int64
	directorMemberID int64
}

const cliMemberFleet = "SELECT fleet_id FROM members WHERE member_id=?1 AND status='active'"

// Recovery also reconciles a deregistered member's run, so its subject keeps its fleet.
const cliInputFleet = "SELECT r.fleet_id FROM member_input_operations i JOIN member_runs r ON r.run_id=i.run_id WHERE i.input_id=?1"

const cliInteractionFleet = "SELECT r.fleet_id FROM member_interactions i JOIN member_runs r ON r.run_id=i.run_id WHERE i.interaction_id=?1"

func directorOnlyOf(command cliCommand) (directorOnly, bool) {
	switch command := command.(type) {
	case memberCommandPrompt:
		return directorOnly{"member prompt", cliMemberFleet, command.memberID, command.directorMemberID}, true
	case memberCommandExec:
		return directorOnly{"member exec", cliMemberFleet, command.memberID, command.directorMemberID}, true
	case memberCommandPress:
		return directorOnly{"member press", cliMemberFleet, command.memberID, command.directorMemberID}, true
	case permissionCommandRespond:
		return directorOnly{"permission respond", cliInteractionFleet, command.interactionID, command.directorMemberID}, true
	case inputCommandReconcile:
		return directorOnly{"input reconcile", cliInputFleet, command.inputID, command.directorMemberID}, true
	case memberRunCommand, setupArgs, doctorArgs,
		fleetCommandCreate, fleetCommandList, fleetCommandShow, fleetCommandDelete,
		memberCommandCreate, memberCommandDelete, memberCommandShow, memberCommandList, memberCommandCapture,
		messageCommandSend, messageCommandPoll, messageCommandShow,
		eventCommandShow, inputCommandShow, permissionCommandInspect:
		return directorOnly{}, false
	default:
		panic("invalid cliCommand")
	}
}

// runArgs runs one invocation from argv and returns its exit code.
func runArgs(argv []string) int {
	args, exit := cliArgsParseFrom(argv)
	if exit != nil {
		exit.write()
		return exit.exitCode()
	}
	err := runParsed(args)
	if err == nil {
		return 0
	}
	failure := cafleetErrorOf(err)
	fmt.Fprintf(os.Stderr, "Error: %s\n", failure.message())
	return failure.exitCode()
}

func runParsed(args cliArgs) error {
	s, err := newSettings(args.databaseURL)
	if err != nil {
		return err
	}
	return dispatch(s, args.command)
}

// commandNotYetPorted is the failure of a command whose body is not ported yet.
func commandNotYetPorted() error {
	return appErrorf("cafleet %s command bodies are not yet ported to Go", Version)
}

// requireDirector requires a Director-only command's --from-member-id to be the root Director of
// the live fleet that owns its subject. An unknown subject passes, so the command reports its own
// not-found error.
func requireDirector(conn *dbConn, guarded directorOnly) error {
	fleet, err := queryRowOptional(conn, guarded.fleetQuery, []any{guarded.subject},
		func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		return err
	}
	fleetID, found := fleet.get()
	if !found {
		return nil
	}
	root, err := queryRowOptional(conn,
		"SELECT 1 FROM fleets WHERE fleet_id=?1 AND deleted_at IS NULL AND director_member_id=?2",
		[]any{fleetID, guarded.directorMemberID}, func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		return err
	}
	if _, isRoot := root.get(); isRoot {
		return nil
	}
	return appErrorf("'cafleet %s' is a Director-only command and member %d is not the root Director of fleet %d; send the request to your Director.",
		guarded.name, guarded.directorMemberID, fleetID)
}

// invocationConnection opens the selected database after the schema-version guard accepts it,
// first through a read-only preflight, so a refused file is never opened for writing.
func invocationConnection(ctx context.Context, databaseURL string, d deadline) (*dbConn, error) {
	missingDatabase := appError("no cafleet database; run 'cafleet setup'")
	open := func(readOnly bool) (*dbConn, error) {
		opened, err := openExisting(databaseURL, readOnly)
		if err != nil {
			return nil, err
		}
		conn, found := opened.get()
		if !found {
			return nil, missingDatabase
		}
		_, err = withDatabaseBudget(ctx, conn, d, func(_ context.Context, conn *dbConn) (struct{}, error) {
			return struct{}{}, classifySelectedSchema(conn, databaseURL).requireCurrent()
		})
		if err != nil {
			return nil, mergeCleanup(err, conn.close())
		}
		return conn, nil
	}
	preflight, err := open(true)
	if err != nil {
		return nil, err
	}
	if err := preflight.close(); err != nil {
		return nil, err
	}
	return open(false)
}

// dispatch runs a parsed command's body; a command whose body is not ported yet fails with
// commandNotYetPorted.
func dispatch(s settings, command cliCommand) (err error) {
	ctx := context.Background()
	switch command := command.(type) {
	case setupArgs:
		return setupRun(s, command)
	case doctorArgs, memberRunCommand, permissionCommandInspect:
		return commandNotYetPorted()
	case fleetCommandCreate, fleetCommandList, fleetCommandShow, fleetCommandDelete,
		memberCommandCreate, memberCommandDelete, memberCommandShow, memberCommandList,
		memberCommandPrompt, memberCommandExec, memberCommandPress, memberCommandCapture,
		messageCommandSend, messageCommandPoll, messageCommandShow,
		eventCommandShow, inputCommandShow, inputCommandReconcile, permissionCommandRespond:
	default:
		panic("invalid cliCommand")
	}
	conn, err := invocationConnection(ctx, s.databaseURL, newDeadline())
	if err != nil {
		return err
	}
	defer func() {
		err = mergeCleanup(err, conn.close())
	}()
	if guarded, ok := directorOnlyOf(command); ok {
		if err := requireDirector(conn, guarded); err != nil {
			return err
		}
	}
	if member, ok := command.(memberCommand); ok {
		if command, err = prepareInput(member); err != nil {
			return err
		}
	}
	switch command.(type) {
	case fleetCommandCreate, fleetCommandList, fleetCommandShow, fleetCommandDelete:
		return fleetRun(ctx, conn, command.(fleetCommand))
	case memberCommandCreate, memberCommandDelete, memberCommandShow, memberCommandList,
		memberCommandPrompt, memberCommandExec, memberCommandPress, memberCommandCapture:
		return memberRun(ctx, conn, command.(memberCommand))
	case messageCommandSend, messageCommandPoll, messageCommandShow:
		return messageRun(ctx, conn, command.(messageCommand))
	case eventCommandShow:
		return eventRun(ctx, conn, command.(eventCommand))
	case inputCommandShow, inputCommandReconcile:
		return inputRun(ctx, conn, command.(inputCommand))
	case permissionCommandRespond:
		return commandNotYetPorted()
	case setupArgs, doctorArgs, memberRunCommand, permissionCommandInspect:
		panic("a command without a guarded connection is dispatched before one is opened")
	default:
		panic("invalid cliCommand")
	}
}

// resolveBody reads the text body from its one source and requires it to be nonblank.
func resolveBody(inline, file opt[string]) (string, error) {
	body, err := readBody(inline, file)
	if err != nil {
		return "", err
	}
	blank := true
	for _, r := range body {
		if !unicode.IsSpace(r) && (r < 0x1c || r > 0x1f) {
			blank = false
			break
		}
	}
	if !blank {
		return body, nil
	}
	path, fromFile := file.get()
	switch {
	case !fromFile:
		return "", usageError("text may not be empty.")
	case path == "-":
		return "", appError("--file -: stdin is empty.")
	default:
		return "", appErrorf("--file %s: file is empty.", path)
	}
}

// resolveCommandBody reads a native command; a file source loses one terminal line ending.
func resolveCommandBody(inline, file opt[string]) (string, error) {
	body, err := readBody(inline, file)
	if err != nil {
		return "", err
	}
	if _, fromFile := file.get(); fromFile {
		if trimmed, ok := strings.CutSuffix(body, "\r\n"); ok {
			body = trimmed
		} else {
			body = strings.TrimSuffix(body, "\n")
		}
	}
	if !validInput(body) {
		return "", usageError("command must be one non-empty line without control characters.")
	}
	return body, nil
}

func readBody(inline, file opt[string]) (string, error) {
	text, hasText := inline.get()
	path, hasFile := file.get()
	switch {
	case hasText && !hasFile:
		return text, nil
	case hasFile && !hasText && path == "-":
		data, err := readStdin()
		if err != nil {
			return "", appErrorf("--file -: %s", ioErrorText(err))
		}
		if !utf8.Valid(data) {
			return "", appError("--file -: file is not valid UTF-8.")
		}
		return string(data), nil
	case hasFile && !hasText:
		data, err := os.ReadFile(path)
		if err != nil {
			message := fmt.Sprintf("--file %s: file is not readable.", path)
			if errors.Is(err, syscall.ENOENT) || errors.Is(err, syscall.EISDIR) {
				message = fmt.Sprintf("--file %s: file does not exist or is not a regular file.", path)
			}
			return "", appErrorf("%s (%s)", message, ioErrorText(err))
		}
		if !utf8.Valid(data) {
			return "", appErrorf("--file %s: file is not valid UTF-8.", path)
		}
		return string(data), nil
	default:
		panic("the parser's required argument group supplies exactly one body source")
	}
}

// emit writes the JSON payload or the text rendering as one output line.
func emit(json bool, payload any, text func() string) error {
	if json {
		return writeOutput(formatJSON(payload))
	}
	return writeOutput(text())
}

func writeOutput(line string) error {
	_, err := io.WriteString(os.Stdout, line+"\n")
	return outputResult(err)
}

// outputResult maps a stdout write result: a reader that closed the pipe has consumed all the
// output it wants, so a broken pipe ends the output without an error.
func outputResult(err error) error {
	switch {
	case err == nil, errors.Is(err, syscall.EPIPE):
		return nil
	default:
		return appErrorf("failed to write output: %s", ioErrorText(err))
	}
}
