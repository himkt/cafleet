package core

import (
	"cmp"
	"encoding/json/jsontext"
	"fmt"
	"math"
	"slices"
	"strconv"
	"strings"
)

const herdrPaneNotFound = "pane_not_found"

func herdrIsPaneNotFound(stderr string) bool {
	code, found := herdrErrorCode(stderr)
	return found && code == herdrPaneNotFound
}

// herdrErrorCode is the herdr error envelope's error.code, when the stderr carries one.
func herdrErrorCode(stderr string) (string, bool) {
	payload, parsed := jsonParse(stderr)
	if !parsed {
		return "", false
	}
	return jsonString(jsonIndex(jsonIndex(payload, "error"), "code"))
}

func herdrParseEnvelope(out string) (jsontext.Value, *multiplexerError) {
	payload, parsed := jsonParse(out)
	if !parsed {
		return nil, newMultiplexerError("herdr returned non-JSON output: " + quoted(out))
	}
	result := jsonIndex(payload, "result")
	if !jsonIsObject(result) {
		return nil, newMultiplexerError("herdr output missing 'result' object: " + quoted(out))
	}
	return result, nil
}

func herdrStrField(value jsontext.Value, context, key string) (string, *multiplexerError) {
	text, isText := jsonString(jsonIndex(value, key))
	if !isText {
		return "", newMultiplexerErrorf("herdr %s missing '%s' field", context, key)
	}
	return text, nil
}

func herdrF64Field(value jsontext.Value, context, key string) (float64, *multiplexerError) {
	number, isNumber := jsonFloat64(jsonIndex(value, key))
	if !isNumber {
		return 0, newMultiplexerErrorf("herdr %s missing '%s' field", context, key)
	}
	return number, nil
}

func round4(value float64) float64 {
	return math.Round(value*10_000) / 10_000
}

// formatF64 renders a float as Rust's f64 Display does: the shortest round-trip digits, never
// in scientific notation.
func formatF64(value float64) string {
	return strconv.FormatFloat(value, 'f', -1, 64)
}

func shlexQuote(token string) string {
	safe := token != "" && !strings.ContainsFunc(token, func(r rune) bool {
		alphanumeric := (r >= 'a' && r <= 'z') || (r >= 'A' && r <= 'Z') || (r >= '0' && r <= '9')
		return !alphanumeric && !strings.ContainsRune("_@%+=:,./-", r)
	})
	if safe {
		return token
	}
	return "'" + strings.ReplaceAll(token, "'", `'"'"'`) + "'"
}

func shlexJoin(command []string) string {
	quotedTokens := make([]string, len(command))
	for i, token := range command {
		quotedTokens[i] = shlexQuote(token)
	}
	return strings.Join(quotedTokens, " ")
}

func herdrPanesOf(result jsontext.Value) ([]jsontext.Value, *multiplexerError) {
	panes, listed := jsonArray(jsonIndex(result, "panes"))
	if !listed {
		return nil, newMultiplexerError("herdr pane list missing 'panes' field")
	}
	return panes, nil
}

// compareOptionalF64 orders optional floats as Rust's Option<f64> does: an absent value first.
func compareOptionalF64(left, right jsontext.Value) int {
	leftValue, leftSet := jsonFloat64(left)
	rightValue, rightSet := jsonFloat64(right)
	switch {
	case leftSet && rightSet:
		return cmp.Compare(leftValue, rightValue)
	case rightSet:
		return -1
	case leftSet:
		return 1
	default:
		return 0
	}
}

// herdrSortByY stably orders entries by rect.y (F13).
func herdrSortByY(entries []jsontext.Value) {
	slices.SortStableFunc(entries, func(a, b jsontext.Value) int {
		return compareOptionalF64(jsonIndex(jsonIndex(a, "rect"), "y"), jsonIndex(jsonIndex(b, "rect"), "y"))
	})
}

