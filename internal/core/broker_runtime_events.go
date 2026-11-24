package core

import (
	"context"
	"encoding/json/jsontext"
	"fmt"
	"slices"
	"strconv"
	"strings"
)

var evidenceFields = []string{"backend", "native_version", "rule_id", "screen_seq"}

// metadataEvidence is the metadata evidence of an observation: its identity fields and their
// digest.
func metadataEvidence(raw jsontext.Value) (jsontext.Value, error) {
	if raw.Kind() != '{' {
		return nil, brokerRuntimeInvalid("evidence", "object required")
	}
	fields := jsonObject{}
	for _, key := range evidenceFields {
		value, present := jsonLookup(raw, key)
		if !present {
			return nil, brokerRuntimeInvalid(key, "required evidence field")
		}
		fields = append(fields, jsonMember{key: key, value: value})
	}
	evidence := jsonValue(fields)
	evidence = jsonWith(evidence, "evidence_digest", evidenceDigest(evidence))
	if err := validateEvidence(evidence); err != nil {
		return nil, err
	}
	return evidence, nil
}

// runEvidence is the metadata evidence of a run-scoped event that no screen form produced.
func runEvidence(run runRecord, ruleID string, screenSeq uint64) (jsontext.Value, error) {
	return metadataEvidence(jsonValue(jsonObject{
		{"backend", run.backend},
		{"native_version", run.nativeVersion},
		{"rule_id", ruleID},
		{"screen_seq", screenSeq},
	}))
}

func evidenceDigest(value jsontext.Value) string {
	canonical, err := canonicalJSON(jsonWithout(value, "evidence_digest"))
	if err != nil {
		panic("evidence is a valid JSON object: " + err.Error())
	}
	return sha256Hex([]byte(canonical))
}

func validateEvidence(value jsontext.Value) error {
	if value.Kind() != '{' {
		return brokerRuntimeInvalid("evidence_json", "object required")
	}
	keys := jsonKeys(value)
	exact := len(keys) == len(evidenceFields)+1
	for _, key := range append(slices.Clone(evidenceFields), "evidence_digest") {
		exact = exact && slices.Contains(keys, key)
	}
	if err := brokerRuntimeRequire(exact, "evidence_json", "metadata evidence has exactly its identity fields and digest"); err != nil {
		return err
	}
	for _, field := range []string{"backend", "rule_id"} {
		text, isString := jsonString(jsonIndex(value, field))
		if err := brokerRuntimeRequire(isString && text != "" && len(text) <= 256, field, "bounded evidence identity required"); err != nil {
			return err
		}
	}
	nativeVersion := jsonIndex(value, "native_version")
	version, isString := jsonString(nativeVersion)
	if err := brokerRuntimeRequire(jsonIsNull(nativeVersion) || (isString && len(version) <= 256), "native_version", "nullable bounded string required"); err != nil {
		return err
	}
	_, sequenced := jsonUint64(jsonIndex(value, "screen_seq"))
	if err := brokerRuntimeRequire(sequenced, "screen_seq", "nonnegative sequence required"); err != nil {
		return err
	}
	return brokerRuntimeRequire(jsonIsString(jsonIndex(value, "evidence_digest"), evidenceDigest(value)), "evidence_digest", "evidence digest mismatch")
}

func mapInteraction(row sqlRow) (interactionRecord, error) {
	var record interactionRecord
	var err error
	column := row.columnIndex
	if record.interactionID, err = row.int64(column("interaction_id")); err != nil {
		return interactionRecord{}, err
	}
	if record.runID, err = row.int64(column("run_id")); err != nil {
		return interactionRecord{}, err
	}
	if record.episodeSeq, err = row.int64(column("episode_seq")); err != nil {
		return interactionRecord{}, err
	}
	if record.classification, err = rowStoredEnum(row, column("classification"), interactionClassTryFrom); err != nil {
		return interactionRecord{}, err
	}
	if record.evidenceDigest, err = row.text(column("evidence_digest")); err != nil {
		return interactionRecord{}, err
	}
	if record.openedAt, err = row.text(column("opened_at")); err != nil {
		return interactionRecord{}, err
	}
	if record.evidenceJSON, err = jsonColumn(row, "evidence_json", false); err != nil {
		return interactionRecord{}, err
	}
	if record.closedAt, err = row.optText(column("closed_at")); err != nil {
		return interactionRecord{}, err
	}
	if record.closeReason, err = rowOptStoredEnum(row, column("close_reason"), interactionCloseReasonTryFrom); err != nil {
		return interactionRecord{}, err
	}
	return record, nil
}

func interaction(_ context.Context, conn *dbConn, id int64) (interactionRecord, error) {
	record, err := queryRow(conn, "SELECT * FROM member_interactions WHERE interaction_id=?1", []any{id}, mapInteraction)
	if err != nil {
		return interactionRecord{}, err
	}
	if err := validateEvidence(record.evidenceJSON); err != nil {
		return interactionRecord{}, err
	}
	matches := jsonIsString(jsonIndex(record.evidenceJSON, "evidence_digest"), record.evidenceDigest)
	if err := brokerRuntimeRequire(matches, "member_interactions.evidence_digest", "stored evidence digest mismatch"); err != nil {
		return interactionRecord{}, err
	}
	return record, nil
}

func mapEvent(row sqlRow) (runtimeEventRecord, error) {
	var event runtimeEventRecord
	var err error
	column := row.columnIndex
	if event.eventID, err = row.int64(column("event_id")); err != nil {
		return runtimeEventRecord{}, err
	}
	if event.runID, err = row.int64(column("run_id")); err != nil {
		return runtimeEventRecord{}, err
	}
	if event.eventSeq, err = row.int64(column("event_seq")); err != nil {
		return runtimeEventRecord{}, err
	}
	if event.interactionID, err = row.optInt64(column("interaction_id")); err != nil {
		return runtimeEventRecord{}, err
	}
	if event.kind, err = rowStoredEnum(row, column("kind"), runtimeEventKindTryFrom); err != nil {
		return runtimeEventRecord{}, err
	}
	if event.observedAt, err = row.text(column("observed_at")); err != nil {
		return runtimeEventRecord{}, err
	}
	if event.payloadJSON, err = jsonColumn(row, "payload_json", false); err != nil {
		return runtimeEventRecord{}, err
	}
	if event.notificationState, err = rowStoredEnum(row, column("notification_state"), notificationStateTryFrom); err != nil {
		return runtimeEventRecord{}, err
	}
	if event.messageID, err = row.optInt64(column("message_id")); err != nil {
		return runtimeEventRecord{}, err
	}
	if event.archiveReason, err = rowOptStoredEnum(row, column("archive_reason"), archiveReasonTryFrom); err != nil {
		return runtimeEventRecord{}, err
	}
	return event, nil
}

const payloadLimit = 16 * 1024

func latestEventSeq(_ context.Context, conn *dbConn, runID int64) (opt[int64], error) {
	return queryRow(conn, "SELECT MAX(event_seq) FROM member_runtime_events WHERE run_id=?1",
		[]any{runID}, func(row sqlRow) (opt[int64], error) { return row.optInt64(0) })
}

