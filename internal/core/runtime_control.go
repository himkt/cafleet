package core

import (
	"bytes"
	"context"
	"encoding/binary"
	"encoding/json/jsontext"
	"errors"
	"io"
	"math"
	"slices"
	"strconv"
	"strings"
	"syscall"
	"time"

	"golang.org/x/sys/unix"
)

const controlProtocolVersion uint32 = 1

const maxControlFrame = 65536

func validateVersion(frame jsontext.Value) error {
	version, isInteger := jsonUint64(jsonIndex(frame, "protocol_version"))
	return ensure(
		jsonIsObject(frame) && isInteger && version == uint64(controlProtocolVersion),
		"control frame requires integer protocol_version 1",
	)
}

func encodeControlFrame(frame jsontext.Value) ([]byte, error) {
	if err := validateVersion(frame); err != nil {
		return nil, err
	}
	return frameControlBody([]byte(jsonText(frame)))
}

func frameControlBody(body []byte) ([]byte, error) {
	if err := ensure(len(body) <= maxControlFrame, "control frame exceeds size limit"); err != nil {
		return nil, err
	}
	framed := binary.BigEndian.AppendUint32(make([]byte, 0, 4+len(body)), uint32(len(body))) //nolint:gosec // the length is at most maxControlFrame
	return append(framed, body...), nil
}

func decodeLifecycleControlBody(body []byte) (jsontext.Value, error) {
	frame, ok := jsonParse(string(body))
	if !ok {
		return nil, runtimeError("malformed control JSON body")
	}
	if err := validateVersion(frame); err != nil {
		return nil, err
	}
	return frame, nil
}

type admittedControlFrame struct {
	body       []byte
	admittedAt time.Time
}

type controlDecoder struct {
	header     [4]byte
	headerLen  int
	body       []byte
	expected   opt[int]
	admittedAt opt[time.Time]
}

func newControlDecoder() *controlDecoder {
	return &controlDecoder{}
}

func (d *controlDecoder) feedAdmitted(data []byte, observedAt time.Time) ([]admittedControlFrame, error) {
	frames := []admittedControlFrame{}
	for len(data) > 0 {
		if !d.expected.set {
			if d.headerLen == 0 {
				d.admittedAt = some(observedAt)
			}
			count := copy(d.header[d.headerLen:], data)
			d.headerLen += count
			data = data[count:]
			if d.headerLen < 4 {
				continue
			}
			length := int(binary.BigEndian.Uint32(d.header[:]))
			if err := ensure(length <= maxControlFrame, "control frame exceeds size limit"); err != nil {
				return nil, err
			}
			d.expected = some(length)
		}
		length, _ := d.expected.get()
		count := min(length-len(d.body), len(data))
		d.body = append(d.body, data[:count]...)
		data = data[count:]
		if len(d.body) == length {
			admittedAt, started := d.admittedAt.get()
			if !started {
				panic("a complete frame consumed its first header byte")
			}
			body := d.body
			if body == nil {
				body = []byte{}
			}
			frames = append(frames, admittedControlFrame{body: body, admittedAt: admittedAt})
			d.body = nil
			d.admittedAt = none[time.Time]()
			d.headerLen = 0
			d.expected = none[int]()
		}
	}
	return frames, nil
}

func (d *controlDecoder) finish() error {
	return ensure(d.headerLen == 0 && !d.expected.set && !d.admittedAt.set, "truncated control frame")
}

type submissionStatus uint8

const (
	submissionStatusAccepted submissionStatus = iota + 1
	submissionStatusQueued
	submissionStatusHeld
	submissionStatusRejected
	submissionStatusUncertain
)

func (s submissionStatus) asStr() string {
	switch s {
	case submissionStatusAccepted:
		return "accepted"
	case submissionStatusQueued:
		return "queued"
	case submissionStatusHeld:
		return "held"
	case submissionStatusRejected:
		return "rejected"
	case submissionStatusUncertain:
		return "uncertain"
	default:
		panic("invalid submissionStatus")
	}
}

func submissionStatusTryFrom(value string) (submissionStatus, error) {
	switch value {
	case "accepted":
		return submissionStatusAccepted, nil
	case "queued":
		return submissionStatusQueued, nil
	case "held":
		return submissionStatusHeld, nil
	case "rejected":
		return submissionStatusRejected, nil
	case "uncertain":
		return submissionStatusUncertain, nil
	default:
		return 0, invalidStoredValueError("submission.status", value)
	}
}

func (s submissionStatus) MarshalText() ([]byte, error) { return []byte(s.asStr()), nil }

func (s *submissionStatus) UnmarshalText(text []byte) error {
	parsed, err := submissionStatusTryFrom(string(text))
	if err != nil {
		return err
	}
	*s = parsed
	return nil
}

type submissionStage uint8

const (
	submissionStageZeroBytes submissionStage = iota + 1
	submissionStagePartial
	submissionStageSubmitted
	submissionStageUnknown
)

func (s submissionStage) asStr() string {
	switch s {
	case submissionStageZeroBytes:
		return "zero_bytes"
	case submissionStagePartial:
		return "partial"
	case submissionStageSubmitted:
		return "submitted"
	case submissionStageUnknown:
		return "unknown"
	default:
		panic("invalid submissionStage")
	}
}

func submissionStageTryFrom(value string) (submissionStage, error) {
	switch value {
	case "zero_bytes":
		return submissionStageZeroBytes, nil
	case "partial":
		return submissionStagePartial, nil
	case "submitted":
		return submissionStageSubmitted, nil
	case "unknown":
		return submissionStageUnknown, nil
	default:
		return 0, invalidStoredValueError("submission.stage", value)
	}
}

func (s submissionStage) MarshalText() ([]byte, error) { return []byte(s.asStr()), nil }

func (s *submissionStage) UnmarshalText(text []byte) error {
	parsed, err := submissionStageTryFrom(string(text))
	if err != nil {
		return err
	}
	*s = parsed
	return nil
}

type submissionResult struct {
	Status    submissionStatus        `json:"status"`
	InputID   opt[int64]              `json:"input_id"`
	Stage     submissionStage         `json:"stage"`
	OutputSeq opt[uint64]             `json:"output_seq"`
	Reason    opt[string]             `json:"reason"`
	Delivery  opt[deliveryProjection] `json:"delivery"`
}

func submissionResultRejectedBeforeWrite(reason string) submissionResult {
	return submissionResult{Status: submissionStatusRejected, Stage: submissionStageZeroBytes, Reason: some(reason)}
}

func submissionResultHeld(reason string) submissionResult {
	return submissionResult{
		Status: submissionStatusHeld,
		Stage:  submissionStageZeroBytes,
		Reason: some(reason),
		Delivery: some(deliveryProjection{
			Status: deliveryStatusHeld,
			Reason: some(reason),
		}),
	}
}

type permissionInspectInput struct {
	InteractionID    int64 `json:"interaction_id"`
	EventID          int64 `json:"event_id"`
	DirectorMemberID int64 `json:"director_member_id"`
}

type permissionRespondInput struct {
	InteractionID      int64
	EventID            int64
	DirectorMemberID   int64
	ExpectedRunID      int64
	ExpectedOwnerEpoch int64
	RequestToken       string
	Key                answerKey
}

func (i permissionRespondInput) inspection() permissionInspectInput {
	return permissionInspectInput{InteractionID: i.InteractionID, EventID: i.EventID, DirectorMemberID: i.DirectorMemberID}
}

// permissionResponseResult is a permission answer's reply. Its Submission carries the status,
// stage, output sequence and reason; a permission answer has no input receipt, so the
// submission's InputID and Delivery are absent.
type permissionResponseResult struct {
	Submission    submissionResult
	MemberID      int64
	RunID         int64
	OwnerEpoch    int64
	InteractionID int64
	EventID       int64
	Key           answerKey
}