type herdrMultiplexer struct {
	backend *backendCore
	// closedTab is the tab the last closePane recorded, which restoreAfterClose rebalances.
	closedTab opt[string]
}

func herdrMultiplexerForServer(runner commandRunner, session string) *herdrMultiplexer {
	return &herdrMultiplexer{backend: &backendCore{
		runner:     runner,
		name:       "herdr",
		paneGone:   herdrIsPaneNotFound,
		serverFlag: "--session",
		server:     session,
	}}
}

func (m *herdrMultiplexer) runJSON(argv []string) (jsontext.Value, *multiplexerError) {
	out, err := m.backend.run(argv)
	if err != nil {
		return nil, err
	}
	return herdrParseEnvelope(out)
}

// paneRead is the result of a pane read; false when herdr reports pane_not_found.
func (m *herdrMultiplexer) paneRead(parts ...string) (jsontext.Value, bool, *multiplexerError) {
	argv := m.backend.argv(parts...)
	out, err := m.backend.runner.run(argv)
	switch {
	case err != nil && err.kind == runErrorFailed && herdrIsPaneNotFound(err.detail):
		return nil, false, nil
	case err != nil:
		return nil, false, m.backend.mapError(argv, err)
	}
	result, parseErr := herdrParseEnvelope(out)
	if parseErr != nil {
		return nil, false, parseErr
	}
	return result, true, nil
}

// paneGeometry is pane's rect.width from the api snapshot layouts and its scroll.viewport_rows
// from the snapshot's panes, joined by pane ID.
func (m *herdrMultiplexer) paneGeometry(pane string) (uint16, uint16, *multiplexerError) {
	result, err := m.runJSON(m.backend.argv("herdr", "api", "snapshot"))
	if err != nil {
		return 0, 0, err
	}
	snapshot := jsonIndex(result, "snapshot")
	entries := func(field string) ([]jsontext.Value, *multiplexerError) {
		listed, isArray := jsonArray(jsonIndex(snapshot, field))
		if !isArray {
			return nil, newMultiplexerErrorf("herdr api snapshot missing '%s' field", field)
		}
		return listed, nil
	}
	absent := func() *multiplexerError { return newMultiplexerErrorf("herdr api snapshot has no pane %s", pane) }
	isPane := func(entry jsontext.Value) bool {
		id, isText := jsonString(jsonIndex(entry, "pane_id"))
		return isText && id == pane
	}
	layouts, err := entries("layouts")
	if err != nil {
		return 0, 0, err
	}
	var layoutEntry opt[int]
	var layoutPanes []jsontext.Value
	for _, layout := range layouts {
		panes, isArray := jsonArray(jsonIndex(layout, "panes"))
		if !isArray {
			continue
		}
		if index := slices.IndexFunc(panes, isPane); index >= 0 {
			layoutPanes, layoutEntry = panes, some(index)
			break
		}
	}
	layoutIndex, found := layoutEntry.get()
	if !found {
		return 0, 0, absent()
	}
	panes, err := entries("panes")
	if err != nil {
		return 0, 0, err
	}
	paneIndex := slices.IndexFunc(panes, isPane)
	if paneIndex < 0 {
		return 0, 0, absent()
	}
	dimension := func(object jsontext.Value, field string) (uint16, *multiplexerError) {
		value, err := herdrF64Field(object, "api snapshot", field)
		if err != nil {
			return 0, err
		}
		if value != math.Trunc(value) || value < 1 || value > math.MaxUint16 {
			return 0, newMultiplexerErrorf("herdr api snapshot %s %s of %s", field, formatF64(value), pane)
		}
		return uint16(value), nil
	}
	cols, err := dimension(jsonIndex(layoutPanes[layoutIndex], "rect"), "width")
	if err != nil {
		return 0, 0, err
	}
	rows, err := dimension(jsonIndex(panes[paneIndex], "scroll"), "viewport_rows")
	if err != nil {
		return 0, 0, err
	}
	return cols, rows, nil
}

