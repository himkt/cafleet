package core

import (
	"context"
	"encoding/binary"
	"encoding/json/jsontext"
	"errors"
	"fmt"
	"math"
	"os"
	"runtime"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"testing"
	"time"

	"golang.org/x/sys/unix"
)

// independentFrame is a u32 big-endian length prefix followed by body, built without the
// production framer.
func independentFrame(body []byte) []byte {
	if uint64(len(body)) > math.MaxUint32 {
		panic("a fixture frame body fits a u32 length")
	}
	frame := binary.BigEndian.AppendUint32(nil, uint32(len(body))) //nolint:gosec // bounded by the check above
	return append(frame, body...)
}

// realTimeBound bounds every wait on a real socket, lock or database in these tests. It sits far
// above what an unloaded exchange takes, so a loaded run still finishes inside it, and an
// assertion that bounded code returns early compares against a fraction of it.
const realTimeBound = 10 * time.Second

func feedValues(decoder *controlDecoder, data []byte) ([]jsontext.Value, error) {
	frames, err := decoder.feedAdmitted(data, time.Now())
	if err != nil {
		return nil, err
	}
	values := make([]jsontext.Value, 0, len(frames))
	for _, frame := range frames {
		value, err := decodeLifecycleControlBody(frame.body)
		if err != nil {
			return nil, err
		}
		values = append(values, value)
	}
	return values, nil
}

// transferSetting transfers the run to this process, then applies the member_runs SET list
// columns, where ?1 is this process's pid and ?2 its start identity.
func transferSetting(t *testing.T, fixture *runnerFixture, columns string) *runLock {
	t.Helper()
	held := acquireRunLock(t, fixture.conn, fixture.run.owner())
	process := currentProcess(t)
	next, err := held.transferToRunner(context.Background(), fixture.conn, fixture.run.owner(), process)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := sqlExecute(fixture.conn, "UPDATE member_runs SET "+columns+" WHERE run_id=?3",
		process.pid, process.startIdentity, next.runID); err != nil {
		t.Fatal(err)
	}
	fixture.refresh(t)
	return held
}

func activate(t *testing.T, fixture *runnerFixture) (*runArtifacts, *runLock) {
	t.Helper()
	artifacts := fixture.artifacts(t)
	held := transferSetting(t, fixture,
		"status='running',runner_pid=?1,runner_start_identity=?2,child_pid=?1,child_start_identity=?2")
	return artifacts, held
}

// boundSocket binds a nonblocking Unix stream socket at path with mode, listening when asked.
func boundSocket(t *testing.T, path string, mode uint32, listening bool) int {
	t.Helper()
	fd := cloexecSocket(t)
	if err := unix.Bind(fd, &unix.SockaddrUnix{Name: path}); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(path, os.FileMode(mode)); err != nil {
		t.Fatal(err)
	}
	if err := unix.SetNonblock(fd, true); err != nil {
		t.Fatal(err)
	}
	info, err := os.Lstat(path)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode()&os.ModeSocket == 0 {
		t.Fatalf("%s is a socket", path)
	}
	if listening {
		if err := unix.Listen(fd, 8); err != nil {
			t.Fatal(err)
		}
	}
	return fd
}

// cloexecSocket is a Unix stream socket marked close-on-exec under the fork lock, so a child
// started concurrently never inherits it; the test closes it.
func cloexecSocket(t *testing.T) int {
	t.Helper()
	syscall.ForkLock.RLock()
	fd, err := unix.Socket(unix.AF_UNIX, unix.SOCK_STREAM, 0)
	if err == nil {
		unix.CloseOnExec(fd)
	}
	syscall.ForkLock.RUnlock()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { closeFD(t, fd) })
	return fd
}

// cloexecSocketpair is a connected Unix stream pair marked close-on-exec under the fork lock;
// the caller owns both ends.
func cloexecSocketpair(t *testing.T) [2]int {
	t.Helper()
	syscall.ForkLock.RLock()
	fds, err := unix.Socketpair(unix.AF_UNIX, unix.SOCK_STREAM, 0)
	if err == nil {
		unix.CloseOnExec(fds[0])
		unix.CloseOnExec(fds[1])
	}
	syscall.ForkLock.RUnlock()
	if err != nil {
		t.Fatal(err)
	}
	return fds
}

func closeFD(t *testing.T, fd int) {
	t.Helper()
	if err := unix.Close(fd); err != nil && !errors.Is(err, unix.EBADF) {
		t.Errorf("closing fixture descriptor %d: %v", fd, err)
	}
}

func retryable(err error) bool {
	return errors.Is(err, unix.EAGAIN) || errors.Is(err, unix.EINTR)
}

// silentServer accepts and holds every connection to path without reading or replying.
func silentServer(t *testing.T, path string) {
	t.Helper()
	listener := boundSocket(t, path, 0o600, true)
	var stop atomic.Bool
	var done sync.WaitGroup
	done.Add(1)
	failure := make(chan error, 1)
	go func() {
		defer done.Done()
		end := time.Now().Add(realTimeBound)
		var connections []int
		defer func() {
			for _, connection := range connections {
				_ = unix.Close(connection)
			}
		}()
		for !stop.Load() && time.Now().Before(end) {
			connection, _, err := unix.Accept(listener)
			switch {
			case err == nil:
				connections = append(connections, connection)
			case retryable(err):
			default:
				failure <- fmt.Errorf("fixture socket accept failed: %w", err)
				return
			}
			time.Sleep(time.Millisecond)
		}
	}()
	t.Cleanup(func() {
		stop.Store(true)
		done.Wait()
		select {
		case err := <-failure:
			t.Error(err)
		default:
		}
	})
}

// readFixtureBytes fills buffer from the nonblocking fd before until.
func readFixtureBytes(fd int, buffer []byte, until time.Time) error {
	for len(buffer) > 0 {
		if !time.Now().Before(until) {
			return errors.New("fixture socket read timed out")
		}
		count, err := unix.Read(fd, buffer)
		switch {
		case err == nil && count == 0:
			return errors.New("fixture socket closed before the expected bytes")
		case err == nil:
			buffer = buffer[count:]
		case retryable(err):
			time.Sleep(time.Millisecond)
		default:
			return err
		}
	}
	return nil
}

// writeFixtureBytes writes data to the nonblocking fd before until.
func writeFixtureBytes(fd int, data []byte, until time.Time) error {
	for len(data) > 0 {
		if !time.Now().Before(until) {
			return errors.New("fixture socket write timed out")
		}
		count, err := unix.Write(fd, data)
		switch {
		case err == nil && count == 0:
			return errors.New("fixture socket write made no progress")
		case err == nil:
			data = data[count:]
		case retryable(err):
			time.Sleep(time.Millisecond)
		default:
			return err
		}
	}
	return nil
}

func jsonSetAt(t *testing.T, document jsontext.Value, raw string, path ...string) jsontext.Value {
	t.Helper()
	return testJSONWith(t, document, jsontext.Value(raw), path...)
}

func jsonRemoveAt(t *testing.T, document jsontext.Value, path ...string) jsontext.Value {
	t.Helper()
	parent := path[:len(path)-1]
	last := path[len(path)-1]
	if !jsonHas(t, jsonAt(t, document, parent...), last) {
		t.Fatalf("%s has %v", document, path)
	}
	if len(parent) == 0 {
		return testJSONWithout(t, document, last)
	}
	return testJSONWith(t, document, testJSONWithout(t, jsonAt(t, document, parent...), last), parent...)
}

func jsonQuoted(t *testing.T, text string) string {
	t.Helper()
	return string(mustJSON(t, text))
}

func decodedInspection(t *testing.T, request string) permissionInspection {
	t.Helper()
	inspection, err := decodePermissionInspection(jsontext.Value(request))
	if err != nil {
		t.Fatalf("the fixture inspection matches the reply shape: %v", err)
	}
	return inspection
}

// testInspection is a tested, visible permission inspection with every identity field set.
func testInspection(t *testing.T) permissionInspection {
	t.Helper()
	return decodedInspection(t, `{"event_id":11,"message_id":14,"member_id":3,"fleet_id":2,"director_member_id":1,`+
		`"run_id":7,"owner_epoch":2,"interaction_id":12,"episode_seq":1,`+
		`"classification":"permission_request","visibility":"visible",`+
		`"backend":"opencode","native_version":"2.1.4","answer_rule_id":"opencode-shell-allow-enter",`+
		`"answer_capability":"tested","capability_reason":null,"supported_decisions":["allow","deny"],`+
		`"supported_choices":[],`+
		`"request_token":"`+strings.Repeat("a", 64)+`","output_seq":5,"evidence":{"rule_id":"opencode-shell-permission"},`+
		`"missing_evidence":["arguments","cwd"]}`)
}

// testQuestionInspection is a tested, visible question inspection offering two dismissal choices.
func testQuestionInspection(t *testing.T) permissionInspection {
	t.Helper()
	return decodedInspection(t, `{"event_id":11,"message_id":14,"member_id":3,"fleet_id":2,"director_member_id":1,`+
		`"run_id":7,"owner_epoch":2,"interaction_id":12,"episode_seq":1,`+
		`"classification":"question","visibility":"visible",`+
		`"backend":"claude","native_version":"2.1.312","answer_rule_id":null,`+
		`"answer_capability":"tested","capability_reason":null,"supported_decisions":[],`+
		`"supported_choices":[`+
		`{"choice_id":"not-now","label":"Not now","scope":"persistent"},`+
		`{"choice_id":"dont-show-again","label":"Don't show again","scope":"persistent"}],`+
		`"request_token":"`+strings.Repeat("a", 64)+`","output_seq":5,"evidence":{"rule_id":"claude-auto-startup"},`+
		`"missing_evidence":[]}`)
}

func TestRuntimeControl_CheckStatusesOrderBySeverityAndRenderTheirWireNames(t *testing.T) {
	t.Parallel()
	statuses := []checkStatus{checkStatusError, checkStatusOk, checkStatusWarn, checkStatusInfo}
	slices.Sort(statuses)
	if !slices.Equal(statuses, []checkStatus{checkStatusOk, checkStatusInfo, checkStatusWarn, checkStatusError}) {
		t.Fatalf("severity order %v", statuses)
	}
	var names []string
	for _, status := range statuses {
		names = append(names, status.asStr())
	}
	if !slices.Equal(names, []string{"ok", "info", "warn", "error"}) {
		t.Fatalf("wire names %v", names)
	}
}

func afterMillis(origin time.Time, millis int) time.Time {
	return origin.Add(time.Duration(millis) * time.Millisecond)
}