// answerWire is the wire's separate decision and choice members, exactly one of them non-null.
type answerWire struct {
	Decision opt[permissionDecision] `json:"decision"`
	Choice   opt[string]             `json:"choice"`
}

func answerWireOf(key answerKey) answerWire {
	decision, isDecision := answerKeyDecisionOf(key)
	choice, isChoice := answerKeyChoiceOf(key)
	wire := answerWire{}
	if isDecision {
		wire.Decision = some(decision)
	}
	if isChoice {
		wire.Choice = some(choice)
	}
	return wire
}

func (w answerWire) key() (answerKey, error) {
	decision, isDecision := w.Decision.get()
	choice, isChoice := w.Choice.get()
	switch {
	case isDecision && !isChoice:
		return answerKeyDecision{decision: decision}, nil
	case isChoice && !isDecision:
		return answerKeyChoice{choice: choice}, nil
	default:
		return nil, runtimeError("control answer requires exactly one decision or choice")
	}
}

type missingEvidence uint8

// The declaration order is also the required reply order.
const (
	missingEvidenceOperation missingEvidence = iota + 1
	missingEvidenceArguments
	missingEvidenceCwd
	missingEvidenceEnvironment
	missingEvidenceTargets
	missingEvidenceEffects
	missingEvidenceChoiceScope
)

func (m missingEvidence) asStr() string {
	switch m {
	case missingEvidenceOperation:
		return "operation"
	case missingEvidenceArguments:
		return "arguments"
	case missingEvidenceCwd:
		return "cwd"
	case missingEvidenceEnvironment:
		return "environment"
	case missingEvidenceTargets:
		return "targets"
	case missingEvidenceEffects:
		return "effects"
	case missingEvidenceChoiceScope:
		return "choice_scope"
	default:
		panic("invalid missingEvidence")
	}
}

func missingEvidenceTryFrom(value string) (missingEvidence, error) {
	switch value {
	case "operation":
		return missingEvidenceOperation, nil
	case "arguments":
		return missingEvidenceArguments, nil
	case "cwd":
		return missingEvidenceCwd, nil
	case "environment":
		return missingEvidenceEnvironment, nil
	case "targets":
		return missingEvidenceTargets, nil
	case "effects":
		return missingEvidenceEffects, nil
	case "choice_scope":
		return missingEvidenceChoiceScope, nil
	default:
		return 0, invalidStoredValueError("missing_evidence", value)
	}
}

func (m missingEvidence) MarshalText() ([]byte, error) { return []byte(m.asStr()), nil }

func (m *missingEvidence) UnmarshalText(text []byte) error {
	parsed, err := missingEvidenceTryFrom(string(text))
	if err != nil {
		return err
	}
	*m = parsed
	return nil
}

type supportedChoice struct {
	ChoiceID string      `json:"choice_id"`
	Label    string      `json:"label"`
	Scope    choiceScope `json:"scope"`
}

type permissionInspection struct {
	EventID            int64                 `json:"event_id"`
	MessageID          int64                 `json:"message_id"`
	MemberID           int64                 `json:"member_id"`
	FleetID            int64                 `json:"fleet_id"`
	DirectorMemberID   int64                 `json:"director_member_id"`
	RunID              int64                 `json:"run_id"`
	OwnerEpoch         int64                 `json:"owner_epoch"`
	InteractionID      int64                 `json:"interaction_id"`
	EpisodeSeq         int64                 `json:"episode_seq"`
	Classification     opt[interactionClass] `json:"classification"`
	Visibility         visibility            `json:"visibility"`
	Backend            string                `json:"backend"`
	NativeVersion      opt[string]           `json:"native_version"`
	AnswerRuleID       opt[string]           `json:"answer_rule_id"`
	AnswerCapability   coverage              `json:"answer_capability"`
	CapabilityReason   opt[string]           `json:"capability_reason"`
	SupportedDecisions []permissionDecision  `json:"supported_decisions"`
	SupportedChoices   []supportedChoice     `json:"supported_choices"`
	RequestToken       opt[string]           `json:"request_token"`
	OutputSeq          uint64                `json:"output_seq"`
	Evidence           jsontext.Value        `json:"evidence"`
	MissingEvidence    []missingEvidence     `json:"missing_evidence"`
}

//sumtype:decl
type receiptAction interface{ isReceiptAction() }

type receiptQuery struct{ OperationKey string }

type receiptRetire struct{ InputID int64 }

type receiptReconcile struct {
	InputID     int64
	Expected    inputState
	Disposition inputDisposition
	EvidenceRef string
}

func (receiptQuery) isReceiptAction()     {}
func (receiptRetire) isReceiptAction()    {}
func (receiptReconcile) isReceiptAction() {}

//sumtype:decl
type inputControlOperation interface{ isInputControlOperation() }

type submitPromptOperation struct{ input submitInput }

type submitExecOperation struct{ input submitInput }

type permissionInspectOperation struct{ input permissionInspectInput }

type permissionRespondOperation struct{ input permissionRespondInput }

type receiptOperation struct{ action receiptAction }

func (submitPromptOperation) isInputControlOperation()      {}
func (submitExecOperation) isInputControlOperation()        {}
func (permissionInspectOperation) isInputControlOperation() {}
func (permissionRespondOperation) isInputControlOperation() {}
func (receiptOperation) isInputControlOperation()           {}

type inputControlRequest struct {
	Identity        controlIdentity
	ExpiresAtMonoNS uint64
	Operation       inputControlOperation
}

//sumtype:decl
type inputControlReply interface{ isInputControlReply() }

type receiptMissing struct{}

type receiptPresent struct {
	InputID    int64
	Submission submissionResult
}

type receiptRetired struct{ InputID int64 }

type receiptSettled struct{ InputID int64 }

func (submissionResult) isInputControlReply()         {}
func (permissionInspection) isInputControlReply()     {}
func (permissionResponseResult) isInputControlReply() {}
func (receiptMissing) isInputControlReply()           {}
func (receiptPresent) isInputControlReply()           {}
func (receiptRetired) isInputControlReply()           {}
func (receiptSettled) isInputControlReply()           {}

// decodeStrictJSON parses one JSON value that repeats no member name at any depth.
func decodeStrictJSON(body []byte) (jsontext.Value, error) {
	malformed := runtimeError("malformed or duplicate control JSON fields")
	decoder := jsontext.NewDecoder(bytes.NewReader(body))
	read, err := decoder.ReadValue()
	if err != nil {
		return nil, malformed
	}
	value := slices.Clone(read)
	if _, err := decoder.ReadToken(); err != io.EOF {
		return nil, malformed
	}
	if !jsonFiniteNumbers(value) {
		return nil, malformed
	}
	return value, nil
}

// jsonFiniteNumbers reports whether every number in value is within the float64 range.
func jsonFiniteNumbers(value jsontext.Value) bool {
	decoder := jsontext.NewDecoder(bytes.NewReader(value))
	for {
		token, err := decoder.ReadToken()
		if err == io.EOF {
			return true
		}
		if err != nil {
			panic("a parsed JSON value is valid: " + err.Error())
		}
		if token.Kind() != '0' {
			continue
		}
		if number, err := strconv.ParseFloat(token.String(), 64); err != nil && math.IsInf(number, 0) {
			return false
		}
	}
}

var inputControlKinds = []string{"submit_prompt", "submit_exec", "permission_inspect", "permission_respond", "receipt"}