func interactionIDForEpisode(conn *dbConn, runID, episodeSeq int64) (int64, error) {
	return queryRow(conn, "SELECT interaction_id FROM member_interactions WHERE run_id=?1 AND episode_seq=?2",
		[]any{runID, episodeSeq}, func(row sqlRow) (int64, error) { return row.int64(0) })
}

func source(kind runtimeEventKind) string {
	switch kind {
	case runtimeEventKindInteractionOpened, runtimeEventKindInteractionClosed:
		return "runner_screen"
	case runtimeEventKindCoverageUnknown, runtimeEventKindInputDeliveryResult, runtimeEventKindInputDeliveryStalled,
		runtimeEventKindRunnerDegraded, runtimeEventKindRunnerLost, runtimeEventKindRunnerExited:
		return "runner_lifecycle"
	default:
		panic("invalid runtimeEventKind")
	}
}

func isDeliveryEvent(kind runtimeEventKind) bool {
	return kind == runtimeEventKindInputDeliveryResult || kind == runtimeEventKindInputDeliveryStalled
}

func getEvent(ctx context.Context, conn *dbConn, eventID int64) (runtimeEventView, error) {
	rows, err := queryRows(conn, "SELECT * FROM member_runtime_events WHERE event_id=?1", []any{eventID}, mapEvent)
	if err != nil {
		return runtimeEventView{}, err
	}
	if len(rows) == 0 {
		return runtimeEventView{}, appError(fmt.Sprintf("Event %d not found", eventID))
	}
	event := rows[0]
	run, err := brokerRuntimeSnapshot(ctx, conn, event.runID)
	if err != nil {
		return runtimeEventView{}, err
	}
	if _, err := historicalOwnership(ctx, conn, run.run); err != nil {
		return runtimeEventView{}, err
	}
	view := runtimeEventView{event: event, run: run, interaction: none[*interactionRecord]()}
	if id, set := event.interactionID.get(); set {
		record, err := interaction(ctx, conn, id)
		if err != nil {
			return runtimeEventView{}, err
		}
		if err := brokerRuntimeRequire(record.runID == event.runID, "member_runtime_events.interaction_id", "cross-run interaction"); err != nil {
			return runtimeEventView{}, err
		}
		view.interaction = some(&record)
	}
	transition, err := original(view)
	if err != nil {
		return runtimeEventView{}, err
	}
	if err := validateTransition(view.run.run, transition); err != nil {
		return runtimeEventView{}, err
	}
	var delivery *deliveryFields
	if isDeliveryEvent(event.kind) {
		published, err := publishedDelivery(ctx, conn, event)
		if err != nil {
			return runtimeEventView{}, err
		}
		delivery = &published
	}
	expected, err := eventPayload(conn, view.run.run, transition, event.eventID, event.interactionID, delivery)
	if err != nil {
		return runtimeEventView{}, err
	}
	if err := brokerRuntimeRequire(jsonEqual(expected, event.payloadJSON), "payload_json", "stored payload does not match its event"); err != nil {
		return runtimeEventView{}, err
	}
	serializedPayload := jsonText(event.payloadJSON)
	if err := brokerRuntimeRequire(len(serializedPayload) <= payloadLimit, "payload_json", "payload exceeds bound"); err != nil {
		return runtimeEventView{}, err
	}
	if event.notificationState == notificationStateInbox {
		messageID, set := event.messageID.get()
		if !set {
			panic("the notification-state CHECK pairs an inbox event with its message")
		}
		message, err := getMessage(ctx, conn, messageID)
		if err != nil {
			return runtimeEventView{}, err
		}
		associated := message.fromMemberID == view.run.run.memberID &&
			message.toMemberID == view.run.run.directorMemberID && message.text == serializedPayload
		if err := brokerRuntimeRequire(associated, "message_id", "event/message association mismatch"); err != nil {
			return runtimeEventView{}, err
		}
	}
	return view, nil
}

var deliveryResultStages = []string{"zero_bytes", "partial", "submitted", "unknown"}

func resultDelivery(ctx context.Context, conn *dbConn, runID, inputID int64, field string) (inputOperation, deliveryProjection, error) {
	input, err := getInput(ctx, conn, inputID)
	if err != nil {
		return inputOperation{}, deliveryProjection{}, err
	}
	_, deferred := input.deferredAt.get()
	_, stopped := input.stopCause.get()
	if err := brokerRuntimeRequire(input.runID == runID && (deferred || stopped), field, "delivery results describe this run's deferred or stopped ordinary inputs"); err != nil {
		return inputOperation{}, deliveryProjection{}, err
	}
	delivery := input.delivery()
	if !deferred && stopped {
		delivery.Status = deliveryStatusUncertain
	}
	return input, delivery, nil
}

func validateResultStage(stage jsontext.Value, field string) error {
	text, isString := jsonString(stage)
	return brokerRuntimeRequire(isString && slices.Contains(deliveryResultStages, text), field, "stage outside its domain")
}

func resultStopCause(input inputOperation, status deliveryStatus) opt[stopCause] {
	if status == deliveryStatusUncertain {
		return input.stopCause
	}
	return none[stopCause]()
}

// publishedDelivery is the delivery fields a stored result publishes, checked against its input
// receipt: an uncertain result keeps the method it published even after the receipt settles.
func publishedDelivery(ctx context.Context, conn *dbConn, event runtimeEventRecord) (deliveryFields, error) {
	payload := event.payloadJSON
	if event.kind == runtimeEventKindInputDeliveryStalled || jsonIsNull(jsonIndex(payload, "input_id")) {
		run, err := getRun(ctx, conn, event.runID)
		if err != nil {
			return deliveryFields{}, err
		}
		if event.kind == runtimeEventKindInputDeliveryStalled {
			return stalledDelivery(ctx, conn, run, payload, true)
		}
		return previewDelivery(ctx, conn, run, payload)
	}
	inputID, identified := jsonInt64(jsonIndex(payload, "input_id"))
	if !identified {
		return deliveryFields{}, brokerRuntimeInvalid("payload_json.input_id", "input identity required")
	}
	input, delivery, err := resultDelivery(ctx, conn, event.runID, inputID, "payload_json.input_id")
	if err != nil {
		return deliveryFields{}, err
	}
	var published deliveryProjection
	if err := decodeStrict(jsonIndex(payload, "delivery"), &published); err != nil {
		return deliveryFields{}, brokerRuntimeInvalid("payload_json.delivery", "delivery projection required")
	}
	var consistent bool
	if published.Status == deliveryStatusUncertain {
		consistent = delivery.Status != deliveryStatusQueued && delivery.Status != deliveryStatusHeld &&
			delivery.Method == published.Method && delivery.Reason == published.Reason
	} else {
		consistent = (published.Status == deliveryStatusSubmitted || published.Status == deliveryStatusNotSubmitted) &&
			published == delivery
	}
	if err := brokerRuntimeRequire(consistent, "payload_json.delivery", "delivery must match the input receipt's settled result"); err != nil {
		return deliveryFields{}, err
	}
	if err := validateResultStage(jsonIndex(payload, "stage"), "payload_json.stage"); err != nil {
		return deliveryFields{}, err
	}
	return deliveryFields{
		inputID:          some(inputID),
		messageID:        none[int64](),
		blockedByInputID: none[int64](),
		cause:            resultStopCause(input, published.Status),
		action:           jsonNull,
		waitingSince:     none[string](),
		archive:          none[archiveReason](),
		delivery:         some(published),
		stage:            jsonIndex(payload, "stage"),
	}, nil
}