func TestRuntimeControl_InputFramesTests_FramesEverySplitAndSingleByteFeedRetainsFirstConsumptionTime(t *testing.T) {
	t.Parallel()
	body := []byte(`{"protocol_version":1,"kind":"health","text":"literal \u4e16\u754c"}`)
	bytes := independentFrame(body)
	origin := time.Now()
	for cut := range len(bytes) {
		decoder := newControlDecoder()
		frames, err := decoder.feedAdmitted(bytes[:cut], origin)
		if err != nil || len(frames) != 0 {
			t.Fatalf("split %d: prefix frames %d, %v", cut, len(frames), err)
		}
		frames, err = decoder.feedAdmitted(bytes[cut:], afterMillis(origin, 100))
		if err != nil || len(frames) != 1 {
			t.Fatalf("split %d: frames %d, %v", cut, len(frames), err)
		}
		if string(frames[0].body) != string(body) {
			t.Fatalf("split %d: body %q", cut, frames[0].body)
		}
		value, err := decodeLifecycleControlBody(frames[0].body)
		if err != nil || !jsonEqual(value, jsontext.Value(body)) {
			t.Fatalf("split %d: decoded %s, %v", cut, value, err)
		}
		want := origin
		if cut == 0 {
			want = afterMillis(origin, 100)
		}
		if !frames[0].admittedAt.Equal(want) {
			t.Fatalf("split %d: admitted at %v, want %v", cut, frames[0].admittedAt, want)
		}
		if err := decoder.finish(); err != nil {
			t.Fatalf("split %d: %v", cut, err)
		}
	}
	decoder := newControlDecoder()
	var frames []admittedControlFrame
	for index, b := range bytes {
		fed, err := decoder.feedAdmitted([]byte{b}, afterMillis(origin, index))
		if err != nil {
			t.Fatal(err)
		}
		frames = append(frames, fed...)
	}
	if len(frames) != 1 || string(frames[0].body) != string(body) || !frames[0].admittedAt.Equal(origin) {
		t.Fatalf("single-byte feed frames %d", len(frames))
	}
	if err := decoder.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeControl_InputFramesTests_FramesEmptyFeedsLeaveAdmissionUnsetAndPreservePartialFrameTime(t *testing.T) {
	t.Parallel()
	origin := time.Now()
	body := []byte(`{"protocol_version":1,"kind":"health"}`)
	bytes := independentFrame(body)
	decoder := newControlDecoder()
	if frames, err := decoder.feedAdmitted(nil, origin); err != nil || len(frames) != 0 {
		t.Fatalf("empty feed %d, %v", len(frames), err)
	}
	if err := decoder.finish(); err != nil {
		t.Fatal(err)
	}
	if frames, err := decoder.feedAdmitted(bytes[:1], afterMillis(origin, 10)); err != nil || len(frames) != 0 {
		t.Fatalf("first byte %d, %v", len(frames), err)
	}
	if decoder.finish() == nil {
		t.Fatal("a partial header is truncated")
	}
	if frames, err := decoder.feedAdmitted(nil, afterMillis(origin, 20)); err != nil || len(frames) != 0 {
		t.Fatalf("empty partial feed %d, %v", len(frames), err)
	}
	frames, err := decoder.feedAdmitted(bytes[1:], afterMillis(origin, 30))
	if err != nil || len(frames) != 1 {
		t.Fatalf("frames %d, %v", len(frames), err)
	}
	if string(frames[0].body) != string(body) || !frames[0].admittedAt.Equal(afterMillis(origin, 10)) {
		t.Fatalf("frame %q at %v", frames[0].body, frames[0].admittedAt)
	}
	if err := decoder.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeControl_InputFramesTests_FramesPartialCompleteAndNewTailKeepEachFirstHeaderTime(t *testing.T) {
	t.Parallel()
	bodies := [][]byte{[]byte("first"), []byte("second"), []byte("third")}
	var encoded [][]byte
	for _, body := range bodies {
		encoded = append(encoded, independentFrame(body))
	}
	origin := time.Now()
	decoder := newControlDecoder()
	if frames, err := decoder.feedAdmitted(encoded[0][:2], origin); err != nil || len(frames) != 0 {
		t.Fatalf("partial header %d, %v", len(frames), err)
	}
	next := slices.Concat(encoded[0][2:], encoded[1], encoded[2][:1])
	frames, err := decoder.feedAdmitted(next, afterMillis(origin, 10))
	if err != nil || len(frames) != 2 {
		t.Fatalf("frames %d, %v", len(frames), err)
	}
	if string(frames[0].body) != "first" || !frames[0].admittedAt.Equal(origin) {
		t.Fatalf("first frame %q at %v", frames[0].body, frames[0].admittedAt)
	}
	if string(frames[1].body) != "second" || !frames[1].admittedAt.Equal(afterMillis(origin, 10)) {
		t.Fatalf("second frame %q at %v", frames[1].body, frames[1].admittedAt)
	}
	if decoder.finish() == nil {
		t.Fatal("a started third header is truncated")
	}
	tail, err := decoder.feedAdmitted(encoded[2][1:], afterMillis(origin, 20))
	if err != nil || len(tail) != 1 {
		t.Fatalf("tail %d, %v", len(tail), err)
	}
	if string(tail[0].body) != "third" || !tail[0].admittedAt.Equal(afterMillis(origin, 10)) {
		t.Fatalf("third frame %q at %v", tail[0].body, tail[0].admittedAt)
	}
	if err := decoder.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeControl_InputFramesTests_FramesInclusiveBodyLimitAndHeaderOnlyOversizeRejection(t *testing.T) {
	t.Parallel()
	origin := time.Now()
	body := []byte(strings.Repeat("x", 65536))
	decoder := newControlDecoder()
	frames, err := decoder.feedAdmitted(independentFrame(body), origin)
	if err != nil || len(frames) != 1 {
		t.Fatalf("frames %d, %v", len(frames), err)
	}
	if string(frames[0].body) != string(body) || !frames[0].admittedAt.Equal(origin) {
		t.Fatal("the inclusive cap frame keeps its body and time")
	}
	if err := decoder.finish(); err != nil {
		t.Fatal(err)
	}
	for _, size := range []uint32{65537, ^uint32(0)} {
		decoder := newControlDecoder()
		header := binary.BigEndian.AppendUint32(nil, size)
		for _, b := range header[:3] {
			if frames, err := decoder.feedAdmitted([]byte{b}, origin); err != nil || len(frames) != 0 {
				t.Fatalf("declared %d: header byte %d, %v", size, len(frames), err)
			}
		}
		if _, err := decoder.feedAdmitted(header[3:], afterMillis(origin, 10)); err == nil {
			t.Fatalf("declared %d is refused at its header", size)
		}
	}
}

func TestRuntimeControl_InputFramesTests_FramesEofRejectsEachPartialBoundaryAndEmptyBodyReachesConsumer(t *testing.T) {
	t.Parallel()
	origin := time.Now()
	bytes := independentFrame([]byte(`{"protocol_version":1,"kind":"health"}`))
	if err := newControlDecoder().finish(); err != nil {
		t.Fatal(err)
	}
	for cut := 1; cut < len(bytes); cut++ {
		decoder := newControlDecoder()
		if frames, err := decoder.feedAdmitted(bytes[:cut], origin); err != nil || len(frames) != 0 {
			t.Fatalf("prefix %d: %d, %v", cut, len(frames), err)
		}
		if decoder.finish() == nil {
			t.Fatalf("prefix %d is truncated", cut)
		}
	}
	decoder := newControlDecoder()
	frames, err := decoder.feedAdmitted(independentFrame(nil), origin)
	if err != nil || len(frames) != 1 || len(frames[0].body) != 0 || !frames[0].admittedAt.Equal(origin) {
		t.Fatalf("empty body frames %d, %v", len(frames), err)
	}
	if err := decoder.finish(); err != nil {
		t.Fatal(err)
	}
	if _, err := feedValues(newControlDecoder(), independentFrame(nil)); err == nil {
		t.Fatal("an empty body is not a control value")
	}
}

func TestRuntimeControl_InputFramesTests_FramesRawBytesPreserveWhitespaceAndInvalidJsonForConsumerValidation(t *testing.T) {
	t.Parallel()
	origin := time.Now()
	cases := []struct {
		body  string
		valid bool
	}{
		{"{\n \"protocol_version\":1,\"kind\":\"health\",\"text\":\"literal \\u00e9\"\n}", true},
		{"{", false},
		{"\xff", false},
		{"null", false},
	}
	for _, c := range cases {
		decoder := newControlDecoder()
		frames, err := decoder.feedAdmitted(independentFrame([]byte(c.body)), origin)
		if err != nil || len(frames) != 1 || string(frames[0].body) != c.body || !frames[0].admittedAt.Equal(origin) {
			t.Fatalf("%q: frames %d, %v", c.body, len(frames), err)
		}
		if err := decoder.finish(); err != nil {
			t.Fatalf("%q: %v", c.body, err)
		}
		if _, err := feedValues(newControlDecoder(), independentFrame([]byte(c.body))); (err == nil) != c.valid {
			t.Fatalf("%q: decode error %v, want valid=%v", c.body, err, c.valid)
		}
	}
}

func TestRuntimeControl_ControlCodecTests_RunnerReviewCodecEncodingPinsBigEndianLengthAndRequiredVersion(t *testing.T) {
	t.Parallel()
	for _, value := range []string{
		`{"protocol_version":1,"kind":"health","identity":{},"text":"literal\n世界"}`,
		`{"protocol_version":1,"result":{"text":"literal\nreply"}}`,
		`{"protocol_version":1,"error":"literal\nerror"}`,
	} {
		encoded, err := encodeControlFrame(jsontext.Value(value))
		if err != nil {
			t.Fatalf("%s: %v", value, err)
		}
		if want := independentFrame([]byte(jsonText(jsontext.Value(value)))); string(encoded) != string(want) {
			t.Fatalf("%s: frame %q, want %q", value, encoded, want)
		}
	}
}

func TestRuntimeControl_ControlCodecTests_RunnerReviewCodecKeepsJsonWhitespaceAndMultipleFramesIndependent(t *testing.T) {
	t.Parallel()
	request := "{\n  \"protocol_version\": 1,\n  \"kind\": \"health\"\n}"
	reply := `{"protocol_version":1,"result":"line one\nline two"}`
	bytes := slices.Concat(independentFrame([]byte(request)), independentFrame([]byte(reply)))
	decoder := newControlDecoder()
	values, err := feedValues(decoder, bytes)
	if err != nil {
		t.Fatal(err)
	}
	if len(values) != 2 || !jsonEqual(values[0], jsontext.Value(request)) || !jsonEqual(values[1], jsontext.Value(reply)) {
		t.Fatalf("values %s", values)
	}
	if err := decoder.finish(); err != nil {
		t.Fatal(err)
	}
}

func TestRuntimeControl_ControlCodecTests_RunnerReviewCodecRequiresIntegerSupportedVersionsInRequestsAndReplies(t *testing.T) {
	t.Parallel()
	for _, envelope := range []string{`{"kind":"health"}`, `{"result":null}`, `{"error":"failure"}`} {
		for _, version := range []string{"", "null", `"1"`, "1.0", "2", "-1"} {
			value := jsontext.Value(envelope)
			if version != "" {
				value = jsonSetAt(t, value, version, "protocol_version")
			}
			if _, err := encodeControlFrame(value); err == nil {
				t.Fatalf("%s encodes", value)
			}
			if _, err := feedValues(newControlDecoder(), independentFrame([]byte(jsonText(value)))); err == nil {
				t.Fatalf("%s decodes", value)
			}
		}
	}
}

func TestRuntimeControl_ControlCodecTests_RunnerReviewCodecRejectsEmptyMalformedAndNonobjectBodies(t *testing.T) {
	t.Parallel()
	for _, body := range []string{"", "{", "\xff", "[]", "null", "1"} {
		if _, err := feedValues(newControlDecoder(), independentFrame([]byte(body))); err == nil {
			t.Fatalf("%q decodes", body)
		}
	}
	for _, value := range []string{"null", "[]", "1"} {
		if _, err := encodeControlFrame(jsontext.Value(value)); err == nil {
			t.Fatalf("%s encodes", value)
		}
	}
}

func TestRuntimeControl_ControlCodecTests_RunnerReviewCodecAcceptsTheInclusiveBodyCapWithoutAnAggregateFrameCap(t *testing.T) {
	t.Parallel()
	value := jsontext.Value(`{"protocol_version":1,"result":""}`)
	overhead := len(jsonText(value))
	value = jsonSetAt(t, value, jsonQuoted(t, strings.Repeat("x", 65536-overhead)), "result")
	body := []byte(jsonText(value))
	if len(body) != 65536 {
		t.Fatalf("body length %d", len(body))
	}
	encoded, err := encodeControlFrame(value)
	if err != nil {
		t.Fatal(err)
	}
	if len(encoded) != 65540 || binary.BigEndian.Uint32(encoded[:4]) != 65536 {
		t.Fatalf("encoded length %d header %x", len(encoded), encoded[:4])
	}
	origin := time.Now()
	decoder := newControlDecoder()
	frames, err := decoder.feedAdmitted(slices.Repeat(independentFrame(body), 3), origin)
	if err != nil || len(frames) != 3 {
		t.Fatalf("frames %d, %v", len(frames), err)
	}
	for index, observed := range frames {
		if !observed.admittedAt.Equal(origin) {
			t.Fatalf("frame %d admitted at %v", index, observed.admittedAt)
		}
		decoded, err := decodeLifecycleControlBody(observed.body)
		if err != nil || !jsonEqual(decoded, value) {
			t.Fatalf("frame %d: %v", index, err)
		}
	}
	if err := decoder.finish(); err != nil {
		t.Fatal(err)
	}
	value = jsonSetAt(t, value, jsonQuoted(t, strings.Repeat("x", 65537-overhead)), "result")
	if _, err := encodeControlFrame(value); err == nil {
		t.Fatal("a 65537-byte body exceeds the cap")
	}
}

// serveOneHealthReply accepts one connection on listener, checks the framed health request it
// reads against identity and writes reply one byte at a time.
func serveOneHealthReply(listener int, identity, reply jsontext.Value, deadline time.Time) error {
	var stream int
	for {
		connection, _, err := unix.Accept(listener)
		if err == nil {
			stream = connection
			break
		}
		if !retryable(err) {
			return fmt.Errorf("fixture socket accept failed: %w", err)
		}
		if !time.Now().Before(deadline) {
			return errors.New("fixture socket accept timed out")
		}
		time.Sleep(min(2*time.Millisecond, time.Until(deadline)))
	}
	defer func() { _ = unix.Close(stream) }()
	if err := unix.SetNonblock(stream, true); err != nil {
		return err
	}
	flags, err := unix.FcntlInt(uintptr(stream), unix.F_GETFL, 0)
	if err != nil {
		return err
	}
	if flags&unix.O_NONBLOCK == 0 {
		return errors.New("accepted fixture stream is nonblocking")
	}
	readDeadline := deadline
	if limit := time.Now().Add(realTimeBound); limit.Before(readDeadline) {
		readDeadline = limit
	}
	header := make([]byte, 4)
	if err := readFixtureBytes(stream, header, readDeadline); err != nil {
		return err
	}
	length := binary.BigEndian.Uint32(header)
	if length == 0 || length > 65536 {
		return fmt.Errorf("actual client sent invalid length %d", length)
	}
	body := make([]byte, length)
	if err := readFixtureBytes(stream, body, readDeadline); err != nil {
		return err
	}
	request := jsontext.Value(body)
	if !jsonEqual(jsonIndex(request, "protocol_version"), jsontext.Value("1")) ||
		!jsonIsString(jsonIndex(request, "kind"), "health") ||
		!jsonEqual(jsonIndex(request, "identity"), identity) {
		return fmt.Errorf("health request %s", body)
	}
	for _, b := range independentFrame([]byte(jsonText(reply))) {
		if err := writeFixtureBytes(stream, []byte{b}, deadline); err != nil {
			return err
		}
	}
	return nil
}

func processWire(t *testing.T, pid int64, startIdentity string) string {
	t.Helper()
	return fmt.Sprintf(`{"pid":%d,"start_identity":%s}`, pid, jsonQuoted(t, startIdentity))
}

func TestRuntimeControl_ControlCodecTests_RunnerReviewActualHealthClientUsesFramingAndValidatesReplyVersions(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		version string
		valid   bool
	}{
		{"integer one", "1", true},
		{"absent", "", false},
		{"two", "2", false},
		{"string", `"1"`, false},
		{"float", "1.0", false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			t.Parallel()
			fixture := newRunnerFixture(t)
			artifacts := fixture.artifacts(t)
			transferSetting(t, fixture, "runner_pid=?1,runner_start_identity=?2")
			process := currentProcess(t)
			run := fixture.run
			listener := boundSocket(t, artifacts.paths.controlSocket, 0o600, true)
			identity := mustJSON(t, controlIdentityFromRun(run))
			reply := jsontext.Value(fmt.Sprintf(`{"result":{"identity":%s,"runner":%s,"child":null,"output_seq":7}}`,
				identity, processWire(t, process.pid, process.startIdentity)))
			if c.version != "" {
				reply = jsonSetAt(t, reply, c.version, "protocol_version")
			}
			deadline := time.Now().Add(realTimeBound)
			served := make(chan error, 1)
			go func() { served <- serveOneHealthReply(listener, identity, reply, deadline) }()
			health, err := probeHealth(run, deadline, systemClock{})
			if (err == nil) != c.valid {
				t.Fatalf("%s: probe error %v", c.name, err)
			}
			if err == nil && health.OutputSeq != 7 {
				t.Fatalf("%s: output_seq %d", c.name, health.OutputSeq)
			}
			if err := <-served; err != nil {
				t.Fatalf("%s: %v", c.name, err)
			}
		})
	}
}

func inputClientRequest(fixture *runnerFixture) inputControlRequest {
	return inputControlRequest{
		Identity:        controlIdentityFromRun(fixture.run),
		ExpiresAtMonoNS: 50_000_000_000,
		Operation: submitPromptOperation{input: submitInput{
			OperationKey: "fixture:typed-client",
			InputKind:    inputKindPrompt,
			Literal:      "literal 世界 '$x'",
		}},
	}
}

func inputClientExpected(t *testing.T, fixture *runnerFixture) jsontext.Value {
	t.Helper()
	run := fixture.run
	return jsontext.Value(fmt.Sprintf(`{"protocol_version":1,"kind":"submit_prompt","identity":{"database_identity_json":%s,"fleet_id":%d,"member_id":%d,"director_member_id":%d,"run_id":%d,"owner_epoch":%d,"launch_nonce":%s},"expires_at_mono_ns":50000000000,"operation_key":"fixture:typed-client","input_kind":"prompt","literal":"literal 世界 '$x'"}`,
		run.databaseIdentityJSON, run.fleetID, run.memberID, run.directorMemberID, run.runID, run.ownerEpoch, jsonQuoted(t, run.launchNonce)))
}

func inputClientReply(status, stage string) []byte {
	reason, delivery := `"fixture uncertainty"`, `{"status":"uncertain","method":"fallback","reason":"submission_plan_unavailable"}`
	if status == "accepted" {
		reason, delivery = "null", `{"status":"submitted","method":"profile","reason":null}`
	}
	return independentFrame(fmt.Appendf(nil,
		`{"protocol_version":1,"result":{"kind":"submission","submission":{"status":%q,"input_id":13,"stage":%q,"output_seq":null,"reason":%s,"delivery":%s}}}`,
		status, stage, reason, delivery))
}

// controlPair is a production client and its nonblocking peer descriptor on a socket pair.
func controlPair(t *testing.T) (*controlClient, int) {
	t.Helper()
	fds := cloexecSocketpair(t)
	for _, fd := range fds {
		if err := unix.SetNonblock(fd, true); err != nil {
			t.Fatal(err)
		}
	}
	client := newControlClient(fds[0])
	t.Cleanup(func() {
		if err := client.close(); err != nil {
			t.Errorf("closing the control client: %v", err)
		}
		closeFD(t, fds[1])
	})
	return client, fds[1]
}

type controlPeer struct {
	fd    int
	stop  *atomic.Bool
	until time.Time
}

func (p controlPeer) active() error {
	if p.stop.Load() {
		return errors.New("owned peer cancelled")
	}
	if !time.Now().Before(p.until) {
		return errors.New("owned peer deadline expired")
	}
	return nil
}

func (p controlPeer) write(data []byte) error {
	for len(data) > 0 {
		if err := p.active(); err != nil {
			return err
		}
		count, err := unix.Write(p.fd, data)
		switch {
		case err == nil && count == 0:
			return errors.New("peer write made no progress")
		case err == nil:
			data = data[count:]
		case retryable(err):
			runtime.Gosched()
		default:
			return fmt.Errorf("peer write: %w", err)
		}
	}
	return nil
}

func (p controlPeer) read(buffer []byte) error {
	for len(buffer) > 0 {
		if err := p.active(); err != nil {
			return err
		}
		count, err := unix.Read(p.fd, buffer)
		switch {
		case err == nil && count == 0:
			return errors.New("peer received truncated request")
		case err == nil:
			buffer = buffer[count:]
		case retryable(err):
			runtime.Gosched()
		default:
			return fmt.Errorf("peer read: %w", err)
		}
	}
	return nil
}

func (p controlPeer) readRequest() (jsontext.Value, error) {
	header := make([]byte, 4)
	if err := p.read(header); err != nil {
		return nil, err
	}
	size := binary.BigEndian.Uint32(header)
	if size > 65536 {
		return nil, errors.New("peer received oversized request")
	}
	body := make([]byte, size)
	if err := p.read(body); err != nil {
		return nil, err
	}
	value, ok := jsonParse(string(body))
	if !ok {
		return nil, fmt.Errorf("peer received invalid JSON %q", body)
	}
	return value, nil
}

type peerObservation struct {
	request              jsontext.Value
	at                   time.Time
	firstFragmentAt      time.Time
	laterFragmentAt      time.Time
	drainedAfterReturnAt time.Time
	extraConnections     int
	extraRequestBytes    int
}

type exchangeOutcome struct {
	reply inputControlReply
	err   error
}

func describeOutcome(outcome exchangeOutcome) string {
	if outcome.err != nil {
		return fmt.Sprintf("Err(%v)", outcome.err)
	}
	if reply, ok := outcome.reply.(submissionResult); ok {
		return fmt.Sprintf("Ok(Submission status=%s, input_id=%v, stage=%s, output_seq=%v, reason=%v)",
			reply.Status.asStr(), reply.InputID, reply.Stage.asStr(), reply.OutputSeq, reply.Reason)
	}
	return fmt.Sprintf("Ok(%T)", outcome.reply)
}

// exchangeWithPeer runs work as an owned peer while exchange runs here, then stops and joins the
// peer.
func exchangeWithPeer(t *testing.T, work func(stop *atomic.Bool, until time.Time) (peerObservation, error), exchange func() (inputControlReply, error)) (exchangeOutcome, peerObservation) {
	t.Helper()
	var stop atomic.Bool
	until := time.Now().Add(2 * realTimeBound)
	type peerResult struct {
		observed peerObservation
		err      error
	}
	done := make(chan peerResult, 1)
	go func() {
		observed, err := work(&stop, until)
		done <- peerResult{observed, err}
	}()
	reply, err := exchange()
	outcome := exchangeOutcome{reply: reply, err: err}
	stop.Store(true)
	peer := <-done
	if peer.err != nil {
		t.Fatalf("primary exchange result: %s; owned peer: %v", describeOutcome(outcome), peer.err)
	}
	return outcome, peer.observed
}

func assertAcceptedSubmission(t *testing.T, outcome exchangeOutcome) {
	t.Helper()
	if outcome.err != nil {
		t.Fatal(outcome.err)
	}
	submission, ok := outcome.reply.(submissionResult)
	if !ok {
		t.Fatalf("actual typed exchange returns submission, got %T", outcome.reply)
	}
	if submission.Status != submissionStatusAccepted || submission.InputID != some[int64](13) ||
		submission.Stage != submissionStageSubmitted || submission.OutputSeq != none[uint64]() || submission.Reason != none[string]() {
		t.Fatalf("accepted submission %s", describeOutcome(outcome))
	}
}

func preloadedExchange(t *testing.T, fixture *runnerFixture, response []byte) (exchangeOutcome, jsontext.Value) {
	t.Helper()
	client, fd := controlPair(t)
	var stop atomic.Bool
	peer := controlPeer{fd: fd, stop: &stop, until: time.Now().Add(realTimeBound)}
	if err := peer.write(response); err != nil {
		t.Fatal(err)
	}
	reply, err := client.exchangeInput(inputClientRequest(fixture), peer.until, systemClock{})
	actual, readErr := peer.readRequest()
	if readErr != nil {
		t.Fatal(readErr)
	}
	return exchangeOutcome{reply: reply, err: err}, actual
}

func TestRuntimeControl_InputClientTests_TypedExchangeSendsAnIndependentFlatRequestWithTheOriginalExpiry(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	outcome, actual := preloadedExchange(t, fixture, inputClientReply("accepted", "submitted"))
	assertAcceptedSubmission(t, outcome)
	assertJSONEqual(t, actual, inputClientExpected(t, fixture), "flat typed request")
}

func TestRuntimeControl_InputClientTests_ExchangeValidatesFragmentedDuplicateAndWrongKindReplies(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	client, fd := controlPair(t)
	response := inputClientReply("accepted", "submitted")
	outcome, observed := exchangeWithPeer(t, func(stop *atomic.Bool, until time.Time) (peerObservation, error) {
		peer := controlPeer{fd: fd, stop: stop, until: until}
		request, err := peer.readRequest()
		if err != nil {
			return peerObservation{}, err
		}
		at := time.Now()
		for chunk := range slices.Chunk(response, 3) {
			if err := peer.write(chunk); err != nil {
				return peerObservation{}, err
			}
			time.Sleep(time.Millisecond)
		}
		return peerObservation{request: request, at: at}, nil
	}, func() (inputControlReply, error) {
		return client.exchangeInput(inputClientRequest(fixture), time.Now().Add(realTimeBound), systemClock{})
	})
	assertAcceptedSubmission(t, outcome)
	assertJSONEqual(t, observed.request, inputClientExpected(t, fixture), "fragmented reply request")
	duplicate := independentFrame([]byte(`{"protocol_version":1,"result":{"kind":"submission","submission":{"status":"accepted","input_id":13,"input_id":13,"stage":"submitted","output_seq":null,"reason":null,"delivery":{"status":"submitted","method":"profile","reason":null}}}}`))
	wrong := independentFrame([]byte(`{"protocol_version":1,"result":{"kind":"receipt","receipt":{"status":"missing","input_id":null,"submission":null}}}`))
	for name, response := range map[string][]byte{"duplicate": duplicate, "wrong kind": wrong} {
		outcome, actual := preloadedExchange(t, fixture, response)
		if outcome.err == nil {
			t.Fatalf("%s reply is refused", name)
		}
		assertJSONEqual(t, actual, inputClientExpected(t, fixture), name)
	}
}

func TestRuntimeControl_InputClientTests_ExchangeRejectsExtraReplyTrailingPartialAndPrematureEof(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	good := inputClientReply("accepted", "submitted")
	cases := []struct {
		name     string
		response []byte
	}{
		{"extra reply", slices.Concat(good, good)},
		{"trailing partial", slices.Concat(good, good[:2])},
	}
	for _, c := range cases {
		outcome, actual := preloadedExchange(t, fixture, c.response)
		if outcome.err == nil {
			t.Fatalf("%s is refused", c.name)
		}
		assertJSONEqual(t, actual, inputClientExpected(t, fixture), c.name)
	}
	client, fd := controlPair(t)
	var stop atomic.Bool
	peer := controlPeer{fd: fd, stop: &stop, until: time.Now().Add(realTimeBound)}
	if err := peer.write(good[:len(good)-1]); err != nil {
		t.Fatal(err)
	}
	if err := unix.Shutdown(fd, unix.SHUT_WR); err != nil {
		t.Fatal(err)
	}
	_, err := client.exchangeInput(inputClientRequest(fixture), peer.until, systemClock{})
	actual, readErr := peer.readRequest()
	if readErr != nil {
		t.Fatal(readErr)
	}
	if err == nil {
		t.Fatal("a premature EOF is refused")
	}
	assertJSONEqual(t, actual, inputClientExpected(t, fixture), "premature EOF")
}

func TestRuntimeControl_InputClientTests_ExpiredAndStalledExchangesKeepTheOriginalDeadline(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	client, fd := controlPair(t)
	if _, err := client.exchangeInput(inputClientRequest(fixture), time.Now().Add(-time.Millisecond), systemClock{}); err == nil {
		t.Fatal("an expired exchange fails")
	}
	observationDeadline := time.Now().Add(100 * time.Millisecond)
	for {
		if !time.Now().Before(observationDeadline) {
			t.Fatal("observing the expired exchange exceeded its bound")
		}
		count, err := unix.Read(fd, make([]byte, 1))
		if err == nil && count == 0 || errors.Is(err, unix.EAGAIN) {
			break
		}
		if errors.Is(err, unix.EINTR) {
			continue
		}
		if err != nil {
			t.Fatalf("observe expired exchange bytes: %v", err)
		}
		t.Fatal("an expired invocation sends zero request bytes")
	}

	client, fd = controlPair(t)
	budget := 3 * time.Second
	fragmentDelay := 1500 * time.Millisecond
	returnTolerance := time.Second
	started := time.Now()
	deadline := started.Add(budget)
	var returned time.Time
	outcome, observed := exchangeWithPeer(t, func(stop *atomic.Bool, until time.Time) (peerObservation, error) {
		peer := controlPeer{fd: fd, stop: stop, until: until}
		request, err := peer.readRequest()
		if err != nil {
			return peerObservation{}, err
		}
		at := time.Now()
		if err := peer.write([]byte{0, 0}); err != nil {
			return peerObservation{}, err
		}
		first := time.Now()
		next := first.Add(fragmentDelay)
		for time.Now().Before(next) {
			if err := peer.active(); err != nil {
				return peerObservation{}, err
			}
			time.Sleep(time.Millisecond)
		}
		if err := peer.write([]byte{0, 20}); err != nil {
			return peerObservation{}, err
		}
		laterAt := time.Now()
		for !stop.Load() && time.Now().Before(until) {
			time.Sleep(time.Millisecond)
		}
		return peerObservation{request: request, at: at, firstFragmentAt: first, laterFragmentAt: laterAt}, nil
	}, func() (inputControlReply, error) {
		reply, err := client.exchangeInput(inputClientRequest(fixture), deadline, systemClock{})
		returned = time.Now()
		return reply, err
	})
	if outcome.err == nil {
		t.Fatal("a stalled exchange fails at its deadline")
	}
	assertJSONEqual(t, observed.request, inputClientExpected(t, fixture), "stalled request")
	first, laterAt := observed.firstFragmentAt, observed.laterFragmentAt
	checks := []struct {
		name string
		ok   bool
	}{
		{"peer observed the actual request before the tested expiry", observed.at.Before(deadline)},
		{"the request precedes the first fragment", !observed.at.After(first)},
		{"the fragments are ordered", first.Before(laterAt)},
		{"the later fragment precedes the deadline", laterAt.Before(deadline)},
		{"the fragments are a delay apart", laterAt.Sub(first) >= fragmentDelay},
		{"the later fragment follows the delay", laterAt.Sub(started) >= fragmentDelay},
		{"the fragment gap exceeds the return tolerance", returnTolerance < laterAt.Sub(first)},
		{"the exchange returns at its deadline", !returned.Before(deadline)},
		{"actual exchange shares its original budget", returned.Before(deadline.Add(returnTolerance))},
		{"the exchange does not restart its budget", returned.Before(laterAt.Add(budget))},
	}
	for _, check := range checks {
		if !check.ok {
			t.Errorf("%s", check.name)
		}
	}
}

func TestRuntimeControl_InputClientTests_DisconnectAfterTheRequestHasNoReconnectOrSecondRequest(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	path := shortTempdir(t) + "/control.sock"
	listener := boundSocket(t, path, 0o600, true)
	var exchangeReturned atomic.Bool
	deadline := time.Now().Add(realTimeBound)
	var returnedAt time.Time
	outcome, observed := exchangeWithPeer(t, func(stop *atomic.Bool, until time.Time) (peerObservation, error) {
		peer := controlPeer{stop: stop, until: until}
		for {
			if err := peer.active(); err != nil {
				return peerObservation{}, err
			}
			connection, _, err := unix.Accept(listener)
			if err == nil {
				peer.fd = connection
				break
			}
			if !errors.Is(err, unix.EAGAIN) {
				return peerObservation{}, err
			}
			runtime.Gosched()
		}
		defer func() { _ = unix.Close(peer.fd) }()
		if err := unix.SetNonblock(peer.fd, true); err != nil {
			return peerObservation{}, err
		}
		request, err := peer.readRequest()
		if err != nil {
			return peerObservation{}, err
		}
		at := time.Now()
		if err := unix.Shutdown(peer.fd, unix.SHUT_WR); err != nil {
			return peerObservation{}, err
		}
		trailing := make([]byte, 4096)
		extraBytes, extraConnections := 0, 0
		for {
			if !time.Now().Before(until) {
				return peerObservation{}, errors.New("disconnect observer expired before final drain")
			}
			afterReturn := exchangeReturned.Load()
			for {
				if !time.Now().Before(until) {
					return peerObservation{}, errors.New("disconnect request drain exceeded its bound")
				}
				count, err := unix.Read(peer.fd, trailing)
				if err == nil && count == 0 || errors.Is(err, unix.EAGAIN) {
					break
				}
				if errors.Is(err, unix.EINTR) {
					continue
				}
				if err != nil {
					return peerObservation{}, err
				}
				extraBytes += count
			}
			for {
				if !time.Now().Before(until) {
					return peerObservation{}, errors.New("disconnect listener drain exceeded its bound")
				}
				extra, _, err := unix.Accept(listener)
				if err == nil {
					extraConnections++
					_ = unix.Close(extra)
					continue
				}
				if errors.Is(err, unix.EAGAIN) {
					break
				}
				if errors.Is(err, unix.EINTR) {
					continue
				}
				return peerObservation{}, err
			}
			if afterReturn {
				return peerObservation{request: request, at: at, drainedAfterReturnAt: time.Now(),
					extraConnections: extraConnections, extraRequestBytes: extraBytes}, nil
			}
			if stop.Load() && !exchangeReturned.Load() {
				return peerObservation{}, errors.New("disconnect observer cancelled before actual exchange return")
			}
			runtime.Gosched()
		}
	}, func() (inputControlReply, error) {
		client, err := controlClientConnect(path, deadline, systemClock{})
		if err != nil {
			t.Errorf("connecting the control client: %v", err)
			exchangeReturned.Store(true)
			return nil, err
		}
		defer func() {
			if err := client.close(); err != nil {
				t.Errorf("closing the control client: %v", err)
			}
		}()
		reply, err := client.exchangeInput(inputClientRequest(fixture), deadline, systemClock{})
		returnedAt = time.Now()
		exchangeReturned.Store(true)
		return reply, err
	})
	if outcome.err == nil {
		t.Fatal("a disconnect after the request fails")
	}
	assertJSONEqual(t, observed.request, inputClientExpected(t, fixture), "disconnect request")
	if observed.extraConnections != 0 || observed.extraRequestBytes != 0 {
		t.Fatalf("extra connections %d, extra request bytes %d", observed.extraConnections, observed.extraRequestBytes)
	}
	if observed.drainedAfterReturnAt.Before(returnedAt) {
		t.Fatal("the final drain follows the exchange return")
	}
}

func TestRuntimeControl_InputClientTests_TextualErrorRemainsErrorAndStructuredUncertaintyRemainsResult(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	outcome, actual := preloadedExchange(t, fixture, independentFrame([]byte(`{"protocol_version":1,"error":"fixture failure"}`)))
	assertJSONEqual(t, actual, inputClientExpected(t, fixture), "textual error request")
	if outcome.err == nil {
		t.Fatal("textual server failure is an error")
	}
	failure := cafleetErrorOf(outcome.err)
	if failure.kind != errorApp || failure.message() != "runner control: fixture failure" {
		t.Fatalf("ordinary application error expected: %#v", failure)
	}
	for _, stage := range []string{"partial", "unknown"} {
		outcome, actual := preloadedExchange(t, fixture, inputClientReply("uncertain", stage))
		assertJSONEqual(t, actual, inputClientExpected(t, fixture), stage)
		if outcome.err != nil {
			t.Fatalf("%s: %v", stage, outcome.err)
		}
		submission, ok := outcome.reply.(submissionResult)
		if !ok {
			t.Fatalf("%s: structured uncertainty remains a submission result, got %T", stage, outcome.reply)
		}
		if submission.Status != submissionStatusUncertain || submission.Stage.asStr() != stage || submission.InputID != some[int64](13) {
			t.Fatalf("%s: %s", stage, describeOutcome(outcome))
		}
	}
}

func TestRuntimeControl_LaunchReadinessTests_RunnerReadinessPostStartHealthPreservesStrictEndpointFailures(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name      string
		mode      uint32
		listening bool
	}{
		{"bound", 0o600, false},
		{"before private mode", 0o644, true},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			t.Parallel()
			fixture := newRunnerFixture(t)
			artifacts := fixture.artifacts(t)
			transferSetting(t, fixture, "status='running',child_pid=?1,child_start_identity=?2")
			listener := boundSocket(t, artifacts.paths.controlSocket, c.mode, c.listening)
			started := time.Now()
			if _, err := probeHealth(fixture.run, started.Add(realTimeBound), systemClock{}); err == nil {
				t.Fatalf("%s: the probe fails", c.name)
			}
			if elapsed := time.Since(started); elapsed >= realTimeBound/2 {
				t.Fatalf("%s: the strict failure took %v", c.name, elapsed)
			}
			if c.listening {
				if _, _, err := unix.Accept(listener); !errors.Is(err, unix.EAGAIN) {
					t.Fatalf("%s: the probe connected to a non-private endpoint: %v", c.name, err)
				}
			}
		})
	}
}