// routesInput reports whether any `kind` member of the request object names an input
// operation.
func routesInput(body []byte) (bool, error) {
	malformed := runtimeError("malformed control request object")
	decoder := jsontext.NewDecoder(bytes.NewReader(body), jsontext.AllowDuplicateNames(true))
	if decoder.PeekKind() != '{' {
		return false, malformed
	}
	if _, err := decoder.ReadToken(); err != nil {
		return false, malformed
	}
	input := false
	for decoder.PeekKind() != '}' {
		name, err := decoder.ReadToken()
		if err != nil {
			return false, malformed
		}
		if name.String() != "kind" {
			if err := decoder.SkipValue(); err != nil {
				return false, malformed
			}
			continue
		}
		value, err := decoder.ReadValue()
		if err != nil {
			return false, malformed
		}
		if kind, isString := jsonString(value); isString && slices.Contains(inputControlKinds, kind) {
			input = true
		}
	}
	if _, err := decoder.ReadToken(); err != nil {
		return false, malformed
	}
	if _, err := decoder.ReadToken(); err != io.EOF {
		return false, malformed
	}
	return input, nil
}

// controlFrame is a control frame object: protocol_version beside the request or reply
// members.
func controlFrame(members jsonObject) jsonObject {
	return append(jsonObject{{"protocol_version", controlProtocolVersion}}, members...)
}

func public(projection jsonObject, field string) any {
	for _, member := range projection {
		if member.key == field {
			return member.value
		}
	}
	panic("a public projection has its " + field + " member")
}

func invalidControlFields() error {
	return runtimeError("invalid control fields")
}

// requireExactFields refuses a decoded control object whose re-encoding differs from the
// original, so an unknown, missing or defaulted member is refused like a wrongly typed one.
func requireExactFields(original jsontext.Value, encoded any) error {
	return ensure(jsonEqual(jsonValue(encoded), original), "control object requires its exact fields")
}

// decodeFlattened decodes the members of object that target names, ignoring every other member,
// as serde does for a flattened struct.
func decodeFlattened[T unknownFieldsIgnored](object jsontext.Value, target T) error {
	if err := decodeStrict(object, target); err != nil {
		return invalidControlFields()
	}
	return nil
}

func positive(ids ...int64) error {
	return ensure(!slices.ContainsFunc(ids, func(id int64) bool { return id <= 0 }), "positive control identity required")
}

func validateChoice(key answerKey) error {
	choice, isChoice := answerKeyChoiceOf(key)
	return ensure(!isChoice || validChoiceID(choice), "control choice requires the choice-id shape")
}

func (r inputControlRequest) validate() error {
	identity := r.Identity
	if validateIdentity(identity.DatabaseIdentityJSON) != nil {
		return runtimeError("invalid control database identity")
	}
	if brokerRuntimeHex(identity.LaunchNonce, 64, "launch_nonce") != nil {
		return runtimeError("invalid control launch nonce")
	}
	if err := positive(identity.FleetID, identity.MemberID, identity.DirectorMemberID, identity.RunID, identity.OwnerEpoch); err != nil {
		return err
	}
	if err := ensure(r.ExpiresAtMonoNS > 0, "positive input expiration required"); err != nil {
		return err
	}
	switch operation := r.Operation.(type) {
	case submitPromptOperation:
		return validateSubmitOperation(operation.input, inputKindPrompt)
	case submitExecOperation:
		return validateSubmitOperation(operation.input, inputKindExec)
	case permissionInspectOperation:
		input := operation.input
		if err := positive(input.InteractionID, input.EventID, input.DirectorMemberID); err != nil {
			return err
		}
		return ensure(input.DirectorMemberID == identity.DirectorMemberID, "control Director identity mismatch")
	case permissionRespondOperation:
		input := operation.input
		if err := validateChoice(input.Key); err != nil {
			return err
		}
		if err := ensure(
			input.DirectorMemberID == identity.DirectorMemberID &&
				input.ExpectedRunID == identity.RunID &&
				input.ExpectedOwnerEpoch == identity.OwnerEpoch,
			"control permission identity mismatch",
		); err != nil {
			return err
		}
		return validateRespond(input)
	case receiptOperation:
		switch action := operation.action.(type) {
		case receiptQuery:
			return validateOperationKey(action.OperationKey)
		case receiptRetire:
			return positive(action.InputID)
		case receiptReconcile:
			if err := validateEvidenceReference(action.EvidenceRef); err != nil {
				return err
			}
			return positive(action.InputID)
		default:
			panic("invalid receiptAction")
		}
	default:
		panic("invalid inputControlOperation")
	}
}

func validateSubmitOperation(input submitInput, kind inputKind) error {
	if err := validateOperationKey(input.OperationKey); err != nil {
		return err
	}
	if err := ensure(validInput(input.Literal), "valid single-line input literal required"); err != nil {
		return err
	}
	return ensure(input.InputKind == kind, "control input kind must match the operation")
}

func (r inputControlRequest) members() jsonObject {
	members := jsonObject{
		{"identity", r.Identity},
		{"expires_at_mono_ns", r.ExpiresAtMonoNS},
	}
	switch operation := r.Operation.(type) {
	case submitPromptOperation:
		return append(members, submitMembers("submit_prompt", operation.input)...)
	case submitExecOperation:
		return append(members, submitMembers("submit_exec", operation.input)...)
	case permissionInspectOperation:
		input := operation.input
		return append(members,
			jsonMember{"kind", "permission_inspect"},
			jsonMember{"interaction_id", input.InteractionID},
			jsonMember{"event_id", input.EventID},
			jsonMember{"director_member_id", input.DirectorMemberID},
		)
	case permissionRespondOperation:
		input := operation.input
		answer := answerWireOf(input.Key)
		return append(members,
			jsonMember{"kind", "permission_respond"},
			jsonMember{"interaction_id", input.InteractionID},
			jsonMember{"event_id", input.EventID},
			jsonMember{"director_member_id", input.DirectorMemberID},
			jsonMember{"expected_run_id", input.ExpectedRunID},
			jsonMember{"expected_owner_epoch", input.ExpectedOwnerEpoch},
			jsonMember{"request_token", input.RequestToken},
			jsonMember{"decision", answer.Decision},
			jsonMember{"choice", answer.Choice},
		)
	case receiptOperation:
		members = append(members, jsonMember{"kind", "receipt"})
		switch action := operation.action.(type) {
		case receiptQuery:
			return append(members, jsonMember{"action", "query"}, jsonMember{"operation_key", action.OperationKey})
		case receiptRetire:
			return append(members, jsonMember{"action", "retire"}, jsonMember{"input_id", action.InputID})
		case receiptReconcile:
			return append(members,
				jsonMember{"action", "reconcile"},
				jsonMember{"input_id", action.InputID},
				jsonMember{"expected_state", action.Expected},
				jsonMember{"disposition", action.Disposition},
				jsonMember{"evidence_ref", action.EvidenceRef},
			)
		default:
			panic("invalid receiptAction")
		}
	default:
		panic("invalid inputControlOperation")
	}
}

func submitMembers(kind string, input submitInput) jsonObject {
	return jsonObject{
		{"kind", kind},
		{"operation_key", input.OperationKey},
		{"input_kind", input.InputKind},
		{"literal", input.Literal},
	}
}

type inputRequestHeaderWire struct {
	Identity        controlIdentity `json:"identity"`
	ExpiresAtMonoNS uint64          `json:"expires_at_mono_ns"`
	Kind            string          `json:"kind"`
}

type submitInputWire struct {
	OperationKey string    `json:"operation_key"`
	InputKind    inputKind `json:"input_kind"`
	Literal      string    `json:"literal"`
}

type permissionInspectInputWire struct {
	InteractionID    int64 `json:"interaction_id"`
	EventID          int64 `json:"event_id"`
	DirectorMemberID int64 `json:"director_member_id"`
}

type permissionRespondInputWire struct {
	InteractionID      int64                   `json:"interaction_id"`
	EventID            int64                   `json:"event_id"`
	DirectorMemberID   int64                   `json:"director_member_id"`
	ExpectedRunID      int64                   `json:"expected_run_id"`
	ExpectedOwnerEpoch int64                   `json:"expected_owner_epoch"`
	RequestToken       string                  `json:"request_token"`
	Decision           opt[permissionDecision] `json:"decision"`
	Choice             opt[string]             `json:"choice"`
}