func (m *herdrMultiplexer) listPanes() ([]jsontext.Value, *multiplexerError) {
	result, err := m.runJSON(m.backend.argv("herdr", "pane", "list"))
	if err != nil {
		return nil, err
	}
	return herdrPanesOf(result)
}

func (m *herdrMultiplexer) core() *backendCore {
	return m.backend
}

func (m *herdrMultiplexer) bounded(d deadline) multiplexer {
	return &herdrMultiplexer{backend: m.backend.bounded(d)}
}

func (m *herdrMultiplexer) name() string {
	return m.backend.name
}

func (m *herdrMultiplexer) ensureAvailable() *multiplexerError {
	return m.backend.ensureAvailable()
}

func (m *herdrMultiplexer) checkKeyTarget(target string, p placement, d deadline) *multiplexerError {
	out, err := m.backend.pressRun(m.backend.argv("herdr", "pane", "list"), d, false)
	if err != nil {
		return err
	}
	result, err := herdrParseEnvelope(out)
	if err != nil {
		return err
	}
	panes, err := herdrPanesOf(result)
	if err != nil {
		return err
	}
	matches := 0
	for _, pane := range panes {
		paneID, err := herdrStrField(pane, "pane list", "pane_id")
		if err != nil {
			return err
		}
		workspace, err := herdrStrField(pane, "pane list", "workspace_id")
		if err != nil {
			return err
		}
		tab, err := herdrStrField(pane, "pane list", "tab_id")
		if err != nil {
			return err
		}
		if paneID == target {
			if workspace != p.muxSession || tab != p.concreteWindowID() {
				return newMultiplexerError(placementMismatch)
			}
			matches++
		}
	}
	if matches != 1 {
		return newMultiplexerError(placementMismatch)
	}
	return nil
}

func (m *herdrMultiplexer) sendKeys(pane string, keys pressKeys, d deadline) *multiplexerError {
	return m.backend.sendKeys(append(m.backend.argv("herdr", "pane", "send-keys", pane), keys.herdrArguments()...), d)
}

func (m *herdrMultiplexer) paneContext(pane string) (opt[multiplexerContext], *multiplexerError) {
	result, found, err := m.paneRead("herdr", "pane", "current", "--pane", pane)
	if err != nil || !found {
		return none[multiplexerContext](), err
	}
	context, err := herdrCurrentContext(result)
	if err != nil {
		return none[multiplexerContext](), err
	}
	return some(context), nil
}

func (m *herdrMultiplexer) paneProcess(pane string) (opt[int64], *multiplexerError) {
	result, found, err := m.paneRead("herdr", "pane", "process-info", "--pane", pane)
	if err != nil || !found {
		return none[int64](), err
	}
	pid, isInteger := jsonInt64(jsonIndex(jsonIndex(result, "process_info"), "shell_pid"))
	if !isInteger {
		return none[int64](), newMultiplexerError("herdr pane process-info missing 'shell_pid' field")
	}
	return some(pid), nil
}

func (m *herdrMultiplexer) renameWindow(windowID, name string) *multiplexerError {
	_, err := m.runJSON(m.backend.argv("herdr", "tab", "rename", windowID, name))
	return err
}

func (m *herdrMultiplexer) windowName(windowID string) (string, *multiplexerError) {
	result, err := m.runJSON(m.backend.argv("herdr", "tab", "get", windowID))
	if err != nil {
		return "", err
	}
	tab := jsonIndex(result, "tab")
	tabID, err := herdrStrField(tab, "tab get", "tab_id")
	if err != nil {
		return "", err
	}
	if tabID != windowID {
		return "", newMultiplexerErrorf("herdr tab get returned tab %s, expected %s", quoted(tabID), quoted(windowID))
	}
	return herdrStrField(tab, "tab get", "label")
}

