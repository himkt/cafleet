//! Rust reference for gate G3: replays replay cases through vt100 0.18.1 with cafleet's
//! per-step splitting (`TerminalState::feed_observed_classified`) and reports the canonical
//! screen dump, its FNV-1a hash, or the rendered text after every observation step.
//!
//! Usage:
//!   vtoracle hash CASES.jsonl                 one line per case: name, then one hash or PANIC per step
//!   vtoracle layout CASES.jsonl               as hash, over the dump without rendered text
//!   vtoracle dump CASES.jsonl NAME STEP       the canonical dump after step STEP of case NAME
//!   vtoracle final CASES.jsonl                the canonical dump after the last step of every case
//!   vtoracle screens CASES.jsonl              JSONL of every distinct rendered screen per case
//!   vtoracle strwidth STRINGS.txt             UnicodeWidthStr::width per hex-encoded UTF-8 line
//!   vtoracle charwidths OUT.bin               one byte per scalar value 0..0x110000 ('s' surrogate, 'n' none, '0'..'2')
//!
//! A case is one JSON object per line: {"name", "rows", "cols", "steps": [{"hex"} | {"resize": [rows, cols]}]}.
use serde_json::Value;
use std::fmt::Write as _;
use std::io::{BufRead, Write};
use std::num::NonZeroU16;
use std::panic::{AssertUnwindSafe, catch_unwind};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Default, Clone)]
struct Cb {
    title: Vec<u8>,
    icon: Vec<u8>,
}

impl vt100::Callbacks for Cb {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = title.to_vec();
    }
    fn set_window_icon_name(&mut self, _: &mut vt100::Screen, name: &[u8]) {
        self.icon = name.to_vec();
    }
}

fn hex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "odd-length hex string");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex digit pair"))
        .collect()
}

fn tohex(out: &mut String, b: &[u8]) {
    for x in b {
        write!(out, "{x:02x}").unwrap();
    }
}

fn color(out: &mut String, c: vt100::Color) {
    match c {
        vt100::Color::Default => out.push('d'),
        vt100::Color::Idx(i) => write!(out, "i{i}").unwrap(),
        vt100::Color::Rgb(r, g, b) => write!(out, "r{r:02x}{g:02x}{b:02x}").unwrap(),
    }
}

fn mode_bits(b: bool, d: bool, i: bool, u: bool, v: bool) -> u8 {
    u8::from(b) | u8::from(d) << 1 | u8::from(i) << 2 | u8::from(u) << 3 | u8::from(v) << 4
}

fn dump(p: &vt100::Parser<Cb>, out: &mut String) {
    dump_with(p, out, true);
}