// previewDelivery is the fields of an uncertain preview's result, which has no input receipt:
// the message must be addressed to the run's member, and the published delivery is uncertain
// with its method.
func previewDelivery(ctx context.Context, conn *dbConn, run runRecord, record jsontext.Value) (deliveryFields, error) {
	messageID, named := jsonInt64(jsonIndex(record, "message_id"))
	if !named {
		return deliveryFields{}, brokerRuntimeInvalid("message_id", "an uncertain preview names its message")
	}
	message, err := getMessage(ctx, conn, messageID)
	if err != nil {
		return deliveryFields{}, err
	}
	if err := brokerRuntimeRequire(message.toMemberID == run.memberID, "message_id", "an uncertain preview addresses the run's member"); err != nil {
		return deliveryFields{}, err
	}
	methodValue := jsonIndex(record, "method")
	if delivery, present := jsonLookup(record, "delivery"); present {
		methodValue = jsonIndex(delivery, "method")
	}
	methodText, isString := jsonString(methodValue)
	method, methodErr := deliveryMethodTryFrom(methodText)
	if !isString || methodErr != nil {
		return deliveryFields{}, brokerRuntimeInvalid("delivery.method", "an uncertain preview records its method")
	}
	if err := validateResultStage(jsonIndex(record, "stage"), "stage"); err != nil {
		return deliveryFields{}, err
	}
	causeText, isString := jsonString(jsonIndex(jsonIndex(record, "cause"), "kind"))
	cause, causeErr := stopCauseTryFrom(causeText)
	if !isString || causeErr != nil {
		return deliveryFields{}, brokerRuntimeInvalid("cause", "an uncertain preview records its stop cause")
	}
	return deliveryFields{
		inputID:          none[int64](),
		messageID:        some(messageID),
		blockedByInputID: none[int64](),
		delivery:         some(deliveryProjection{Status: deliveryStatusUncertain, Method: some(method), Reason: none[string]()}),
		stage:            jsonIndex(record, "stage"),
		cause:            some(cause),
		action:           jsonNull,
		waitingSince:     none[string](),
		archive:          none[archiveReason](),
	}, nil
}

// original is the transition a stored event recorded.
func original(view runtimeEventView) (observedTransition, error) {
	payload := view.event.payloadJSON
	closeReason := none[interactionCloseReason]()
	if value, present := jsonLookup(payload, "close_reason"); present && !jsonIsNull(value) {
		text, isString := jsonString(value)
		if !isString {
			return observedTransition{}, brokerRuntimeInvalid("close_reason", "string required")
		}
		reason, err := interactionCloseReasonTryFrom(text)
		if err != nil {
			return observedTransition{}, err
		}
		closeReason = some(reason)
	}
	episodeSeq, classification := none[int64](), none[interactionClass]()
	if record, set := view.interaction.get(); set {
		episodeSeq, classification = some(record.episodeSeq), some(record.classification)
	}
	var cause jsontext.Value
	switch view.event.kind {
	case runtimeEventKindInputDeliveryStalled:
		cause = jsonValue(jsonObject{
			{"kind", "input_delivery_stalled"},
			{"input_id", jsonIndex(payload, "input_id")},
			{"message_id", jsonIndex(payload, "message_id")},
			{"blocked_by_input_id", jsonIndex(payload, "blocked_by_input_id")},
			{"waiting_since", jsonIndex(payload, "waiting_since")},
		})
	case runtimeEventKindInputDeliveryResult:
		record := jsonObject{
			{"kind", "input_delivery_result"},
			{"input_id", jsonIndex(payload, "input_id")},
			{"stage", jsonIndex(payload, "stage")},
		}
		if jsonIsNull(jsonIndex(payload, "input_id")) {
			record = record.with("message_id", jsonIndex(payload, "message_id"))
			record = record.with("method", jsonIndex(jsonIndex(payload, "delivery"), "method"))
		}
		if diagnostic, present := jsonLookup(payload, "cause"); present {
			record = record.with("cause", diagnostic)
		}
		cause = jsonValue(record)
	case runtimeEventKindCoverageUnknown, runtimeEventKindInteractionOpened, runtimeEventKindInteractionClosed,
		runtimeEventKindRunnerDegraded, runtimeEventKindRunnerLost, runtimeEventKindRunnerExited:
		if value, present := jsonLookup(payload, "cause"); present && !jsonIsNull(value) {
			cause = value
		}
	default:
		panic("invalid runtimeEventKind")
	}
	return observedTransition{
		RunID:          view.event.runID,
		EventSeq:       view.event.eventSeq,
		EpisodeSeq:     episodeSeq,
		Kind:           view.event.kind,
		Classification: classification,
		ObservedAt:     view.event.observedAt,
		EvidenceJSON:   jsonIndex(payload, "evidence"),
		CloseReason:    closeReason,
		CauseJSON:      cause,
	}, nil
}

// inputDeliveryResultCause is the runner's internal record of a deferred input's first result:
// its input, its stage and the receipt's recorded uncertain cause, carried as the transition's
// cause.
func inputDeliveryResultCause(inputID int64, stage string, cause opt[stopCause]) jsontext.Value {
	record := jsonObject{{"kind", "input_delivery_result"}, {"input_id", inputID}, {"stage", stage}}
	if recorded, set := cause.get(); set {
		record = record.with("cause", stopCausePayload(recorded))
	}
	return jsonValue(record)
}

// previewDeliveryResultCause is the runner's internal record of an uncertain preview's result,
// which has no input receipt: its message, the delivery method it was typed with, its written
// stage and its stop cause.
func previewDeliveryResultCause(messageID int64, method deliveryMethod, stage string, cause stopCause) jsontext.Value {
	return jsonValue(jsonObject{
		{"kind", "input_delivery_result"},
		{"input_id", nil},
		{"message_id", messageID},
		{"method", method.asStr()},
		{"stage", stage},
		{"cause", stopCausePayload(cause)},
	})
}

func stopCausePayload(cause stopCause) jsontext.Value {
	return jsonValue(jsonObject{{"kind", cause.asStr()}})
}