func (m *herdrMultiplexer) windowHeight(_ string, anchors []string) (int64, *multiplexerError) {
	if len(anchors) == 0 {
		panic("a window height read names an anchor")
	}
	var lastErr *multiplexerError
	for _, anchor := range anchors {
		result, err := m.runJSON(m.backend.argv("herdr", "pane", "layout", "--pane", anchor))
		if err == nil {
			height, isInteger := jsonInt64(jsonIndex(jsonIndex(jsonIndex(result, "layout"), "area"), "height"))
			if isInteger {
				return height, nil
			}
			err = newMultiplexerError("herdr pane layout missing 'area' field")
		}
		lastErr = err
	}
	return 0, lastErr
}

func (m *herdrMultiplexer) spawnMemberPane(target paneTarget, paneName, cwd string, command []string) (spawnedPane, *multiplexerError) {
	var spawned spawnedPane
	var reparenting herdrReparenting
	switch target := target.(type) {
	case columnTarget:
		split, moves, err := m.splitInto(target, cwd)
		if err != nil {
			return spawnedPane{}, err
		}
		spawned, reparenting = split, moves
	case paneTargetNewWindow:
		created, err := m.createTab(target.session, target.windowName, cwd)
		if err != nil {
			return spawnedPane{}, err
		}
		spawned = created
	default:
		panic("invalid paneTarget")
	}
	owned := newPaneOwnership(spawned.paneID, func(id string) *multiplexerError {
		return killPane(m, id, spawned.windowID)
	})
	if _, err := m.backend.run(m.backend.argv("herdr", "pane", "run", spawned.paneID, shlexJoin(command))); err != nil {
		return spawnedPane{}, err.withPaneCleanup(owned.rollback())
	}
	owned.finish()
	// Naming, re-parenting and equalization are best effort once the command runs.
	_, _ = m.backend.run(m.backend.argv("herdr", "pane", "rename", spawned.paneID, paneName))
	if reparenting != nil {
		if err := m.reparent(reparenting, spawned); err != nil {
			reportLayoutReparenting(err)
		}
	}
	_ = m.equalizeTab(spawned.paneID)
	return spawned, nil
}

// capturePane reads the raw terminal buffer (no envelope); captureWindow enforces the last-N
// window client-side, since the daemon may return more rows than requested.
func (m *herdrMultiplexer) capturePane(pane string, lines int64) (string, *multiplexerError) {
	if err := checkCaptureLines(lines); err != nil {
		return "", err
	}
	fetchDepth := lines + captureOverFetchLines
	raw, err := m.backend.run(m.backend.argv("herdr", "pane", "read", pane, "--format", "ansi", "--source", "recent-unwrapped", "--lines", strconv.FormatInt(fetchDepth, 10)))
	if err != nil {
		return "", err
	}
	return captureWindow(raw, lines), nil
}

// captureScreen reads the visible screen; herdr exposes neither the cursor nor the screen
// mode. The visible read keeps leading and interior blank rows at their positions and drops
// only trailing blank rows, so its lines fill the screen from the top.
func (m *herdrMultiplexer) captureScreen(pane string) (screenCapture, *multiplexerError) {
	cols, rows, err := m.paneGeometry(pane)
	if err != nil {
		return screenCapture{}, err
	}
	body, err := m.backend.run(m.backend.argv("herdr", "pane", "read", pane, "--source", "visible", "--format", "ansi"))
	if err != nil {
		return screenCapture{}, err
	}
	printed := strings.Count(strings.TrimSuffix(body, "\n"), "\n") + 1
	if printed > int(rows) {
		return screenCapture{}, newMultiplexerErrorf("herdr pane read of %s printed %d rows for a %d-row viewport", pane, printed, rows)
	}
	return screenCapture{rows: rows, cols: cols, body: body}, nil
}

func (m *herdrMultiplexer) typeLiteralUntil(pane, literal string, _ func() bool) (bool, *multiplexerError) {
	if err := m.backend.acknowledgedRun(m.backend.argv("herdr", "pane", "send-text", pane, literal), "text"); err != nil {
		return false, err
	}
	return true, nil
}