func TestRuntimeControl_RunnerHealthRealSilentEndpointIsBoundedAndProbeIsReadOnly(t *testing.T) {
	t.Parallel()
	fixture := newRunnerFixture(t)
	artifacts, _ := activate(t, fixture)
	silentServer(t, artifacts.paths.controlSocket)
	before, err := os.ReadFile(fixture.databaseFile(t))
	if err != nil {
		t.Fatal(err)
	}
	start := time.Now()
	if _, err := probeHealth(fixture.run, start.Add(80*time.Millisecond), systemClock{}); err == nil {
		t.Fatal("a silent endpoint fails the probe")
	}
	if elapsed := time.Since(start); elapsed >= realTimeBound/2 {
		t.Fatalf("the silent probe took %v", elapsed)
	}
	after, err := os.ReadFile(fixture.databaseFile(t))
	if err != nil {
		t.Fatal(err)
	}
	if string(after) != string(before) {
		t.Fatal("the probe is read-only")
	}
}

func healthWire(t *testing.T, run runRecord) jsontext.Value {
	t.Helper()
	runnerPID, _ := run.runnerPID.get()
	runnerStart, _ := run.runnerStartIdentity.get()
	childPID, _ := run.childPID.get()
	childStart, _ := run.childStartIdentity.get()
	return jsontext.Value(fmt.Sprintf(`{"identity":%s,"runner":%s,"child":%s,"output_seq":17,"observation":{"visibility":"unknown","eligibility":{"state":"hold","reason":"unknown"},"pending_count":0,"gap_count":0,"degraded":false,"diagnostic":null,"writer_diagnostic":null,"gap_open_since":null}}`,
		mustJSON(t, controlIdentityFromRun(run)), processWire(t, runnerPID, runnerStart), processWire(t, childPID, childStart)))
}

