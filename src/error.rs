use std::borrow::Cow;
use std::fmt;

pub(crate) type Result<T> = std::result::Result<T, CafleetError>;

#[derive(Debug, Clone)]
pub enum CafleetError {
    Usage(String),
    App(String),
    DatabaseBusy(String),
    /// An SQLite I/O-class failure; it behaves like `App` except under `transient_storage`.
    DatabaseIo(String),
    DeadlineExceeded,
    WithDiagnostics {
        cause: Box<CafleetError>,
        diagnostics: Vec<String>,
    },
    InvalidStoredValue {
        field: String,
        value: String,
    },
}

impl CafleetError {
    pub(crate) fn with_diagnostic(self, diagnostic: impl Into<String>) -> Self {
        Self::WithDiagnostics {
            cause: Box::new(self),
            diagnostics: vec![diagnostic.into()],
        }
    }
    /// Append a compensation diagnostic without changing the primary exit category.
    pub(crate) fn with_cleanup(self, diagnostic: impl std::fmt::Display) -> Self {
        let message = format!("{self}\n{diagnostic}");
        match self {
            Self::Usage(_) => Self::Usage(message),
            Self::WithDiagnostics { cause, .. } if cause.exit_code() == 2 => Self::Usage(message),
            Self::App(_)
            | Self::InvalidStoredValue { .. }
            | Self::DatabaseBusy(_)
            | Self::DatabaseIo(_)
            | Self::DeadlineExceeded
            | Self::WithDiagnostics { .. } => Self::App(message),
        }
    }

    /// Whether a storage attempt failing with this cause may succeed when retried unchanged.
    pub(crate) fn transient_storage(&self) -> bool {
        match self {
            Self::WithDiagnostics { cause, .. } => cause.transient_storage(),
            Self::DatabaseBusy(_) | Self::DeadlineExceeded | Self::DatabaseIo(_) => true,
            Self::Usage(_) | Self::App(_) | Self::InvalidStoredValue { .. } => false,
        }
    }

    /// Keep the primary failure and append the cleanup failure when both fail.
    pub(crate) fn merge_cleanup<T>(
        result: std::result::Result<T, Self>,
        cleanup: std::result::Result<(), Self>,
    ) -> std::result::Result<T, Self> {
        match (result, cleanup) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(primary), Err(cleanup)) => Err(primary.with_cleanup(cleanup.message())),
            (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        }
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            CafleetError::WithDiagnostics { cause, .. } => cause.exit_code(),
            CafleetError::Usage(_) => 2,
            CafleetError::App(_)
            | CafleetError::DatabaseBusy(_)
            | CafleetError::DatabaseIo(_)
            | CafleetError::DeadlineExceeded
            | CafleetError::InvalidStoredValue { .. } => 1,
        }
    }

    pub fn message(&self) -> Cow<'_, str> {
        match self {
            CafleetError::WithDiagnostics { cause, diagnostics } => {
                Cow::Owned(format!("{cause}\n{}", diagnostics.join("\n")))
            }
            CafleetError::Usage(message)
            | CafleetError::App(message)
            | CafleetError::DatabaseBusy(message)
            | CafleetError::DatabaseIo(message) => Cow::Borrowed(message),
            CafleetError::DeadlineExceeded => Cow::Borrowed("operation exceeded its deadline"),
            CafleetError::InvalidStoredValue { field, value } => {
                Cow::Owned(format!("invalid stored value for {field}: {value}"))
            }
        }
    }
}

impl fmt::Display for CafleetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl From<rusqlite::Error> for CafleetError {
    fn from(e: rusqlite::Error) -> Self {
        match e.sqlite_error_code() {
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
                return CafleetError::DatabaseBusy(format!("database error: {e}"));
            }
            Some(
                rusqlite::ErrorCode::SystemIoFailure
                | rusqlite::ErrorCode::DiskFull
                | rusqlite::ErrorCode::CannotOpen
                | rusqlite::ErrorCode::OutOfMemory
                | rusqlite::ErrorCode::ReadOnly
                | rusqlite::ErrorCode::FileLockingProtocolFailed,
            ) => return CafleetError::DatabaseIo(format!("database error: {e}")),
            _ => {}
        }
        if let rusqlite::Error::FromSqlConversionFailure(_, _, source) = &e
            && let Some(error) = source.downcast_ref::<CafleetError>()
        {
            return error.clone();
        }
        CafleetError::App(format!("database error: {e}"))
    }
}

impl std::error::Error for CafleetError {}

#[cfg(test)]
mod tests {
    use crate::error::*;

    #[test]
    fn usage_error_maps_to_exit_code_2() {
        assert_eq!(CafleetError::Usage("bad flag".to_string()).exit_code(), 2);
    }

    #[test]
    fn app_error_maps_to_exit_code_1() {
        assert_eq!(
            CafleetError::App("runtime conflict".to_string()).exit_code(),
            1
        );
    }

    #[test]
    fn message_returns_the_carried_string() {
        assert_eq!(CafleetError::Usage("u".to_string()).message(), "u");
        assert_eq!(CafleetError::App("a".to_string()).message(), "a");
    }

    struct TransientCase {
        name: &'static str,
        error: fn() -> CafleetError,
        transient: bool,
    }