/// The canonical dump; without `text` it omits every byte of rendered text (title, icon,
/// contents, row strings and cell contents) and keeps the layout and attributes.
fn dump_with(p: &vt100::Parser<Cb>, out: &mut String, text: bool) {
    let s = p.screen();
    let (r, c) = s.size();
    let (r, c) = (r.get(), c.get());
    let (cr, cc) = s.cursor_position();
    writeln!(out, "size {r} {c}\ncursor {cr} {cc}").unwrap();
    let mouse = match s.mouse_protocol_mode() {
        vt100::MouseProtocolMode::None => 0,
        vt100::MouseProtocolMode::Press => 1,
        vt100::MouseProtocolMode::PressRelease => 2,
        vt100::MouseProtocolMode::ButtonMotion => 3,
        vt100::MouseProtocolMode::AnyMotion => 4,
    };
    let enc = match s.mouse_protocol_encoding() {
        vt100::MouseProtocolEncoding::Default => 0,
        vt100::MouseProtocolEncoding::Utf8 => 1,
        vt100::MouseProtocolEncoding::Sgr => 2,
    };
    let style = match s.cursor_style() {
        vt100::CursorStyle::Default => 0,
        vt100::CursorStyle::BlinkBlock => 1,
        vt100::CursorStyle::SteadyBlock => 2,
        vt100::CursorStyle::BlinkUnderline => 3,
        vt100::CursorStyle::SteadyUnderline => 4,
        vt100::CursorStyle::BlinkVerticalBar => 5,
        vt100::CursorStyle::SteadyVerticalBar => 6,
    };
    writeln!(
        out,
        "modes {} {} {} {} {} {mouse} {enc} {style}",
        u8::from(s.alternate_screen()),
        u8::from(s.bracketed_paste()),
        u8::from(s.hide_cursor()),
        u8::from(s.application_keypad()),
        u8::from(s.application_cursor())
    )
    .unwrap();
    out.push_str("attrs ");
    color(out, s.fgcolor());
    out.push(' ');
    color(out, s.bgcolor());
    writeln!(
        out,
        " {:02x}",
        mode_bits(s.bold(), s.dim(), s.italic(), s.underline(), s.inverse())
    )
    .unwrap();
    if text {
        out.push_str("title ");
        tohex(out, &p.callbacks().title);
        out.push_str(" icon ");
        tohex(out, &p.callbacks().icon);
        out.push_str("\ncontents ");
        tohex(out, s.contents().as_bytes());
        out.push('\n');
    }
    for (row, row_text) in s.rows(0, c).enumerate() {
        let row = u16::try_from(row).expect("row index fits u16");
        write!(out, "row {row} {} ", u8::from(s.row_wrapped(row))).unwrap();
        if text {
            tohex(out, row_text.as_bytes());
        }
        out.push('\n');
        for col in 0..c {
            let cell = s.cell(row, col).expect("cell in range");
            let bits = mode_bits(
                cell.bold(),
                cell.dim(),
                cell.italic(),
                cell.underline(),
                cell.inverse(),
            );
            let plain = !cell.has_contents()
                && !cell.is_wide()
                && !cell.is_wide_continuation()
                && cell.fgcolor() == vt100::Color::Default
                && cell.bgcolor() == vt100::Color::Default
                && bits == 0;
            if plain {
                continue;
            }
            write!(
                out,
                "cell {row} {col} {}{}{} ",
                u8::from(cell.has_contents()),
                u8::from(cell.is_wide()),
                u8::from(cell.is_wide_continuation())
            )
            .unwrap();
            color(out, cell.fgcolor());
            out.push(' ');
            color(out, cell.bgcolor());
            write!(out, " {bits:02x} ").unwrap();
            if text {
                tohex(out, cell.contents().as_bytes());
            }
            out.push('\n');
        }
    }
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

enum Step {
    Bytes(Vec<u8>),
    Resize(u16, u16),
}

fn dimension(v: &Value) -> u16 {
    u16::try_from(v.as_u64().expect("dimension is an unsigned integer"))
        .expect("dimension fits u16")
}

fn substeps(case: &Value) -> Vec<Step> {
    let mut out = Vec::new();
    for step in case["steps"].as_array().expect("steps array") {
        if let Some(h) = step.get("hex") {
            let bytes = hex(h.as_str().expect("hex string"));
            let mut start = 0;
            for (i, b) in bytes.iter().enumerate() {
                if b.is_ascii_control() {
                    if i > start {
                        out.push(Step::Bytes(bytes[start..i].to_vec()));
                    }
                    out.push(Step::Bytes(bytes[i..i + 1].to_vec()));
                    start = i + 1;
                }
            }
            if start < bytes.len() {
                out.push(Step::Bytes(bytes[start..].to_vec()));
            }
        } else if let Some(r) = step.get("resize") {
            out.push(Step::Resize(dimension(&r[0]), dimension(&r[1])));
        } else {
            panic!("step is neither hex nor resize: {step}");
        }
    }
    out
}

fn nz(v: u16) -> NonZeroU16 {
    NonZeroU16::new(v).expect("nonzero size")
}

/// Runs a case, calling `each` with the step index and parser after each step; `None` marks a
/// panic, after which the case ends. `each` returns false to stop early.
fn run(case: &Value, mut each: impl FnMut(usize, Option<&vt100::Parser<Cb>>) -> bool) {
    let mut p = vt100::Parser::new_with_callbacks(
        nz(dimension(&case["rows"])),
        nz(dimension(&case["cols"])),
        0,
        Cb::default(),
    );
    for (i, step) in substeps(case).into_iter().enumerate() {
        let ok = catch_unwind(AssertUnwindSafe(|| match &step {
            Step::Bytes(b) => p.process(b),
            Step::Resize(r, c) => p.set_size(nz(*r), nz(*c)),
        }))
        .is_ok();
        if !ok {
            each(i, None);
            return;
        }
        if !each(i, Some(&p)) {
            return;
        }
    }
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn screen_record(name: &str, step: usize, p: &vt100::Parser<Cb>) -> Value {
    let s = p.screen();
    let cols = s.size().1.get();
    let rows: Vec<String> = s.rows(0, cols).collect();
    serde_json::json!({
        "name": name,
        "step": step,
        "title": lossy(&p.callbacks().title),
        "icon": lossy(&p.callbacks().icon),
        "contents": s.contents(),
        "rows": rows,
    })
}

fn for_each_case(path: &str, mut f: impl FnMut(&str, &Value)) {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("open {path}: {e}"));
    for line in std::io::BufReader::new(file).lines() {
        let line = line.expect("read case line");
        if line.trim().is_empty() {
            continue;
        }
        let case: Value = serde_json::from_str(&line).expect("case is JSON");
        let name = case["name"].as_str().expect("case name").to_owned();
        f(&name, &case);
    }
}

fn hex_lines(path: &str) -> Vec<String> {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("open {path}: {e}"));
    std::io::BufReader::new(file)
        .lines()
        .map(|line| {
            let bytes = hex(&line.expect("read line"));
            String::from_utf8(bytes).expect("valid UTF-8 string")
        })
        .collect()
}

