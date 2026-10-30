use serde_json::Value;

use crate::config::Settings;
use crate::error::CafleetError;
use crate::multiplexer::AnyMultiplexer;
use crate::output::format_json;
use crate::runtime::resolve_mux;
use crate::runtime::system::read_stdin;

pub(crate) fn available_mux(settings: &Settings) -> Result<AnyMultiplexer, CafleetError> {
    let mux = resolve_mux(settings).map_err(|e| CafleetError::App(e.to_string()))?;
    mux.ensure_available()
        .map_err(|e| CafleetError::App(e.to_string()))?;
    Ok(mux)
}

pub(crate) fn checked(
    validate: fn(&str) -> Result<(), CafleetError>,
) -> impl Fn(&str) -> Result<String, String> + Clone + Send + Sync + 'static {
    move |value| {
        validate(value).map_err(|cause| cause.to_string())?;
        Ok(value.into())
    }
}

pub(crate) fn nonblank(
    message: &'static str,
) -> impl Fn(&str) -> Result<String, String> + Clone + Send + Sync + 'static {
    move |value| {
        if value.trim().is_empty() {
            Err(message.into())
        } else {
            Ok(value.into())
        }
    }
}

fn is_blank(text: &str) -> bool {
    text.chars()
        .all(|c| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

pub(crate) fn resolve_body(
    inline: Option<&str>,
    file: Option<&str>,
    flag: &str,
) -> Result<String, CafleetError> {
    let body = read_body(inline, file, flag)?;
    if !is_blank(&body) {
        return Ok(body);
    }
    Err(match file {
        None => CafleetError::Usage("text may not be empty.".to_string()),
        Some("-") => CafleetError::App(format!("{flag} -: stdin is empty.")),
        Some(path) => CafleetError::App(format!("{flag} {path}: file is empty.")),
    })
}

pub(crate) fn resolve_command_body(
    inline: Option<&str>,
    file: Option<&str>,
) -> Result<String, CafleetError> {
    let mut body = read_body(inline, file, "--file")?;
    if file.is_some() {
        if body.ends_with("\r\n") {
            body.truncate(body.len() - 2);
        } else if body.ends_with('\n') {
            body.pop();
        }
    }
    if !crate::input::valid_input(&body) {
        return Err(CafleetError::Usage(
            "command must be one non-empty line without control characters.".into(),
        ));
    }
    Ok(body)
}

fn read_body(inline: Option<&str>, file: Option<&str>, flag: &str) -> Result<String, CafleetError> {
    match (inline, file) {
        (Some(text), None) => Ok(text.to_string()),
        (None, Some("-")) => {
            let bytes = read_stdin().map_err(|e| CafleetError::App(format!("{flag} -: {e}")))?;
            String::from_utf8(bytes)
                .map_err(|_| CafleetError::App(format!("{flag} -: file is not valid UTF-8.")))
        }
        (None, Some(path)) => {
            let bytes = std::fs::read(path).map_err(|e| {
                let message = match e.kind() {
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::IsADirectory => {
                        format!("{flag} {path}: file does not exist or is not a regular file.")
                    }
                    _ => format!("{flag} {path}: file is not readable."),
                };
                CafleetError::App(format!("{message} ({e})"))
            })?;
            String::from_utf8(bytes)
                .map_err(|_| CafleetError::App(format!("{flag} {path}: file is not valid UTF-8.")))
        }
        (Some(_), Some(_)) | (None, None) => {
            unreachable!("clap's required argument group supplies exactly one body source")
        }
    }
}

pub(crate) fn emit(json: bool, payload: &Value, text: impl FnOnce() -> String) {
    if json {
        println!("{}", format_json(payload));
    } else {
        println!("{}", text());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_files_preserve_exact_bytes_and_report_invalid_sources() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("body.txt");
        let name = path.to_str().unwrap();
        for text in ["界🙂\nnext\r\n", "{literal}\ttext", " padded "] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(resolve_body(None, Some(name), "--file").unwrap(), text);
        }
        for text in ["", " \t\n", "\u{1c}\u{1f}", "\u{2003}"] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(
                resolve_body(Some(text), None, "--file")
                    .unwrap_err()
                    .exit_code(),
                2
            );
            assert_eq!(
                resolve_body(None, Some(name), "--file")
                    .unwrap_err()
                    .exit_code(),
                1
            );
        }
        std::fs::write(&path, [0xff]).unwrap();
        assert!(
            resolve_body(None, Some(name), "--file")
                .unwrap_err()
                .to_string()
                .contains("UTF-8")
        );
        std::fs::remove_file(&path).unwrap();
        assert!(
            resolve_body(None, Some(name), "--file")
                .unwrap_err()
                .to_string()
                .contains(name)
        );
        assert!(resolve_body(None, Some(dir.path().to_str().unwrap()), "--file").is_err());
    }

    #[test]
    fn native_command_files_strip_only_one_terminal_line_ending() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("command.txt");
        let name = path.to_str().unwrap();
        for (source, expected) in [
            ("printf '界🙂'", "printf '界🙂'"),
            ("echo literal\n", "echo literal"),
            ("echo literal\r\n", "echo literal"),
            ("  echo literal  \n", "  echo literal  "),
        ] {
            std::fs::write(&path, source).unwrap();
            assert_eq!(resolve_command_body(None, Some(name)).unwrap(), expected);
        }
        for source in [
            "",
            "  ",
            "echo x\n\n",
            "echo x\r",
            "echo\tx",
            "one\ntwo",
            "\0",
            "\u{7f}",
        ] {
            std::fs::write(&path, source).unwrap();
            assert!(
                resolve_command_body(None, Some(name)).is_err(),
                "{source:?}"
            );
        }
        assert_eq!(
            resolve_command_body(Some("echo literal"), None).unwrap(),
            "echo literal"
        );
        assert!(resolve_command_body(Some("echo literal\n"), None).is_err());
    }
}
