package core

import (
	"errors"
	"testing"
)

func TestError_UsageErrorMapsToExitCode2(t *testing.T) {
	t.Parallel()
	if got := usageError("bad flag").exitCode(); got != 2 {
		t.Fatalf("usage exit code = %d, want 2", got)
	}
}

func TestError_AppErrorMapsToExitCode1(t *testing.T) {
	t.Parallel()
	if got := appError("runtime conflict").exitCode(); got != 1 {
		t.Fatalf("app exit code = %d, want 1", got)
	}
}

func TestError_MessageReturnsTheCarriedString(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		failure *cafleetError
		want    string
	}{
		{name: "usage", failure: usageError("u"), want: "u"},
		{name: "app", failure: appError("a"), want: "a"},
		{name: "deadline exceeded", failure: deadlineExceededError(), want: "operation exceeded its deadline"},
		{name: "invalid stored value", failure: invalidStoredValueError("cause", "{}"), want: "invalid stored value for cause: {}"},
		{name: "diagnostics", failure: appError("primary").withDiagnostic("inner").withDiagnostic("outer"), want: "primary\ninner\nouter"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := tc.failure.message(); got != tc.want {
				t.Fatalf("%s: message = %q, want %q", tc.name, got, tc.want)
			}
			if got := tc.failure.Error(); got != tc.want {
				t.Fatalf("%s: Error = %q, want %q", tc.name, got, tc.want)
			}
		})
	}
}

func TestError_TransientStorageHoldsExactlyForBusyDeadlineAndIoCauses(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name      string
		failure   func() *cafleetError
		transient bool
	}{
		{name: "usage", failure: func() *cafleetError { return usageError("bad flag") }, transient: false},
		{name: "app", failure: func() *cafleetError { return appError("Input 7 not found") }, transient: false},
		{name: "database busy", failure: func() *cafleetError { return databaseBusyError("database error: database is locked") }, transient: true},
		{name: "deadline exceeded", failure: deadlineExceededError, transient: true},
		{name: "database io", failure: func() *cafleetError { return databaseIOError("database error: disk I/O error") }, transient: true},
		{name: "invalid stored value", failure: func() *cafleetError { return invalidStoredValueError("cause", "{}") }, transient: false},
		{name: "diagnostics over database busy", failure: func() *cafleetError { return databaseBusyError("busy").withDiagnostic("context") }, transient: true},
		{name: "diagnostics over deadline exceeded", failure: func() *cafleetError { return deadlineExceededError().withDiagnostic("context") }, transient: true},
		{name: "diagnostics over database io", failure: func() *cafleetError { return databaseIOError("io").withDiagnostic("context") }, transient: true},
		{name: "diagnostics over app", failure: func() *cafleetError { return appError("stale runtime owner").withDiagnostic("context") }, transient: false},
		{
			name: "nested diagnostics over database busy",
			failure: func() *cafleetError {
				return databaseBusyError("busy").withDiagnostic("inner").withDiagnostic("outer")
			},
			transient: true,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := tc.failure().transientStorage(); got != tc.transient {
				t.Fatalf("%s: transientStorage = %v, want %v", tc.name, got, tc.transient)
			}
		})
	}
}

func TestError_SqliteErrorCodesMapToTheirStorageClass(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name   string
		code   int
		mapped errorKind
	}{
		{name: "IOERR", code: 10, mapped: errorDatabaseIO},
		{name: "IOERR_WRITE", code: 778, mapped: errorDatabaseIO},
		{name: "FULL", code: 13, mapped: errorDatabaseIO},
		{name: "CANTOPEN", code: 14, mapped: errorDatabaseIO},
		{name: "NOMEM", code: 7, mapped: errorDatabaseIO},
		{name: "READONLY", code: 8, mapped: errorDatabaseIO},
		{name: "PROTOCOL", code: 15, mapped: errorDatabaseIO},
		{name: "BUSY", code: 5, mapped: errorDatabaseBusy},
		{name: "LOCKED", code: 6, mapped: errorDatabaseBusy},
		{name: "INTERRUPT", code: 9, mapped: errorDatabaseInterrupted},
		{name: "CONSTRAINT", code: 19, mapped: errorApp},
		{name: "CONSTRAINT_TRIGGER", code: 1811, mapped: errorApp},
		{name: "CORRUPT", code: 11, mapped: errorApp},
		{name: "NOTADB", code: 26, mapped: errorApp},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			mapped := sqliteFailure(tc.code, tc.name+" fixture")
			if got, want := mapped.message(), "database error: "+tc.name+" fixture"; got != want {
				t.Fatalf("%s: message = %q, want %q", tc.name, got, want)
			}
			if mapped.exitCode() != 1 {
				t.Fatalf("%s: exit code = %d, want 1", tc.name, mapped.exitCode())
			}
			if mapped.kind != tc.mapped {
				t.Fatalf("%s: kind = %v, want %v", tc.name, mapped.kind, tc.mapped)
			}
			transient := tc.mapped == errorDatabaseIO || tc.mapped == errorDatabaseBusy
			if mapped.transientStorage() != transient {
				t.Fatalf("%s: transientStorage = %v, want %v", tc.name, mapped.transientStorage(), transient)
			}
			if mapped.interrupted() != (tc.mapped == errorDatabaseInterrupted) {
				t.Fatalf("%s: interrupted = %v", tc.name, mapped.interrupted())
			}
		})
	}
}