type receiptActionTagWire struct {
	Action string `json:"action"`
}

type receiptQueryWire struct {
	OperationKey string `json:"operation_key"`
}

type receiptRetireWire struct {
	InputID int64 `json:"input_id"`
}

type receiptReconcileWire struct {
	InputID     int64            `json:"input_id"`
	Expected    inputState       `json:"expected_state"`
	Disposition inputDisposition `json:"disposition"`
	EvidenceRef string           `json:"evidence_ref"`
}

func (*inputRequestHeaderWire) ignoresUnknownFields()     {}
func (*submitInputWire) ignoresUnknownFields()            {}
func (*permissionInspectInputWire) ignoresUnknownFields() {}
func (*permissionRespondInputWire) ignoresUnknownFields() {}
func (*receiptActionTagWire) ignoresUnknownFields()       {}
func (*receiptQueryWire) ignoresUnknownFields()           {}
func (*receiptRetireWire) ignoresUnknownFields()          {}
func (*receiptReconcileWire) ignoresUnknownFields()       {}

func decodeInputOperation(value jsontext.Value, kind string) (inputControlOperation, error) {
	switch kind {
	case "submit_prompt", "submit_exec":
		var wire submitInputWire
		if err := decodeFlattened(value, &wire); err != nil {
			return nil, err
		}
		input := submitInput(wire)
		if kind == "submit_prompt" {
			return submitPromptOperation{input: input}, nil
		}
		return submitExecOperation{input: input}, nil
	case "permission_inspect":
		var wire permissionInspectInputWire
		if err := decodeFlattened(value, &wire); err != nil {
			return nil, err
		}
		return permissionInspectOperation{input: permissionInspectInput(wire)}, nil
	case "permission_respond":
		var wire permissionRespondInputWire
		if err := decodeFlattened(value, &wire); err != nil {
			return nil, err
		}
		key, err := answerWire{Decision: wire.Decision, Choice: wire.Choice}.key()
		if err != nil {
			return nil, invalidControlFields()
		}
		return permissionRespondOperation{input: permissionRespondInput{
			InteractionID:      wire.InteractionID,
			EventID:            wire.EventID,
			DirectorMemberID:   wire.DirectorMemberID,
			ExpectedRunID:      wire.ExpectedRunID,
			ExpectedOwnerEpoch: wire.ExpectedOwnerEpoch,
			RequestToken:       wire.RequestToken,
			Key:                key,
		}}, nil
	case "receipt":
		action, err := decodeReceiptAction(value)
		if err != nil {
			return nil, err
		}
		return receiptOperation{action: action}, nil
	default:
		return nil, invalidControlFields()
	}
}

func decodeReceiptAction(value jsontext.Value) (receiptAction, error) {
	var tag receiptActionTagWire
	if err := decodeFlattened(value, &tag); err != nil {
		return nil, err
	}
	switch tag.Action {
	case "query":
		var wire receiptQueryWire
		if err := decodeFlattened(value, &wire); err != nil {
			return nil, err
		}
		return receiptQuery(wire), nil
	case "retire":
		var wire receiptRetireWire
		if err := decodeFlattened(value, &wire); err != nil {
			return nil, err
		}
		return receiptRetire(wire), nil
	case "reconcile":
		var wire receiptReconcileWire
		if err := decodeFlattened(value, &wire); err != nil {
			return nil, err
		}
		return receiptReconcile(wire), nil
	default:
		return nil, invalidControlFields()
	}
}

func decodeInputRequest(body []byte) (inputControlRequest, error) {
	value, err := decodeStrictJSON(body)
	if err != nil {
		return inputControlRequest{}, err
	}
	if err := validateVersion(value); err != nil {
		return inputControlRequest{}, err
	}
	var header inputRequestHeaderWire
	if err := decodeFlattened(value, &header); err != nil {
		return inputControlRequest{}, err
	}
	operation, err := decodeInputOperation(value, header.Kind)
	if err != nil {
		return inputControlRequest{}, err
	}
	request := inputControlRequest{Identity: header.Identity, ExpiresAtMonoNS: header.ExpiresAtMonoNS, Operation: operation}
	if err := requireExactFields(value, controlFrame(request.members())); err != nil {
		return inputControlRequest{}, err
	}
	if err := request.validate(); err != nil {
		return inputControlRequest{}, err
	}
	return request, nil
}

// lifecycleRequest is a lifecycle control request; each variant carries the frame's
// protocol_version.
//
//sumtype:decl
type lifecycleRequest interface{ isLifecycleRequest() }

type lifecyclePrepare struct {
	ProtocolVersion uint32          `json:"protocol_version"`
	Identity        controlIdentity `json:"identity"`
	RemainingMS     uint64          `json:"remaining_ms"`
}

type lifecycleStart struct {
	ProtocolVersion uint32          `json:"protocol_version"`
	Identity        controlIdentity `json:"identity"`
	PaneID          string          `json:"pane_id"`
}

type lifecycleHealth struct {
	ProtocolVersion uint32          `json:"protocol_version"`
	Identity        controlIdentity `json:"identity"`
}

type lifecycleShutdown struct {
	ProtocolVersion uint32          `json:"protocol_version"`
	Identity        controlIdentity `json:"identity"`
}

func (lifecyclePrepare) isLifecycleRequest()  {}
func (lifecycleStart) isLifecycleRequest()    {}
func (lifecycleHealth) isLifecycleRequest()   {}
func (lifecycleShutdown) isLifecycleRequest() {}

func lifecycleRequestHealth(run runRecord) lifecycleRequest {
	return lifecycleHealth{ProtocolVersion: controlProtocolVersion, Identity: controlIdentityFromRun(run)}
}

func lifecycleRequestShutdown(run runRecord) lifecycleRequest {
	return lifecycleShutdown{ProtocolVersion: controlProtocolVersion, Identity: controlIdentityFromRun(run)}
}

// lifecycleRequestWire is the kind-tagged object of request.
func lifecycleRequestWire(request lifecycleRequest) (jsontext.Value, error) {
	var object jsonObject
	switch request := request.(type) {
	case lifecyclePrepare:
		object = jsonObject{{"kind", "prepare"}, {"protocol_version", request.ProtocolVersion}, {"identity", request.Identity}, {"remaining_ms", request.RemainingMS}}
	case lifecycleStart:
		object = jsonObject{{"kind", "start"}, {"protocol_version", request.ProtocolVersion}, {"identity", request.Identity}, {"pane_id", request.PaneID}}
	case lifecycleHealth:
		object = jsonObject{{"kind", "health"}, {"protocol_version", request.ProtocolVersion}, {"identity", request.Identity}}
	case lifecycleShutdown:
		object = jsonObject{{"kind", "shutdown"}, {"protocol_version", request.ProtocolVersion}, {"identity", request.Identity}}
	default:
		panic("invalid lifecycleRequest")
	}
	encoded, err := encodeJSON(object)
	if err != nil {
		return nil, ioError(cafleetErrorOf(err).message())
	}
	return encoded, nil
}

func decodeLifecycleRequest(value jsontext.Value) (lifecycleRequest, error) {
	decoded, err := decodeTagged(jsonDeduplicated(value), "kind", map[string]func() lifecycleRequest{
		"prepare":  func() lifecycleRequest { return &lifecyclePrepare{} },
		"start":    func() lifecycleRequest { return &lifecycleStart{} },
		"health":   func() lifecycleRequest { return &lifecycleHealth{} },
		"shutdown": func() lifecycleRequest { return &lifecycleShutdown{} },
	})
	if err != nil {
		return nil, runtimeError("invalid lifecycle control fields")
	}
	switch request := decoded.(type) {
	case *lifecyclePrepare:
		return *request, nil
	case *lifecycleStart:
		return *request, nil
	case *lifecycleHealth:
		return *request, nil
	case *lifecycleShutdown:
		return *request, nil
	default:
		panic("invalid lifecycleRequest")
	}
}