// hexBytes renders bytes as Rust's `{:02x?}` does for a byte slice.
func hexBytes(data []byte) string {
	parts := make([]string, len(data))
	for i, b := range data {
		parts[i] = fmt.Sprintf("%02x", b)
	}
	return "[" + strings.Join(parts, ", ") + "]"
}

func (m *herdrMultiplexer) pressSubmit(pane string, submit []byte) *multiplexerError {
	keys, mapped := pressKeysFromBytes(submit)
	if !mapped {
		return newMultiplexerError("herdr has no key mapping for the submit bytes " + hexBytes(submit))
	}
	return m.backend.acknowledgedRun(append(m.backend.argv("herdr", "pane", "send-keys", pane), keys.herdrArguments()...), "submit key")
}

func (m *herdrMultiplexer) closePane(pane, windowID string) *multiplexerError {
	if err := m.backend.runToleratingMissing(m.backend.argv("herdr", "pane", "close", pane)); err != nil {
		return err
	}
	m.closedTab = some(windowID)
	return nil
}

func (m *herdrMultiplexer) restoreAfterClose() *multiplexerError {
	tabID, closed := m.closedTab.get()
	m.closedTab = none[string]()
	if !closed {
		return nil
	}
	return m.resizeAfterClose(tabID)
}

// herdrReparenting is the pane moves that rebuild a full-height column after its first pane
// was split rightward.
//
//sumtype:decl
type herdrReparenting interface {
	isHerdrReparenting()
}

// herdrReparentingRight moves the left chain L1..Lk (k >= 2) back under L1 beside the new
// right pane.
type herdrReparentingRight struct {
	left []string
}

// herdrReparentingLeft moves the right chain R1..Rm (m >= 1) to the right of the new left pane.
type herdrReparentingLeft struct {
	right []string
}

func (herdrReparentingRight) isHerdrReparenting() {}
func (herdrReparentingLeft) isHerdrReparenting()  {}

func (m *herdrMultiplexer) splitPane(paneID, direction, cwd string) (string, *multiplexerError) {
	out, err := m.backend.run(m.backend.argv("herdr", "pane", "split", paneID, "--direction", direction, "--no-focus", "--cwd", cwd))
	var result jsontext.Value
	if err == nil {
		result, err = herdrParseEnvelope(out)
	}
	if err != nil {
		return "", err.withPaneCleanup(paneCleanupUnknown{})
	}
	id, err := herdrStrField(jsonIndex(result, "pane"), "pane split", "pane_id")
	if err != nil {
		return "", err.withPaneCleanup(paneCleanupUnknown{})
	}
	if strings.TrimSpace(id) == "" {
		return "", newMultiplexerError("herdr pane split returned an empty pane ID").withPaneCleanup(paneCleanupUnknown{})
	}
	return id, nil
}

// splitInto splits the live column order read from the anchor's tab: the column's bottom pane
// downward, or the opposite column's top pane rightward for an empty column.
func (m *herdrMultiplexer) splitInto(column columnTarget, cwd string) (spawnedPane, herdrReparenting, *multiplexerError) {
	anchors := append(slices.Clone(column.columnPanes), column.otherPanes...)
	if len(anchors) == 0 {
		panic("a column target names a pane of its window")
	}
	anchor := anchors[0]
	panes, _, err := m.readTabLayout(anchor)
	if err != nil {
		return spawnedPane{}, nil, err
	}
	columns, err := herdrColumnsOf(panes)
	if err != nil {
		return spawnedPane{}, nil, err
	}
	index := slices.IndexFunc(columns, func(chain []string) bool { return slices.Contains(chain, anchor) })
	if index < 0 {
		return spawnedPane{}, nil, newMultiplexerErrorf("herdr pane layout of %s does not contain %s", anchor, anchor)
	}
	chain := columns[index]
	var splitTarget, direction string
	var reparenting herdrReparenting
	switch {
	case len(column.columnPanes) > 0:
		splitTarget, direction = chain[len(chain)-1], "down"
	default:
		switch column.side {
		case columnSideRight:
			if len(chain) > 1 {
				reparenting = herdrReparentingRight{left: chain}
			}
		case columnSideLeft:
			reparenting = herdrReparentingLeft{right: chain}
		default:
			panic("invalid columnSide")
		}
		splitTarget, direction = chain[0], "right"
	}
	paneID, err := m.splitPane(splitTarget, direction, cwd)
	if err != nil {
		return spawnedPane{}, nil, err
	}
	return spawnedPane{paneID: paneID, windowID: column.windowID}, reparenting, nil
}

