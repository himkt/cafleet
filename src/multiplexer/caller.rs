use crate::coding_agent::CodingAgent;
use crate::error::CafleetError;
use crate::multiplexer::CommandRunner;
use crate::multiplexer::DEFAULT_HERDR_SESSION;
use crate::multiplexer::Multiplexer;
use crate::multiplexer::MuxBackend;
use crate::multiplexer::argv;
use crate::multiplexer::caller_server;
use crate::multiplexer::tmux_socket_reply;
use crate::runtime::ProcessTable;
use std::collections::HashMap;
use std::rc::Rc;

const MAX_ANCESTRY: usize = 4096;

/// `CommandRunner` passes the process environment to every child, so a stale `TMUX` would
/// select its server instead of the default one.
const DEFAULT_TMUX_SOCKET: &[&str] = &[
    "env",
    "-u",
    "TMUX",
    "tmux",
    "display-message",
    "-p",
    "#{socket_path}",
];

const TMUX_GUIDANCE: &str = "the tmux command prompt (the user presses the tmux prefix key in the Director pane, then types :display-message -d 0 \"#{pane_id} #{socket_path}\" and Enter; the status line shows the pane ID, then the server socket)";

const HERDR_GUIDANCE: &str = "! env -u HERDR_PANE_ID herdr --session <session> pane current --current (typed by the user into the Director's composer while the Director pane is focused; <session> is the herdr session the user attached, default unless herdr was started with --session <name>, and herdr session list shows the running sessions; the pane ID is result.pane.pane_id and the server is <session>)";

const RERUN: &str = "then run cafleet fleet create with --pane <pane-id> --mux-server <server>";

pub(crate) struct CallerMux {
    pub(crate) backend: MuxBackend,
    pub(crate) server: String,
    pub(crate) mux: Box<dyn Multiplexer>,
}