// jsonDeduplicated is value with every repeated member name collapsed into its first position
// holding its last value, at every depth, as serde_json's Value keeps a repeated member.
func jsonDeduplicated(value jsontext.Value) jsontext.Value {
	if members, isObject := jsonMembers(value); isObject {
		object := jsonObject{}
		for _, member := range members {
			object = object.with(member.key, jsonDeduplicated(member.value))
		}
		return jsonValue(object)
	}
	if items, isArray := jsonArray(value); isArray {
		deduplicated := make([]jsontext.Value, 0, len(items))
		for _, item := range items {
			deduplicated = append(deduplicated, jsonDeduplicated(item))
		}
		return jsonValue(deduplicated)
	}
	return value
}

//sumtype:decl
type serviceControlRequest interface{ isServiceControlRequest() }

type lifecycleServiceRequest struct{ request lifecycleRequest }

type inputServiceRequest struct{ request inputControlRequest }

func (lifecycleServiceRequest) isServiceControlRequest() {}
func (inputServiceRequest) isServiceControlRequest()     {}

func decodeServiceRequest(body []byte) (serviceControlRequest, error) {
	input, err := routesInput(body)
	if err != nil {
		return nil, err
	}
	if input {
		request, err := decodeInputRequest(body)
		if err != nil {
			return nil, err
		}
		return inputServiceRequest{request: request}, nil
	}
	value, err := decodeLifecycleControlBody(body)
	if err != nil {
		return nil, err
	}
	request, err := decodeLifecycleRequest(value)
	if err != nil {
		return nil, err
	}
	return lifecycleServiceRequest{request: request}, nil
}

func encodeFrame(members jsonObject) ([]byte, error) {
	body, err := encodeJSON(controlFrame(members))
	if err != nil {
		return nil, ioError(cafleetErrorOf(err).message())
	}
	return frameControlBody(body)
}

func encodeInputRequest(request inputControlRequest) ([]byte, error) {
	if err := request.validate(); err != nil {
		return nil, err
	}
	return encodeFrame(request.members())
}

// encodeErrorReply is the {protocol_version, error} reply envelope.
func encodeErrorReply(cause *cafleetError) ([]byte, error) {
	return encodeFrame(jsonObject{{"error", cause.message()}})
}

// encodeResultReply is the {protocol_version, result} reply envelope.
func encodeResultReply(result any) ([]byte, error) {
	return encodeFrame(jsonObject{{"result", result}})
}

// decodeReply decodes an exact reply envelope whose result decode reads and encode reproduces; a
// runner error becomes `runner control: <cause>`.
func decodeReply[T any](body []byte, decode func(jsontext.Value) (T, error), encode func(T) any) (T, error) {
	var zero T
	value, err := decodeStrictJSON(body)
	if err != nil {
		return zero, err
	}
	if err := validateVersion(value); err != nil {
		return zero, err
	}
	members, _ := jsonMembers(value)
	index := slices.IndexFunc(members, func(member jsonRawMember) bool {
		return member.key == "result" || member.key == "error"
	})
	if index < 0 {
		return zero, invalidControlFields()
	}
	selected := members[index]
	if selected.key == "error" {
		cause, isString := jsonString(selected.value)
		if !isString {
			return zero, invalidControlFields()
		}
		if err := requireExactFields(value, controlFrame(jsonObject{{"error", cause}})); err != nil {
			return zero, err
		}
		return zero, runtimeError("runner control: " + cause)
	}
	result, err := decode(selected.value)
	if err != nil {
		return zero, invalidControlFields()
	}
	if err := requireExactFields(value, controlFrame(jsonObject{{"result", encode(result)}})); err != nil {
		return zero, err
	}
	return result, nil
}

// resultWire is the kind-tagged result of an input reply; payload is its submission, request
// or response member, and receipt its receipt member.
type resultWire struct {
	kind    string
	payload jsontext.Value
	receipt receiptWire
}

// receiptWire is the status-tagged receipt; inputID and submission are null where the status
// carries none.
type receiptWire struct {
	status     string
	inputID    opt[int64]
	submission jsontext.Value
}

type resultKindWire struct {
	Kind string `json:"kind"`
}

type submissionPayloadWire struct {
	Submission jsontext.Value `json:"submission"`
}

type inspectionPayloadWire struct {
	Request jsontext.Value `json:"request"`
}

type responsePayloadWire struct {
	Response jsontext.Value `json:"response"`
}

type receiptPayloadWire struct {
	Receipt jsontext.Value `json:"receipt"`
}

type receiptStatusWire struct {
	Status string `json:"status"`
}

type receiptFieldsWire struct {
	InputID    jsontext.Value `json:"input_id"`
	Submission jsontext.Value `json:"submission"`
}

func (*resultKindWire) ignoresUnknownFields()        {}
func (*submissionPayloadWire) ignoresUnknownFields() {}
func (*inspectionPayloadWire) ignoresUnknownFields() {}
func (*responsePayloadWire) ignoresUnknownFields()   {}
func (*receiptPayloadWire) ignoresUnknownFields()    {}
func (*receiptStatusWire) ignoresUnknownFields()     {}
func (*receiptFieldsWire) ignoresUnknownFields()     {}

func decodeResultWire(value jsontext.Value) (resultWire, error) {
	var tag resultKindWire
	if err := decodeFlattened(value, &tag); err != nil {
		return resultWire{}, err
	}
	switch tag.Kind {
	case "submission":
		var wire submissionPayloadWire
		err := decodeFlattened(value, &wire)
		return resultWire{kind: tag.Kind, payload: wire.Submission}, err
	case "inspection":
		var wire inspectionPayloadWire
		err := decodeFlattened(value, &wire)
		return resultWire{kind: tag.Kind, payload: wire.Request}, err
	case "permission_response":
		var wire responsePayloadWire
		err := decodeFlattened(value, &wire)
		return resultWire{kind: tag.Kind, payload: wire.Response}, err
	case "receipt":
		var wire receiptPayloadWire
		if err := decodeFlattened(value, &wire); err != nil {
			return resultWire{}, err
		}
		receipt, err := decodeReceiptWire(wire.Receipt)
		return resultWire{kind: tag.Kind, receipt: receipt}, err
	default:
		return resultWire{}, invalidControlFields()
	}
}

func decodeReceiptWire(value jsontext.Value) (receiptWire, error) {
	var tag receiptStatusWire
	if err := decodeFlattened(value, &tag); err != nil {
		return receiptWire{}, err
	}
	var fields receiptFieldsWire
	if err := decodeFlattened(value, &fields); err != nil {
		return receiptWire{}, err
	}
	inputID, isInteger := jsonInt64(fields.InputID)
	submissionNull := jsonIsNull(fields.Submission)
	switch tag.Status {
	case "missing":
		if !jsonIsNull(fields.InputID) || !submissionNull {
			return receiptWire{}, invalidControlFields()
		}
		return receiptWire{status: tag.Status, submission: jsonNull}, nil
	case "present":
		if !isInteger {
			return receiptWire{}, invalidControlFields()
		}
		return receiptWire{status: tag.Status, inputID: some(inputID), submission: fields.Submission}, nil
	case "retired", "settled":
		if !isInteger || !submissionNull {
			return receiptWire{}, invalidControlFields()
		}
		return receiptWire{status: tag.Status, inputID: some(inputID), submission: jsonNull}, nil
	default:
		return receiptWire{}, invalidControlFields()
	}
}