fn main() {
    std::panic::set_hook(Box::new(|info| eprintln!("oracle panic: {info}")));
    let args: Vec<String> = std::env::args().collect();
    assert!(args.len() >= 3, "usage: vtoracle MODE PATH [ARGS]");
    let (mode, path) = (args[1].as_str(), args[2].as_str());
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let mut buf = String::new();
    match mode {
        "hash" => for_each_case(path, |name, case| {
            write!(out, "{name}").unwrap();
            run(case, |_, p| {
                match p {
                    Some(p) => {
                        buf.clear();
                        dump(p, &mut buf);
                        write!(out, " {:016x}", fnv(&buf)).unwrap();
                    }
                    None => write!(out, " PANIC").unwrap(),
                }
                true
            });
            writeln!(out).unwrap();
        }),
        "layout" => for_each_case(path, |name, case| {
            write!(out, "{name}").unwrap();
            run(case, |_, p| {
                match p {
                    Some(p) => {
                        buf.clear();
                        dump_with(p, &mut buf, false);
                        write!(out, " {:016x}", fnv(&buf)).unwrap();
                    }
                    None => write!(out, " PANIC").unwrap(),
                }
                true
            });
            writeln!(out).unwrap();
        }),
        "dump" => {
            assert!(args.len() == 5, "usage: vtoracle dump CASES NAME STEP");
            let want: usize = args[4].parse().expect("step index");
            let mut found = false;
            for_each_case(path, |name, case| {
                if name != args[3] {
                    return;
                }
                found = true;
                run(case, |i, p| {
                    if i < want {
                        return true;
                    }
                    match p {
                        Some(p) => {
                            buf.clear();
                            dump(p, &mut buf);
                            out.write_all(buf.as_bytes()).unwrap();
                        }
                        None => writeln!(out, "PANIC").unwrap(),
                    }
                    false
                });
            });
            assert!(found, "no case named {}", args[3]);
        }
        "final" => for_each_case(path, |name, case| {
            let mut last = String::new();
            run(case, |_, p| {
                last.clear();
                match p {
                    Some(p) => dump(p, &mut last),
                    None => last.push_str("PANIC\n"),
                }
                true
            });
            write!(out, "=== {name}\n{last}").unwrap();
        }),
        "screens" => for_each_case(path, |name, case| {
            let mut previous = String::new();
            run(case, |i, p| {
                let Some(p) = p else {
                    writeln!(
                        out,
                        "{}",
                        serde_json::json!({"name": name, "step": i, "panic": true})
                    )
                    .unwrap();
                    return true;
                };
                let record = screen_record(name, i, p);
                let key = serde_json::json!([record["title"], record["icon"], record["rows"]])
                    .to_string();
                if key != previous {
                    writeln!(out, "{record}").unwrap();
                    previous = key;
                }
                true
            });
        }),
        "strwidth" => {
            for s in hex_lines(path) {
                writeln!(out, "{}", s.width()).unwrap();
            }
        }
        "charwidths" => {
            let table: Vec<u8> = (0u32..0x110000)
                .map(|cp| match char::from_u32(cp) {
                    None => b's',
                    Some(c) => match c.width() {
                        None => b'n',
                        Some(w) => b'0' + u8::try_from(w.min(2)).expect("width fits u8"),
                    },
                })
                .collect();
            std::fs::write(path, table).unwrap_or_else(|e| panic!("write {path}: {e}"));
        }
        _ => panic!("unknown mode {mode}"),
    }
}