func validateTransition(run runRecord, transition observedTransition) error {
	if err := brokerRuntimeRequire(transition.RunID == run.runID && transition.EventSeq > 0, "event_seq", "run or sequence mismatch"); err != nil {
		return err
	}
	if err := rfc3339(transition.ObservedAt, "observed_at"); err != nil {
		return err
	}
	if err := validateEvidence(transition.EvidenceJSON); err != nil {
		return err
	}
	if err := brokerRuntimeRequire(jsonIsString(jsonIndex(transition.EvidenceJSON, "backend"), run.backend), "evidence.backend", "backend mismatch"); err != nil {
		return err
	}
	sequence, episodic := transition.EpisodeSeq.get()
	_, classified := transition.Classification.get()
	if err := brokerRuntimeRequire(episodic == classified, "episode_seq", "episode and classification must be paired"); err != nil {
		return err
	}
	if episodic {
		if err := brokerRuntimeRequire(sequence > 0, "episode_seq", "positive episode required"); err != nil {
			return err
		}
	}
	interactive := transition.Kind == runtimeEventKindInteractionOpened || transition.Kind == runtimeEventKindInteractionClosed
	if err := brokerRuntimeRequire(!interactive || episodic, "episode_seq", "interactive transition requires episode"); err != nil {
		return err
	}
	_, closing := transition.CloseReason.get()
	if err := brokerRuntimeRequire((transition.Kind == runtimeEventKindInteractionClosed) == closing, "close_reason", "closure reason must match closure event"); err != nil {
		return err
	}
	if source(transition.Kind) != "runner_lifecycle" {
		return brokerRuntimeRequire(transition.CauseJSON == nil, "cause_json", "screen events carry only post-policy screen evidence")
	}
	if err := brokerRuntimeRequire(!episodic && !classified, "episode_seq", "run-level events have no interaction"); err != nil {
		return err
	}
	switch transition.Kind {
	case runtimeEventKindInputDeliveryResult:
		return validateDeliveryCause(transition)
	case runtimeEventKindInputDeliveryStalled:
		return validateStalledRecord(transition)
	case runtimeEventKindCoverageUnknown, runtimeEventKindInteractionOpened, runtimeEventKindInteractionClosed,
		runtimeEventKindRunnerDegraded, runtimeEventKindRunnerLost, runtimeEventKindRunnerExited:
		return validateCause(transition)
	default:
		panic("invalid runtimeEventKind")
	}
}

var stalledRecordKeys = []string{"kind", "input_id", "message_id", "blocked_by_input_id", "waiting_since"}

func hasExactlyKeys(object jsontext.Value, keys []string) bool {
	present := jsonKeys(object)
	exact := len(present) == len(keys)
	for _, key := range keys {
		exact = exact && slices.Contains(present, key)
	}
	return exact
}

func hasOnlyKeys(object jsontext.Value, allowed []string) bool {
	for _, key := range jsonKeys(object) {
		if !slices.Contains(allowed, key) {
			return false
		}
	}
	return true
}

func namedID(id jsontext.Value) bool {
	value, ok := jsonInt64(id)
	return ok && value > 0
}

func validateStalledRecord(transition observedTransition) error {
	record := transition.CauseJSON
	if record == nil || record.Kind() != '{' || !hasExactlyKeys(record, stalledRecordKeys) {
		return brokerRuntimeInvalid("cause_json", "stalled delivery record required")
	}
	ids := []jsontext.Value{jsonIndex(record, "input_id"), jsonIndex(record, "message_id"), jsonIndex(record, "blocked_by_input_id")}
	valid := jsonIsString(jsonIndex(record, "kind"), "input_delivery_stalled")
	named := 0
	for _, id := range ids {
		valid = valid && (jsonIsNull(id) || namedID(id))
		if namedID(id) {
			named++
		}
	}
	if err := brokerRuntimeRequire(valid && named == 1, "cause_json", "a stalled record names exactly one waiting input, message or blocker and its wait start"); err != nil {
		return err
	}
	waitingSince, isString := jsonString(jsonIndex(record, "waiting_since"))
	if !isString {
		return brokerRuntimeInvalid("waiting_since", "timestamp required")
	}
	return rfc3339(waitingSince, "waiting_since")
}

func validateDeliveryCause(transition observedTransition) error {
	cause := transition.CauseJSON
	if cause == nil || cause.Kind() != '{' {
		return brokerRuntimeInvalid("cause_json", "delivery result record required")
	}
	inputIDValue, hasInputID := jsonLookup(cause, "input_id")
	preview := hasInputID && jsonIsNull(inputIDValue)
	keys := []string{"kind", "input_id", "stage", "cause"}
	if preview {
		keys = []string{"kind", "input_id", "message_id", "method", "stage", "cause"}
	}
	var identified bool
	if preview {
		messageID, hasMessage := jsonLookup(cause, "message_id")
		methodValue, hasMethod := jsonLookup(cause, "method")
		method, isString := jsonString(methodValue)
		_, methodErr := deliveryMethodTryFrom(method)
		_, hasCause := jsonLookup(cause, "cause")
		identified = hasMessage && namedID(messageID) && hasMethod && isString && methodErr == nil && hasCause
	} else {
		identified = hasInputID && namedID(inputIDValue)
	}
	stage, isString := jsonString(jsonIndex(cause, "stage"))
	valid := hasOnlyKeys(cause, keys) && jsonIsString(jsonIndex(cause, "kind"), "input_delivery_result") &&
		identified && isString && slices.Contains(deliveryResultStages, stage)
	if err := brokerRuntimeRequire(valid, "cause_json", "a delivery result names its input or preview message and its stage"); err != nil {
		return err
	}
	if uncertain, present := jsonLookup(cause, "cause"); present {
		kind, isString := jsonString(jsonIndex(uncertain, "kind"))
		_, kindErr := stopCauseTryFrom(kind)
		recorded := uncertain.Kind() == '{' && len(jsonKeys(uncertain)) == 1 && isString && kindErr == nil
		if err := brokerRuntimeRequire(recorded, "cause_json.cause.kind", "a delivery result cause is a recorded uncertain cause"); err != nil {
			return err
		}
	}
	return nil
}

func compatibleCause(event runtimeEventKind, cause runtimeCauseKind) bool {
	switch event {
	case runtimeEventKindCoverageUnknown:
		return cause == runtimeCauseKindCoverageUnknown
	case runtimeEventKindRunnerExited:
		return cause == runtimeCauseKindNativeExit
	case runtimeEventKindRunnerLost:
		return cause == runtimeCauseKindMonitorLost || cause == runtimeCauseKindRuntimePathMissing
	case runtimeEventKindRunnerDegraded:
		return cause == runtimeCauseKindStoragePressure || cause == runtimeCauseKindStorageRecovered ||
			cause == runtimeCauseKindObserverError || cause == runtimeCauseKindEventRejected
	case runtimeEventKindInteractionOpened, runtimeEventKindInteractionClosed,
		runtimeEventKindInputDeliveryResult, runtimeEventKindInputDeliveryStalled:
		return false
	default:
		panic("invalid runtimeEventKind")
	}
}