func degradedGapWire(t *testing.T, run runRecord) jsontext.Value {
	t.Helper()
	value := healthWire(t, run)
	for _, field := range []struct{ name, raw string }{
		{"eligibility", `{"state":"hold","reason":"gap"}`},
		{"degraded", "true"},
		{"gap_count", "1"},
		{"diagnostic", `"sink_unavailable"`},
		{"writer_diagnostic", `"database error: database is locked"`},
		{"gap_open_since", `"2026-11-21T12:04:08.000000+00:00"`},
	} {
		value = jsonSetAt(t, value, field.raw, "observation", field.name)
	}
	return value
}

func activatedFixture(t *testing.T) *runnerFixture {
	t.Helper()
	fixture := newRunnerFixture(t)
	activate(t, fixture)
	return fixture
}

func jsonOptString(value jsontext.Value) opt[string] {
	if text, ok := jsonString(value); ok {
		return some(text)
	}
	return none[string]()
}

func TestRuntimeControl_ObservationContract_HealthDecodeAcceptsWriterDiagnosticAndGapOpenSinceForADegradedGap(t *testing.T) {
	t.Parallel()
	fixture := activatedFixture(t)
	cases := []struct {
		name  string
		shape map[string]string
	}{
		{"a degraded gap with both fields", nil},
		{"a terminal-fenced gap without a pending gap", map[string]string{"gap_open_since": "null"}},
		{"a degraded gap whose writer recovered", map[string]string{"writer_diagnostic": "null"}},
		{"a 512-byte writer diagnostic", map[string]string{"writer_diagnostic": jsonQuoted(t, "label: "+strings.Repeat("x", 505))}},
		{"a 512-byte writer diagnostic whose escaping doubles it past 1024 bytes", map[string]string{"writer_diagnostic": jsonQuoted(t, "q: "+strings.Repeat(`"`, 509))}},
	}
	for _, c := range cases {
		value := degradedGapWire(t, fixture.run)
		for field, raw := range c.shape {
			value = jsonSetAt(t, value, raw, "observation", field)
		}
		expected := jsonAt(t, value, "observation")
		decoded, err := healthSnapshotDecode(value, fixture.run)
		if err != nil {
			t.Fatalf("%s: %v", c.name, err)
		}
		observation, ok := decoded.Observation.get()
		if !ok {
			t.Fatalf("%s: the observation decodes", c.name)
		}
		if err := observation.validate(); err != nil {
			t.Fatalf("%s: %v", c.name, err)
		}
		if observation.WriterDiagnostic != jsonOptString(jsonIndex(expected, "writer_diagnostic")) {
			t.Fatalf("%s: writer diagnostic %v", c.name, observation.WriterDiagnostic)
		}
		if observation.GapOpenSince != jsonOptString(jsonIndex(expected, "gap_open_since")) {
			t.Fatalf("%s: gap_open_since %v", c.name, observation.GapOpenSince)
		}
	}
}

func TestRuntimeControl_ObservationContract_HealthDecodeRefusesWriterDiagnosticAndGapOpenSinceOutsideTheirRules(t *testing.T) {
	t.Parallel()
	fixture := activatedFixture(t)
	cases := []struct {
		name  string
		shape [][2]string
	}{
		{"a writer diagnostic without degradation", [][2]string{
			{"eligibility", `{"state":"hold","reason":"unknown"}`}, {"degraded", "false"},
			{"diagnostic", "null"}, {"gap_open_since", "null"},
		}},
		{"an empty writer diagnostic", [][2]string{{"writer_diagnostic", `""`}}},
		{"a writer diagnostic longer than 512 bytes", [][2]string{{"writer_diagnostic", jsonQuoted(t, "label: "+strings.Repeat("x", 506))}}},
		{"a writer diagnostic with a control character", [][2]string{{"writer_diagnostic", `"database error:\nlocked"`}}},
		{"a non-string writer diagnostic", [][2]string{{"writer_diagnostic", "7"}}},
		{"gap_open_since without degradation", [][2]string{
			{"eligibility", `{"state":"hold","reason":"gap"}`}, {"degraded", "false"},
			{"diagnostic", "null"}, {"writer_diagnostic", "null"},
		}},
		{"gap_open_since outside a gap hold", [][2]string{{"eligibility", `{"state":"hold","reason":"unknown"}`}}},
		{"gap_open_since that is not RFC 3339", [][2]string{{"gap_open_since", `"yesterday"`}}},
	}
	for _, c := range cases {
		value := degradedGapWire(t, fixture.run)
		for _, field := range c.shape {
			value = jsonSetAt(t, value, field[1], "observation", field[0])
		}
		if _, err := healthSnapshotDecode(value, fixture.run); err == nil {
			t.Fatalf("%s decodes", c.name)
		}
	}
}

func TestRuntimeControl_ObservationContract_HealthDecodeWithoutObservationValidatesIdentityWithoutGrantingComposerProof(t *testing.T) {
	t.Parallel()
	fixture := activatedFixture(t)
	value := testJSONWithout(t, healthWire(t, fixture.run), "observation")
	decoded, err := healthSnapshotDecode(value, fixture.run)
	if err != nil {
		t.Fatal(err)
	}
	if _, present := decoded.Observation.get(); present {
		t.Fatal("an absent observation stays absent")
	}
}

func TestRuntimeControl_ObservationContract_HealthDecodeAcceptsBoundedHoldAndCheckedEmptyComposerStates(t *testing.T) {
	t.Parallel()
	fixture := activatedFixture(t)
	for _, visibility := range []string{"visible", "obscured", "not_visible", "unknown"} {
		value := healthWire(t, fixture.run)
		value = jsonSetAt(t, value, jsonQuoted(t, visibility), "observation", "visibility")
		value = jsonSetAt(t, value, "130", "observation", "pending_count")
		value = jsonSetAt(t, value, "true", "observation", "degraded")
		value = jsonSetAt(t, value, `"sink_unavailable"`, "observation", "diagnostic")
		decoded, err := healthSnapshotDecode(value, fixture.run)
		if err != nil {
			t.Fatalf("%s: %v", visibility, err)
		}
		observation, ok := decoded.Observation.get()
		if !ok {
			t.Fatalf("%s: the observation decodes", visibility)
		}
		if err := observation.validate(); err != nil {
			t.Fatalf("%s: %v", visibility, err)
		}
	}
	value := healthWire(t, fixture.run)
	value = jsonSetAt(t, value, `"not_visible"`, "observation", "visibility")
	value = jsonSetAt(t, value, `{"state":"composer_empty"}`, "observation", "eligibility")
	if _, err := healthSnapshotDecode(value, fixture.run); err != nil {
		t.Fatalf("a checked empty composer decodes: %v", err)
	}
}

func TestRuntimeControl_ObservationContract_HealthDecodeRejectsUnknownWireFieldsEnumsUnboundedAndPrivateDiagnostics(t *testing.T) {
	t.Parallel()
	fixture := activatedFixture(t)
	cases := []struct {
		name  string
		path  []string
		value string
	}{
		{"unknown_field", []string{"screen"}, `"private screen"`},
		{"visibility", []string{"visibility"}, `"invented"`},
		{"state", []string{"eligibility", "state"}, `"allow"`},
		{"reason", []string{"eligibility", "reason"}, `"invented"`},
		{"pending", []string{"pending_count"}, "131"},
		{"negative", []string{"gap_count"}, "-1"},
		{"float", []string{"pending_count"}, "0.5"},
		{"raw_diagnostic", []string{"diagnostic"}, `"private exception configuration canary"`},
		{"long_diagnostic", []string{"diagnostic"}, jsonQuoted(t, strings.Repeat("x", 129))},
		{"unicode_diagnostic", []string{"diagnostic"}, `"世界"`},
	}
	for _, c := range cases {
		value := jsonSetAt(t, healthWire(t, fixture.run), c.value, append([]string{"observation"}, c.path...)...)
		if _, err := healthSnapshotDecode(value, fixture.run); err == nil {
			t.Fatalf("%s decodes", c.name)
		}
	}
}

func TestRuntimeControl_ObservationContract_HealthDecodeRequiresHoldForVisibilityPressureAndDegradation(t *testing.T) {
	t.Parallel()
	fixture := activatedFixture(t)
	for _, field := range []string{"visible", "obscured", "unknown", "pending_count", "degraded"} {
		value := healthWire(t, fixture.run)
		value = jsonSetAt(t, value, `"not_visible"`, "observation", "visibility")
		value = jsonSetAt(t, value, `{"state":"composer_empty"}`, "observation", "eligibility")
		switch field {
		case "pending_count":
			value = jsonSetAt(t, value, "1", "observation", field)
		case "degraded":
			value = jsonSetAt(t, value, "true", "observation", field)
		default:
			value = jsonSetAt(t, value, jsonQuoted(t, field), "observation", "visibility")
		}
		if _, err := healthSnapshotDecode(value, fixture.run); err == nil {
			t.Fatalf("%s decodes beside a composer_empty eligibility", field)
		}
	}
}

func TestRuntimeControl_ObservationContract_HealthDecodePreservesProcessGenerationAndControlIdentityChecks(t *testing.T) {
	t.Parallel()
	fixture := activatedFixture(t)
	cases := []struct {
		name  string
		path  []string
		value string
	}{
		{"epoch", []string{"identity", "owner_epoch"}, fmt.Sprint(fixture.run.ownerEpoch + 1)},
		{"nonce", []string{"identity", "launch_nonce"}, jsonQuoted(t, strings.Repeat("b", 64))},
		{"runner", []string{"runner", "start_identity"}, `"stale generation"`},
		{"child", []string{"child", "start_identity"}, `"stale generation"`},
	}
	for _, c := range cases {
		value := jsonSetAt(t, healthWire(t, fixture.run), c.value, c.path...)
		if _, err := healthSnapshotDecode(value, fixture.run); err == nil {
			t.Fatalf("%s decodes", c.name)
		}
	}
}