func (m *herdrMultiplexer) createTab(session, windowName, cwd string) (spawnedPane, *multiplexerError) {
	result, err := m.runJSON(m.backend.argv("herdr", "tab", "create", "--workspace", session, "--cwd", cwd, "--label", windowName, "--no-focus"))
	if err != nil {
		return spawnedPane{}, err.withPaneCleanup(paneCleanupUnknown{})
	}
	windowID, err := herdrStrField(jsonIndex(result, "tab"), "tab create", "tab_id")
	if err != nil {
		return spawnedPane{}, err.withPaneCleanup(paneCleanupUnknown{})
	}
	paneID, err := herdrStrField(jsonIndex(result, "root_pane"), "tab create", "pane_id")
	if err != nil {
		return spawnedPane{}, err.withPaneCleanup(paneCleanupUnknown{})
	}
	return spawnedPane{paneID: paneID, windowID: windowID}, nil
}

func (m *herdrMultiplexer) reparent(reparenting herdrReparenting, spawned spawnedPane) *multiplexerError {
	tab := spawned.windowID
	switch moves := reparenting.(type) {
	case herdrReparentingRight:
		left := moves.left
		temporary, err := m.moveToNewTab(left[1])
		if err != nil {
			return err
		}
		for i := 2; i < len(left); i++ {
			if err := m.moveInto(left[i], temporary, "down", left[i-1]); err != nil {
				return err
			}
		}
		for i := 1; i < len(left); i++ {
			if err := m.moveInto(left[i], tab, "down", left[i-1]); err != nil {
				return err
			}
		}
	case herdrReparentingLeft:
		right := moves.right
		temporary, err := m.moveToNewTab(right[0])
		if err != nil {
			return err
		}
		for i := 1; i < len(right); i++ {
			if err := m.moveInto(right[i], temporary, "down", right[i-1]); err != nil {
				return err
			}
		}
		if err := m.moveInto(right[0], tab, "right", spawned.paneID); err != nil {
			return err
		}
		for i := 1; i < len(right); i++ {
			if err := m.moveInto(right[i], tab, "down", right[i-1]); err != nil {
				return err
			}
		}
	default:
		panic("invalid herdrReparenting")
	}
	return nil
}

// moved is the move_result of a pane move that actually moved pane.
func (m *herdrMultiplexer) moved(pane string, parts ...string) (jsontext.Value, *multiplexerError) {
	result, err := m.runJSON(m.backend.argv(parts...))
	if err != nil {
		return nil, err
	}
	moveResult := jsonIndex(result, "move_result")
	if changed, isBool := jsonBool(jsonIndex(moveResult, "changed")); !isBool || !changed {
		return nil, newMultiplexerErrorf("herdr pane move of %s changed nothing: %s", pane, jsonText(result))
	}
	return moveResult, nil
}

// moveToNewTab moves pane into a new unfocused tab and returns that temporary tab's ID.
func (m *herdrMultiplexer) moveToNewTab(pane string) (string, *multiplexerError) {
	moveResult, err := m.moved(pane, "herdr", "pane", "move", pane, "--new-tab", "--no-focus")
	if err != nil {
		return "", err
	}
	return herdrStrField(jsonIndex(moveResult, "created_tab"), "pane move", "tab_id")
}