func validateCause(transition observedTransition) error {
	cause := transition.CauseJSON
	if cause == nil || cause.Kind() != '{' {
		return brokerRuntimeInvalid("cause_json", "typed lifecycle cause required")
	}
	kindText, isString := jsonString(jsonIndex(cause, "kind"))
	if _, present := jsonLookup(cause, "kind"); !present || !isString {
		return brokerRuntimeInvalid("cause_json.kind", "typed cause kind required")
	}
	kind, err := runtimeCauseKindTryFrom(kindText)
	if err != nil {
		return err
	}
	if err := brokerRuntimeRequire(compatibleCause(transition.Kind, kind), "cause_json.kind", "cause does not match lifecycle event"); err != nil {
		return err
	}
	var allowed []string
	switch kind {
	case runtimeCauseKindEventRejected:
		return validateRejectionCause(cause)
	case runtimeCauseKindNativeExit:
		allowed = []string{"kind", "exit_code", "exit_signal"}
	case runtimeCauseKindStoragePressure, runtimeCauseKindStorageRecovered, runtimeCauseKindObserverError:
		allowed = []string{"kind", "gap_count", "gap_started_at", "gap_ended_at"}
	case runtimeCauseKindCoverageUnknown, runtimeCauseKindMonitorLost, runtimeCauseKindRuntimePathMissing:
		allowed = []string{"kind"}
	default:
		panic("invalid runtimeCauseKind")
	}
	if err := brokerRuntimeRequire(hasOnlyKeys(cause, allowed), "cause_json", "unknown lifecycle cause field"); err != nil {
		return err
	}
	for _, key := range []string{"exit_code", "exit_signal", "gap_count"} {
		if value, present := jsonLookup(cause, key); present {
			_, integer := jsonInt64(value)
			if err := brokerRuntimeRequire(jsonIsNull(value) || integer, key, "nullable integer required"); err != nil {
				return err
			}
		}
	}
	for _, key := range []string{"gap_started_at", "gap_ended_at"} {
		if value, present := jsonLookup(cause, key); present && !jsonIsNull(value) {
			text, isString := jsonString(value)
			if !isString {
				return brokerRuntimeInvalid(key, "timestamp string required")
			}
			if err := rfc3339(text, key); err != nil {
				return err
			}
		}
	}
	if kind == runtimeCauseKindNativeExit {
		_, exitCode := jsonInt64(jsonIndex(cause, "exit_code"))
		_, exitSignal := jsonInt64(jsonIndex(cause, "exit_signal"))
		if err := brokerRuntimeRequire(exitCode || exitSignal, "cause_json", "actual native exit status required"); err != nil {
			return err
		}
	}
	return nil
}

var rejectionCauseFields = []string{"kind", "rejected_kind", "rejected_observed_at", "error"}

func validateRejectionCause(cause jsontext.Value) error {
	if err := brokerRuntimeRequire(hasExactlyKeys(cause, rejectionCauseFields), "cause_json", "an event_rejected cause has exactly its rejected kind, time and error"); err != nil {
		return err
	}
	rejectedKind, isString := jsonString(jsonIndex(cause, "rejected_kind"))
	_, kindErr := runtimeEventKindTryFrom(rejectedKind)
	if err := brokerRuntimeRequire(isString && kindErr == nil, "cause_json.rejected_kind", "a runtime event kind required"); err != nil {
		return err
	}
	observedAt, isString := jsonString(jsonIndex(cause, "rejected_observed_at"))
	if !isString {
		return brokerRuntimeInvalid("cause_json.rejected_observed_at", "timestamp required")
	}
	if err := rfc3339(observedAt, "cause_json.rejected_observed_at"); err != nil {
		return err
	}
	text, isString := jsonString(jsonIndex(cause, "error"))
	return brokerRuntimeRequire(isString && isDiagnosticText(text), "cause_json.error", "a sanitized diagnostic text required")
}

const diagnosticTextLimit = 512

func isDiagnosticControl(c rune) bool {
	return c <= '\x1f' || c == '\x7f'
}

func isDiagnosticText(text string) bool {
	return text != "" && len(text) <= diagnosticTextLimit && strings.TrimSpace(text) == text &&
		!strings.ContainsFunc(text, isDiagnosticControl)
}

// diagnosticText is a runner diagnostic as stored or transmitted: control characters become
// spaces, invalid UTF-8 becomes U+FFFD, and the trimmed text keeps at most 512 bytes. Every
// caller passes a non-empty error message.
func diagnosticText(message string) string {
	replaced := strings.Map(func(c rune) rune {
		if isDiagnosticControl(c) {
			return ' '
		}
		return c
	}, message)
	text := strings.TrimSpace(replaced)
	if len(text) > diagnosticTextLimit {
		boundary := diagnosticTextLimit
		for !utf8RuneStart(text, boundary) {
			boundary--
		}
		text = strings.TrimRightFunc(text[:boundary], isSpace)
	}
	if text == "" {
		panic("a runner diagnostic is a non-empty error message")
	}
	return text
}

func utf8RuneStart(text string, index int) bool {
	return index == 0 || index == len(text) || text[index]&0xC0 != 0x80
}

func isSpace(c rune) bool {
	return strings.TrimSpace(string(c)) == ""
}

// eventRejectedTransition is the run-level runner_degraded event that takes a never-persistable
// transition's sequence slot.
func eventRejectedTransition(run runRecord, rejected observedTransition, screenSeq uint64, cause *cafleetError, observedAt string) (observedTransition, error) {
	evidence, err := runEvidence(run, "runner-lifecycle", screenSeq)
	if err != nil {
		return observedTransition{}, err
	}
	return observedTransition{
		RunID:          rejected.RunID,
		EventSeq:       rejected.EventSeq,
		EpisodeSeq:     none[int64](),
		Kind:           runtimeEventKindRunnerDegraded,
		Classification: none[interactionClass](),
		ObservedAt:     observedAt,
		EvidenceJSON:   evidence,
		CloseReason:    none[interactionCloseReason](),
		CauseJSON: jsonValue(jsonObject{
			{"kind", runtimeCauseKindEventRejected.asStr()},
			{"rejected_kind", rejected.Kind.asStr()},
			{"rejected_observed_at", rejected.ObservedAt},
			{"error", diagnosticText(cause.message())},
		}),
	}, nil
}

// isEventRejection reports whether transition is a rejection event, which is never itself
// replaced.
func isEventRejection(transition observedTransition) bool {
	return transition.Kind == runtimeEventKindRunnerDegraded && transition.CauseJSON != nil &&
		jsonIsString(jsonIndex(transition.CauseJSON, "kind"), runtimeCauseKindEventRejected.asStr())
}