func (w resultWire) encode() any {
	switch w.kind {
	case "submission":
		return jsonObject{{"kind", w.kind}, {"submission", w.payload}}
	case "inspection":
		return jsonObject{{"kind", w.kind}, {"request", w.payload}}
	case "permission_response":
		return jsonObject{{"kind", w.kind}, {"response", w.payload}}
	case "receipt":
		return jsonObject{{"kind", w.kind}, {"receipt", jsonObject{
			{"status", w.receipt.status},
			{"input_id", w.receipt.inputID},
			{"submission", w.receipt.submission},
		}}}
	default:
		panic("invalid resultWire kind " + w.kind)
	}
}

func positiveInput(submission submissionResult) error {
	id, set := submission.InputID.get()
	return ensure(!set || id > 0, "positive control input ID required")
}

func validateSubmission(submission submissionResult) error {
	if err := positiveInput(submission); err != nil {
		return err
	}
	queued := submission.Status == submissionStatusQueued
	delivery, delivered := submission.Delivery.get()
	if err := ensure(
		queued == (delivered && delivery.Status == deliveryStatusQueued),
		"queued submission and queued delivery must agree",
	); err != nil {
		return err
	}
	if queued {
		return ensure(
			submission.Stage == submissionStageZeroBytes && submission.InputID.set,
			"queued submission requires zero bytes and a durable input ID",
		)
	}
	return nil
}

func validateInspection(inspection permissionInspection) error {
	if err := positive(
		inspection.EventID,
		inspection.MessageID,
		inspection.MemberID,
		inspection.FleetID,
		inspection.DirectorMemberID,
		inspection.RunID,
		inspection.OwnerEpoch,
		inspection.InteractionID,
		inspection.EpisodeSeq,
	); err != nil {
		return err
	}
	if token, set := inspection.RequestToken.get(); set {
		if err := validateRequestToken(token); err != nil {
			return err
		}
	}
	if err := validateOrder(inspection.SupportedDecisions); err != nil {
		return err
	}
	if err := validateOrder(inspection.MissingEvidence); err != nil {
		return err
	}
	choices := map[string]bool{}
	if err := ensure(
		!slices.ContainsFunc(inspection.SupportedChoices, func(choice supportedChoice) bool {
			duplicate := choices[choice.ChoiceID]
			choices[choice.ChoiceID] = true
			return !validChoiceID(choice.ChoiceID) || duplicate
		}),
		"control inspection choices require unique shaped choice ids",
	); err != nil {
		return err
	}
	classification, classified := inspection.Classification.get()
	if err := ensure(
		len(inspection.SupportedChoices) == 0 ||
			(len(inspection.SupportedDecisions) == 0 && classified && classification == interactionClassQuestion),
		"control inspection choices belong only to a question",
	); err != nil {
		return err
	}
	if validateEvidence(inspection.Evidence) != nil {
		return runtimeError("invalid control inspection evidence")
	}
	return nil
}

// validateOrder requires values to follow their enum's declaration order without repeats; the
// iota constants number that order.
func validateOrder[T ~uint8](values []T) error {
	for index := 1; index < len(values); index++ {
		if values[index] <= values[index-1] {
			return runtimeError("control inspection subset order mismatch")
		}
	}
	return nil
}

func validateReply(reply inputControlReply) error {
	switch reply := reply.(type) {
	case submissionResult:
		return validateSubmission(reply)
	case permissionInspection:
		return validateInspection(reply)
	case permissionResponseResult:
		if err := ensure(
			!reply.Submission.InputID.set && !reply.Submission.Delivery.set,
			"a permission response carries no input receipt",
		); err != nil {
			return err
		}
		if err := positive(reply.MemberID, reply.RunID, reply.OwnerEpoch, reply.InteractionID, reply.EventID); err != nil {
			return err
		}
		return validateChoice(reply.Key)
	case receiptMissing:
		return nil
	case receiptPresent:
		if err := positive(reply.InputID); err != nil {
			return err
		}
		if err := validateSubmission(reply.Submission); err != nil {
			return err
		}
		return ensure(reply.Submission.InputID == some(reply.InputID), "receipt input identity mismatch")
	case receiptRetired:
		return positive(reply.InputID)
	case receiptSettled:
		return positive(reply.InputID)
	default:
		panic("invalid inputControlReply")
	}
}

func decodeSubmissionResult(value jsontext.Value) (submissionResult, error) {
	var submission submissionResult
	if decodeStrict(value, &submission) != nil {
		return submissionResult{}, invalidControlFields()
	}
	if err := requireExactFields(value, submission); err != nil {
		return submissionResult{}, err
	}
	return submission, nil
}

type permissionResponseResultWire struct {
	Status        submissionStatus        `json:"status"`
	InputID       opt[int64]              `json:"input_id"`
	Stage         submissionStage         `json:"stage"`
	OutputSeq     opt[uint64]             `json:"output_seq"`
	Reason        opt[string]             `json:"reason"`
	Delivery      opt[deliveryProjection] `json:"delivery"`
	MemberID      int64                   `json:"member_id"`
	RunID         int64                   `json:"run_id"`
	OwnerEpoch    int64                   `json:"owner_epoch"`
	InteractionID int64                   `json:"interaction_id"`
	EventID       int64                   `json:"event_id"`
	Decision      opt[permissionDecision] `json:"decision"`
	Choice        opt[string]             `json:"choice"`
}

func (*permissionResponseResultWire) ignoresUnknownFields() {}

func decodePermissionResponseResult(value jsontext.Value) (permissionResponseResult, error) {
	var wire permissionResponseResultWire
	if err := decodeFlattened(value, &wire); err != nil {
		return permissionResponseResult{}, err
	}
	key, err := answerWire{Decision: wire.Decision, Choice: wire.Choice}.key()
	if err != nil {
		return permissionResponseResult{}, invalidControlFields()
	}
	response := permissionResponseResult{
		Submission: submissionResult{
			Status:    wire.Status,
			InputID:   wire.InputID,
			Stage:     wire.Stage,
			OutputSeq: wire.OutputSeq,
			Reason:    wire.Reason,
			Delivery:  wire.Delivery,
		},
		MemberID:      wire.MemberID,
		RunID:         wire.RunID,
		OwnerEpoch:    wire.OwnerEpoch,
		InteractionID: wire.InteractionID,
		EventID:       wire.EventID,
		Key:           key,
	}
	if err := requireExactFields(value, public(outputJSONPermissionResponseResult(response), "response")); err != nil {
		return permissionResponseResult{}, err
	}
	return response, nil
}

func decodePermissionInspection(value jsontext.Value) (permissionInspection, error) {
	var inspection permissionInspection
	if decodeStrict(value, &inspection) != nil {
		return permissionInspection{}, invalidControlFields()
	}
	if err := requireExactFields(value, public(outputJSONPermissionInspection(inspection), "request")); err != nil {
		return permissionInspection{}, err
	}
	return inspection, nil
}

func decodeResult(result resultWire) (inputControlReply, error) {
	switch result.kind {
	case "submission":
		return decodeSubmissionResult(result.payload)
	case "inspection":
		return decodePermissionInspection(result.payload)
	case "permission_response":
		return decodePermissionResponseResult(result.payload)
	case "receipt":
		inputID, _ := result.receipt.inputID.get()
		switch result.receipt.status {
		case "missing":
			return receiptMissing{}, nil
		case "present":
			submission, err := decodeSubmissionResult(result.receipt.submission)
			if err != nil {
				return nil, err
			}
			return receiptPresent{InputID: inputID, Submission: submission}, nil
		case "retired":
			return receiptRetired{InputID: inputID}, nil
		case "settled":
			return receiptSettled{InputID: inputID}, nil
		default:
			panic("invalid receiptWire status " + result.receipt.status)
		}
	default:
		panic("invalid resultWire kind " + result.kind)
	}
}