func (m *herdrMultiplexer) moveInto(pane, tab, direction, target string) *multiplexerError {
	_, err := m.moved(pane, "herdr", "pane", "move", pane, "--tab", tab, "--split", direction, "--target-pane", target, "--no-focus")
	return err
}

func (m *herdrMultiplexer) readTabLayout(anchorPaneID string) ([]jsontext.Value, []jsontext.Value, *multiplexerError) {
	result, err := m.runJSON(m.backend.argv("herdr", "pane", "layout", "--pane", anchorPaneID))
	if err != nil {
		return nil, nil, err
	}
	layout := jsonIndex(result, "layout")
	panes, isArray := jsonArray(jsonIndex(layout, "panes"))
	if !isArray {
		return nil, nil, newMultiplexerError("herdr pane layout missing 'panes' field")
	}
	splits, isArray := jsonArray(jsonIndex(layout, "splits"))
	if !isArray {
		return nil, nil, newMultiplexerError("herdr pane layout missing 'splits' field")
	}
	return panes, splits, nil
}

// equalizeTab equalizes every column of the anchor's tab.
func (m *herdrMultiplexer) equalizeTab(anchorPaneID string) *multiplexerError {
	panes, splits, err := m.readTabLayout(anchorPaneID)
	if err != nil {
		return err
	}
	return m.equalizeColumns(panes, splits)
}

func (m *herdrMultiplexer) equalizeColumns(panes, splits []jsontext.Value) *multiplexerError {
	columns, err := herdrGroupColumns(panes)
	if err != nil {
		return err
	}
	for _, column := range columns {
		if len(column) >= 2 {
			if err := m.equalizeColumn(column, splits); err != nil {
				return err
			}
		}
	}
	return nil
}

// equalizeColumn drives a column of N stacked panes to equal heights. The column is a
// right-leaning chain of `down` splits at the column's x and width: split_k separates pane_k
// from the subtree below it, so equal heights mean split_k has ratio 1/(N-k). `resize
// --amount` is a signed delta on a split's ratio, so each split takes one arithmetic resize.
func (m *herdrMultiplexer) equalizeColumn(column, splits []jsontext.Value) *multiplexerError {
	n := len(column)
	columnX, err := herdrF64Field(jsonIndex(column[0], "rect"), "pane layout", "x")
	if err != nil {
		return err
	}
	columnWidth, err := herdrF64Field(jsonIndex(column[0], "rect"), "pane layout", "width")
	if err != nil {
		return err
	}
	var downSplits []jsontext.Value
	for _, split := range splits {
		if direction, isText := jsonString(jsonIndex(split, "direction")); !isText || direction != "down" {
			continue
		}
		x, err := herdrF64Field(jsonIndex(split, "rect"), "pane layout", "x")
		if err != nil {
			return err
		}
		if x != columnX {
			continue
		}
		width, err := herdrF64Field(jsonIndex(split, "rect"), "pane layout", "width")
		if err != nil {
			return err
		}
		if width == columnWidth {
			downSplits = append(downSplits, split)
		}
	}
	herdrSortByY(downSplits)
	if len(downSplits) != n-1 {
		return nil
	}
	for i, split := range downSplits {
		ratio, err := herdrF64Field(split, "pane layout", "ratio")
		if err != nil {
			return err
		}
		delta := round4(1/float64(n-i) - ratio)
		if math.Abs(delta) < 1e-3 {
			continue
		}
		pane, direction, amount := column[i], "down", delta
		if delta < 0 {
			pane, direction, amount = column[i+1], "up", -delta
		}
		paneID, err := herdrStrField(pane, "pane layout", "pane_id")
		if err != nil {
			return err
		}
		if _, err := m.backend.run(m.backend.argv("herdr", "pane", "resize", "--pane", paneID, "--direction", direction, "--amount", formatF64(amount))); err != nil {
			return err
		}
	}
	return nil
}

