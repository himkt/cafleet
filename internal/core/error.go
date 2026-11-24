package core

import (
	"errors"
	"fmt"
	"strings"
)

type errorKind uint8

const (
	errorUsage errorKind = iota + 1
	errorApp
	errorDatabaseBusy
	// An SQLite I/O-class failure; it behaves like app except under transientStorage.
	errorDatabaseIO
	// A statement that SQLite interrupted; it behaves like app until a deadline scope that has
	// passed turns it into deadlineExceeded.
	errorDatabaseInterrupted
	errorDeadlineExceeded
	errorWithDiagnostics
	errorInvalidStoredValue
)

type cafleetError struct {
	kind        errorKind
	text        string
	cause       *cafleetError
	diagnostics []string
	field       string
	value       string
}

func usageError(message string) *cafleetError {
	return &cafleetError{kind: errorUsage, text: message}
}

func usageErrorf(format string, args ...any) *cafleetError {
	return usageError(fmt.Sprintf(format, args...))
}

func appError(message string) *cafleetError {
	return &cafleetError{kind: errorApp, text: message}
}

func appErrorf(format string, args ...any) *cafleetError {
	return appError(fmt.Sprintf(format, args...))
}

func databaseBusyError(message string) *cafleetError {
	return &cafleetError{kind: errorDatabaseBusy, text: message}
}

func databaseIOError(message string) *cafleetError {
	return &cafleetError{kind: errorDatabaseIO, text: message}
}

func databaseInterruptedError(message string) *cafleetError {
	return &cafleetError{kind: errorDatabaseInterrupted, text: message}
}

func deadlineExceededError() *cafleetError {
	return &cafleetError{kind: errorDeadlineExceeded}
}

func invalidStoredValueError(field, value string) *cafleetError {
	return &cafleetError{kind: errorInvalidStoredValue, field: field, value: value}
}

// cafleetErrorOf returns the cafleet error carried by err. Every error that crosses a function
// boundary inside this package is a *cafleetError, so any other error is a bug.
func cafleetErrorOf(err error) *cafleetError {
	var cafleet *cafleetError
	if !errors.As(err, &cafleet) {
		panic(fmt.Sprintf("an error crossing a package boundary is not a cafleet error: %v", err))
	}
	return cafleet
}

func (e *cafleetError) withDiagnostic(diagnostic string) *cafleetError {
	return &cafleetError{kind: errorWithDiagnostics, cause: e, diagnostics: []string{diagnostic}}
}

// withCleanup appends a compensation diagnostic without changing the primary exit category.
func (e *cafleetError) withCleanup(diagnostic string) *cafleetError {
	message := e.message() + "\n" + diagnostic
	switch e.kind {
	case errorUsage:
		return usageError(message)
	case errorWithDiagnostics:
		if e.cause.exitCode() == 2 {
			return usageError(message)
		}
		return appError(message)
	case errorApp,
		errorInvalidStoredValue,
		errorDatabaseBusy,
		errorDatabaseIO,
		errorDatabaseInterrupted,
		errorDeadlineExceeded:
		return appError(message)
	default:
		panic("invalid errorKind")
	}
}

func (e *cafleetError) interrupted() bool {
	switch e.kind {
	case errorWithDiagnostics:
		return e.cause.interrupted()
	case errorDatabaseInterrupted:
		return true
	case errorUsage,
		errorApp,
		errorDatabaseBusy,
		errorDatabaseIO,
		errorDeadlineExceeded,
		errorInvalidStoredValue:
		return false
	default:
		panic("invalid errorKind")
	}
}

// transientStorage reports whether a storage attempt failing with this cause may succeed when
// retried unchanged.
func (e *cafleetError) transientStorage() bool {
	switch e.kind {
	case errorWithDiagnostics:
		return e.cause.transientStorage()
	case errorDatabaseBusy, errorDeadlineExceeded, errorDatabaseIO:
		return true
	case errorUsage, errorApp, errorDatabaseInterrupted, errorInvalidStoredValue:
		return false
	default:
		panic("invalid errorKind")
	}
}

// mergeCleanup keeps the primary failure and appends the cleanup failure when both fail.
func mergeCleanup(primary error, cleanup error) error {
	switch {
	case primary != nil && cleanup != nil:
		return cafleetErrorOf(primary).withCleanup(cafleetErrorOf(cleanup).message())
	case primary != nil:
		return primary
	default:
		return cleanup
	}
}

func (e *cafleetError) exitCode() int {
	switch e.kind {
	case errorWithDiagnostics:
		return e.cause.exitCode()
	case errorUsage:
		return 2
	case errorApp,
		errorDatabaseBusy,
		errorDatabaseIO,
		errorDatabaseInterrupted,
		errorDeadlineExceeded,
		errorInvalidStoredValue:
		return 1
	default:
		panic("invalid errorKind")
	}
}

func (e *cafleetError) message() string {
	switch e.kind {
	case errorWithDiagnostics:
		return e.cause.message() + "\n" + strings.Join(e.diagnostics, "\n")
	case errorUsage,
		errorApp,
		errorDatabaseBusy,
		errorDatabaseIO,
		errorDatabaseInterrupted:
		return e.text
	case errorDeadlineExceeded:
		return "operation exceeded its deadline"
	case errorInvalidStoredValue:
		return "invalid stored value for " + e.field + ": " + e.value
	default:
		panic("invalid errorKind")
	}
}

func (e *cafleetError) Error() string {
	return e.message()
}

// SQLite primary result codes (the low byte of an extended code).
const (
	sqliteNomem     = 7
	sqliteReadonly  = 8
	sqliteInterrupt = 9
	sqliteIoerr     = 10
	sqliteBusy      = 5
	sqliteLocked    = 6
	sqliteFull      = 13
	sqliteCantopen  = 14
	sqliteProtocol  = 15
)

// sqliteFailure classifies an SQLite failure by its (primary or extended) result code;
// message is SQLite's own message text with no driver prefix.
func sqliteFailure(code int, message string) *cafleetError {
	text := "database error: " + message
	switch code & 0xff {
	case sqliteBusy, sqliteLocked:
		return databaseBusyError(text)
	case sqliteIoerr, sqliteFull, sqliteCantopen, sqliteNomem, sqliteReadonly, sqliteProtocol:
		return databaseIOError(text)
	case sqliteInterrupt:
		return databaseInterruptedError(text)
	default:
		return appError(text)
	}
}