func TestRuntimeControl_MissingEvidenceWireValuesFollowDeclarationOrder(t *testing.T) {
	t.Parallel()
	values := []missingEvidence{
		missingEvidenceOperation, missingEvidenceArguments, missingEvidenceCwd, missingEvidenceEnvironment,
		missingEvidenceTargets, missingEvidenceEffects, missingEvidenceChoiceScope,
	}
	for index, value := range values {
		if value != missingEvidence(index+1) {
			t.Fatalf("%s is declared at position %d", value.asStr(), index)
		}
		if encoded := mustJSON(t, value); string(encoded) != jsonQuoted(t, value.asStr()) {
			t.Fatalf("%s encodes as %s", value.asStr(), encoded)
		}
	}
}

type wireOperation struct {
	kind   string
	action string
}

var wireOperations = []wireOperation{
	{"submit_prompt", ""},
	{"submit_exec", ""},
	{"permission_inspect", ""},
	{"permission_respond", ""},
	{"receipt", "query"},
	{"receipt", "retire"},
	{"receipt", "reconcile"},
}

func (o wireOperation) String() string {
	if o.action == "" {
		return o.kind
	}
	return o.kind + "/" + o.action
}

type wireCase struct {
	fixture *runnerFixture
}

func newWireCase(t *testing.T) wireCase {
	t.Helper()
	return wireCase{fixture: newRunnerFixture(t)}
}

func (c wireCase) request(t *testing.T, kind, action string) jsontext.Value {
	t.Helper()
	run := c.fixture.run
	object := jsonObject{
		{"protocol_version", 1},
		{"kind", kind},
		{"identity", mustJSON(t, controlIdentityFromRun(run))},
		{"expires_at_mono_ns", uint64(50_000_000_000)},
	}
	var fields string
	switch kind {
	case "submit_prompt":
		fields = `{"operation_key":"fixture:prompt","input_kind":"prompt","literal":"literal 世界 '$x'"}`
	case "submit_exec":
		fields = `{"operation_key":"fixture:exec","input_kind":"exec","literal":"printf literal"}`
	case "permission_inspect":
		fields = fmt.Sprintf(`{"interaction_id":12,"event_id":11,"director_member_id":%d}`, run.directorMemberID)
	case "permission_respond":
		fields = fmt.Sprintf(`{"interaction_id":12,"event_id":11,"director_member_id":%d,"expected_run_id":%d,"expected_owner_epoch":%d,"request_token":%s,"decision":"allow","choice":null}`,
			run.directorMemberID, run.runID, run.ownerEpoch, jsonQuoted(t, strings.Repeat("a", 64)))
	case "receipt":
		switch action {
		case "query":
			fields = `{"action":"query","operation_key":"fixture:query"}`
		case "retire":
			fields = `{"action":"retire","input_id":13}`
		case "reconcile":
			fields = `{"action":"reconcile","input_id":13,"expected_state":"uncertain","disposition":"submitted","evidence_ref":"fixture:settlement"}`
		default:
			t.Fatalf("receipt fixture action %q", action)
		}
	default:
		t.Fatalf("request fixture kind %q", kind)
	}
	members, ok := jsonMembers(jsontext.Value(fields))
	if !ok {
		t.Fatalf("fixture fields %s", fields)
	}
	for _, member := range members {
		object = append(object, jsonMember{key: member.key, value: member.value})
	}
	return mustJSON(t, object)
}

func (c wireCase) inspection(t *testing.T, supported bool) jsontext.Value {
	t.Helper()
	run := c.fixture.run
	native := mustJSON(t, run.nativeVersion)
	evidence, err := metadataEvidence(jsontext.Value(fmt.Sprintf(`{"backend":"codex","native_version":%s,"rule_id":"fixture-wire-evidence","screen_seq":2,"operation":null,"choices":null,"excerpt":null,"truncated":false}`, native)))
	if err != nil {
		t.Fatal(err)
	}
	classification, visibility, rule, capability, reason, decisions, token := "null", `"unknown"`, "null", `"unknown"`, `"fixture unsupported"`, "[]", "null"
	if supported {
		classification, visibility, rule, capability, reason, decisions, token = `"permission_request"`, `"visible"`, `"fixture-rule"`, `"tested"`, "null", `["allow","deny"]`, jsonQuoted(t, strings.Repeat("a", 64))
	}
	return jsontext.Value(fmt.Sprintf(`{"event_id":11,"message_id":14,"member_id":%d,"fleet_id":%d,"director_member_id":%d,"run_id":%d,"owner_epoch":%d,"interaction_id":12,"episode_seq":1,"classification":%s,"visibility":%s,"backend":"codex","native_version":%s,"answer_rule_id":%s,"answer_capability":%s,"capability_reason":%s,"supported_decisions":%s,"supported_choices":[],"request_token":%s,"output_seq":2,"evidence":%s,"missing_evidence":["operation","arguments","cwd","environment","targets","effects","choice_scope"]}`,
		run.memberID, run.fleetID, run.directorMemberID, run.runID, run.ownerEpoch, classification, visibility, native, rule, capability, reason, decisions, token, jsonText(evidence)))
}

func (c wireCase) questionInspection(t *testing.T, supported bool) jsontext.Value {
	t.Helper()
	value := c.inspection(t, supported)
	value = jsonSetAt(t, value, `"question"`, "classification")
	value = jsonSetAt(t, value, `"visible"`, "visibility")
	value = jsonSetAt(t, value, "[]", "supported_decisions")
	value = jsonSetAt(t, value, "[]", "missing_evidence")
	if supported {
		value = jsonSetAt(t, value, `"fixture-question-decline"`, "answer_rule_id")
		value = jsonSetAt(t, value, `[{"choice_id":"decline","label":"No, keep manual mode","scope":"persistent"},{"choice_id":"later","label":"Ask later","scope":"persistent"}]`, "supported_choices")
	} else {
		value = jsonSetAt(t, value, `"question has no tested answer"`, "capability_reason")
	}
	return value
}

func (c wireCase) choiceRequest(t *testing.T, choice string) jsontext.Value {
	t.Helper()
	value := c.request(t, "permission_respond", "")
	value = jsonSetAt(t, value, "null", "decision")
	return jsonSetAt(t, value, jsonQuoted(t, choice), "choice")
}

func (c wireCase) response(t *testing.T) jsontext.Value {
	t.Helper()
	run := c.fixture.run
	return jsontext.Value(fmt.Sprintf(`{"status":"accepted","member_id":%d,"run_id":%d,"owner_epoch":%d,"interaction_id":12,"event_id":11,"decision":"allow","choice":null,"stage":"submitted","output_seq":2,"reason":null}`,
		run.memberID, run.runID, run.ownerEpoch))
}

func (c wireCase) choiceResponse(t *testing.T, choice string) jsontext.Value {
	t.Helper()
	value := c.response(t)
	value = jsonSetAt(t, value, "null", "decision")
	return jsonSetAt(t, value, jsonQuoted(t, choice), "choice")
}

func wireBytes(value jsontext.Value) []byte {
	return []byte(jsonText(value))
}

func wireDecoded(t *testing.T, value jsontext.Value) inputControlRequest {
	t.Helper()
	request, err := decodeInputRequest(wireBytes(value))
	if err != nil {
		t.Fatalf("%s decodes: %v", value, err)
	}
	return request
}

// wireRefusal requires the decoder to accept good and refuse bad, returning the refusal's
// message.
func wireRefusal(t *testing.T, good jsontext.Value, bad []byte) string {
	t.Helper()
	wireDecoded(t, good)
	_, err := decodeInputRequest(bad)
	if err == nil {
		t.Fatalf("invalid original request rejects: %s", bad)
	}
	return cafleetErrorOf(err).message()
}

func wireRejection(t *testing.T, good jsontext.Value, bad []byte) {
	t.Helper()
	wireRefusal(t, good, bad)
}

func wirePrefix(value jsontext.Value, fields string) []byte {
	return []byte("{" + fields + jsonText(value)[1:])
}

func wireDuplicate(t *testing.T, value jsontext.Value, field string) []byte {
	t.Helper()
	if !jsonHas(t, value, field) {
		t.Fatalf("%s has %s", value, field)
	}
	return wirePrefix(value, jsonQuoted(t, field)+":"+jsonText(jsonAt(t, value, field))+",")
}

func wireNestedDuplicate(t *testing.T, value jsontext.Value, path []string, field string) []byte {
	t.Helper()
	original := jsonText(value)
	target := jsonText(jsonAt(t, value, path...))
	if count := strings.Count(original, target); count != 1 {
		t.Fatalf("the duplicate target %v occurs %d times", path, count)
	}
	return []byte(strings.Replace(original, target, string(wireDuplicate(t, jsontext.Value(target), field)), 1))
}

func wireSubmission(status, stage, id string) jsontext.Value {
	var delivery string
	switch status {
	case "accepted":
		delivery = `{"status":"submitted","method":"profile","reason":null}`
	case "queued":
		delivery = `{"status":"queued","method":null,"reason":"interaction_open"}`
	case "held":
		delivery = `{"status":"held","method":null,"reason":"observer_unavailable"}`
	case "rejected":
		delivery = `{"status":"not_submitted","method":null,"reason":"runner_stopped"}`
	default:
		delivery = `{"status":"uncertain","method":"fallback","reason":"submission_plan_unavailable"}`
	}
	reason := `"fixture outcome"`
	if status == "accepted" {
		reason = "null"
	}
	return jsontext.Value(fmt.Sprintf(`{"status":%q,"input_id":%s,"stage":%q,"output_seq":null,"reason":%s,"delivery":%s}`, status, id, stage, reason, delivery))
}

func wireReply(kind, field string, payload jsontext.Value) jsontext.Value {
	return jsontext.Value(fmt.Sprintf(`{"protocol_version":1,"result":{"kind":%q,%q:%s}}`, kind, field, jsonText(payload)))
}

func wireDecodedReply(t *testing.T, good, request jsontext.Value) inputControlReply {
	t.Helper()
	reply, err := decodeInputReply(wireBytes(good), wireDecoded(t, request))
	if err != nil {
		t.Fatalf("%s decodes for %s: %v", good, request, err)
	}
	return reply
}

// wireRejectReply requires the decoder to accept good and refuse bad for request.
func wireRejectReply(t *testing.T, good jsontext.Value, bad []byte, request jsontext.Value) {
	t.Helper()
	wireDecodedReply(t, good, request)
	if _, err := decodeInputReply(bad, wireDecoded(t, request)); err == nil {
		t.Fatalf("invalid original reply rejects: %s", bad)
	}
}

func assertWireSubmission(t *testing.T, actual submissionResult, expected jsontext.Value) {
	t.Helper()
	status, _ := jsonString(jsonIndex(expected, "status"))
	stage, _ := jsonString(jsonIndex(expected, "stage"))
	inputID := none[int64]()
	if id, ok := jsonInt64(jsonIndex(expected, "input_id")); ok {
		inputID = some(id)
	}
	outputSeq := none[uint64]()
	if seq, ok := jsonUint64(jsonIndex(expected, "output_seq")); ok {
		outputSeq = some(seq)
	}
	if actual.Status.asStr() != status || actual.Stage.asStr() != stage || actual.InputID != inputID ||
		actual.OutputSeq != outputSeq || actual.Reason != jsonOptString(jsonIndex(expected, "reason")) {
		t.Fatalf("submission %+v, want %s", actual, expected)
	}
	assertJSONEqual(t, jsonAt(t, mustJSON(t, actual), "delivery"), jsonIndex(expected, "delivery"), "submission delivery")
}

func frameBody(t *testing.T, frame []byte) []byte {
	t.Helper()
	if len(frame) < 4 || int(binary.BigEndian.Uint32(frame[:4])) != len(frame)-4 {
		t.Fatalf("frame %q has its exact length prefix", frame)
	}
	return frame[4:]
}

func assertFrame(t *testing.T, frame []byte, expected jsontext.Value) {
	t.Helper()
	body := frameBody(t, frame)
	if len(body) > 65536 {
		t.Fatalf("frame body of %d bytes exceeds the cap", len(body))
	}
	assertJSONEqual(t, jsontext.Value(body), expected, "encoded frame")
}

func wirePadded(t *testing.T, value jsontext.Value, target int, path ...string) jsontext.Value {
	t.Helper()
	value = jsonSetAt(t, value, `""`, path...)
	overhead := len(wireBytes(value))
	if target < overhead+3 {
		t.Fatalf("target %d below overhead %d", target, overhead)
	}
	value = jsonSetAt(t, value, jsonQuoted(t, "界"+strings.Repeat("x", target-overhead-3)), path...)
	if length := len(wireBytes(value)); length != target {
		t.Fatalf("padded length %d, want %d", length, target)
	}
	return value
}

func mustEncodeInputRequest(t *testing.T, request inputControlRequest) []byte {
	t.Helper()
	encoded, err := encodeInputRequest(request)
	if err != nil {
		t.Fatal(err)
	}
	return encoded
}

func mustEncodeInputReply(t *testing.T, reply inputControlReply) []byte {
	t.Helper()
	encoded, err := encodeInputReply(reply)
	if err != nil {
		t.Fatal(err)
	}
	return encoded
}