pub(crate) struct CallerPane {
    pub(crate) variable: &'static str,
    pub(crate) pane: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WalkEnd {
    Complete,
    Stopped { pid: i64 },
    Unreadable { pid: i64, cause: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ancestry {
    /// Caller first.
    pub(crate) entries: Vec<(i64, Vec<String>)>,
    pub(crate) end: WalkEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SharedServer {
    pub(crate) label: String,
    pub(crate) pid: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    PaneNotFound,
    PaneNotAncestor { pane_pid: i64 },
    WalkStopped { pid: i64 },
    NestedPidNamespace,
    ProcessUnreadable { pid: i64, cause: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verification {
    Verified { pane_pid: i64 },
    Refused(Refusal),
}

pub(crate) fn walk(table: &dyn ProcessTable) -> Ancestry {
    let mut entries: Vec<(i64, Vec<String>)> = Vec::new();
    let mut pid = table.own_pid();
    let end = loop {
        if pid == 1 {
            break WalkEnd::Complete;
        }
        if entries.len() == MAX_ANCESTRY {
            let (last, _) = entries.last().expect("a capped walk has recorded entries");
            break WalkEnd::Stopped { pid: *last };
        }
        match table.entry(pid) {
            Err(error) => {
                break WalkEnd::Unreadable {
                    pid,
                    cause: error.to_string(),
                };
            }
            Ok(None) => break WalkEnd::Stopped { pid },
            Ok(Some(entry)) => {
                entries.push((pid, entry.argv));
                if entry.parent == 0 {
                    break WalkEnd::Stopped { pid };
                }
                pid = entry.parent;
            }
        }
    };
    Ancestry { entries, end }
}

/// Row 1: the nearest ancestor that matches a shared-server signature.
pub(crate) fn shared_server(ancestry: &Ancestry) -> Option<SharedServer> {
    ancestry.entries.iter().find_map(|(pid, argv)| {
        CodingAgent::all().iter().find_map(|agent| {
            agent
                .shared_server_signatures()
                .iter()
                .find(|signature| signature.matches(argv))
                .map(|signature| SharedServer {
                    label: signature.label(*agent),
                    pid: *pid,
                })
        })
    })
}

fn lookup(env: &HashMap<String, String>) -> impl Fn(&str) -> Option<String> + '_ {
    |name: &str| env.get(name).filter(|value| !value.is_empty()).cloned()
}

fn available(
    backend: MuxBackend,
    server: String,
    runner: Rc<dyn CommandRunner>,
) -> Result<CallerMux, CafleetError> {
    let mux = backend.for_server(&server, runner);
    mux.ensure_available()?;
    Ok(CallerMux {
        backend,
        server,
        mux,
    })
}

/// Backend detection, server resolution and availability, without `--pane`.
pub(crate) fn resolve_env_mux(
    env: &HashMap<String, String>,
    runner: Rc<dyn CommandRunner>,
) -> Result<CallerMux, CafleetError> {
    let backend = MuxBackend::detect(lookup(env))?;
    let server = caller_server(backend, lookup(env), runner.as_ref())?;
    available(backend, server, runner)
}

fn pane_backend(pane: &str) -> Result<MuxBackend, CafleetError> {
    if let Some(digits) = pane.strip_prefix('%')
        && !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Ok(MuxBackend::Tmux);
    }
    if let Some((workspace, rest)) = pane.split_once(':')
        && !pane.starts_with('%')
        && !workspace.is_empty()
        && !rest.is_empty()
        && !rest.contains(':')
    {
        return Ok(MuxBackend::Herdr);
    }
    Err(CafleetError::App(format!(
        "--pane {pane} is neither a tmux pane ID (%N) nor a herdr pane ID (<workspace>:<pane>)"
    )))
}

/// The backend selected by an explicit `--pane` ID form, its server and availability.
pub(crate) fn resolve_pane_mux(
    pane: &str,
    mux_server: Option<String>,
    env: &HashMap<String, String>,
    runner: Rc<dyn CommandRunner>,
) -> Result<CallerMux, CafleetError> {
    let backend = pane_backend(pane)?;
    let server = match mux_server {
        Some(server) => server,
        None if MuxBackend::detect(lookup(env)).is_ok_and(|detected| detected == backend) => {
            caller_server(backend, lookup(env), runner.as_ref())?
        }
        None => match backend {
            MuxBackend::Tmux => tmux_socket_reply(runner.as_ref(), &argv(DEFAULT_TMUX_SOCKET))?,
            MuxBackend::Herdr => DEFAULT_HERDR_SESSION.to_owned(),
        },
    };
    available(backend, server, runner)
}

fn pane_variable(backend: MuxBackend) -> &'static str {
    match backend {
        MuxBackend::Tmux => "TMUX_PANE",
        MuxBackend::Herdr => "HERDR_PANE_ID",
    }
}

/// The non-empty `TMUX_PANE` / `HERDR_PANE_ID` for `backend`, or `None`.
pub(crate) fn env_pane(backend: MuxBackend, env: &HashMap<String, String>) -> Option<CallerPane> {
    let variable = pane_variable(backend);
    lookup(env)(variable).map(|pane| CallerPane { variable, pane })
}

/// Rows 2–7 for an env pane after `shared_server` returned `None`.
pub(crate) fn verify_pane(
    mux: &dyn Multiplexer,
    table: &dyn ProcessTable,
    ancestry: &Ancestry,
    pane: &str,
) -> Result<Verification, CafleetError> {
    let Some(pane_pid) = mux.pane_process(pane)? else {
        return Ok(Verification::Refused(Refusal::PaneNotFound));
    };
    if table.nested_pid_namespace()? {
        return Ok(Verification::Refused(Refusal::NestedPidNamespace));
    }
    if ancestry.entries.iter().any(|(pid, _)| *pid == pane_pid) {
        return Ok(Verification::Verified { pane_pid });
    }
    Ok(Verification::Refused(match &ancestry.end {
        WalkEnd::Unreadable { pid, cause } => Refusal::ProcessUnreadable {
            pid: *pid,
            cause: cause.clone(),
        },
        WalkEnd::Stopped { pid } => Refusal::WalkStopped { pid: *pid },
        WalkEnd::Complete => Refusal::PaneNotAncestor { pane_pid },
    }))
}

/// The user-run command for one multiplexer.
pub(crate) fn identification_guidance(backend: MuxBackend) -> &'static str {
    match backend {
        MuxBackend::Tmux => TMUX_GUIDANCE,
        MuxBackend::Herdr => HERDR_GUIDANCE,
    }
}

/// The refusal bodies, without the `cafleet fleet create ` prefix.
pub(crate) fn shared_server_message(shared: &SharedServer) -> String {
    let SharedServer { label, pid } = shared;
    format!(
        "cannot verify this session's pane: a shared {label} server (pid {pid}) runs this command, so its multiplexer environment can belong to another pane; ask the user to read the Director pane ID and server with {TMUX_GUIDANCE} in tmux, or with {HERDR_GUIDANCE} in herdr, {RERUN}"
    )
}

pub(crate) fn unverified_message(
    caller: &CallerPane,
    refusal: &Refusal,
    mux: &CallerMux,
) -> String {
    let cause = match refusal {
        Refusal::PaneNotFound => format!(
            "the pane does not exist on the {} server {}",
            mux.backend.name(),
            mux.server
        ),
        Refusal::PaneNotAncestor { pane_pid } => {
            format!("the pane's process {pane_pid} is not an ancestor of this command")
        }
        Refusal::WalkStopped { pid } => {
            format!("the process ancestry stopped at pid {pid} before reaching pid 1")
        }
        Refusal::NestedPidNamespace => "this command runs in a nested PID namespace".to_owned(),
        Refusal::ProcessUnreadable { cause, .. } => {
            format!("the process ancestry could not be read: {cause}")
        }
    };
    format!(
        "cannot verify that {}={} is this session's pane: {cause}; ask the user to read the Director pane ID and server with {}, {RERUN}",
        caller.variable,
        caller.pane,
        identification_guidance(mux.backend)
    )
}

pub(crate) fn unset_message(backend: MuxBackend) -> String {
    format!(
        "cannot identify its {} pane because {} is not set; ask the user to read the Director pane ID and server with {}, {RERUN}",
        backend.name(),
        pane_variable(backend),
        identification_guidance(backend)
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::rc::Rc;

    use crate::multiplexer::HerdrMultiplexer;
    use crate::multiplexer::Multiplexer;
    use crate::multiplexer::MuxBackend;
    use crate::multiplexer::RunError;
    use crate::multiplexer::TmuxMultiplexer;
    use crate::multiplexer::caller::*;
    use crate::multiplexer::test_support::FakeRunner;
    use crate::multiplexer::test_support::argv;
    use crate::multiplexer::test_support::env;
    use crate::multiplexer::test_support::herdr_envelope;
    use crate::multiplexer::test_support::herdr_error_stderr;
    use crate::runtime::test_support::FixtureProcessTable;

    const TMUX_SOCKET: &str = "/tmp/tmux-1000/default";
    const HERDR_SESSION: &str = "work";
    const UNREADABLE: &str = "cannot read /proc/412/stat: Permission denied (os error 13)";
    const TMUX_GUIDANCE: &str = "the tmux command prompt (the user presses the tmux prefix key in the Director pane, then types :display-message -d 0 \"#{pane_id} #{socket_path}\" and Enter; the status line shows the pane ID, then the server socket)";
    const HERDR_GUIDANCE: &str = "! env -u HERDR_PANE_ID herdr --session <session> pane current --current (typed by the user into the Director's composer while the Director pane is focused; <session> is the herdr session the user attached, default unless herdr was started with --session <name>, and herdr session list shows the running sessions; the pane ID is result.pane.pane_id and the server is <session>)";
    const RERUN: &str = "then run cafleet fleet create with --pane <pane-id> --mux-server <server>";

    fn tmux_mux(runner: &Rc<FakeRunner>) -> TmuxMultiplexer {
        TmuxMultiplexer::for_server(runner.clone(), TMUX_SOCKET)
    }

    fn herdr_mux(runner: &Rc<FakeRunner>) -> HerdrMultiplexer {
        HerdrMultiplexer::for_server(runner.clone(), HERDR_SESSION)
    }

    /// Queues the tmux `#{pane_id}|#{pane_pid}` reply for `pane`; `None` is tmux's missing-pane `|`.
    fn answer_tmux(runner: &FakeRunner, pane: &str, pid: Option<i64>) {
        runner.respond(Ok(match pid {
            Some(pid) => format!("{pane}|{pid}\n"),
            None => "|\n".to_string(),
        }));
    }

    /// Queues the herdr `pane process-info` reply; `None` is herdr's `pane_not_found` error.
    fn answer_herdr(runner: &FakeRunner, pid: Option<i64>) {
        runner.respond(match pid {
            Some(pid) => Ok(herdr_envelope(
                serde_json::json!({"process_info": {"shell_pid": pid}}),
            )),
            None => Err(RunError::Failed {
                stderr: herdr_error_stderr("pane_not_found"),
            }),
        });
    }

    fn table(own_pid: i64, chain: &[(i64, i64, &[&str])]) -> FixtureProcessTable {
        chain.iter().fold(
            FixtureProcessTable::new(own_pid),
            |table, (pid, parent, argv)| table.process(*pid, *parent, argv),
        )
    }

    /// `30 → 20 → 10 → 1`: the caller, a shell and a multiplexer server.
    fn plain_chain() -> FixtureProcessTable {
        table(
            30,
            &[
                (30, 20, &["cafleet", "fleet", "create"]),
                (20, 10, &["/bin/sh"]),
                (10, 1, &["tmux", "new-session"]),
            ],
        )
    }

    fn strings(tokens: &[&str]) -> Vec<String> {
        argv(tokens)
    }

    /// Row 1 through `shared_server`, then rows 2–7 through `verify_pane`.
    fn decide(
        table: &FixtureProcessTable,
        mux: &dyn Multiplexer,
        pane: &str,
    ) -> Result<Verification, SharedServer> {
        let ancestry = walk(table);
        match shared_server(&ancestry) {
            Some(shared) => Err(shared),
            None => Ok(verify_pane(mux, table, &ancestry, pane).unwrap()),
        }
    }

    fn shared(label: &str, pid: i64) -> Result<Verification, SharedServer> {
        Err(SharedServer {
            label: label.to_string(),
            pid,
        })
    }

    mod walk_termination {
        use crate::multiplexer::caller::tests::*;

        #[test]
        fn reaching_pid_1_completes_without_reading_it() {
            assert_eq!(
                walk(&plain_chain()),
                Ancestry {
                    entries: vec![
                        (30, strings(&["cafleet", "fleet", "create"])),
                        (20, strings(&["/bin/sh"])),
                        (10, strings(&["tmux", "new-session"])),
                    ],
                    end: WalkEnd::Complete,
                }
            );
        }

        #[test]
        fn own_pid_1_completes_with_no_entries() {
            assert_eq!(
                walk(&FixtureProcessTable::new(1)),
                Ancestry {
                    entries: Vec::new(),
                    end: WalkEnd::Complete,
                }
            );
        }

        #[test]
        fn an_absent_entry_stops_at_that_pid() {
            assert_eq!(
                walk(&table(30, &[(30, 20, &["cafleet"])])),
                Ancestry {
                    entries: vec![(30, strings(&["cafleet"]))],
                    end: WalkEnd::Stopped { pid: 20 },
                }
            );
        }

        #[test]
        fn parent_0_records_the_entry_and_stops_there() {
            assert_eq!(
                walk(&table(30, &[(30, 20, &["cafleet"]), (20, 0, &["init"])])),
                Ancestry {
                    entries: vec![(30, strings(&["cafleet"])), (20, strings(&["init"]))],
                    end: WalkEnd::Stopped { pid: 20 },
                }
            );
        }

        #[test]
        fn a_read_error_ends_unreadable_with_the_whole_message() {
            assert_eq!(
                walk(&table(30, &[(30, 412, &["cafleet"])]).unreadable(412, UNREADABLE)),
                Ancestry {
                    entries: vec![(30, strings(&["cafleet"]))],
                    end: WalkEnd::Unreadable {
                        pid: 412,
                        cause: UNREADABLE.to_string(),
                    },
                }
            );
        }

        #[test]
        fn a_chain_longer_than_4096_entries_stops_at_the_last_recorded_pid() {
            let own = 100_000;
            let table = (0..5000).fold(FixtureProcessTable::new(own), |table, step| {
                table.process(own - step, own - step - 1, &["sh"])
            });
            let ancestry = walk(&table);
            assert_eq!(ancestry.entries.len(), 4096);
            assert_eq!(ancestry.entries[0].0, own);
            assert_eq!(ancestry.entries[4095].0, own - 4095);
            assert_eq!(ancestry.end, WalkEnd::Stopped { pid: own - 4095 });
        }

        #[test]
        fn a_parent_cycle_terminates_at_the_cap() {
            let ancestry = walk(&table(50, &[(50, 50, &["loop"])]));
            assert_eq!(ancestry.entries.len(), 4096);
            assert_eq!(ancestry.end, WalkEnd::Stopped { pid: 50 });
        }
    }

    mod decision_table {
        use crate::multiplexer::caller::tests::*;

        #[test]
        fn row_1_refuses_at_the_nearest_shared_server_ancestor() {
            let table = table(
                30,
                &[
                    (30, 20, &["cafleet"]),
                    (
                        20,
                        15,
                        &["python", "-m", "hermes_cli.main", "gateway", "run"],
                    ),
                    (15, 10, &["/opt/bin/codex", "app-server", "--list"]),
                    (10, 1, &["tmux"]),
                ],
            );
            assert_eq!(
                shared_server(&walk(&table)),
                Some(SharedServer {
                    label: "hermes gateway run".to_string(),
                    pid: 20,
                })
            );
        }

        #[test]
        fn row_1_is_none_without_a_shared_server_ancestor() {
            assert_eq!(shared_server(&walk(&plain_chain())), None);
        }

        #[test]
        fn row_1_inspects_the_entries_recorded_before_an_unreadable_end() {
            let table = table(
                30,
                &[(30, 20, &["cafleet"]), (20, 412, &["codex", "app-server"])],
            )
            .unreadable(412, UNREADABLE);
            assert_eq!(
                shared_server(&walk(&table)),
                Some(SharedServer {
                    label: "codex app-server".to_string(),
                    pid: 20,
                })
            );
        }

        #[test]
        fn row_2_a_missing_pane_refuses_before_the_namespace_check() {
            let table = plain_chain().nested();
            let runner = FakeRunner::with_binary("tmux");
            answer_tmux(&runner, "%3", None);
            assert_eq!(
                verify_pane(&tmux_mux(&runner), &table, &walk(&table), "%3").unwrap(),
                Verification::Refused(Refusal::PaneNotFound)
            );
        }

        #[test]
        fn row_2_reads_the_pane_through_the_multiplexer_on_its_server() {
            let table = plain_chain();
            let runner = FakeRunner::with_binary("herdr");
            answer_herdr(&runner, None);
            assert_eq!(
                verify_pane(&herdr_mux(&runner), &table, &walk(&table), "w1:p2").unwrap(),
                Verification::Refused(Refusal::PaneNotFound)
            );
            assert_eq!(
                runner.run_argvs(),
                vec![strings(&[
                    "herdr",
                    "--session",
                    HERDR_SESSION,
                    "pane",
                    "process-info",
                    "--pane",
                    "w1:p2",
                ])]
            );
        }

        #[test]
        fn row_3_a_nested_namespace_refuses_even_an_ancestor_pane() {
            let table = plain_chain().nested();
            let runner = FakeRunner::with_binary("tmux");
            answer_tmux(&runner, "%3", Some(20));
            assert_eq!(
                verify_pane(&tmux_mux(&runner), &table, &walk(&table), "%3").unwrap(),
                Verification::Refused(Refusal::NestedPidNamespace)
            );
        }

        #[test]
        fn row_4_an_ancestor_pane_verifies() {
            let table = plain_chain();
            for (backend, pane) in [("tmux", "%3"), ("herdr", "w1:p2")] {
                let runner = FakeRunner::with_binary(backend);
                let mux: Box<dyn Multiplexer> = if backend == "tmux" {
                    answer_tmux(&runner, pane, Some(20));
                    Box::new(tmux_mux(&runner))
                } else {
                    answer_herdr(&runner, Some(20));
                    Box::new(herdr_mux(&runner))
                };
                assert_eq!(
                    verify_pane(&*mux, &table, &walk(&table), pane).unwrap(),
                    Verification::Verified { pane_pid: 20 },
                    "{backend}"
                );
            }
        }

        #[test]
        fn row_4_precedes_an_unreadable_or_stopped_walk_end() {
            for table in [
                table(30, &[(30, 20, &["cafleet"]), (20, 412, &["/bin/sh"])])
                    .unreadable(412, UNREADABLE),
                table(30, &[(30, 20, &["cafleet"]), (20, 15, &["/bin/sh"])]),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                answer_tmux(&runner, "%3", Some(20));
                assert_eq!(
                    verify_pane(&tmux_mux(&runner), &table, &walk(&table), "%3").unwrap(),
                    Verification::Verified { pane_pid: 20 }
                );
            }
        }

        #[test]
        fn row_5_an_unreadable_walk_refuses_with_its_pid_and_cause() {
            let table = table(30, &[(30, 412, &["cafleet"])]).unreadable(412, UNREADABLE);
            let runner = FakeRunner::with_binary("tmux");
            answer_tmux(&runner, "%3", Some(99));
            assert_eq!(
                verify_pane(&tmux_mux(&runner), &table, &walk(&table), "%3").unwrap(),
                Verification::Refused(Refusal::ProcessUnreadable {
                    pid: 412,
                    cause: UNREADABLE.to_string(),
                })
            );
        }

        #[test]
        fn row_6_a_stopped_walk_refuses_with_its_pid() {
            for table in [
                table(30, &[(30, 20, &["cafleet"])]),
                table(30, &[(30, 20, &["cafleet"]), (20, 0, &["init"])]),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                answer_tmux(&runner, "%3", Some(99));
                assert_eq!(
                    verify_pane(&tmux_mux(&runner), &table, &walk(&table), "%3").unwrap(),
                    Verification::Refused(Refusal::WalkStopped { pid: 20 })
                );
            }
        }

        #[test]
        fn row_7_a_completed_walk_without_the_pane_refuses_with_the_pane_pid() {
            let table = plain_chain();
            let runner = FakeRunner::with_binary("tmux");
            answer_tmux(&runner, "%3", Some(99));
            assert_eq!(
                verify_pane(&tmux_mux(&runner), &table, &walk(&table), "%3").unwrap(),
                Verification::Refused(Refusal::PaneNotAncestor { pane_pid: 99 })
            );
        }

        #[test]
        fn a_pane_process_command_failure_propagates_as_an_error() {
            let table = plain_chain();
            let runner = FakeRunner::with_binary("tmux");
            runner.respond(Err(RunError::Failed {
                stderr: "no server running on /tmp/tmux-1000/default".to_string(),
            }));
            assert!(verify_pane(&tmux_mux(&runner), &table, &walk(&table), "%3").is_err());
        }

        #[test]
        fn a_namespace_read_failure_propagates_as_an_error() {
            let message = "cannot read /proc/self/status: Permission denied (os error 13)";
            let table = plain_chain().namespace_error(message);
            let runner = FakeRunner::with_binary("tmux");
            answer_tmux(&runner, "%3", Some(20));
            assert_eq!(
                verify_pane(&tmux_mux(&runner), &table, &walk(&table), "%3")
                    .unwrap_err()
                    .to_string(),
                message
            );
        }
    }

    mod evidence_fixtures {
        use crate::multiplexer::caller::tests::*;

        type Chain = &'static [(i64, i64, &'static [&'static str])];

        const CAFLEET: &[&str] = &["cafleet", "…"];
        const CODEX_DAEMON: &[&str] = &[
            "/root/.codex/packages/app-server-daemon/releases/0.167.1-x86_64-unknown-linux-musl/bin/codex",
            "app-server",
            "--list",
        ];
        const CODEX_PANE_A: &[&str] = &[
            "codex",
            "--enable",
            "daemon_auto_start",
            "--dangerously-bypass-approvals-and-sandbox",
            "<prompt>",
        ];
        const OPENCODE: &str =
            "/root/.local/share/mise/installs/aqua-anomalyco-opencode/v2.2.5/opencode";
        const TMUX_SERVER_PID: i64 = 399808;
        const TMUX_SERVER: &[&str] = &[
            "tmux",
            "-L",
            "cafleet-probe",
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "probe",
            "-x",
            "220",
            "-y",
            "50",
            "-c",
            "/tmp/cafleet-env-probe/work",
        ];
        const HERDR_SERVER_PID: i64 = 3340413;
        const HERDR_SERVER: &[&str] = &[
            "/root/.local/share/mise/installs/aqua-ogulcancelik-herdr/v0.9.5/herdr",
            "server",
        ];

        enum Expected {
            SharedServer(&'static str, i64),
            Verified,
            NotAncestor,
        }

        /// A tmux case: the chain stops below the tmux server, whose pid is every top parent.
        struct Case {
            name: &'static str,
            own_pid: i64,
            chain: Chain,
            pane: &'static str,
            pane_pid: Option<i64>,
            expected: Expected,
        }

        impl Case {
            fn verification(&self, pane_pid: Option<i64>) -> Result<Verification, SharedServer> {
                match self.expected {
                    Expected::SharedServer(label, pid) => shared(label, pid),
                    Expected::Verified => Ok(Verification::Verified {
                        pane_pid: pane_pid.expect("a verified case names its pane pid"),
                    }),
                    Expected::NotAncestor => Ok(Verification::Refused(Refusal::PaneNotAncestor {
                        pane_pid: pane_pid.expect("a row-7 case names its pane pid"),
                    })),
                }
            }

            fn tmux_table(&self) -> FixtureProcessTable {
                table(self.own_pid, self.chain).process(TMUX_SERVER_PID, 1, TMUX_SERVER)
            }

            /// The pane's shell between each agent and the herdr server; the shell is the pane pid.
            fn herdr_shell(pid: i64) -> i64 {
                pid + 1
            }

            fn herdr_table(&self) -> FixtureProcessTable {
                self.chain
                    .iter()
                    .fold(
                        FixtureProcessTable::new(self.own_pid),
                        |table, (pid, parent, argv)| {
                            if *parent == TMUX_SERVER_PID {
                                table.process(*pid, Self::herdr_shell(*pid), argv).process(
                                    Self::herdr_shell(*pid),
                                    HERDR_SERVER_PID,
                                    &["/bin/sh"],
                                )
                            } else {
                                table.process(*pid, *parent, argv)
                            }
                        },
                    )
                    .process(HERDR_SERVER_PID, 1, HERDR_SERVER)
            }

            fn herdr_pane_pid(&self) -> Option<i64> {
                self.pane_pid.map(|pid| {
                    let agent = self
                        .chain
                        .iter()
                        .any(|(entry, parent, _)| *entry == pid && *parent == TMUX_SERVER_PID);
                    if agent { Self::herdr_shell(pid) } else { pid }
                })
            }

            fn herdr_pane(&self) -> String {
                format!("w5:p{}", &self.pane[1..])
            }
        }

        fn tmux_cases() -> Vec<Case> {
            vec![
                Case {
                    name: "Codex daemon, pane A",
                    own_pid: 401748,
                    chain: &[
                        (401748, 401380, CAFLEET),
                        (401380, 400625, CODEX_DAEMON),
                        (400625, TMUX_SERVER_PID, CODEX_PANE_A),
                    ],
                    pane: "%3",
                    pane_pid: Some(400625),
                    expected: Expected::SharedServer("codex app-server", 401380),
                },
                Case {
                    name: "Codex daemon, pane C (pane A's variables)",
                    own_pid: 402502,
                    chain: &[
                        (402502, 401380, CAFLEET),
                        (401380, 400625, CODEX_DAEMON),
                        (400625, TMUX_SERVER_PID, CODEX_PANE_A),
                    ],
                    pane: "%3",
                    pane_pid: Some(400625),
                    expected: Expected::SharedServer("codex app-server", 401380),
                },
                Case {
                    name: "Codex pane C after pane A closed",
                    own_pid: 402611,
                    chain: &[(402611, 401380, CAFLEET), (401380, 1, CODEX_DAEMON)],
                    pane: "%3",
                    pane_pid: None,
                    expected: Expected::SharedServer("codex app-server", 401380),
                },
                Case {
                    name: "Codex --no-daemon",
                    own_pid: 403233,
                    chain: &[
                        (403233, 402835, CAFLEET),
                        (
                            402835,
                            TMUX_SERVER_PID,
                            &[
                                "codex",
                                "--no-daemon",
                                "--dangerously-bypass-approvals-and-sandbox",
                                "<prompt>",
                            ],
                        ),
                    ],
                    pane: "%6",
                    pane_pid: Some(402835),
                    expected: Expected::Verified,
                },
                Case {
                    name: "Codex --approve-for-me",
                    own_pid: 402101,
                    chain: &[
                        (402101, 401813, CAFLEET),
                        (
                            401813,
                            TMUX_SERVER_PID,
                            &[
                                "codex",
                                "--enable",
                                "daemon_auto_start",
                                "--approve-for-me",
                                "<prompt>",
                            ],
                        ),
                    ],
                    pane: "%4",
                    pane_pid: Some(401813),
                    expected: Expected::Verified,
                },
                Case {
                    name: "Claude Code",
                    own_pid: 403317,
                    chain: &[
                        (403317, 403316, CAFLEET),
                        (403316, 403012, &["/bin/bash", "-c", "<script>"]),
                        (
                            403012,
                            TMUX_SERVER_PID,
                            &[
                                "claude",
                                "--allowedTools",
                                "Bash(sh /tmp/cafleet-env-probe/detect.sh *)",
                                "--",
                                "<prompt>",
                            ],
                        ),
                    ],
                    pane: "%7",
                    pane_pid: Some(403012),
                    expected: Expected::Verified,
                },
                Case {
                    name: "pi",
                    own_pid: 403373,
                    chain: &[
                        (403373, 403148, CAFLEET),
                        (403148, TMUX_SERVER_PID, &["pi", "<prompt>"]),
                    ],
                    pane: "%8",
                    pane_pid: Some(403148),
                    expected: Expected::Verified,
                },
                Case {
                    name: "OpenCode --standalone",
                    own_pid: 403403,
                    chain: &[
                        (403403, 403187, CAFLEET),
                        (
                            403187,
                            403176,
                            &[OPENCODE, "serve", "--stdio", "--port", "0"],
                        ),
                        (
                            403176,
                            TMUX_SERVER_PID,
                            &["opencode", "--standalone", "--auto", "--prompt", "<prompt>"],
                        ),
                    ],
                    pane: "%9",
                    pane_pid: Some(403176),
                    expected: Expected::Verified,
                },
                Case {
                    name: "OpenCode service, first client",
                    own_pid: 403443,
                    chain: &[
                        (403443, 403264, CAFLEET),
                        (403264, 403252, &[OPENCODE, "serve", "--service"]),
                        (
                            403252,
                            TMUX_SERVER_PID,
                            &["opencode", "--auto", "--prompt", "<prompt>"],
                        ),
                    ],
                    pane: "%10",
                    pane_pid: Some(403252),
                    expected: Expected::Verified,
                },
                Case {
                    name: "OpenCode service, second client",
                    own_pid: 405295,
                    chain: &[
                        (405295, 403264, CAFLEET),
                        (403264, 403252, &[OPENCODE, "serve", "--service"]),
                        (
                            403252,
                            TMUX_SERVER_PID,
                            &["opencode", "--auto", "--prompt", "<prompt>"],
                        ),
                    ],
                    pane: "%13",
                    pane_pid: Some(405228),
                    expected: Expected::NotAncestor,
                },
            ]
        }

        #[test]
        fn every_tmux_case_reaches_its_expected_row() {
            for case in tmux_cases() {
                let runner = FakeRunner::with_binary("tmux");
                let row_1 = matches!(case.expected, Expected::SharedServer(..));
                if !row_1 {
                    answer_tmux(&runner, case.pane, case.pane_pid);
                }
                assert_eq!(
                    decide(&case.tmux_table(), &tmux_mux(&runner), case.pane),
                    case.verification(case.pane_pid),
                    "{}",
                    case.name
                );
                if row_1 {
                    assert!(runner.run_argvs().is_empty(), "{}", case.name);
                }
            }
        }

        #[test]
        fn every_herdr_case_with_the_pane_shell_reaches_its_expected_row() {
            for case in tmux_cases() {
                let runner = FakeRunner::with_binary("herdr");
                let row_1 = matches!(case.expected, Expected::SharedServer(..));
                if !row_1 {
                    answer_herdr(&runner, case.herdr_pane_pid());
                }
                assert_eq!(
                    decide(&case.herdr_table(), &herdr_mux(&runner), &case.herdr_pane()),
                    case.verification(case.herdr_pane_pid()),
                    "{}",
                    case.name
                );
                if row_1 {
                    assert!(runner.run_argvs().is_empty(), "{}", case.name);
                }
            }
        }

        #[test]
        fn pane_c_would_pass_row_4_alone_because_pane_a_is_its_ancestor() {
            let case = tmux_cases()
                .into_iter()
                .find(|case| case.name == "Codex daemon, pane C (pane A's variables)")
                .expect("the pane C fixture exists");
            let table = case.tmux_table();
            let runner = FakeRunner::with_binary("tmux");
            answer_tmux(&runner, case.pane, case.pane_pid);
            assert_eq!(
                verify_pane(&tmux_mux(&runner), &table, &walk(&table), case.pane).unwrap(),
                Verification::Verified { pane_pid: 400625 }
            );
        }

        const HERMES_PYTHON: &str =
            "/home/linuxbrew/.linuxbrew/Cellar/hermes-agent/0.22.4_1/libexec/bin/python";
        const HERMES_GATEWAY: &[&str] = &[
            HERMES_PYTHON,
            "-m",
            "hermes_cli.main",
            "--profile",
            "default",
            "gateway",
            "run",
            "--quiet",
            "--idle-exit",
        ];
        const HERMES_CHAT: &[&str] = &[
            HERMES_PYTHON,
            "/home/linuxbrew/.linuxbrew/Cellar/hermes-agent/0.22.4_1/libexec/bin/hermes",
            "chat",
            "--cli",
            "--yolo",
            "-q",
            "<prompt>",
        ];

        #[test]
        fn tmux_hermes_refuses_at_row_1_although_its_chat_pane_is_an_ancestor() {
            let tmux_server = [
                "tmux",
                "-L",
                "cafleet-probe",
                "-f",
                "/dev/null",
                "new-session",
                "-d",
                "-s",
                "probe",
                "-x",
                "220",
                "-y",
                "50",
                "-c",
                "/tmp/cafleet-env-probe/work",
            ];
            let table = table(
                428897,
                &[
                    (
                        428897,
                        428895,
                        &["sh", "/tmp/cafleet-env-probe/capture.sh", "hermes"],
                    ),
                    (428895, 428596, &["/usr/bin/bash", "-c", "<script>"]),
                    (428596, 428571, HERMES_GATEWAY),
                    (428571, 428475, HERMES_CHAT),
                    (428475, 1, &tmux_server),
                ],
            );
            let runner = FakeRunner::with_binary("tmux");
            assert_eq!(
                decide(&table, &tmux_mux(&runner), "%1"),
                shared("hermes gateway run", 428596)
            );
            assert!(runner.run_argvs().is_empty());

            answer_tmux(&runner, "%1", Some(428571));
            assert_eq!(
                verify_pane(&tmux_mux(&runner), &table, &walk(&table), "%1").unwrap(),
                Verification::Verified { pane_pid: 428571 },
                "row 4 alone would accept the chat client pane"
            );
        }

        #[test]
        fn herdr_hermes_with_the_pane_shell_refuses_at_row_1() {
            let table = table(
                428897,
                &[
                    (
                        428897,
                        428895,
                        &["sh", "/tmp/cafleet-env-probe/capture.sh", "hermes"],
                    ),
                    (428895, 428596, &["/usr/bin/bash", "-c", "<script>"]),
                    (428596, 428571, HERMES_GATEWAY),
                    (428571, 428572, HERMES_CHAT),
                    (428572, HERDR_SERVER_PID, &["/bin/sh"]),
                    (HERDR_SERVER_PID, 1, HERDR_SERVER),
                ],
            );
            let runner = FakeRunner::with_binary("herdr");
            assert_eq!(
                decide(&table, &herdr_mux(&runner), "w5:p2"),
                shared("hermes gateway run", 428596)
            );
            assert!(runner.run_argvs().is_empty());
        }

        #[test]
        fn herdr_codex_daemon_pane_a_refuses_at_row_1() {
            let table = table(
                408076,
                &[
                    (408076, 407596, CAFLEET),
                    (407596, 407442, CODEX_DAEMON),
                    (407442, 407363, CODEX_PANE_A),
                    (407363, HERDR_SERVER_PID, &["/bin/sh"]),
                    (HERDR_SERVER_PID, 1, HERDR_SERVER),
                ],
            );
            let runner = FakeRunner::with_binary("herdr");
            assert_eq!(
                decide(&table, &herdr_mux(&runner), "w5:p1"),
                shared("codex app-server", 407596)
            );
            assert!(runner.run_argvs().is_empty());
        }

        #[test]
        fn herdr_real_director_session_verifies_its_pane_shell() {
            let table = table(
                410391,
                &[
                    (410391, 410390, CAFLEET),
                    (410390, 363045, &["/bin/bash", "-c", "<script>"]),
                    (363045, 362690, &["claude"]),
                    (362690, 362679, &["zsh"]),
                    (362679, HERDR_SERVER_PID, &["/bin/sh"]),
                    (HERDR_SERVER_PID, 1, HERDR_SERVER),
                ],
            );
            let runner = FakeRunner::with_binary("herdr");
            answer_herdr(&runner, Some(362679));
            assert_eq!(
                decide(&table, &herdr_mux(&runner), "w4:p3N"),
                Ok(Verification::Verified { pane_pid: 362679 })
            );
        }
    }

    mod pane_form {
        use crate::multiplexer::caller::tests::*;

        #[test]
        fn a_percent_and_digits_selects_tmux() {
            let runner = FakeRunner::with_binary("tmux");
            let caller = resolve_pane_mux(
                "%12",
                Some(TMUX_SOCKET.to_string()),
                &HashMap::new(),
                runner.clone(),
            )
            .unwrap();
            assert_eq!(caller.backend, MuxBackend::Tmux);
            assert_eq!(caller.server, TMUX_SOCKET);
        }

        #[test]
        fn a_workspace_colon_pane_selects_herdr() {
            let runner = FakeRunner::with_binary("herdr");
            let caller = resolve_pane_mux(
                "w4:p3N",
                Some(HERDR_SESSION.to_string()),
                &HashMap::new(),
                runner.clone(),
            )
            .unwrap();
            assert_eq!(caller.backend, MuxBackend::Herdr);
            assert_eq!(caller.server, HERDR_SESSION);
        }

        #[test]
        fn any_other_form_fails_before_server_resolution_or_availability() {
            for pane in ["12", "%", "%1a", "a:b:c", ":p1", "w4:", "%1:2"] {
                let runner = FakeRunner::without_binaries();
                let error = match resolve_pane_mux(
                    pane,
                    None,
                    &env(&[("TMUX", "/tmp/tmux-1000/default,123,0")]),
                    runner.clone(),
                ) {
                    Ok(_) => panic!("{pane} is not a pane ID form"),
                    Err(error) => error,
                };
                assert_eq!(
                    error.to_string(),
                    format!(
                        "--pane {pane} is neither a tmux pane ID (%N) nor a herdr pane ID (<workspace>:<pane>)"
                    )
                );
                assert_eq!(error.exit_code(), 1, "{pane}");
                assert!(runner.events().is_empty(), "{pane}");
            }
        }
    }

    mod pane_server_selection {
        use crate::multiplexer::caller::tests::*;

        const AMBIGUOUS: &[(&str, &str)] = &[
            ("HERDR_ENV", "1"),
            ("HERDR_SESSION", "stale"),
            ("TMUX", "/tmp/stale/default,1,0"),
        ];

        fn default_socket_argv() -> Vec<String> {
            strings(&[
                "env",
                "-u",
                "TMUX",
                "tmux",
                "display-message",
                "-p",
                "#{socket_path}",
            ])
        }

        fn resolve(
            pane: &str,
            mux_server: Option<&str>,
            pairs: &[(&str, &str)],
            runner: &Rc<FakeRunner>,
        ) -> Result<CallerMux, crate::error::CafleetError> {
            resolve_pane_mux(
                pane,
                mux_server.map(str::to_owned),
                &env(pairs),
                runner.clone(),
            )
        }

        #[test]
        fn mux_server_wins_over_every_environment_and_targets_that_server() {
            for pairs in [AMBIGUOUS, &[("TMUX", "/tmp/other/default,1,0")][..], &[]] {
                let runner = FakeRunner::with_binary("tmux");
                let caller = resolve("%5", Some("/tmp/tmux-0/default"), pairs, &runner).unwrap();
                assert_eq!(
                    (caller.backend, caller.server.as_str()),
                    (MuxBackend::Tmux, "/tmp/tmux-0/default")
                );
                assert!(runner.run_argvs().is_empty(), "{pairs:?}");
                answer_tmux(&runner, "%5", Some(77));
                assert_eq!(caller.mux.pane_process("%5").unwrap(), Some(77));
                assert_eq!(
                    runner.run_argvs(),
                    vec![strings(&[
                        "tmux",
                        "-S",
                        "/tmp/tmux-0/default",
                        "display-message",
                        "-p",
                        "-t",
                        "%5",
                        "#{pane_id}|#{pane_pid}",
                    ])]
                );
            }
            let runner = FakeRunner::with_binary("herdr");
            let caller = resolve("w4:p1", Some("main"), AMBIGUOUS, &runner).unwrap();
            assert_eq!(
                (caller.backend, caller.server.as_str()),
                (MuxBackend::Herdr, "main")
            );
            assert!(runner.run_argvs().is_empty());
        }

        #[test]
        fn an_environment_on_the_derived_backend_uses_the_caller_server() {
            let runner = FakeRunner::with_binary("tmux");
            let caller = resolve(
                "%5",
                None,
                &[("TMUX", "/tmp/tmux-1000/default,123,0")],
                &runner,
            )
            .unwrap();
            assert_eq!(
                (caller.backend, caller.server.as_str()),
                (MuxBackend::Tmux, "/tmp/tmux-1000/default")
            );
            assert!(runner.run_argvs().is_empty());

            let runner = FakeRunner::with_binary("herdr");
            let caller = resolve(
                "w4:p1",
                None,
                &[("HERDR_ENV", "1"), ("HERDR_SESSION", "work")],
                &runner,
            )
            .unwrap();
            assert_eq!(
                (caller.backend, caller.server.as_str()),
                (MuxBackend::Herdr, "work")
            );
            assert!(runner.run_argvs().is_empty());
        }

        #[test]
        fn a_tmux_pane_outside_a_tmux_environment_asks_the_default_server_without_tmux() {
            for pairs in [
                AMBIGUOUS,
                &[("HERDR_ENV", "1"), ("HERDR_SESSION", "work")][..],
                &[],
            ] {
                let runner = FakeRunner::with_binary("tmux");
                runner.respond(Ok("/tmp/tmux-0/default\n".to_string()));
                let caller = resolve("%5", None, pairs, &runner).unwrap();
                assert_eq!(
                    (caller.backend, caller.server.as_str()),
                    (MuxBackend::Tmux, "/tmp/tmux-0/default"),
                    "{pairs:?}"
                );
                assert_eq!(runner.run_argvs(), vec![default_socket_argv()], "{pairs:?}");
            }
        }

        #[test]
        fn a_herdr_pane_outside_a_herdr_environment_uses_the_default_session_without_commands() {
            for pairs in [
                AMBIGUOUS,
                &[("TMUX", "/tmp/tmux-1000/default,123,0")][..],
                &[],
            ] {
                let runner = FakeRunner::with_binary("herdr");
                let caller = resolve("w4:p1", None, pairs, &runner).unwrap();
                assert_eq!(
                    (caller.backend, caller.server.as_str()),
                    (MuxBackend::Herdr, "default"),
                    "{pairs:?}"
                );
                assert!(runner.run_argvs().is_empty(), "{pairs:?}");
            }
        }

        #[test]
        fn an_empty_or_failed_default_server_reply_names_mux_server() {
            for reply in [
                Ok("\n".to_string()),
                Err(RunError::Failed {
                    stderr: "no server running on /tmp/tmux-0/default".to_string(),
                }),
            ] {
                let runner = FakeRunner::with_binary("tmux");
                runner.respond(reply);
                let error = match resolve("%5", None, AMBIGUOUS, &runner) {
                    Ok(_) => panic!("no default tmux server resolves no server"),
                    Err(error) => error,
                };
                assert_eq!(
                    error.to_string(),
                    "no tmux server found; pass --mux-server <socket path>"
                );
                assert_eq!(runner.run_argvs(), vec![default_socket_argv()]);
            }
        }

        #[test]
        fn a_missing_binary_fails_availability() {
            let runner = FakeRunner::without_binaries();
            let error = match resolve("%5", Some(TMUX_SOCKET), &[], &runner) {
                Ok(_) => panic!("an unavailable multiplexer resolves no caller mux"),
                Err(error) => error,
            };
            assert_eq!(error.to_string(), "tmux binary not found on PATH");
        }
    }

    mod env_resolution {
        use crate::multiplexer::caller::tests::*;

        #[test]
        fn env_pane_reads_the_backend_variable_and_treats_empty_as_unset() {
            let pairs = env(&[("TMUX_PANE", "%3"), ("HERDR_PANE_ID", "w5:p1")]);
            let tmux = env_pane(MuxBackend::Tmux, &pairs).expect("TMUX_PANE is set");
            assert_eq!((tmux.variable, tmux.pane.as_str()), ("TMUX_PANE", "%3"));
            let herdr = env_pane(MuxBackend::Herdr, &pairs).expect("HERDR_PANE_ID is set");
            assert_eq!(
                (herdr.variable, herdr.pane.as_str()),
                ("HERDR_PANE_ID", "w5:p1")
            );
            let empty = env(&[("TMUX_PANE", ""), ("HERDR_PANE_ID", "")]);
            assert!(env_pane(MuxBackend::Tmux, &empty).is_none());
            assert!(env_pane(MuxBackend::Herdr, &empty).is_none());
            assert!(env_pane(MuxBackend::Tmux, &env(&[("HERDR_PANE_ID", "w5:p1")])).is_none());
        }

        #[test]
        fn resolve_env_mux_detects_the_backend_and_its_caller_server() {
            let runner = FakeRunner::with_binary("tmux");
            let caller = resolve_env_mux(
                &env(&[
                    ("TMUX", "/tmp/tmux-1000/default,123,0"),
                    ("TMUX_PANE", "%3"),
                ]),
                runner.clone(),
            )
            .unwrap();
            assert_eq!(
                (caller.backend, caller.server.as_str()),
                (MuxBackend::Tmux, "/tmp/tmux-1000/default")
            );

            let runner = FakeRunner::with_binary("herdr");
            let caller = resolve_env_mux(
                &env(&[("HERDR_ENV", "1"), ("HERDR_SESSION", "work")]),
                runner.clone(),
            )
            .unwrap();
            assert_eq!(
                (caller.backend, caller.server.as_str()),
                (MuxBackend::Herdr, "work")
            );
            assert!(runner.run_argvs().is_empty());
        }

        #[test]
        fn resolve_env_mux_keeps_the_detection_and_availability_messages() {
            let ambiguous = match resolve_env_mux(
                &env(&[("HERDR_ENV", "1"), ("TMUX", "/tmp/tmux")]),
                FakeRunner::with_binary("tmux"),
            ) {
                Ok(_) => panic!("an ambiguous environment detects no backend"),
                Err(error) => error,
            };
            assert_eq!(
                ambiguous.to_string(),
                "ambiguous multiplexer environment: both HERDR_ENV and TMUX are set; \
                 unset the one that does not belong to this session"
            );
            let missing = match resolve_env_mux(
                &env(&[("TMUX", "/tmp/tmux-1000/default,123,0")]),
                FakeRunner::without_binaries(),
            ) {
                Ok(_) => panic!("an unavailable multiplexer resolves no caller mux"),
                Err(error) => error,
            };
            assert_eq!(missing.to_string(), "tmux binary not found on PATH");
        }
    }

    mod refusal_messages {
        use crate::multiplexer::caller::tests::*;

        fn caller_mux(backend: MuxBackend, server: &str) -> CallerMux {
            CallerMux {
                backend,
                server: server.to_string(),
                mux: backend.for_server(server, FakeRunner::without_binaries()),
            }
        }

        fn caller_pane(variable: &'static str, pane: &str) -> CallerPane {
            CallerPane {
                variable,
                pane: pane.to_string(),
            }
        }

        #[test]
        fn identification_guidance_names_the_user_run_command_per_multiplexer() {
            assert_eq!(identification_guidance(MuxBackend::Tmux), TMUX_GUIDANCE);
            assert_eq!(identification_guidance(MuxBackend::Herdr), HERDR_GUIDANCE);
        }

        #[test]
        fn shared_server_message_names_both_guidances() {
            let refusal = SharedServer {
                label: "codex app-server".to_string(),
                pid: 407596,
            };
            assert_eq!(
                format!("cafleet fleet create {}", shared_server_message(&refusal)),
                "cafleet fleet create cannot verify this session's pane: a shared codex app-server server (pid 407596) runs this command, so its multiplexer environment can belong to another pane; ask the user to read the Director pane ID and server with the tmux command prompt (the user presses the tmux prefix key in the Director pane, then types :display-message -d 0 \"#{pane_id} #{socket_path}\" and Enter; the status line shows the pane ID, then the server socket) in tmux, or with ! env -u HERDR_PANE_ID herdr --session <session> pane current --current (typed by the user into the Director's composer while the Director pane is focused; <session> is the herdr session the user attached, default unless herdr was started with --session <name>, and herdr session list shows the running sessions; the pane ID is result.pane.pane_id and the server is <session>) in herdr, then run cafleet fleet create with --pane <pane-id> --mux-server <server>"
            );
            assert_eq!(
                shared_server_message(&SharedServer {
                    label: "hermes gateway run".to_string(),
                    pid: 403471,
                }),
                format!(
                    "cannot verify this session's pane: a shared hermes gateway run server (pid 403471) runs this command, so its multiplexer environment can belong to another pane; ask the user to read the Director pane ID and server with {TMUX_GUIDANCE} in tmux, or with {HERDR_GUIDANCE} in herdr, {RERUN}"
                )
            );
        }

        #[test]
        fn unverified_message_states_each_cause_on_both_multiplexers() {
            let causes = |backend: &str, server: &str| {
                [
                    (
                        Refusal::PaneNotFound,
                        format!("the pane does not exist on the {backend} server {server}"),
                    ),
                    (
                        Refusal::PaneNotAncestor { pane_pid: 405228 },
                        "the pane's process 405228 is not an ancestor of this command".to_string(),
                    ),
                    (
                        Refusal::WalkStopped { pid: 20 },
                        "the process ancestry stopped at pid 20 before reaching pid 1".to_string(),
                    ),
                    (
                        Refusal::NestedPidNamespace,
                        "this command runs in a nested PID namespace".to_string(),
                    ),
                    (
                        Refusal::ProcessUnreadable {
                            pid: 412,
                            cause: UNREADABLE.to_string(),
                        },
                        format!("the process ancestry could not be read: {UNREADABLE}"),
                    ),
                ]
            };
            for (backend, server, variable, pane, guidance) in [
                (
                    MuxBackend::Tmux,
                    TMUX_SOCKET,
                    "TMUX_PANE",
                    "%3",
                    TMUX_GUIDANCE,
                ),
                (
                    MuxBackend::Herdr,
                    HERDR_SESSION,
                    "HERDR_PANE_ID",
                    "w5:p1",
                    HERDR_GUIDANCE,
                ),
            ] {
                let mux = caller_mux(backend, server);
                let caller = caller_pane(variable, pane);
                for (refusal, cause) in causes(backend.name(), server) {
                    assert_eq!(
                        unverified_message(&caller, &refusal, &mux),
                        format!(
                            "cannot verify that {variable}={pane} is this session's pane: {cause}; ask the user to read the Director pane ID and server with {guidance}, {RERUN}"
                        )
                    );
                }
            }
        }

        #[test]
        fn unset_message_names_the_backend_variable_and_its_guidance() {
            assert_eq!(
                unset_message(MuxBackend::Tmux),
                format!(
                    "cannot identify its tmux pane because TMUX_PANE is not set; ask the user to read the Director pane ID and server with {TMUX_GUIDANCE}, {RERUN}"
                )
            );
            assert_eq!(
                unset_message(MuxBackend::Herdr),
                format!(
                    "cannot identify its herdr pane because HERDR_PANE_ID is not set; ask the user to read the Director pane ID and server with {HERDR_GUIDANCE}, {RERUN}"
                )
            );
        }
    }
}