func (m *herdrMultiplexer) resizeAfterClose(tabID string) *multiplexerError {
	anchor, found, err := m.survivingPaneInTab(tabID)
	if err != nil || !found {
		return err
	}
	panes, splits, err := m.readTabLayout(anchor)
	if err != nil {
		return err
	}
	if len(panes) == 1 {
		return m.restoreLonePaneFullWidth(panes[0], splits)
	}
	return m.equalizeColumns(panes, splits)
}

func (m *herdrMultiplexer) survivingPaneInTab(tabID string) (string, bool, *multiplexerError) {
	panes, err := m.listPanes()
	if err != nil {
		return "", false, err
	}
	for _, pane := range panes {
		if tab, isText := jsonString(jsonIndex(pane, "tab_id")); isText && tab == tabID {
			paneID, err := herdrStrField(pane, "pane list", "pane_id")
			return paneID, err == nil, err
		}
	}
	return "", false, nil
}

// restoreLonePaneFullWidth resizes away exactly one residual `right` split; any other residue
// is left alone.
func (m *herdrMultiplexer) restoreLonePaneFullWidth(pane jsontext.Value, splits []jsontext.Value) *multiplexerError {
	if len(splits) != 1 {
		return nil
	}
	if direction, isText := jsonString(jsonIndex(splits[0], "direction")); !isText || direction != "right" {
		return nil
	}
	ratio, err := herdrF64Field(splits[0], "pane layout", "ratio")
	if err != nil {
		return err
	}
	delta := round4(1 - ratio)
	if delta < 1e-3 {
		return nil
	}
	paneID, err := herdrStrField(pane, "pane layout", "pane_id")
	if err != nil {
		return err
	}
	_, err = m.backend.run(m.backend.argv("herdr", "pane", "resize", "--pane", paneID, "--direction", "right", "--amount", formatF64(delta)))
	return err
}

func herdrCurrentContext(result jsontext.Value) (multiplexerContext, *multiplexerError) {
	pane := jsonIndex(result, "pane")
	session, err := herdrStrField(pane, "pane current", "workspace_id")
	if err != nil {
		return multiplexerContext{}, err
	}
	windowID, err := herdrStrField(pane, "pane current", "tab_id")
	if err != nil {
		return multiplexerContext{}, err
	}
	paneID, err := herdrStrField(pane, "pane current", "pane_id")
	if err != nil {
		return multiplexerContext{}, err
	}
	return multiplexerContext{session: session, windowID: windowID, paneID: paneID}, nil
}

// herdrGroupColumns is the layout's columns (panes sharing rect.x) left to right, each sorted
// top to bottom.
func herdrGroupColumns(panes []jsontext.Value) ([][]jsontext.Value, *multiplexerError) {
	type column struct {
		x     float64
		panes []jsontext.Value
	}
	var columns []column
	for _, pane := range panes {
		x, err := herdrF64Field(jsonIndex(pane, "rect"), "pane layout", "x")
		if err != nil {
			return nil, err
		}
		index := slices.IndexFunc(columns, func(c column) bool { return c.x == x })
		if index < 0 {
			columns = append(columns, column{x: x, panes: []jsontext.Value{pane}})
		} else {
			columns[index].panes = append(columns[index].panes, pane)
		}
	}
	slices.SortStableFunc(columns, func(a, b column) int { return cmp.Compare(a.x, b.x) })
	grouped := make([][]jsontext.Value, len(columns))
	for i, c := range columns {
		herdrSortByY(c.panes)
		grouped[i] = c.panes
	}
	return grouped, nil
}

// herdrColumnsOf is herdrGroupColumns as pane IDs.
func herdrColumnsOf(panes []jsontext.Value) ([][]string, *multiplexerError) {
	columns, err := herdrGroupColumns(panes)
	if err != nil {
		return nil, err
	}
	ids := make([][]string, len(columns))
	for i, column := range columns {
		for _, pane := range column {
			paneID, err := herdrStrField(pane, "pane layout", "pane_id")
			if err != nil {
				return nil, err
			}
			ids[i] = append(ids[i], paneID)
		}
	}
	return ids, nil
}