func TestError_DatabaseIoBehavesLikeAppOutsideClassification(t *testing.T) {
	t.Parallel()
	io := databaseIOError("database error: disk I/O error")
	if io.exitCode() != 1 {
		t.Fatalf("exit code = %d, want 1", io.exitCode())
	}
	if io.message() != "database error: disk I/O error" || io.Error() != "database error: disk I/O error" {
		t.Fatalf("message = %q, Error = %q", io.message(), io.Error())
	}
	cleaned := io.withCleanup("cleanup failed")
	if cleaned.kind != errorApp || cleaned.message() != "database error: disk I/O error\ncleanup failed" {
		t.Fatalf("withCleanup = %v %q", cleaned.kind, cleaned.message())
	}
}

func TestError_WithCleanupKeepsThePrimaryExitCategory(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		primary *cafleetError
		kind    errorKind
	}{
		{name: "usage", primary: usageError("bad flag"), kind: errorUsage},
		{name: "diagnostics over usage", primary: usageError("bad flag").withDiagnostic("context"), kind: errorUsage},
		{name: "app", primary: appError("conflict"), kind: errorApp},
		{name: "database busy", primary: databaseBusyError("busy"), kind: errorApp},
		{name: "deadline exceeded", primary: deadlineExceededError(), kind: errorApp},
		{name: "diagnostics over app", primary: appError("conflict").withDiagnostic("context"), kind: errorApp},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			want := tc.primary.message() + "\ncleanup failed"
			cleaned := tc.primary.withCleanup("cleanup failed")
			if cleaned.kind != tc.kind || cleaned.message() != want {
				t.Fatalf("%s: withCleanup = %v %q, want %v %q", tc.name, cleaned.kind, cleaned.message(), tc.kind, want)
			}
		})
	}
}

func TestError_MergeCleanupKeepsThePrimaryFailureAndAppendsTheCleanupFailure(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name    string
		primary error
		cleanup error
		want    string
		kind    errorKind
	}{
		{name: "both succeed"},
		{name: "primary fails", primary: usageError("bad flag"), want: "bad flag", kind: errorUsage},
		{name: "cleanup fails", cleanup: appError("cleanup failed"), want: "cleanup failed", kind: errorApp},
		{name: "both fail", primary: usageError("bad flag"), cleanup: appError("cleanup failed"), want: "bad flag\ncleanup failed", kind: errorUsage},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			merged := mergeCleanup(tc.primary, tc.cleanup)
			if tc.want == "" {
				if merged != nil {
					t.Fatalf("%s: mergeCleanup = %v, want nil", tc.name, merged)
				}
				return
			}
			var typed *cafleetError
			if !errors.As(merged, &typed) {
				t.Fatalf("%s: mergeCleanup = %T, want *cafleetError", tc.name, merged)
			}
			if typed.kind != tc.kind || typed.message() != tc.want {
				t.Fatalf("%s: mergeCleanup = %v %q, want %v %q", tc.name, typed.kind, typed.message(), tc.kind, tc.want)
			}
		})
	}
}

func TestError_InterruptedLooksThroughDiagnostics(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name        string
		failure     *cafleetError
		interrupted bool
	}{
		{name: "interrupted", failure: databaseInterruptedError("database error: interrupted"), interrupted: true},
		{name: "diagnostics over interrupted", failure: databaseInterruptedError("interrupted").withDiagnostic("context"), interrupted: true},
		{name: "app", failure: appError("conflict"), interrupted: false},
		{name: "deadline exceeded", failure: deadlineExceededError(), interrupted: false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()
			if got := tc.failure.interrupted(); got != tc.interrupted {
				t.Fatalf("%s: interrupted = %v, want %v", tc.name, got, tc.interrupted)
			}
			if tc.failure.exitCode() != 1 {
				t.Fatalf("%s: exit code = %d, want 1", tc.name, tc.failure.exitCode())
			}
			if tc.interrupted && tc.failure.transientStorage() {
				t.Fatalf("%s: an interrupted statement is not a transient storage failure", tc.name)
			}
		})
	}
}