func applyInteraction(ctx context.Context, conn *dbConn, transition observedTransition) (opt[int64], error) {
	sequence, episodic := transition.EpisodeSeq.get()
	if !episodic {
		return none[int64](), nil
	}
	class, classified := transition.Classification.get()
	if !classified {
		panic("validated episode has classification")
	}
	if transition.Kind == runtimeEventKindInteractionOpened {
		latest, err := queryRow(conn, "SELECT MAX(episode_seq) FROM member_interactions WHERE run_id=?1",
			[]any{transition.RunID}, func(row sqlRow) (opt[int64], error) { return row.optInt64(0) })
		if err != nil {
			return none[int64](), err
		}
		previous, opened := latest.get()
		if err := brokerRuntimeRequire(!opened || sequence > previous, "episode_seq", "episode sequence is not increasing"); err != nil {
			return none[int64](), err
		}
		digest, isString := jsonString(jsonIndex(transition.EvidenceJSON, "evidence_digest"))
		if !isString {
			panic("validated digest")
		}
		if _, err := conn.execute(
			`INSERT INTO member_interactions(run_id,episode_seq,classification,evidence_digest,opened_at,evidence_json)
             VALUES(?1,?2,?3,?4,?5,?6)`,
			transition.RunID, sequence, class.asStr(), digest, transition.ObservedAt, jsonText(transition.EvidenceJSON)); err != nil {
			return none[int64](), err
		}
		return some(conn.raw.LastInsertRowID()), nil
	}
	id, err := interactionIDForEpisode(conn, transition.RunID, sequence)
	if err != nil {
		return none[int64](), err
	}
	current, err := interaction(ctx, conn, id)
	if err != nil {
		return none[int64](), err
	}
	_, closed := current.closedAt.get()
	if err := brokerRuntimeRequire(current.classification == class && !closed, "interaction_id", "episode is closed or classification changed"); err != nil {
		return none[int64](), err
	}
	if reason, set := transition.CloseReason.get(); set {
		if _, err := conn.execute("UPDATE member_interactions SET closed_at=?1,close_reason=?2 WHERE interaction_id=?3",
			transition.ObservedAt, reason.asStr(), id); err != nil {
			return none[int64](), err
		}
	}
	return some(id), nil
}

// deliveryFields is the public fields an input delivery result adds from its input receipt.
type deliveryFields struct {
	inputID          opt[int64]
	messageID        opt[int64]
	blockedByInputID opt[int64]
	delivery         opt[deliveryProjection]
	stage            jsontext.Value
	cause            opt[stopCause]
	action           jsontext.Value
	waitingSince     opt[string]
	// archive is set for a record that commits as a silent archived row instead of notifying
	// the Director.
	archive opt[archiveReason]
}

func actionDatabaseArgs(conn *dbConn) ([]string, error) {
	url := "sqlite:///" + conn.path()
	defaultURL, err := defaultDatabaseURL()
	if err != nil {
		return nil, err
	}
	if url == defaultURL {
		return []string{}, nil
	}
	return []string{"--database-url", url}, nil
}

// stalledDelivery is the fields of a stalled event: a queued ordinary head of this run, a
// message addressed to the run's member, or a prompt/exec input of this run that blocks them in
// any state. A new record whose head is no longer ready archives as head_not_ready.
func stalledDelivery(ctx context.Context, conn *dbConn, run runRecord, record jsontext.Value, published bool) (deliveryFields, error) {
	inputID, hasInput := jsonInt64(jsonIndex(record, "input_id"))
	messageID, hasMessage := jsonInt64(jsonIndex(record, "message_id"))
	blocker, hasBlocker := jsonInt64(jsonIndex(record, "blocked_by_input_id"))
	archive := none[archiveReason]()
	var action jsontext.Value
	var err error
	switch {
	case hasInput && !hasMessage && !hasBlocker:
		input, err := getInput(ctx, conn, inputID)
		if err != nil {
			return deliveryFields{}, err
		}
		_, deferred := input.deferredAt.get()
		if err := brokerRuntimeRequire(input.runID == run.runID && deferred, "input_id", "a stalled head is a queued ordinary input of this run"); err != nil {
			return deliveryFields{}, err
		}
		if !published && input.state != inputStateReady {
			archive = some(archiveReasonHeadNotReady)
		}
		if action, err = reviewStalledDeliveryAction(conn, run); err != nil {
			return deliveryFields{}, err
		}
	case !hasInput && hasMessage && !hasBlocker:
		message, err := getMessage(ctx, conn, messageID)
		if err != nil {
			return deliveryFields{}, err
		}
		if err := brokerRuntimeRequire(message.toMemberID == run.memberID, "message_id", "a stalled preview addresses the run's member"); err != nil {
			return deliveryFields{}, err
		}
		if action, err = reviewStalledDeliveryAction(conn, run); err != nil {
			return deliveryFields{}, err
		}
	case !hasInput && !hasMessage && hasBlocker:
		blocking, err := getInput(ctx, conn, blocker)
		if err != nil {
			return deliveryFields{}, err
		}
		if err := brokerRuntimeRequire(blocking.runID == run.runID, "blocked_by_input_id", "a stalled blocker is an input of this run"); err != nil {
			return deliveryFields{}, err
		}
		if action, err = reconcileUncertainInputAction(conn, blocker); err != nil {
			return deliveryFields{}, err
		}
	default:
		return deliveryFields{}, brokerRuntimeInvalid("input_id", "a stalled event names exactly one waiting input, message or blocker")
	}
	waitingSince, isString := jsonString(jsonIndex(record, "waiting_since"))
	if !isString {
		return deliveryFields{}, brokerRuntimeInvalid("waiting_since", "timestamp required")
	}
	if err = rfc3339(waitingSince, "waiting_since"); err != nil {
		return deliveryFields{}, err
	}
	optional := func(value int64, set bool) opt[int64] {
		if set {
			return some(value)
		}
		return none[int64]()
	}
	return deliveryFields{
		inputID:          optional(inputID, hasInput),
		messageID:        optional(messageID, hasMessage),
		blockedByInputID: optional(blocker, hasBlocker),
		delivery:         none[deliveryProjection](),
		stage:            jsonNull,
		cause:            none[stopCause](),
		waitingSince:     some(waitingSince),
		action:           action,
		archive:          archive,
	}, nil
}

func reviewStalledDeliveryAction(conn *dbConn, run runRecord) (jsontext.Value, error) {
	argv, err := memberCaptureArgv(conn, run.memberID)
	if err != nil {
		return nil, err
	}
	return jsonValue(jsonObject{
		{"kind", "review_stalled_delivery"},
		{"instruction", reviewStalledDeliveryInstruction},
		{"inspect_argv", argv},
	}), nil
}

func reconcileUncertainInputAction(conn *dbConn, blocker int64) (jsontext.Value, error) {
	databaseArgs, err := actionDatabaseArgs(conn)
	if err != nil {
		return nil, err
	}
	argv := append([]string{"cafleet", "input", "show", strconv.FormatInt(blocker, 10), "--json"}, databaseArgs...)
	return jsonValue(jsonObject{
		{"kind", "reconcile_uncertain_input"},
		{"instruction", reconcileUncertainInputInstruction},
		{"inspect_argv", argv},
	}), nil
}

const reconcileUncertainInputInstruction = "Inspect the uncertain input and the member's pane, then settle the input with input reconcile from that evidence. Queued input and message previews resume once it settles."

func memberCaptureArgv(conn *dbConn, memberID int64) ([]string, error) {
	databaseArgs, err := actionDatabaseArgs(conn)
	if err != nil {
		return nil, err
	}
	return append([]string{"cafleet", "member", "capture", strconv.FormatInt(memberID, 10), "--json"}, databaseArgs...), nil
}