func validateBinding(reply inputControlReply, request inputControlRequest) error {
	identity := request.Identity
	matches := false
	switch operation := request.Operation.(type) {
	case submitPromptOperation, submitExecOperation:
		if submission, ok := reply.(submissionResult); ok {
			reason, reasoned := submission.Reason.get()
			matches = submission.Delivery.set ||
				submission.Status == submissionStatusRejected &&
					submission.Stage == submissionStageZeroBytes &&
					!submission.InputID.set &&
					!submission.OutputSeq.set &&
					reasoned && strings.TrimSpace(reason) != ""
		}
	case permissionInspectOperation:
		if result, ok := reply.(permissionInspection); ok {
			input := operation.input
			matches = result.EventID == input.EventID &&
				result.InteractionID == input.InteractionID &&
				result.DirectorMemberID == input.DirectorMemberID &&
				result.MemberID == identity.MemberID &&
				result.FleetID == identity.FleetID &&
				result.RunID == identity.RunID &&
				result.OwnerEpoch == identity.OwnerEpoch
		}
	case permissionRespondOperation:
		if result, ok := reply.(permissionResponseResult); ok {
			input := operation.input
			matches = result.EventID == input.EventID &&
				result.InteractionID == input.InteractionID &&
				result.MemberID == identity.MemberID &&
				result.RunID == input.ExpectedRunID &&
				result.OwnerEpoch == input.ExpectedOwnerEpoch &&
				result.Key == input.Key
		}
	case receiptOperation:
		switch action := operation.action.(type) {
		case receiptQuery:
			_, missing := reply.(receiptMissing)
			_, present := reply.(receiptPresent)
			matches = missing || present
		case receiptRetire:
			retired, ok := reply.(receiptRetired)
			matches = ok && retired.InputID == action.InputID
		case receiptReconcile:
			settled, ok := reply.(receiptSettled)
			matches = ok && settled.InputID == action.InputID
		default:
			panic("invalid receiptAction")
		}
	default:
		panic("invalid inputControlOperation")
	}
	return ensure(matches, "control reply kind or request identity mismatch")
}

func decodeInputReply(body []byte, request inputControlRequest) (inputControlReply, error) {
	result, err := decodeReply(body, decodeResultWire, resultWire.encode)
	if err != nil {
		return nil, err
	}
	reply, err := decodeResult(result)
	if err != nil {
		return nil, err
	}
	if err := validateReply(reply); err != nil {
		return nil, err
	}
	if err := validateBinding(reply, request); err != nil {
		return nil, err
	}
	return reply, nil
}

func encodeResult(reply inputControlReply) jsonObject {
	switch reply := reply.(type) {
	case submissionResult:
		return jsonObject{{"kind", "submission"}, {"submission", reply}}
	case permissionInspection:
		return jsonObject{{"kind", "inspection"}, {"request", public(outputJSONPermissionInspection(reply), "request")}}
	case permissionResponseResult:
		return jsonObject{{"kind", "permission_response"}, {"response", public(outputJSONPermissionResponseResult(reply), "response")}}
	case receiptMissing:
		return receiptResult("missing", none[int64](), nil)
	case receiptPresent:
		return receiptResult("present", some(reply.InputID), reply.Submission)
	case receiptRetired:
		return receiptResult("retired", some(reply.InputID), nil)
	case receiptSettled:
		return receiptResult("settled", some(reply.InputID), nil)
	default:
		panic("invalid inputControlReply")
	}
}

func receiptResult(status string, inputID opt[int64], submission any) jsonObject {
	if submission == nil {
		submission = jsonNull
	}
	return jsonObject{{"kind", "receipt"}, {"receipt", jsonObject{
		{"status", status},
		{"input_id", inputID},
		{"submission", submission},
	}}}
}

func encodeInputReply(reply inputControlReply) ([]byte, error) {
	if err := validateReply(reply); err != nil {
		return nil, err
	}
	return encodeResultReply(encodeResult(reply))
}

func pause(deadline time.Time, c clock) error {
	if err := runtimeCheckDeadline(c.now(), deadline); err != nil {
		return err
	}
	napUntil(c, deadline)
	return nil
}

// controlClient owns one connected nonblocking stream socket to a runner's control endpoint.
type controlClient struct {
	fd      int
	decoder *controlDecoder
}

func newControlClient(fd int) *controlClient {
	return &controlClient{fd: fd, decoder: newControlDecoder()}
}

type endpointPhase uint8

const (
	endpointPhaseStarting endpointPhase = iota + 1
	endpointPhaseReady
)

func controlClientConnect(path string, deadline time.Time, c clock) (*controlClient, error) {
	return controlClientConnectInPhase(path, deadline, endpointPhaseReady, c)
}

func controlClientConnectStarting(path string, deadline time.Time, c clock) (*controlClient, error) {
	return controlClientConnectInPhase(path, deadline, endpointPhaseStarting, c)
}

func controlClientConnectInPhase(path string, deadline time.Time, phase endpointPhase, c clock) (*controlClient, error) {
	starting := phase == endpointPhaseStarting
	for {
		if err := runtimeCheckDeadline(c.now(), deadline); err != nil {
			return nil, err
		}
		var meta unix.Stat_t
		if err := unix.Lstat(path, &meta); err != nil {
			if errors.Is(err, unix.ENOENT) && starting {
				if err := pause(deadline, c); err != nil {
					return nil, err
				}
				continue
			}
			return nil, ioError(ioErrorText(err))
		}
		if err := ensure(
			meta.Mode&unix.S_IFMT == unix.S_IFSOCK && meta.Uid == uid(),
			"private control socket identity required",
		); err != nil {
			return nil, err
		}
		if meta.Mode&0o777 != 0o600 {
			if !starting {
				return nil, runtimeError("private control socket identity required")
			}
			if err := pause(deadline, c); err != nil {
				return nil, err
			}
			continue
		}
		client, retry, err := controlClientDial(path, starting)
		if err != nil {
			return nil, err
		}
		if !retry {
			return client, nil
		}
		if err := pause(deadline, c); err != nil {
			return nil, err
		}
	}
}

// controlClientDial makes one nonblocking connection attempt; retry reports a refusal the caller
// waits out.
func controlClientDial(path string, starting bool) (*controlClient, bool, error) {
	if len(path) >= len(unix.RawSockaddrUnix{}.Path) {
		return nil, false, ioError(errnoText(unix.ENAMETOOLONG))
	}
	fd, err := controlSocket()
	if err != nil {
		return nil, false, ioError(errnoText(err))
	}
	err = unix.Connect(fd, &unix.SockaddrUnix{Name: path})
	switch {
	case err == nil, errors.Is(err, unix.EINPROGRESS):
		return newControlClient(fd), false, nil
	case errors.Is(err, unix.EAGAIN),
		starting && (errors.Is(err, unix.ECONNREFUSED) || errors.Is(err, unix.ENOENT)):
		closeControlSocket(fd)
		return nil, true, nil
	default:
		closeControlSocket(fd)
		return nil, false, ioError(errnoText(err))
	}
}

func controlSocket() (int, error) {
	syscall.ForkLock.RLock()
	defer syscall.ForkLock.RUnlock()
	fd, err := unix.Socket(unix.AF_UNIX, unix.SOCK_STREAM, 0)
	if err != nil {
		return -1, err
	}
	unix.CloseOnExec(fd)
	if err := unix.SetNonblock(fd, true); err != nil {
		closeControlSocket(fd)
		return -1, err
	}
	return fd, nil
}

func closeControlSocket(fd int) {
	if err := unix.Close(fd); err != nil && !errors.Is(err, unix.EINTR) {
		panic("closing an owned control socket failed: " + err.Error())
	}
}

func (c *controlClient) close() error {
	if err := unix.Close(c.fd); err != nil {
		return ioError(ioErrorText(err))
	}
	return nil
}