func TestRuntimeControl_RequestValidFlatOperationsAndLifecycleRoutingHavePositiveBaselines(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	for _, operation := range wireOperations {
		value := c.request(t, operation.kind, operation.action)
		request := wireDecoded(t, value)
		if !request.Identity.equal(controlIdentityFromRun(c.fixture.run)) || request.ExpiresAtMonoNS != 50_000_000_000 {
			t.Fatalf("%s: envelope %+v", operation, request)
		}
		switch decoded := request.Operation.(type) {
		case submitPromptOperation:
			if decoded.input.InputKind != inputKindPrompt || decoded.input.Literal != "literal 世界 '$x'" {
				t.Fatalf("%s: %+v", operation, decoded)
			}
		case submitExecOperation:
			if decoded.input.InputKind != inputKindExec {
				t.Fatalf("%s: %+v", operation, decoded)
			}
		case permissionInspectOperation:
			if decoded.input.InteractionID != 12 || decoded.input.EventID != 11 {
				t.Fatalf("%s: %+v", operation, decoded)
			}
		case permissionRespondOperation:
			if decoded.input.ExpectedRunID != c.fixture.run.runID ||
				decoded.input.Key != answerKey(answerKeyDecision{decision: permissionDecisionAllow}) ||
				decoded.input.RequestToken != strings.Repeat("a", 64) {
				t.Fatalf("%s: %+v", operation, decoded)
			}
		case receiptOperation:
			switch action := decoded.action.(type) {
			case receiptQuery:
				if action.OperationKey != "fixture:query" {
					t.Fatalf("%s: %+v", operation, action)
				}
			case receiptRetire:
				if action.InputID != 13 {
					t.Fatalf("%s: %+v", operation, action)
				}
			case receiptReconcile:
				if action.InputID != 13 || action.Expected != inputStateUncertain ||
					action.Disposition != inputDispositionSubmitted || action.EvidenceRef != "fixture:settlement" {
					t.Fatalf("%s: %+v", operation, action)
				}
			default:
				t.Fatalf("%s: receipt action %T", operation, action)
			}
		default:
			t.Fatalf("%s: operation %T", operation, decoded)
		}
		routed, err := decodeServiceRequest(wireBytes(value))
		if err != nil {
			t.Fatalf("%s: %v", operation, err)
		}
		if _, ok := routed.(inputServiceRequest); !ok {
			t.Fatalf("%s routes to the input operation, got %T", operation, routed)
		}
	}
	identity := jsonAt(t, c.request(t, "submit_prompt", ""), "identity")
	lifecycles := []struct {
		kind, field, value string
	}{
		{"health", "", ""},
		{"prepare", "remaining_ms", "5"},
		{"start", "pane_id", `"%1"`},
		{"shutdown", "", ""},
	}
	for _, lifecycle := range lifecycles {
		value := jsontext.Value(fmt.Sprintf(`{"protocol_version":1,"kind":%q,"identity":%s}`, lifecycle.kind, jsonText(identity)))
		if lifecycle.field != "" {
			value = jsonSetAt(t, value, lifecycle.value, lifecycle.field)
		}
		routed, err := decodeServiceRequest(wireBytes(value))
		if err != nil {
			t.Fatalf("%s: %v", lifecycle.kind, err)
		}
		actual, ok := routed.(lifecycleServiceRequest)
		if !ok {
			t.Fatalf("%s routes to the lifecycle, got %T", lifecycle.kind, routed)
		}
		wire, err := lifecycleRequestWire(actual.request)
		if err != nil {
			t.Fatalf("%s: %v", lifecycle.kind, err)
		}
		assertJSONEqual(t, wire, value, lifecycle.kind)
		value = jsonSetAt(t, value, "true", "fixture_unknown")
		if _, err := decodeServiceRequest(wireBytes(value)); err == nil {
			t.Fatalf("%s with an unknown field decodes", lifecycle.kind)
		}
	}
}

func TestRuntimeControl_RequestInputThenLifecycleDuplicateKindCannotEscapeStrictRouting(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	good := c.request(t, "submit_prompt", "")
	wireDecoded(t, good)
	legacy := jsonSetAt(t, good, `"health"`, "kind")
	if _, err := decodeServiceRequest(wirePrefix(legacy, `"kind":"submit_prompt",`)); err == nil {
		t.Fatal("an input kind before a lifecycle kind decodes")
	}
	if _, err := decodeServiceRequest(wirePrefix(good, `"kind":"health",`)); err == nil {
		t.Fatal("a lifecycle kind before an input kind decodes")
	}
}

func TestRuntimeControl_RequestEveryEnvelopeAndOperationFieldRejectsRawDuplication(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	for _, operation := range wireOperations {
		good := c.request(t, operation.kind, operation.action)
		for _, field := range testJSONKeys(t, good) {
			raw := wireDuplicate(t, good, field)
			wireRejection(t, good, raw)
			if _, err := decodeServiceRequest(raw); err == nil {
				t.Fatalf("%s %s: a duplicated field routes", operation, field)
			}
		}
	}
}

func TestRuntimeControl_RequestIdentityAndDatabaseIdentityRejectNestedRawDuplicates(t *testing.T) {
	t.Parallel()
	good := newWireCase(t).request(t, "submit_prompt", "")
	for _, path := range [][]string{{"identity"}, {"identity", "database_identity_json"}} {
		for _, field := range testJSONKeys(t, jsonAt(t, good, path...)) {
			wireRejection(t, good, wireNestedDuplicate(t, good, path, field))
		}
	}
}

func TestRuntimeControl_RequestRequiredFieldsDistinguishAbsenceFromExplicitNull(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	for _, operation := range wireOperations {
		good := c.request(t, operation.kind, operation.action)
		for _, field := range testJSONKeys(t, good) {
			wireRejection(t, good, wireBytes(jsonRemoveAt(t, good, field)))
		}
		for _, field := range testJSONKeys(t, jsonAt(t, good, "identity")) {
			wireRejection(t, good, wireBytes(jsonRemoveAt(t, good, "identity", field)))
		}
	}
}

func TestRuntimeControl_ExactReencodingRefusesDefaultedFlattenedFloatAndDuplicatedFields(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	prompt := c.request(t, "submit_prompt", "")
	respond := c.request(t, "permission_respond", "")
	retire := c.request(t, "receipt", "retire")

	wireRejection(t, prompt, wireBytes(jsonSetAt(t, prompt, "50000000000.0", "expires_at_mono_ns")))
	wireRejection(t, respond, wireBytes(jsonRemoveAt(t, respond, "choice")))
	wireRejection(t, respond, wireDuplicate(t, respond, "decision"))

	accepted := wireReply("submission", "submission", wireSubmission("accepted", "submitted", "13"))
	wireRejectReply(t, accepted, wireBytes(jsonRemoveAt(t, accepted, "result", "submission", "delivery")), prompt)
	wireRejectReply(t, accepted, wireBytes(jsonSetAt(t, accepted, "13.0", "result", "submission", "input_id")), prompt)

	response := wireReply("permission_response", "response", c.response(t))
	flattened := testJSONWith(t, response, jsonAt(t, wireSubmission("accepted", "submitted", "13"), "delivery"), "result", "response", "delivery")
	wireRejectReply(t, response, wireBytes(flattened), respond)
	wireRejectReply(t, response, wireBytes(jsonRemoveAt(t, response, "result", "response", "choice")), respond)

	retired := wireReply("receipt", "receipt", jsontext.Value(`{"status":"retired","input_id":13,"submission":null}`))
	wireRejectReply(t, retired, wireBytes(jsonRemoveAt(t, retired, "result", "receipt", "submission")), retire)
}

func TestRuntimeControl_RequestUnknownFieldsAndOtherReceiptActionFieldsReject(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	for _, operation := range wireOperations {
		good := c.request(t, operation.kind, operation.action)
		wireRejection(t, good, wireBytes(jsonSetAt(t, good, "true", "unexpected")))
		wireRejection(t, good, wireBytes(jsonSetAt(t, good, "true", "identity", "unexpected")))
	}
	for _, foreign := range []struct{ action, field, value string }{
		{"query", "input_id", "13"},
		{"retire", "operation_key", `"fixture:key"`},
		{"retire", "expected_state", `"ready"`},
		{"reconcile", "operation_key", `"fixture:key"`},
	} {
		good := c.request(t, "receipt", foreign.action)
		wireRejection(t, good, wireBytes(jsonSetAt(t, good, foreign.value, foreign.field)))
	}
}

func TestRuntimeControl_RequestNumericTypeVersionExpiryAndEnumDomainsAreStrict(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	good := c.request(t, "permission_respond", "")
	for _, path := range [][]string{
		{"protocol_version"}, {"expires_at_mono_ns"}, {"identity", "run_id"}, {"identity", "fleet_id"},
		{"identity", "member_id"}, {"identity", "director_member_id"}, {"identity", "owner_epoch"},
		{"interaction_id"}, {"event_id"}, {"director_member_id"}, {"expected_run_id"}, {"expected_owner_epoch"},
	} {
		for _, invalid := range []string{"null", "true", `"1"`, "1.0", "0", "-1"} {
			wireRejection(t, good, wireBytes(jsonSetAt(t, good, invalid, path...)))
		}
	}
	for _, invalid := range []string{"2", "18446744073709551615"} {
		wireRejection(t, good, wireBytes(jsonSetAt(t, good, invalid, "protocol_version")))
	}
	wireDecoded(t, jsonSetAt(t, good, "18446744073709551615", "expires_at_mono_ns"))
	overflow := strings.Replace(string(wireBytes(good)), "50000000000", "18446744073709551616", 1)
	wireRejection(t, good, []byte(overflow))
	wireDecoded(t, jsonSetAt(t, good, "9223372036854775807", "event_id"))
	wireRejection(t, good, wireBytes(jsonSetAt(t, good, "9223372036854775808", "event_id")))
	wireRejection(t, good, wireBytes(jsonSetAt(t, good, `"ALLOW"`, "decision")))
	for _, domain := range []struct{ kind, action, field, invalid string }{
		{"submit_prompt", "", "input_kind", "permission_response"},
		{"receipt", "query", "action", "unknown"},
		{"receipt", "reconcile", "expected_state", "unknown"},
		{"receipt", "reconcile", "disposition", "unknown"},
	} {
		good := c.request(t, domain.kind, domain.action)
		wireRejection(t, good, wireBytes(jsonSetAt(t, good, jsonQuoted(t, domain.invalid), domain.field)))
	}
}

func TestRuntimeControl_RequestKindPayloadPermissionBindingAndReferenceValidation(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	literalKey := jsonSetAt(t, c.request(t, "submit_prompt", ""), jsonQuoted(t, "\nfixture:literal-key\x7f"), "operation_key")
	prompt, ok := wireDecoded(t, literalKey).Operation.(submitPromptOperation)
	if !ok || prompt.input.OperationKey != "\nfixture:literal-key\x7f" {
		t.Fatalf("a literal operation key is preserved: %+v", prompt)
	}
	for _, invalid := range []struct{ kind, field, value string }{
		{"submit_prompt", "input_kind", `"exec"`},
		{"submit_exec", "input_kind", `"prompt"`},
		{"submit_prompt", "literal", `" "`},
		{"submit_exec", "literal", `"a\nb"`},
		{"submit_prompt", "operation_key", `" "`},
	} {
		good := c.request(t, invalid.kind, "")
		wireRejection(t, good, wireBytes(jsonSetAt(t, good, invalid.value, invalid.field)))
	}
	good := c.request(t, "submit_prompt", "")
	for b := range 33 {
		character := byte(b)
		if b == 32 {
			character = 127
		}
		literal := "a" + string(rune(character)) + "b"
		wireRejection(t, good, wireBytes(jsonSetAt(t, good, jsonQuoted(t, literal), "literal")))
	}
	good = c.request(t, "permission_respond", "")
	run := c.fixture.run
	for _, invalid := range []struct{ field, value string }{
		{"expected_run_id", fmt.Sprint(run.runID + 1)},
		{"expected_owner_epoch", fmt.Sprint(run.ownerEpoch + 1)},
		{"director_member_id", fmt.Sprint(run.memberID)},
		{"request_token", jsonQuoted(t, strings.Repeat("A", 64))},
		{"request_token", jsonQuoted(t, strings.Repeat("a", 63))},
	} {
		wireRejection(t, good, wireBytes(jsonSetAt(t, good, invalid.value, invalid.field)))
	}
}

func TestRuntimeControl_RequestEncodingIsFlatWithRequiredNullsAndExactUtf8FrameCap(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	for _, operation := range wireOperations {
		good := c.request(t, operation.kind, operation.action)
		assertFrame(t, mustEncodeInputRequest(t, wireDecoded(t, good)), good)
	}
	good := wirePadded(t, c.request(t, "submit_prompt", ""), 65536, "literal")
	assertFrame(t, mustEncodeInputRequest(t, wireDecoded(t, good)), good)
	oversized := wirePadded(t, c.request(t, "submit_prompt", ""), 65537, "literal")
	request := wireDecoded(t, c.request(t, "submit_prompt", ""))
	prompt := request.Operation.(submitPromptOperation)
	prompt.input.Literal, _ = jsonString(jsonIndex(oversized, "literal"))
	request.Operation = prompt
	if _, err := encodeInputRequest(request); err == nil {
		t.Fatal("a 65537-byte request body exceeds the cap")
	}
	invalid := wireDecoded(t, c.request(t, "submit_prompt", ""))
	invalid.ExpiresAtMonoNS = 0
	if _, err := encodeInputRequest(invalid); err == nil {
		t.Fatal("a zero expiration does not encode")
	}
}

func TestRuntimeControl_InvalidRequestDiagnosticsKeepPayloadCanariesPrivate(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	canary := strings.Repeat("12bd", 16)
	good := jsonSetAt(t, c.request(t, "permission_respond", ""), jsonQuoted(t, canary), "request_token")
	for _, field := range []string{"request_token", "decision", "expected_run_id"} {
		message := wireRefusal(t, good, wireBytes(jsonSetAt(t, good, `"PRIVATE_INVALID_8d14"`, field)))
		for _, private := range []string{canary, "PRIVATE_INVALID_8d14"} {
			if strings.Contains(message, private) {
				t.Fatalf("%s: the diagnostic %q keeps %s private", field, message, private)
			}
		}
	}
	good = jsonSetAt(t, c.request(t, "submit_prompt", ""), `"PRIVATE_LITERAL_42a1"`, "literal")
	if message := wireRefusal(t, good, wireDuplicate(t, good, "literal")); strings.Contains(message, "PRIVATE_LITERAL_42a1") {
		t.Fatalf("the duplicate diagnostic %q keeps the literal private", message)
	}
}

func TestRuntimeControl_ReplyStructuredSubmissionStatesDecodeWithExactScalarValues(t *testing.T) {
	t.Parallel()
	request := newWireCase(t).request(t, "submit_prompt", "")
	for _, state := range []struct{ status, stage, id string }{
		{"accepted", "submitted", "13"},
		{"queued", "zero_bytes", "13"},
		{"held", "zero_bytes", "null"},
		{"rejected", "zero_bytes", "null"},
		{"uncertain", "partial", "13"},
		{"uncertain", "submitted", "13"},
		{"uncertain", "unknown", "13"},
	} {
		payload := wireSubmission(state.status, state.stage, state.id)
		reply := wireDecodedReply(t, wireReply("submission", "submission", payload), request)
		actual, ok := reply.(submissionResult)
		if !ok {
			t.Fatalf("%s/%s: submission result has exact kind, got %T", state.status, state.stage, reply)
		}
		assertWireSubmission(t, actual, payload)
	}
}

func TestRuntimeControl_ReplyQueuedSubmissionRequiresZeroBytesADurableIdAndADeliveryProjection(t *testing.T) {
	t.Parallel()
	request := newWireCase(t).request(t, "submit_prompt", "")
	good := wireReply("submission", "submission", wireSubmission("queued", "zero_bytes", "13"))
	for _, bad := range []struct{ field, value string }{
		{"stage", `"partial"`},
		{"stage", `"submitted"`},
		{"input_id", "null"},
		{"delivery", "null"},
		{"delivery", `{"status":"queued","method":"bogus","reason":"interaction_open"}`},
		{"delivery", `{"status":"queued","method":null,"reason":"interaction_open","extra":true}`},
		{"delivery", `{"status":"queued","method":null}`},
	} {
		wireRejectReply(t, good, wireBytes(jsonSetAt(t, good, bad.value, "result", "submission", bad.field)), request)
	}
	wireRejectReply(t, good, wireBytes(jsonRemoveAt(t, good, "result", "submission", "delivery")), request)
}