const reviewStalledDeliveryInstruction = "Capture the member's pane. When the composer holds text, submit or clear it with member press keys the capture establishes, or ask the user to clear or submit it on the member's native surface."

// eventPayload is the Rust payload: the complete public payload of an event.
func eventPayload(_ *dbConn, run runRecord, transition observedTransition, eventID int64, interactionID opt[int64], delivery *deliveryFields) (jsontext.Value, error) {
	classification := none[string]()
	if class, set := transition.Classification.get(); set {
		classification = some(class.asStr())
	}
	payload := jsonObject{
		{"event_id", eventID},
		{"run_id", run.runID},
		{"event_seq", transition.EventSeq},
		{"member_id", run.memberID},
		{"fleet_id", run.fleetID},
		{"director_member_id", run.directorMemberID},
		{"kind", transition.Kind.asStr()},
		{"interaction_id", interactionID},
		{"classification", classification},
		{"observed_at", transition.ObservedAt},
		{"source", source(transition.Kind)},
		{"decision_outcome", "unknown"},
		{"evidence", transition.EvidenceJSON},
	}
	if reason, set := transition.CloseReason.get(); set {
		payload = payload.with("close_reason", reason.asStr())
	}
	if transition.CauseJSON != nil && !isDeliveryEvent(transition.Kind) {
		payload = payload.with("cause", transition.CauseJSON)
	}
	if delivery != nil {
		if cause, set := delivery.cause.get(); set {
			payload = payload.with("cause", stopCausePayload(cause))
		}
		payload = payload.with("input_id", delivery.inputID)
		payload = payload.with("message_id", delivery.messageID)
		waitingSince, waiting := delivery.waitingSince.get()
		projection, projected := delivery.delivery.get()
		switch {
		case waiting && !projected:
			payload = payload.with("blocked_by_input_id", delivery.blockedByInputID)
			payload = payload.with("waiting_since", waitingSince)
		case !waiting && projected:
			payload = payload.with("delivery", projection.toJSON())
			payload = payload.with("stage", delivery.stage)
		default:
			panic("a delivery result has a projection and a stalled event a wait start")
		}
		payload = payload.with("director_action", delivery.action)
	} else {
		action, err := directorAction(run, transition.Kind, transition.Classification, eventID, interactionID)
		if err != nil {
			return nil, err
		}
		payload = payload.with("director_action", action)
	}
	return jsonValue(payload), nil
}

func persistTransition(ctx context.Context, conn *dbConn, owner runOwner, transition observedTransition, mode persistenceMode) (runtimeEventRecord, bool, error) {
	type persisted struct {
		event    runtimeEventRecord
		inserted bool
	}
	result, err := ownedTransaction(ctx, conn, owner, func(ctx context.Context, conn *dbConn, run runRecord) (persisted, error) {
		event, inserted, err := persistTransitionInTransaction(ctx, conn, run, transition, mode)
		return persisted{event: event, inserted: inserted}, err
	})
	return result.event, result.inserted, err
}

// duplicatedDelivery is the persisted delivery record of the same kind that names the same
// waiting input, preview or blocker; stalled head and preview records match only unblocked rows.
func duplicatedDelivery(conn *dbConn, run runRecord, kind runtimeEventKind, record jsontext.Value) (opt[int64], error) {
	unblocked := ""
	if kind == runtimeEventKindInputDeliveryStalled {
		unblocked = " AND json_type(payload_json,'$.blocked_by_input_id')='null'"
	}
	var condition string
	var id int64
	if inputID, set := jsonInt64(jsonIndex(record, "input_id")); set {
		condition, id = "json_extract(payload_json,'$.input_id')=?3"+unblocked, inputID
	} else if messageID, set := jsonInt64(jsonIndex(record, "message_id")); set {
		condition, id = "json_type(payload_json,'$.input_id')='null' AND json_extract(payload_json,'$.message_id')=?3"+unblocked, messageID
	} else {
		blocker, set := jsonInt64(jsonIndex(record, "blocked_by_input_id"))
		if !set {
			panic("a validated delivery record names its input, message or blocker")
		}
		condition, id = "json_extract(payload_json,'$.blocked_by_input_id')=?3", blocker
	}
	return queryRowOptional(conn,
		"SELECT event_id FROM member_runtime_events WHERE run_id=?1 AND kind=?2 AND "+condition,
		[]any{run.runID, kind.asStr(), id}, func(row sqlRow) (int64, error) { return row.int64(0) })
}