    const TRANSIENT_CASES: &[TransientCase] = &[
        TransientCase {
            name: "usage",
            error: || CafleetError::Usage("bad flag".into()),
            transient: false,
        },
        TransientCase {
            name: "app",
            error: || CafleetError::App("Input 7 not found".into()),
            transient: false,
        },
        TransientCase {
            name: "database busy",
            error: || CafleetError::DatabaseBusy("database error: database is locked".into()),
            transient: true,
        },
        TransientCase {
            name: "deadline exceeded",
            error: || CafleetError::DeadlineExceeded,
            transient: true,
        },
        TransientCase {
            name: "database io",
            error: || CafleetError::DatabaseIo("database error: disk I/O error".into()),
            transient: true,
        },
        TransientCase {
            name: "invalid stored value",
            error: || CafleetError::InvalidStoredValue {
                field: "cause".into(),
                value: "{}".into(),
            },
            transient: false,
        },
        TransientCase {
            name: "diagnostics over database busy",
            error: || CafleetError::DatabaseBusy("busy".into()).with_diagnostic("context"),
            transient: true,
        },
        TransientCase {
            name: "diagnostics over deadline exceeded",
            error: || CafleetError::DeadlineExceeded.with_diagnostic("context"),
            transient: true,
        },
        TransientCase {
            name: "diagnostics over database io",
            error: || CafleetError::DatabaseIo("io".into()).with_diagnostic("context"),
            transient: true,
        },
        TransientCase {
            name: "diagnostics over app",
            error: || CafleetError::App("stale runtime owner".into()).with_diagnostic("context"),
            transient: false,
        },
        TransientCase {
            name: "nested diagnostics over database busy",
            error: || {
                CafleetError::DatabaseBusy("busy".into())
                    .with_diagnostic("inner")
                    .with_diagnostic("outer")
            },
            transient: true,
        },
    ];

    #[test]
    fn transient_storage_holds_exactly_for_busy_deadline_and_io_causes() {
        for case in TRANSIENT_CASES {
            assert_eq!(
                (case.error)().transient_storage(),
                case.transient,
                "{}",
                case.name
            );
        }
    }

    #[derive(Debug)]
    enum Mapped {
        Busy,
        Io,
        App,
    }

    struct CodeCase {
        name: &'static str,
        code: std::ffi::c_int,
        mapped: Mapped,
    }

    const CODE_CASES: &[CodeCase] = &[
        CodeCase {
            name: "IOERR",
            code: rusqlite::ffi::SQLITE_IOERR,
            mapped: Mapped::Io,
        },
        CodeCase {
            name: "IOERR_WRITE",
            code: rusqlite::ffi::SQLITE_IOERR_WRITE,
            mapped: Mapped::Io,
        },
        CodeCase {
            name: "FULL",
            code: rusqlite::ffi::SQLITE_FULL,
            mapped: Mapped::Io,
        },
        CodeCase {
            name: "CANTOPEN",
            code: rusqlite::ffi::SQLITE_CANTOPEN,
            mapped: Mapped::Io,
        },
        CodeCase {
            name: "NOMEM",
            code: rusqlite::ffi::SQLITE_NOMEM,
            mapped: Mapped::Io,
        },
        CodeCase {
            name: "READONLY",
            code: rusqlite::ffi::SQLITE_READONLY,
            mapped: Mapped::Io,
        },
        CodeCase {
            name: "PROTOCOL",
            code: rusqlite::ffi::SQLITE_PROTOCOL,
            mapped: Mapped::Io,
        },
        CodeCase {
            name: "BUSY",
            code: rusqlite::ffi::SQLITE_BUSY,
            mapped: Mapped::Busy,
        },
        CodeCase {
            name: "LOCKED",
            code: rusqlite::ffi::SQLITE_LOCKED,
            mapped: Mapped::Busy,
        },
        CodeCase {
            name: "CONSTRAINT",
            code: rusqlite::ffi::SQLITE_CONSTRAINT,
            mapped: Mapped::App,
        },
        CodeCase {
            name: "CONSTRAINT_TRIGGER",
            code: rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER,
            mapped: Mapped::App,
        },
        CodeCase {
            name: "CORRUPT",
            code: rusqlite::ffi::SQLITE_CORRUPT,
            mapped: Mapped::App,
        },
        CodeCase {
            name: "NOTADB",
            code: rusqlite::ffi::SQLITE_NOTADB,
            mapped: Mapped::App,
        },
    ];

    #[test]
    fn sqlite_error_codes_map_to_their_storage_class() {
        for case in CODE_CASES {
            let name = case.name;
            let mapped = CafleetError::from(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(case.code),
                Some(format!("{name} fixture")),
            ));
            let message = mapped.message().into_owned();
            assert!(message.starts_with("database error: "), "{name}: {message}");
            assert_eq!(mapped.exit_code(), 1, "{name}");
            match case.mapped {
                Mapped::Io => {
                    assert!(
                        matches!(mapped, CafleetError::DatabaseIo(_)),
                        "{name}: {mapped:?}"
                    );
                    assert!(mapped.transient_storage(), "{name}");
                }
                Mapped::Busy => {
                    assert!(
                        matches!(mapped, CafleetError::DatabaseBusy(_)),
                        "{name}: {mapped:?}"
                    );
                    assert!(mapped.transient_storage(), "{name}");
                }
                Mapped::App => {
                    assert!(matches!(mapped, CafleetError::App(_)), "{name}: {mapped:?}");
                    assert!(!mapped.transient_storage(), "{name}");
                }
            }
        }
    }

    #[test]
    fn database_io_behaves_like_app_outside_classification() {
        let io = CafleetError::DatabaseIo("database error: disk I/O error".into());
        assert_eq!(io.exit_code(), 1);
        assert_eq!(io.message(), "database error: disk I/O error");
        assert_eq!(io.to_string(), "database error: disk I/O error");
        let cleaned = io.with_cleanup("cleanup failed");
        assert!(
            matches!(&cleaned, CafleetError::App(message) if message == "database error: disk I/O error\ncleanup failed"),
            "{cleaned:?}"
        );
    }
}