func TestRuntimeControl_ReplyInspectionSupportedAndUnknownPreserveCompletePublicObject(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	request := c.request(t, "permission_inspect", "")
	for _, supported := range []bool{false, true} {
		good := wireReply("inspection", "request", c.inspection(t, supported))
		actual := wireDecodedReply(t, good, request)
		if _, ok := actual.(permissionInspection); !ok {
			t.Fatalf("supported=%v: inspection result has exact kind, got %T", supported, actual)
		}
		assertFrame(t, mustEncodeInputReply(t, actual), good)
	}
}

func TestRuntimeControl_ReplyPermissionResponseFlattensSubmissionIntoPublicShape(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	request := c.request(t, "permission_respond", "")
	good := wireReply("permission_response", "response", c.response(t))
	reply := wireDecodedReply(t, good, request)
	actual, ok := reply.(permissionResponseResult)
	if !ok {
		t.Fatalf("response result has exact kind, got %T", reply)
	}
	run := c.fixture.run
	if actual.Submission.InputID != none[int64]() || actual.Submission.Status != submissionStatusAccepted ||
		actual.Submission.Stage != submissionStageSubmitted || actual.Submission.OutputSeq != some[uint64](2) ||
		actual.Submission.Reason != none[string]() {
		t.Fatalf("flattened submission %+v", actual.Submission)
	}
	if actual.MemberID != run.memberID || actual.RunID != run.runID || actual.OwnerEpoch != run.ownerEpoch ||
		actual.InteractionID != 12 || actual.EventID != 11 || actual.Key != answerKey(answerKeyDecision{decision: permissionDecisionAllow}) {
		t.Fatalf("response identity %+v", actual)
	}
	nested := testJSONWith(t, good, wireSubmission("accepted", "submitted", "13"), "result", "response", "submission")
	wireRejectReply(t, good, wireBytes(nested), request)
}

func TestRuntimeControl_RequestPermissionRespondCarriesExactlyOneOfADecisionAndAChoice(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	respond, ok := wireDecoded(t, c.request(t, "permission_respond", "")).Operation.(permissionRespondOperation)
	if !ok || respond.input.Key != answerKey(answerKeyDecision{decision: permissionDecisionAllow}) {
		t.Fatalf("permission response operation %+v", respond)
	}
	for _, choice := range []string{"decline", "dont-show-again", "not-now", "a", strings.Repeat("a", 64)} {
		good := c.choiceRequest(t, choice)
		respond, ok := wireDecoded(t, good).Operation.(permissionRespondOperation)
		if !ok || respond.input.Key != answerKey(answerKeyChoice{choice: choice}) {
			t.Fatalf("%s: permission response operation %+v", choice, respond)
		}
		assertFrame(t, mustEncodeInputRequest(t, wireDecoded(t, good)), good)
	}
	good := c.choiceRequest(t, "decline")
	wireRejection(t, good, wireBytes(jsonSetAt(t, good, `"deny"`, "decision")))
	wireRejection(t, good, wireBytes(jsonSetAt(t, good, "null", "choice")))
	for _, invalid := range []string{
		`"Decline"`, `"ask_later"`, `"-decline"`, `"decline-"`, `"dont--show"`, `"dont show"`, `""`,
		jsonQuoted(t, strings.Repeat("a", 65)), "1", `["decline"]`,
	} {
		wireRejection(t, good, wireBytes(jsonSetAt(t, good, invalid, "choice")))
	}
	wireRejection(t, good, wireBytes(jsonRemoveAt(t, good, "choice")))
	wireRejection(t, good, wireDuplicate(t, good, "choice"))
}

func TestRuntimeControl_ReplyQuestionInspectionCarriesItsSupportedChoicesAndNoDecisions(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	request := c.request(t, "permission_inspect", "")
	for _, supported := range []bool{true, false} {
		payload := c.questionInspection(t, supported)
		good := wireReply("inspection", "request", payload)
		reply := wireDecodedReply(t, good, request)
		actual, ok := reply.(permissionInspection)
		if !ok {
			t.Fatalf("supported=%v: inspection result has exact kind, got %T", supported, reply)
		}
		if actual.Classification != some(interactionClassQuestion) || len(actual.SupportedDecisions) != 0 ||
			len(actual.MissingEvidence) != 0 {
			t.Fatalf("supported=%v: question inspection %+v", supported, actual)
		}
		assertJSONEqual(t, mustJSON(t, actual.SupportedChoices), jsonAt(t, payload, "supported_choices"), "supported choices")
		if actual.CapabilityReason != jsonOptString(jsonIndex(payload, "capability_reason")) ||
			actual.RequestToken != jsonOptString(jsonIndex(payload, "request_token")) {
			t.Fatalf("supported=%v: reason %v token %v", supported, actual.CapabilityReason, actual.RequestToken)
		}
		encoded := mustEncodeInputReply(t, actual)
		assertFrame(t, encoded, good)
		choices, _ := jsonArray(jsonAt(t, jsontext.Value(frameBody(t, encoded)), "result", "request", "supported_choices"))
		var keys [][]string
		for _, choice := range choices {
			keys = append(keys, testJSONKeys(t, choice))
		}
		var want [][]string
		if supported {
			want = [][]string{{"choice_id", "label", "scope"}, {"choice_id", "label", "scope"}}
		}
		if !slices.EqualFunc(keys, want, slices.Equal[[]string]) {
			t.Fatalf("supported=%v: choice keys %v", supported, keys)
		}
	}
	good := wireReply("inspection", "request", c.questionInspection(t, true))
	for _, bad := range []struct{ name, choices string }{
		{"unknown scope", `[{"choice_id":"decline","label":"Keep","scope":"forever"}]`},
		{"missing label", `[{"choice_id":"decline","scope":"persistent"}]`},
		{"extra field", `[{"choice_id":"decline","label":"Keep","scope":"persistent","stage":"0d"}]`},
		{"choice id outside its shape", `[{"choice_id":"Decline","label":"Keep","scope":"persistent"}]`},
		{"duplicate choice", `[{"choice_id":"decline","label":"Keep","scope":"persistent"},{"choice_id":"decline","label":"Keep","scope":"persistent"}]`},
		{"non-array", `{"choice_id":"decline"}`},
	} {
		value := jsonSetAt(t, good, bad.choices, "result", "request", "supported_choices")
		if _, err := decodeInputReply(wireBytes(value), wireDecoded(t, request)); err == nil {
			t.Fatalf("%s decodes", bad.name)
		}
	}
	wireRejectReply(t, good, wireBytes(jsonSetAt(t, good, `["allow"]`, "result", "request", "supported_decisions")), request)
}

func TestRuntimeControl_ReplyChoiceResponseNamesTheRequestedChoiceAfterANullDecision(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	request := c.choiceRequest(t, "decline")
	good := wireReply("permission_response", "response", c.choiceResponse(t, "decline"))
	reply := wireDecodedReply(t, good, request)
	actual, ok := reply.(permissionResponseResult)
	if !ok || actual.Key != answerKey(answerKeyChoice{choice: "decline"}) {
		t.Fatalf("choice response %T %+v", reply, reply)
	}
	assertFrame(t, mustEncodeInputReply(t, actual), good)
	for _, bad := range []struct{ field, value string }{
		{"choice", `"later"`},
		{"choice", "null"},
		{"decision", `"allow"`},
	} {
		wireRejectReply(t, good, wireBytes(jsonSetAt(t, good, bad.value, "result", "response", bad.field)), request)
	}
	decisionRequest := c.request(t, "permission_respond", "")
	decisionGood := wireReply("permission_response", "response", c.response(t))
	wireRejectReply(t, decisionGood, wireBytes(jsonSetAt(t, decisionGood, `"decline"`, "result", "response", "choice")), decisionRequest)
}

func TestRuntimeControl_ReplyRejectedAnswerKindMismatchesDecodeWithTheirReasons(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	for _, rejected := range []struct {
		request, response jsontext.Value
		reason            string
	}{
		{c.choiceRequest(t, "decline"), c.choiceResponse(t, "decline"), "choice answers apply only to question interactions"},
		{c.request(t, "permission_respond", ""), c.response(t), "decision answers apply only to permission requests"},
		{c.choiceRequest(t, "decline"), c.choiceResponse(t, "decline"), "choice decline has no tested answer for this question"},
	} {
		response := jsonSetAt(t, rejected.response, `"rejected"`, "status")
		response = jsonSetAt(t, response, `"zero_bytes"`, "stage")
		response = jsonSetAt(t, response, "null", "output_seq")
		response = jsonSetAt(t, response, jsonQuoted(t, rejected.reason), "reason")
		reply := wireDecodedReply(t, wireReply("permission_response", "response", response), rejected.request)
		actual, ok := reply.(permissionResponseResult)
		if !ok {
			t.Fatalf("%s: response result has exact kind, got %T", rejected.reason, reply)
		}
		if actual.Submission.Status != submissionStatusRejected || actual.Submission.Stage != submissionStageZeroBytes ||
			actual.Submission.Reason != some(rejected.reason) {
			t.Fatalf("%s: %+v", rejected.reason, actual.Submission)
		}
	}
}

func TestRuntimeControl_ReplyReceiptVariantsRequireExactIdsAndNulls(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	for _, variant := range []struct {
		action, status, id string
		payload            jsontext.Value
	}{
		{"query", "missing", "null", jsontext.Value("null")},
		{"query", "present", "13", wireSubmission("accepted", "submitted", "13")},
		{"retire", "retired", "13", jsontext.Value("null")},
		{"reconcile", "settled", "13", jsontext.Value("null")},
	} {
		request := c.request(t, "receipt", variant.action)
		good := wireReply("receipt", "receipt", jsontext.Value(fmt.Sprintf(`{"status":%q,"input_id":%s,"submission":%s}`, variant.status, variant.id, variant.payload)))
		switch reply := wireDecodedReply(t, good, request).(type) {
		case receiptMissing:
			if variant.status != "missing" {
				t.Fatalf("%s decodes as missing", variant.status)
			}
		case receiptPresent:
			if variant.status != "present" || reply.InputID != 13 {
				t.Fatalf("%s decodes as present %d", variant.status, reply.InputID)
			}
			assertWireSubmission(t, reply.Submission, variant.payload)
		case receiptRetired:
			if variant.status != "retired" || reply.InputID != 13 {
				t.Fatalf("%s decodes as retired %d", variant.status, reply.InputID)
			}
		case receiptSettled:
			if variant.status != "settled" || reply.InputID != 13 {
				t.Fatalf("%s decodes as settled %d", variant.status, reply.InputID)
			}
		case submissionResult, permissionInspection, permissionResponseResult:
			t.Fatalf("%s: receipt result has exact kind, got %T", variant.status, reply)
		default:
			t.Fatalf("%s: receipt result has exact kind, got %T", variant.status, reply)
		}
	}
}

func TestRuntimeControl_ReplyEnvelopeRequiresVersionExactFieldsAndResultXorTextError(t *testing.T) {
	t.Parallel()
	request := newWireCase(t).request(t, "submit_prompt", "")
	good := wireReply("submission", "submission", wireSubmission("accepted", "submitted", "13"))
	for _, field := range []string{"protocol_version", "result"} {
		wireRejectReply(t, good, wireBytes(jsonRemoveAt(t, good, field)), request)
	}
	for _, invalid := range []string{"null", "true", `"1"`, "1.0", "0", "2"} {
		wireRejectReply(t, good, wireBytes(jsonSetAt(t, good, invalid, "protocol_version")), request)
	}
	for _, invalid := range []string{"null", "true", "{}"} {
		wireRejectReply(t, good, []byte(`{"protocol_version":1,"error":`+invalid+`}`), request)
	}
	wireRejectReply(t, good, wireBytes(jsonSetAt(t, good, `"fixture"`, "error")), request)
	wireRejectReply(t, good, wireBytes(jsonSetAt(t, good, "true", "unexpected")), request)
	wireRejectReply(t, good, wireDuplicate(t, good, "protocol_version"), request)
	wireRejectReply(t, good, wireDuplicate(t, good, "result"), request)
}

func TestRuntimeControl_ReplyOriginalObjectsRejectNestedDuplicates(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	for _, nested := range []struct {
		request, good jsontext.Value
		paths         [][]string
	}{
		{
			c.request(t, "submit_prompt", ""),
			wireReply("submission", "submission", wireSubmission("accepted", "submitted", "13")),
			[][]string{{"result"}, {"result", "submission"}},
		},
		{
			c.request(t, "permission_inspect", ""),
			wireReply("inspection", "request", c.inspection(t, true)),
			[][]string{{"result"}, {"result", "request"}, {"result", "request", "evidence"}},
		},
		{
			c.request(t, "receipt", "query"),
			wireReply("receipt", "receipt", jsontext.Value(`{"status":"present","input_id":13,"submission":`+string(wireSubmission("accepted", "submitted", "13"))+`}`)),
			[][]string{{"result"}, {"result", "receipt"}, {"result", "receipt", "submission"}},
		},
	} {
		for _, path := range nested.paths {
			for _, field := range testJSONKeys(t, jsonAt(t, nested.good, path...)) {
				wireRejectReply(t, nested.good, wireNestedDuplicate(t, nested.good, path, field), nested.request)
			}
		}
	}
}

func TestRuntimeControl_ReplyMissingNullExtraAndDomainValidationIsRecursive(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	for _, recursive := range []struct {
		request, good jsontext.Value
		paths         [][]string
	}{
		{
			c.request(t, "submit_prompt", ""),
			wireReply("submission", "submission", wireSubmission("held", "zero_bytes", "null")),
			[][]string{{"result"}, {"result", "submission"}},
		},
		{
			c.request(t, "permission_inspect", ""),
			wireReply("inspection", "request", c.inspection(t, false)),
			[][]string{{"result"}, {"result", "request"}},
		},
		{
			c.request(t, "permission_respond", ""),
			wireReply("permission_response", "response", c.response(t)),
			[][]string{{"result"}, {"result", "response"}},
		},
		{
			c.request(t, "receipt", "query"),
			wireReply("receipt", "receipt", jsontext.Value(`{"status":"missing","input_id":null,"submission":null}`)),
			[][]string{{"result"}, {"result", "receipt"}},
		},
	} {
		for _, path := range recursive.paths {
			for _, field := range testJSONKeys(t, jsonAt(t, recursive.good, path...)) {
				fieldPath := append(slices.Clone(path), field)
				wireRejectReply(t, recursive.good, wireBytes(jsonRemoveAt(t, recursive.good, fieldPath...)), recursive.request)
				wireRejectReply(t, recursive.good, wireBytes(jsonSetAt(t, recursive.good, "true", fieldPath...)), recursive.request)
			}
			extra := append(slices.Clone(path), "unexpected")
			wireRejectReply(t, recursive.good, wireBytes(jsonSetAt(t, recursive.good, "true", extra...)), recursive.request)
		}
	}
	request := c.request(t, "permission_inspect", "")
	good := wireReply("inspection", "request", c.inspection(t, true))
	privateEvidence := jsonSetAt(t, good, `"PRIVATE_EVIDENCE_1d7a"`, "result", "request", "evidence", "operation")
	wireDecodedReply(t, good, request)
	_, err := decodeInputReply(wireBytes(privateEvidence), wireDecoded(t, request))
	if err == nil {
		t.Fatal("invalid metadata evidence rejects before decoding success")
	}
	if strings.Contains(cafleetErrorOf(err).message(), "PRIVATE_EVIDENCE_1d7a") {
		t.Fatal("the evidence diagnostic keeps its content private")
	}
	for _, bad := range []struct{ field, value string }{
		{"visibility", `"missing"`},
		{"classification", `"composer_empty"`},
		{"answer_capability", `"guessed"`},
		{"supported_decisions", `["permit"]`},
		{"missing_evidence", `["invented"]`},
		{"output_seq", "-1"},
		{"event_id", "0"},
		{"request_token", jsonQuoted(t, strings.Repeat("A", 64))},
	} {
		wireRejectReply(t, good, wireBytes(jsonSetAt(t, good, bad.value, "result", "request", bad.field)), request)
	}
	request = c.request(t, "submit_prompt", "")
	good = wireReply("submission", "submission", wireSubmission("accepted", "submitted", "13"))
	for _, bad := range []struct{ field, value string }{
		{"status", `"unknown"`},
		{"stage", `"invented"`},
		{"input_id", "0"},
		{"output_seq", "-1"},
		{"reason", "true"},
	} {
		wireRejectReply(t, good, wireBytes(jsonSetAt(t, good, bad.value, "result", "submission", bad.field)), request)
	}
}