func (c *controlClient) exchange(request lifecycleRequest, deadline time.Time, cl clock) (jsontext.Value, error) {
	wire, err := lifecycleRequestWire(request)
	if err != nil {
		return nil, err
	}
	encoded, err := encodeControlFrame(wire)
	if err != nil {
		return nil, err
	}
	frame, err := c.exchangeBytes(encoded, deadline, cl)
	if err != nil {
		return nil, err
	}
	return decodeReply(frame.body, func(value jsontext.Value) (jsontext.Value, error) { return value, nil }, func(value jsontext.Value) any { return value })
}

func (c *controlClient) exchangeInput(request inputControlRequest, deadline time.Time, cl clock) (inputControlReply, error) {
	if err := runtimeCheckDeadline(cl.now(), deadline); err != nil {
		return nil, err
	}
	encoded, err := encodeInputRequest(request)
	if err != nil {
		return nil, err
	}
	frame, err := c.exchangeBytes(encoded, deadline, cl)
	if err != nil {
		return nil, err
	}
	reply, err := decodeInputReply(frame.body, request)
	if err != nil {
		return nil, err
	}
	if err := runtimeCheckDeadline(cl.now(), deadline); err != nil {
		return nil, err
	}
	return reply, nil
}

func (c *controlClient) exchangeBytes(data []byte, deadline time.Time, cl clock) (admittedControlFrame, error) {
	for offset := 0; offset < len(data); {
		if err := runtimeCheckDeadline(cl.now(), deadline); err != nil {
			return admittedControlFrame{}, err
		}
		count, err := unix.Write(c.fd, data[offset:])
		switch {
		case errors.Is(err, unix.EAGAIN), errors.Is(err, unix.EINTR):
			if err := pause(deadline, cl); err != nil {
				return admittedControlFrame{}, err
			}
		case err != nil:
			return admittedControlFrame{}, ioError(ioErrorText(err))
		case count == 0:
			return admittedControlFrame{}, runtimeError("control endpoint disconnected during write")
		default:
			offset += count
		}
	}
	buffer := make([]byte, 4096)
	for {
		if err := runtimeCheckDeadline(cl.now(), deadline); err != nil {
			return admittedControlFrame{}, err
		}
		count, err := unix.Read(c.fd, buffer)
		switch {
		case errors.Is(err, unix.EAGAIN), errors.Is(err, unix.EINTR):
			if err := pause(deadline, cl); err != nil {
				return admittedControlFrame{}, err
			}
		case err != nil:
			return admittedControlFrame{}, ioError(ioErrorText(err))
		case count == 0:
			if err := c.decoder.finish(); err != nil {
				return admittedControlFrame{}, err
			}
			return admittedControlFrame{}, runtimeError("control endpoint disconnected before reply")
		default:
			frames, err := c.decoder.feedAdmitted(buffer[:count], cl.now())
			if err != nil {
				return admittedControlFrame{}, err
			}
			if len(frames) == 0 {
				continue
			}
			if err := ensure(len(frames) == 1, "unexpected multiple control replies"); err != nil {
				return admittedControlFrame{}, err
			}
			if err := c.decoder.finish(); err != nil {
				return admittedControlFrame{}, err
			}
			return frames[0], nil
		}
	}
}

// runtimeControlRequest connects to run's control endpoint and exchanges one lifecycle request.
func runtimeControlRequest(run runRecord, request lifecycleRequest, deadline time.Time, c clock) (jsontext.Value, error) {
	paths, err := runtimePathsForRun(run.databaseIdentityJSON, run.runID)
	if err != nil {
		return nil, err
	}
	client, err := controlClientConnect(paths.controlSocket, deadline, c)
	if err != nil {
		return nil, err
	}
	reply, err := client.exchange(request, deadline, c)
	return reply, mergeCleanup(err, client.close())
}

func requestShutdown(ctx context.Context, conn *dbConn, run runRecord, deadline time.Time, c clock) (shutdownOutcome, error) {
	if _, err := validateRunPaths(ctx, conn, run); err != nil {
		return shutdownOutcome{}, err
	}
	result, err := runtimeControlRequest(run, lifecycleRequestShutdown(run), deadline, c)
	if err != nil {
		return shutdownOutcome{}, err
	}
	outcome, err := shutdownOutcomeDecode(result)
	if err != nil {
		return shutdownOutcome{}, err
	}
	current, err := getRun(ctx, conn, run.runID)
	if err != nil {
		return shutdownOutcome{}, err
	}
	if err := ensure(current.ownerEpoch == run.ownerEpoch, "shutdown ownership changed"); err != nil {
		return shutdownOutcome{}, err
	}
	return outcome, nil
}

type healthSnapshot struct {
	Identity    controlIdentity        `json:"identity"`
	Runner      processIdentity        `json:"runner"`
	Child       opt[processIdentity]   `json:"child"`
	OutputSeq   uint64                 `json:"output_seq"`
	Observation opt[observationHealth] `json:"observation"`
}

func (*healthSnapshot) ignoresUnknownFields() {}

// MarshalJSONTo writes the snapshot, omitting an absent observation.
func (h healthSnapshot) MarshalJSONTo(encoder *jsontext.Encoder) error {
	object := jsonObject{
		{"identity", h.Identity},
		{"runner", h.Runner},
		{"child", h.Child},
		{"output_seq", h.OutputSeq},
	}
	if observation, observed := h.Observation.get(); observed {
		object = append(object, jsonMember{"observation", observation})
	}
	return object.MarshalJSONTo(encoder)
}

func healthSnapshotDecode(value jsontext.Value, run runRecord) (healthSnapshot, error) {
	var result healthSnapshot
	if err := decodeStrict(value, &result); err != nil {
		return healthSnapshot{}, ioError(cafleetErrorOf(err).message())
	}
	if err := result.validate(run); err != nil {
		return healthSnapshot{}, err
	}
	return result, nil
}

func (h healthSnapshot) validate(run runRecord) error {
	if err := h.Identity.validate(run); err != nil {
		return err
	}
	if observation, observed := h.Observation.get(); observed {
		if err := observation.validate(); err != nil {
			return err
		}
	}
	runnerMatches, err := h.Runner.matches()
	if err != nil {
		return err
	}
	if err := ensure(
		run.runnerPID == some(h.Runner.pid) &&
			run.runnerStartIdentity == some(h.Runner.startIdentity) &&
			run.ownerPID == h.Runner.pid &&
			run.ownerStartIdentity == h.Runner.startIdentity &&
			runnerMatches,
		"health runner generation mismatch",
	); err != nil {
		return err
	}
	child, hasChild := h.Child.get()
	if !hasChild {
		return ensure(!run.childPID.set, "health omitted native child identity")
	}
	childMatches, err := child.matches()
	if err != nil {
		return err
	}
	return ensure(
		run.childPID == some(child.pid) && run.childStartIdentity == some(child.startIdentity) && childMatches,
		"health child generation mismatch",
	)
}

func probeHealth(run runRecord, deadline time.Time, c clock) (healthSnapshot, error) {
	reply, err := runtimeControlRequest(run, lifecycleRequestHealth(run), deadline, c)
	if err != nil {
		return healthSnapshot{}, err
	}
	return healthSnapshotDecode(reply, run)
}

// checkStatus is a doctor row's status; its constants are in severity order.
type checkStatus uint8

const (
	checkStatusOk checkStatus = iota + 1
	checkStatusInfo
	checkStatusWarn
	checkStatusError
)

func (s checkStatus) asStr() string {
	switch s {
	case checkStatusOk:
		return "ok"
	case checkStatusInfo:
		return "info"
	case checkStatusWarn:
		return "warn"
	case checkStatusError:
		return "error"
	default:
		panic("invalid checkStatus")
	}
}

type submitInput struct {
	OperationKey string    `json:"operation_key"`
	InputKind    inputKind `json:"input_kind"`
	Literal      string    `json:"literal"`
}
