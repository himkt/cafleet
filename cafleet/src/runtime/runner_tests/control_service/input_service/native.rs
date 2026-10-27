use super::*;
use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::sys::termios::{LocalFlags, SetArg, tcgetattr, tcsetattr};
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;

fn receipt_inputs(
    conn: &rusqlite::Connection,
    run_id: i64,
    expires: u64,
) -> rusqlite::Result<Vec<Value>> {
    loop {
        assert!(
            Clock.sample().unwrap().monotonic_ns < expires,
            "original owned native lifetime"
        );
        let result = (|| {
            let mut statement = conn.prepare("SELECT input_id,state FROM member_input_operations WHERE run_id=?1 ORDER BY input_id")?;
            let rows = statement.query_map([run_id], |row| {
                Ok(json!({"input_id":row.get::<_,i64>(0)?,"state":row.get::<_,String>(1)?}))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
        })();
        match result {
            Err(cause)
                if matches!(
                    cause.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
                ) =>
            {
                let now = Clock.sample().unwrap().monotonic_ns;
                assert!(now < expires, "original owned native lifetime");
                std::thread::sleep(
                    Duration::from_millis(2).min(Duration::from_nanos(expires - now)),
                );
            }
            result => return result,
        }
    }
}

pub(super) fn run(path: &Path) {
    let config: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let directory = path.parent().unwrap();
    let expires = config["expires_at_mono_ns"].as_u64().unwrap();
    let run_id = config["run_id"].as_i64().unwrap();
    let conn = crate::db::connect(config["database_url"].as_str().unwrap()).unwrap();
    conn.busy_timeout(Duration::ZERO).unwrap();
    let stdin = std::io::stdin();
    let mut terminal = tcgetattr(&stdin).unwrap();
    terminal.local_flags.remove(LocalFlags::ECHO);
    tcsetattr(&stdin, SetArg::TCSANOW, &terminal).unwrap();
    let flags = OFlag::from_bits_retain(fcntl(&stdin, FcntlArg::F_GETFL).unwrap());
    fcntl(&stdin, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).unwrap();
    let mut journal = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join("service-input-lines.jsonl"))
        .unwrap();
    private_write(
        &directory.join("service-input-native-identity.json"),
        &serde_json::to_vec(&ProcessIdentity::current().unwrap()).unwrap(),
    );
    let mut output = std::io::stdout().lock();
    output.write_all(b"review-native-ready\r\n").unwrap();
    output.write_all(SCREEN).unwrap();
    output.flush().unwrap();
    let mut input = stdin.lock();
    let mut pending = Vec::new();
    loop {
        assert!(
            Clock.sample().unwrap().monotonic_ns < expires,
            "original owned native lifetime"
        );
        let mut bytes = [0; 4096];
        match input.read(&mut bytes) {
            Ok(0) => return,
            Ok(count) => pending.extend_from_slice(&bytes[..count]),
            Err(cause)
                if matches!(cause.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
            Err(cause) => panic!("owned synthetic native input: {cause}"),
        }
        assert!(pending.len() <= 65536);
        while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let mut line = pending.drain(..=end).collect::<Vec<_>>();
            assert_eq!(line.pop(), Some(b'\n'));
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8(line).unwrap();
            if line == "finish" {
                return;
            }
            let inputs = receipt_inputs(&conn, run_id, expires).unwrap();
            journal
                .write_all(&serde_json::to_vec(&json!({"line":line,"inputs":inputs})).unwrap())
                .unwrap();
            journal.write_all(b"\n").unwrap();
            journal.sync_all().unwrap();
            output
                .write_all(format!("review-answer={line}\r\n").as_bytes())
                .unwrap();
            output.write_all(SCREEN).unwrap();
            output.flush().unwrap();
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