func TestRuntimeControl_ReplyWellFormedOtherOperationResultKindRejects(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	pairs := []struct{ request, good jsontext.Value }{
		{c.request(t, "submit_prompt", ""), wireReply("submission", "submission", wireSubmission("accepted", "submitted", "13"))},
		{c.request(t, "permission_inspect", ""), wireReply("inspection", "request", c.inspection(t, true))},
		{c.request(t, "permission_respond", ""), wireReply("permission_response", "response", c.response(t))},
		{c.request(t, "receipt", "query"), wireReply("receipt", "receipt", jsontext.Value(`{"status":"missing","input_id":null,"submission":null}`))},
	}
	for index, pair := range pairs {
		for other, alternative := range pairs {
			if index != other {
				wireRejectReply(t, pair.good, wireBytes(alternative.good), pair.request)
			}
		}
	}
}

func TestRuntimeControl_ReplyOperationSpecificIdentitiesAndReceiptIdsMatchRequest(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	for _, binding := range []struct {
		request, good jsontext.Value
		payload       string
		fields        []string
	}{
		{
			c.request(t, "permission_inspect", ""),
			wireReply("inspection", "request", c.inspection(t, true)),
			"request",
			[]string{"event_id", "member_id", "fleet_id", "director_member_id", "run_id", "owner_epoch", "interaction_id"},
		},
		{
			c.request(t, "permission_respond", ""),
			wireReply("permission_response", "response", c.response(t)),
			"response",
			[]string{"event_id", "member_id", "run_id", "owner_epoch", "interaction_id"},
		},
	} {
		for _, field := range binding.fields {
			current, ok := jsonInt64(jsonAt(t, binding.good, "result", binding.payload, field))
			if !ok {
				t.Fatalf("%s.%s is an integer", binding.payload, field)
			}
			bad := jsonSetAt(t, binding.good, fmt.Sprint(current+1), "result", binding.payload, field)
			wireRejectReply(t, binding.good, wireBytes(bad), binding.request)
		}
		if binding.payload == "response" {
			wireRejectReply(t, binding.good, wireBytes(jsonSetAt(t, binding.good, `"deny"`, "result", binding.payload, "decision")), binding.request)
		}
	}
	request := c.request(t, "receipt", "query")
	good := wireReply("receipt", "receipt", jsontext.Value(`{"status":"present","input_id":13,"submission":`+string(wireSubmission("accepted", "submitted", "13"))+`}`))
	wireRejectReply(t, good, wireBytes(jsonSetAt(t, good, "14", "result", "receipt", "submission", "input_id")), request)
	for _, receipt := range []struct{ action, status string }{{"retire", "retired"}, {"reconcile", "settled"}} {
		request := c.request(t, "receipt", receipt.action)
		good := wireReply("receipt", "receipt", jsontext.Value(fmt.Sprintf(`{"status":%q,"input_id":13,"submission":null}`, receipt.status)))
		wireRejectReply(t, good, wireBytes(jsonSetAt(t, good, "14", "result", "receipt", "input_id")), request)
	}
}

func TestRuntimeControl_ReplyEncodingMatchesIndependentShapesPublicOrderAndBodyCap(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	for _, shape := range []struct{ request, expected jsontext.Value }{
		{c.request(t, "submit_prompt", ""), wireReply("submission", "submission", wireSubmission("accepted", "submitted", "13"))},
		{c.request(t, "permission_inspect", ""), wireReply("inspection", "request", c.inspection(t, false))},
		{c.request(t, "permission_respond", ""), wireReply("permission_response", "response", c.response(t))},
		{c.request(t, "receipt", "query"), wireReply("receipt", "receipt", jsontext.Value(`{"status":"missing","input_id":null,"submission":null}`))},
	} {
		actual := wireDecodedReply(t, shape.expected, shape.request)
		encoded := mustEncodeInputReply(t, actual)
		assertFrame(t, encoded, shape.expected)
		body := jsontext.Value(frameBody(t, encoded))
		kind, _ := jsonString(jsonIndex(shape.request, "kind"))
		switch kind {
		case "permission_inspect":
			want := strings.Split("event_id,message_id,member_id,fleet_id,director_member_id,run_id,owner_epoch,interaction_id,episode_seq,classification,visibility,backend,native_version,answer_rule_id,answer_capability,capability_reason,supported_decisions,supported_choices,request_token,output_seq,evidence,missing_evidence", ",")
			if keys := testJSONKeys(t, jsonAt(t, body, "result", "request")); !slices.Equal(keys, want) {
				t.Fatalf("inspection key order %v", keys)
			}
		case "permission_respond":
			want := strings.Split("status,member_id,run_id,owner_epoch,interaction_id,event_id,decision,choice,stage,output_seq,reason", ",")
			if keys := testJSONKeys(t, jsonAt(t, body, "result", "response")); !slices.Equal(keys, want) {
				t.Fatalf("response key order %v", keys)
			}
		}
	}
	failure, err := encodeErrorReply(appError("fixture failure"))
	if err != nil {
		t.Fatal(err)
	}
	assertFrame(t, failure, jsontext.Value(`{"protocol_version":1,"error":"fixture failure"}`))
	request := c.request(t, "submit_prompt", "")
	base := wireReply("submission", "submission", wireSubmission("uncertain", "unknown", "13"))
	exact := wirePadded(t, base, 65536, "result", "submission", "reason")
	assertFrame(t, mustEncodeInputReply(t, wireDecodedReply(t, exact, request)), exact)
	oversized := wirePadded(t, base, 65537, "result", "submission", "reason")
	reason, _ := jsonString(jsonAt(t, oversized, "result", "submission", "reason"))
	actual := submissionResult{
		Status:    submissionStatusUncertain,
		InputID:   some[int64](13),
		Stage:     submissionStageUnknown,
		OutputSeq: none[uint64](),
		Reason:    some(reason),
		Delivery: some(deliveryProjection{
			Status: deliveryStatusUncertain,
			Method: some(deliveryMethodFallback),
			Reason: some("submission_plan_unavailable"),
		}),
	}
	if _, err := encodeInputReply(actual); err == nil {
		t.Fatal("a 65537-byte reply body exceeds the cap")
	}
}

const (
	wirePayloadCanary  = "WIRE_R1_PRIVATE_LITERAL_CANARY"
	wireIdentityCanary = "WIRE_R1_PRIVATE_IDENTITY_CANARY"
)

func identityControl(t *testing.T, c wireCase) jsontext.Value {
	t.Helper()
	good := jsonSetAt(t, c.request(t, "submit_prompt", ""), jsonQuoted(t, wirePayloadCanary), "literal")
	request := wireDecoded(t, good)
	if !request.Identity.equal(controlIdentityFromRun(c.fixture.run)) {
		t.Fatalf("identity %+v", request.Identity)
	}
	assertFrame(t, mustEncodeInputRequest(t, request), good)
	return good
}

func assertPrivateIdentityError(t *testing.T, err error, good jsontext.Value, label string) {
	t.Helper()
	failure := cafleetErrorOf(err)
	diagnostic := fmt.Sprintf("%#v\n%v\n%s", failure, failure, failure.message())
	canonical, _ := jsonString(jsonAt(t, good, "identity", "database_identity_json", "canonical_path"))
	instance, _ := jsonString(jsonAt(t, good, "identity", "database_identity_json", "instance_id"))
	nonce, _ := jsonString(jsonAt(t, good, "identity", "launch_nonce"))
	for _, private := range []string{wirePayloadCanary, wireIdentityCanary, canonical, instance, nonce} {
		if strings.Contains(diagnostic, private) {
			t.Fatalf("%s: identity validation preserves payload privacy: %s", label, diagnostic)
		}
	}
}

func TestRuntimeControl_Identity_IdentityGenuineAndOwningDomainControlsPreserveLiteralIdentity(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	good := identityControl(t, c)
	uppercase := jsonSetAt(t, good, jsonQuoted(t, strings.Repeat("ABCDEF0123456789", 2)), "identity", "database_identity_json", "instance_id")
	uppercase = jsonSetAt(t, uppercase, jsonQuoted(t, strings.Repeat("ABCDEF0123456789", 4)), "identity", "launch_nonce")
	mixed := jsonSetAt(t, uppercase, jsonQuoted(t, strings.Repeat("aBcDeF0123456789", 2)), "identity", "database_identity_json", "instance_id")
	mixed = jsonSetAt(t, mixed, jsonQuoted(t, strings.Repeat("aBcDeF0123456789", 4)), "identity", "launch_nonce")
	extended := jsonSetAt(t, good, fmt.Sprintf(`{"fixture":%s,"nested":[1,null]}`, jsonQuoted(t, wireIdentityCanary)), "identity", "database_identity_json", "extension")
	bounds := jsonSetAt(t, good, "0", "identity", "database_identity_json", "device")
	bounds = jsonSetAt(t, bounds, "18446744073709551615", "identity", "database_identity_json", "inode")
	reverseBounds := jsonSetAt(t, good, "18446744073709551615", "identity", "database_identity_json", "device")
	reverseBounds = jsonSetAt(t, reverseBounds, "0", "identity", "database_identity_json", "inode")
	lexicalAbsolute := jsonSetAt(t, good, jsonQuoted(t, c.fixture.dir+"/absent-parent/../unallocated.sqlite"), "identity", "database_identity_json", "canonical_path")
	for _, value := range []struct {
		name  string
		value jsontext.Value
	}{
		{"genuine", good}, {"uppercase", uppercase}, {"mixed", mixed}, {"extended", extended},
		{"bounds", bounds}, {"reverse bounds", reverseBounds}, {"lexical absolute", lexicalAbsolute},
	} {
		decoded, err := decodeInputRequest(wireBytes(value.value))
		if err != nil {
			t.Fatalf("%s: %v", value.name, err)
		}
		assertJSONEqual(t, decoded.Identity.DatabaseIdentityJSON, jsonAt(t, value.value, "identity", "database_identity_json"), value.name)
		if nonce, _ := jsonString(jsonAt(t, value.value, "identity", "launch_nonce")); decoded.Identity.LaunchNonce != nonce {
			t.Fatalf("%s: launch nonce %q", value.name, decoded.Identity.LaunchNonce)
		}
		assertFrame(t, mustEncodeInputRequest(t, decoded), value.value)
	}
}

func badIdentityPaths() []string {
	return []string{
		`"` + wireIdentityCanary + `/relative.sqlite"`, `""`, `"."`, `"../database.sqlite"`,
		"null", "17", "true", "[]", "{}",
	}
}

func badIdentityHex(length int) []string {
	return []string{
		`""`,
		`"` + strings.Repeat("a", length-1) + `"`,
		`"` + strings.Repeat("a", length+1) + `"`,
		`"` + strings.Repeat("g", length) + `"`,
		`"` + wireIdentityCanary + strings.Repeat("g", length) + `"`,
		`"` + strings.Repeat("a", length-1) + ` "`,
		`"` + strings.Repeat("é", length/2) + `"`,
		"null", "17", "true", "[]", "{}",
	}
}

func badIdentityNumbers() []string {
	return []string{
		"-1", "-9223372036854775808", "1.0", "1.5", "1.8446744073709552e19",
		"null", `"` + wireIdentityCanary + `"`, "true", "[]", "{}",
	}
}

func TestRuntimeControl_Identity_IdentityDecodeAndEncodeRefuseEveryInvalidOwningIdentityPrivately(t *testing.T) {
	t.Parallel()
	c := newWireCase(t)
	good := identityControl(t, c)
	const launchNonce = ""
	invalidIdentities := []struct {
		name     string
		field    string
		absent   bool
		invalids []string
	}{
		{"a database identity without canonical_path", "canonical_path", true, nil},
		{"a database identity without instance_id", "instance_id", true, nil},
		{"a database identity without device", "device", true, nil},
		{"a database identity without inode", "inode", true, nil},
		{"a canonical_path that is not an absolute string", "canonical_path", false, badIdentityPaths()},
		{"an instance_id that is not 32 ASCII hex characters", "instance_id", false, badIdentityHex(32)},
		{"a device that is not an unsigned 64-bit integer", "device", false, badIdentityNumbers()},
		{"an inode that is not an unsigned 64-bit integer", "inode", false, badIdentityNumbers()},
		{"a launch nonce that is not 64 ASCII hex characters", launchNonce, false, badIdentityHex(64)},
	}
	databasePath := []string{"identity", "database_identity_json"}
	for _, row := range invalidIdentities {
		replacements := row.invalids
		if row.absent {
			replacements = []string{""}
		}
		for index, replacement := range replacements {
			label := fmt.Sprintf("%s: value %d", row.name, index)
			bad := good
			database := jsonAt(t, good, databasePath...)
			nonce, encodable := c.fixture.run.launchNonce, true
			switch {
			case row.field == launchNonce:
				nonce, encodable = jsonString(jsontext.Value(replacement))
				bad = jsonSetAt(t, bad, replacement, "identity", "launch_nonce")
			case row.absent:
				bad = jsonRemoveAt(t, bad, append(slices.Clone(databasePath), row.field)...)
				database = jsonRemoveAt(t, database, row.field)
			default:
				bad = jsonSetAt(t, bad, replacement, append(slices.Clone(databasePath), row.field)...)
				database = jsonSetAt(t, database, replacement, row.field)
			}
			_, err := decodeInputRequest(wireBytes(bad))
			if err == nil {
				t.Fatalf("owning identity schema rejects decode case: %s", label)
			}
			assertPrivateIdentityError(t, err, good, label)
			if !encodable {
				continue
			}
			request := wireDecoded(t, good)
			request.Identity.DatabaseIdentityJSON = database
			request.Identity.LaunchNonce = nonce
			_, err = encodeInputRequest(request)
			if err == nil {
				t.Fatalf("owning identity schema rejects constructed encode case: %s", label)
			}
			assertPrivateIdentityError(t, err, good, label)
		}
	}
}