func persistTransitionInTransaction(ctx context.Context, conn *dbConn, run runRecord, transition observedTransition, mode persistenceMode) (runtimeEventRecord, bool, error) {
	if err := validateTransition(run, transition); err != nil {
		return runtimeEventRecord{}, false, err
	}
	if err := brokerRuntimeRequire(mode != persistenceModeRecovery || run.ownerKind == ownerKindReconciler, "owner_kind", "recovery requires reconciler ownership"); err != nil {
		return runtimeEventRecord{}, false, err
	}
	previous, err := queryRowOptional(conn, "SELECT event_id FROM member_runtime_events WHERE run_id=?1 AND event_seq=?2",
		[]any{run.runID, transition.EventSeq}, func(row sqlRow) (int64, error) { return row.int64(0) })
	if err != nil {
		return runtimeEventRecord{}, false, err
	}
	if id, set := previous.get(); set {
		existing, err := getEvent(ctx, conn, id)
		if err != nil {
			return runtimeEventRecord{}, false, err
		}
		recorded, err := original(existing)
		if err != nil {
			return runtimeEventRecord{}, false, err
		}
		if err := brokerRuntimeRequire(recorded.equal(transition), "event_seq", "conflicting original transition"); err != nil {
			return runtimeEventRecord{}, false, err
		}
		return existing.event, false, nil
	}
	if err := brokerRuntimeRequire(mode == persistenceModeRecovery || !run.status.terminal(), "member_runs.status", "terminal runs accept only committed retries or owned recovery"); err != nil {
		return runtimeEventRecord{}, false, err
	}
	if err := brokerRuntimeRequire(transition.Kind != runtimeEventKindRunnerLost || mode == persistenceModeRecovery, "kind", "observer loss requires explicit recovery ownership"); err != nil {
		return runtimeEventRecord{}, false, err
	}
	historicalArchive, err := historicalOwnership(ctx, conn, run)
	if err != nil {
		return runtimeEventRecord{}, false, err
	}
	_, archived := historicalArchive.get()
	if err := brokerRuntimeRequire(mode == persistenceModeRecovery || (!archived && run.ownerKind == ownerKindRunner), "notification_state", "live notification requires active runner ownership"); err != nil {
		return runtimeEventRecord{}, false, err
	}
	if !archived {
		if _, err := requireLiveFleet(ctx, conn, run.fleetID); err != nil {
			return runtimeEventRecord{}, false, err
		}
	}
	duplicate := false
	var fields *deliveryFields
	if isDeliveryEvent(transition.Kind) {
		record := transition.CauseJSON
		if record == nil {
			panic("validated delivery results carry their record")
		}
		found, err := duplicatedDelivery(conn, run, transition.Kind, record)
		if err != nil {
			return runtimeEventRecord{}, false, err
		}
		_, duplicate = found.get()
		var computed deliveryFields
		if transition.Kind == runtimeEventKindInputDeliveryStalled {
			computed, err = stalledDelivery(ctx, conn, run, record, false)
		} else if inputID, set := jsonInt64(jsonIndex(record, "input_id")); set {
			computed, err = resultDeliveryFields(ctx, conn, run, inputID, record)
		} else {
			computed, err = previewDelivery(ctx, conn, run, record)
		}
		if err != nil {
			return runtimeEventRecord{}, false, err
		}
		fields = &computed
	}
	archiveReason := historicalArchive
	if !archived && fields != nil {
		if duplicate {
			archiveReason = some(archiveReasonDuplicateDelivery)
		} else {
			archiveReason = fields.archive
		}
	}
	latest, err := latestEventSeq(ctx, conn, run.runID)
	if err != nil {
		return runtimeEventRecord{}, false, err
	}
	latestSeq, _ := latest.get()
	if err := brokerRuntimeRequire(transition.EventSeq == latestSeq+1, "event_seq", "event sequence is not the run's next value"); err != nil {
		return runtimeEventRecord{}, false, err
	}
	interactionID, err := applyInteraction(ctx, conn, transition)
	if err != nil {
		return runtimeEventRecord{}, false, err
	}
	state, messageID := notificationStateArchived, none[int64]()
	if _, archive := archiveReason.get(); !archive {
		message, err := insertMessageValidated(ctx, conn, run.memberID, run.directorMemberID, "{}")
		if err != nil {
			return runtimeEventRecord{}, false, err
		}
		state, messageID = notificationStateInbox, some(message.messageID)
	}
	archiveText := none[string]()
	if reason, set := archiveReason.get(); set {
		archiveText = some(reason.asStr())
	}
	if _, err := conn.execute(
		`INSERT INTO member_runtime_events(run_id,event_seq,interaction_id,kind,observed_at,payload_json,notification_state,message_id,archive_reason)
             VALUES(?1,?2,?3,?4,?5,'{}',?6,?7,?8)`,
		run.runID, transition.EventSeq, interactionID, transition.Kind.asStr(), transition.ObservedAt,
		state.asStr(), messageID, archiveText); err != nil {
		return runtimeEventRecord{}, false, err
	}
	eventID := conn.raw.LastInsertRowID()
	encoded, err := eventPayload(conn, run, transition, eventID, interactionID, fields)
	if err != nil {
		return runtimeEventRecord{}, false, err
	}
	payload := string(encoded)
	if err := brokerRuntimeRequire(len(payload) <= payloadLimit, "payload_json", "complete payload exceeds bound"); err != nil {
		return runtimeEventRecord{}, false, err
	}
	if _, err := conn.execute("UPDATE member_runtime_events SET payload_json=?1 WHERE event_id=?2", payload, eventID); err != nil {
		return runtimeEventRecord{}, false, err
	}
	if id, set := messageID.get(); set {
		if _, err := conn.execute("UPDATE messages SET text=?1 WHERE message_id=?2", payload, id); err != nil {
			return runtimeEventRecord{}, false, err
		}
	}
	if transition.Kind == runtimeEventKindRunnerExited {
		if transition.CauseJSON == nil {
			panic("validateCause requires a native exit cause for runnerExited")
		}
		exitCode := none[int64]()
		if code, set := jsonInt64(jsonIndex(transition.CauseJSON, "exit_code")); set {
			exitCode = some(code)
		}
		if _, err := conn.execute("UPDATE member_runs SET status='exited',ended_at=?1,exit_code=?2 WHERE run_id=?3",
			transition.ObservedAt, exitCode, run.runID); err != nil {
			return runtimeEventRecord{}, false, err
		}
	}
	view, err := getEvent(ctx, conn, eventID)
	if err != nil {
		return runtimeEventRecord{}, false, err
	}
	return view.event, true, nil
}

func resultDeliveryFields(ctx context.Context, conn *dbConn, run runRecord, inputID int64, record jsontext.Value) (deliveryFields, error) {
	input, delivery, err := resultDelivery(ctx, conn, run.runID, inputID, "input_id")
	if err != nil {
		return deliveryFields{}, err
	}
	if err := validateResultStage(jsonIndex(record, "stage"), "cause_json.stage"); err != nil {
		return deliveryFields{}, err
	}
	return deliveryFields{
		inputID:          some(inputID),
		messageID:        none[int64](),
		blockedByInputID: none[int64](),
		cause:            resultStopCause(input, delivery.Status),
		action:           jsonNull,
		waitingSince:     none[string](),
		archive:          none[archiveReason](),
		delivery:         some(delivery),
		stage:            jsonIndex(record, "stage"),
	}, nil
}

const (
	reviewInstruction   = "Verify this event and the current request. Evaluate the operation and its risk. In the Director session's effective auto mode, decide allow or deny; otherwise obtain the user's permission. Submit the selected answer through the request-bound CAFleet response command, or give that complete command to the user. Inspect the native result and continuation afterward."
	questionInstruction = "Verify this event and the current question. Inspect it for the choices CAFleet can answer and their scope. In the Director session's effective auto mode, choose one of those choices; otherwise obtain the user's choice. Submit it through the request-bound CAFleet response command with that choice, or give that complete command to the user. When the inspection lists no choice, ask the user to answer on the member's native surface. Inspect the native result and continuation afterward."
)

func directorAction(run runRecord, kind runtimeEventKind, classification opt[interactionClass], eventID int64, interactionID opt[int64]) (jsontext.Value, error) {
	class, classified := classification.get()
	if kind != runtimeEventKindInteractionOpened || !classified {
		return jsonNull, nil
	}
	var action, instruction string
	switch class {
	case interactionClassPermissionRequest:
		action, instruction = "review_permission_request", reviewInstruction
	case interactionClassQuestion:
		action, instruction = "review_question", questionInstruction
	case interactionClassUnknownInteraction:
		return jsonNull, nil
	default:
		panic("invalid interactionClass")
	}
	id, set := interactionID.get()
	if !set {
		return nil, brokerRuntimeInvalid("interaction_id", "an answerable opening requires an interaction")
	}
	if err := brokerRuntimeRequire(eventID > 0 && id > 0 && run.directorMemberID > 0, "director_action", "allocated positive action identities required"); err != nil {
		return nil, err
	}
	return jsonValue(jsonObject{
		{"kind", action},
		{"instruction", instruction},
		{"inspect_argv", []string{
			"cafleet", "permission", "inspect", strconv.FormatInt(id, 10), "--event-id", strconv.FormatInt(eventID, 10),
			"--from-member-id", strconv.FormatInt(run.directorMemberID, 10), "--json",
		}},
	}), nil
}
