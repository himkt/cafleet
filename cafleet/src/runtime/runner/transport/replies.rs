use std::time::{Duration, Instant};

pub(crate) const REPLY_PREFIX_BOUND: Duration = Duration::from_millis(100);
const CSI_PARAMETER_LIMIT: usize = 64;
const STRING_BODY_LIMIT: usize = 4096;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const PASTE_END: &[u8] = b"\x1b[201~";

#[derive(Clone, Copy, PartialEq, Eq)]
enum StringKind {
    Osc,
    Dcs,
    Apc,
}

#[derive(Default)]
enum State {
    #[default]
    Ground,
    Escape,
    Csi {
        parameters: Vec<u8>,
        intermediate: bool,
    },
    StringBody {
        kind: StringKind,
        length: usize,
    },
    StringEscape,
}

/// Decides how many forwarded outer-terminal bytes count as manual input.
/// It never holds bytes: forwarding happens before and independently of it.
#[derive(Default)]
pub(super) struct ReplyClassifier {
    state: State,
    pending: usize,
    last_byte: Option<Instant>,
    paste_end_matched: Option<usize>,
}

pub(super) struct Classified {
    pub(super) manual: usize,
}

impl ReplyClassifier {
    pub(super) fn feed(&mut self, bytes: &[u8], now: Instant) -> Classified {
        let manual = bytes.iter().map(|&byte| self.classify(byte, now)).sum();
        Classified { manual }
    }

    pub(super) fn expire(&mut self, now: Instant) -> Classified {
        let manual = match self.last_byte {
            Some(last) if now.saturating_duration_since(last) >= REPLY_PREFIX_BOUND => {
                self.resolve_manual()
            }
            _ => 0,
        };
        Classified { manual }
    }

    pub(super) fn finish(&mut self) -> Classified {
        Classified {
            manual: self.resolve_manual(),
        }
    }

    pub(super) fn pending(&self) -> bool {
        self.pending > 0
    }

    fn classify(&mut self, byte: u8, now: Instant) -> usize {
        let manual = match self.paste_end_matched {
            Some(matched) => {
                self.advance_paste(matched, byte);
                1
            }
            None => self.step(byte),
        };
        self.last_byte = self.pending().then_some(now);
        manual
    }

    fn advance_paste(&mut self, matched: usize, byte: u8) {
        let matched = if byte == PASTE_END[matched] {
            matched + 1
        } else if byte == ESC {
            1
        } else {
            0
        };
        self.paste_end_matched = (matched < PASTE_END.len()).then_some(matched);
    }

    fn step(&mut self, byte: u8) -> usize {
        match std::mem::take(&mut self.state) {
            State::Ground => self.ground(byte),
            State::Escape => match byte {
                b'[' => self.extend(State::Csi {
                    parameters: Vec::new(),
                    intermediate: false,
                }),
                b']' => self.open_string(StringKind::Osc),
                b'P' => self.open_string(StringKind::Dcs),
                b'_' => self.open_string(StringKind::Apc),
                _ => self.invalid(byte),
            },
            State::Csi {
                mut parameters,
                intermediate,
            } => match byte {
                0x20..=0x3f if parameters.len() == CSI_PARAMETER_LIMIT => self.invalid(byte),
                0x30..=0x3f if !intermediate => {
                    parameters.push(byte);
                    self.extend(State::Csi {
                        parameters,
                        intermediate,
                    })
                }
                0x20..=0x2f => {
                    parameters.push(byte);
                    self.extend(State::Csi {
                        parameters,
                        intermediate: true,
                    })
                }
                0x40..=0x7e => self.complete_csi(&parameters, byte),
                _ => self.invalid(byte),
            },
            State::StringBody { kind, length } => match byte {
                0x20..=0x7e | 0x80..=0xff if length == STRING_BODY_LIMIT => self.invalid(byte),
                0x20..=0x7e | 0x80..=0xff => self.extend(State::StringBody {
                    kind,
                    length: length + 1,
                }),
                BEL if kind == StringKind::Osc => self.complete_reply(),
                ESC => self.extend(State::StringEscape),
                _ => self.invalid(byte),
            },
            State::StringEscape => match byte {
                b'\\' => self.complete_reply(),
                _ => self.invalid(byte),
            },
        }
    }

    fn ground(&mut self, byte: u8) -> usize {
        if byte == ESC {
            self.state = State::Escape;
            self.pending = 1;
            0
        } else {
            1
        }
    }

    fn open_string(&mut self, kind: StringKind) -> usize {
        self.extend(State::StringBody { kind, length: 0 })
    }

    fn extend(&mut self, state: State) -> usize {
        self.state = state;
        self.pending += 1;
        0
    }

    fn invalid(&mut self, byte: u8) -> usize {
        self.resolve_manual() + self.ground(byte)
    }

    fn resolve_manual(&mut self) -> usize {
        self.state = State::Ground;
        self.last_byte = None;
        std::mem::take(&mut self.pending)
    }

    fn complete_reply(&mut self) -> usize {
        self.pending = 0;
        0
    }

    fn complete_csi(&mut self, parameters: &[u8], final_byte: u8) -> usize {
        let length = std::mem::take(&mut self.pending) + 1;
        if is_csi_reply(parameters, final_byte) {
            return 0;
        }
        if parameters == b"200" && final_byte == b'~' {
            self.paste_end_matched = Some(0);
        }
        length
    }
}

fn is_csi_reply(parameters: &[u8], final_byte: u8) -> bool {
    let (marker, rest) = match parameters.split_first() {
        Some((&marker @ (b'?' | b'>'), rest)) => (Some(marker), rest),
        _ => (None, parameters),
    };
    let split = rest
        .iter()
        .position(|byte| (0x20..=0x2f).contains(byte))
        .unwrap_or(rest.len());
    let (list, intermediates) = rest.split_at(split);
    let arity = |min, max| parameter_count(list).is_some_and(|count| (min..=max).contains(&count));
    match (marker, intermediates, final_byte) {
        (None, b"", b'R') => arity(2, 2),
        (Some(b'?'), b"", b'R') => arity(2, 3),
        (None, b"", b'n') => list == b"0" || list == b"3",
        (Some(b'?'), b"", b'n') => arity(1, 4),
        (Some(b'?'), b"", b'c') => arity(1, 32),
        (Some(b'>'), b"", b'c') => arity(1, 4),
        (Some(b'?') | None, b"$", b'y') => arity(2, 2),
        (None, b"", b't') => arity(1, 3),
        (Some(b'?'), b"", b'u') => arity(1, 1),
        (None, b"", b'I' | b'O') => list.is_empty(),
        _ => false,
    }
}

/// Counts `;`-joined digit parameters, or `None` when any parameter is empty
/// or holds a non-digit byte.
fn parameter_count(list: &[u8]) -> Option<usize> {
    list.split(|&byte| byte == b';')
        .map(|parameter| !parameter.is_empty() && parameter.iter().all(u8::is_ascii_digit))
        .try_fold(0, |count, valid| valid.then_some(count + 1))
}

#[cfg(test)]
impl ReplyClassifier {
    pub(super) fn backdate(&mut self, by: Duration) {
        let last = self
            .last_byte
            .expect("backdating requires a pending reply prefix");
        self.last_byte = Some(
            last.checked_sub(by)
                .expect("backdated reply prefix instant is representable"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn classifier() -> ReplyClassifier {
        ReplyClassifier::default()
    }

    fn feed_once(bytes: &[u8]) -> (usize, bool) {
        let mut classifier = classifier();
        let manual = classifier.feed(bytes, Instant::now()).manual;
        (manual, classifier.pending())
    }

    fn assert_reply(bytes: &[u8]) {
        assert_eq!(
            feed_once(bytes),
            (0, false),
            "{:?} is a complete allowlisted reply",
            String::from_utf8_lossy(bytes)
        );
    }

    fn assert_manual(bytes: &[u8]) {
        assert_eq!(
            feed_once(bytes),
            (bytes.len(), false),
            "{:?} counts every byte as manual",
            String::from_utf8_lossy(bytes)
        );
    }

    fn joined(params: &[&str]) -> String {
        params.join(";")
    }

    mod allowlist {
        use super::*;

        #[test]
        fn reply_classifier_cursor_position_reports_are_replies() {
            assert_reply(b"\x1b[24;80R");
            assert_reply(b"\x1b[1;1R");
            assert_reply(b"\x1b[?24;80R");
            assert_reply(b"\x1b[?24;80;1R");
        }

        #[test]
        fn reply_classifier_modified_f3_collides_with_a_cursor_position_report() {
            assert_reply(b"\x1b[1;5R");
        }

        #[test]
        fn reply_classifier_device_status_reports_are_replies() {
            assert_reply(b"\x1b[0n");
            assert_reply(b"\x1b[3n");
            assert_reply(b"\x1b[?1n");
            assert_reply(b"\x1b[?997;1n");
            assert_reply(b"\x1b[?1;2;3;4n");
        }

        #[test]
        fn reply_classifier_device_attribute_reports_are_replies() {
            assert_reply(b"\x1b[?1;2c");
            assert_reply(b"\x1b[?62;22c");
            assert_reply(b"\x1b[>0c");
            assert_reply(b"\x1b[>1;10;0c");
            assert_reply(b"\x1b[>41;380;0;1c");
        }

        #[test]
        fn reply_classifier_primary_device_attributes_accept_thirty_two_parameters() {
            let reply = format!("\x1b[?{}c", joined(&["1"; 32]));
            assert_eq!(
                reply.len() - 3,
                CSI_PARAMETER_LIMIT,
                "the private marker and 32 parameters fill the CSI parameter limit exactly"
            );
            assert_reply(reply.as_bytes());
        }

        #[test]
        fn reply_classifier_mode_reports_are_replies() {
            assert_reply(b"\x1b[?1004;1$y");
            assert_reply(b"\x1b[?2026;2$y");
            assert_reply(b"\x1b[4;2$y");
        }

        #[test]
        fn reply_classifier_window_reports_are_replies() {
            assert_reply(b"\x1b[1t");
            assert_reply(b"\x1b[2t");
            assert_reply(b"\x1b[4;600;800t");
            assert_reply(b"\x1b[8;24;80t");
        }

        #[test]
        fn reply_classifier_kitty_keyboard_flags_report_is_a_reply() {
            assert_reply(b"\x1b[?1u");
            assert_reply(b"\x1b[?31u");
        }

        #[test]
        fn reply_classifier_focus_events_are_replies() {
            assert_reply(b"\x1b[I");
            assert_reply(b"\x1b[O");
        }

        #[test]
        fn reply_classifier_osc_replies_terminate_with_bel_or_st() {
            assert_reply(b"\x1b]11;rgb:0000/0000/0000\x07");
            assert_reply(b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\");
            assert_reply(b"\x1b]4;1;rgb:cdcd/0000/0000\x1b\\");
        }

        #[test]
        fn reply_classifier_osc_body_accepts_bytes_at_or_above_0x80() {
            assert_reply("\x1b]2;caf\u{e9}\x07".as_bytes());
        }

        #[test]
        fn reply_classifier_dcs_replies_terminate_with_st() {
            assert_reply(b"\x1bP>|XTerm(380)\x1b\\");
            assert_reply(b"\x1bP1+r544e=787465726d\x1b\\");
            assert_reply(b"\x1bP1$r0m\x1b\\");
            assert_reply(b"\x1bP!|00000000\x1b\\");
        }

        #[test]
        fn reply_classifier_apc_replies_terminate_with_st() {
            assert_reply(b"\x1b_Gi=31;OK\x1b\\");
            assert_reply(b"\x1b_Gi=1;ENOENT:file not found\x1b\\");
        }

        #[test]
        fn reply_classifier_string_body_at_the_limit_is_a_reply() {
            let mut reply = b"\x1b]".to_vec();
            reply.extend(std::iter::repeat_n(b'x', STRING_BODY_LIMIT));
            reply.push(0x07);
            assert_reply(&reply);
        }

        #[test]
        fn reply_classifier_a_burst_of_replies_counts_zero() {
            assert_reply(
                b"\x1b[I\x1b]11;rgb:0000/0000/0000\x07\x1b[24;80R\x1b[O\x1bP>|XTerm(380)\x1b\\",
            );
        }
    }

    mod arity {
        use super::*;

        #[test]
        fn reply_classifier_window_report_with_four_parameters_is_manual() {
            assert_manual(b"\x1b[8;24;80;1t");
        }

        #[test]
        fn reply_classifier_cursor_position_report_with_one_parameter_is_manual() {
            assert_manual(b"\x1b[5R");
            assert_manual(b"\x1b[?5R");
            assert_manual(b"\x1b[?1;2;3;4R");
        }

        #[test]
        fn reply_classifier_empty_parameters_are_manual() {
            assert_manual(b"\x1b[8;t");
            assert_manual(b"\x1b[;24R");
            assert_manual(b"\x1b[24;;80R");
            assert_manual(b"\x1b[?n");
            assert_manual(b"\x1b[?c");
        }

        #[test]
        fn reply_classifier_device_status_outside_its_forms_is_manual() {
            assert_manual(b"\x1b[1n");
            assert_manual(b"\x1b[5n");
            assert_manual(b"\x1b[?1;2;3;4;5n");
        }

        #[test]
        fn reply_classifier_device_attributes_outside_their_arity_are_manual() {
            assert_manual(b"\x1b[>1;2;3;4;5c");
            assert_manual(b"\x1b[1c");
        }

        #[test]
        fn reply_classifier_mode_reports_outside_their_arity_are_manual() {
            assert_manual(b"\x1b[?1$y");
            assert_manual(b"\x1b[?1;2;3$y");
            assert_manual(b"\x1b[4$y");
        }

        #[test]
        fn reply_classifier_kitty_flags_with_more_than_one_parameter_are_manual() {
            assert_manual(b"\x1b[?1;2u");
        }

        #[test]
        fn reply_classifier_focus_with_parameters_or_intermediates_is_manual() {
            assert_manual(b"\x1b[1I");
            assert_manual(b"\x1b[?I");
            assert_manual(b"\x1b[ O");
        }

        #[test]
        fn reply_classifier_unlisted_intermediate_is_manual() {
            assert_manual(b"\x1b[1;1 R");
            assert_manual(b"\x1b[1$t");
        }
    }

    mod manual {
        use super::*;

        #[test]
        fn reply_classifier_text_and_c0_controls_are_manual_immediately() {
            assert_manual(b"hello");
            assert_manual("caf\u{e9} \u{65e5}\u{672c}".as_bytes());
            assert_manual(b"\r");
            assert_manual(b"\t");
            assert_manual(b"\x03");
            assert_manual(b"\x7f");
            assert_manual(b"\x07");
        }

        #[test]
        fn reply_classifier_cursor_and_function_keys_are_manual() {
            assert_manual(b"\x1b[A");
            assert_manual(b"\x1b[1;5C");
            assert_manual(b"\x1b[3~");
            assert_manual(b"\x1b[15;2~");
            assert_manual(b"\x1b[Z");
        }

        #[test]
        fn reply_classifier_kitty_key_events_are_manual() {
            assert_manual(b"\x1b[97u");
            assert_manual(b"\x1b[97;5u");
            assert_manual(b"\x1b[97;5;97u");
        }

        #[test]
        fn reply_classifier_mouse_reports_are_manual() {
            assert_manual(b"\x1b[<0;10;20M");
            assert_manual(b"\x1b[<0;10;20m");
            assert_manual(b"\x1b[<64;5;5M");
            assert_manual(b"\x1b[M !!");
        }

        #[test]
        fn reply_classifier_ss3_and_alt_modified_keys_are_manual() {
            assert_manual(b"\x1bOP");
            assert_manual(b"\x1bOA");
            assert_manual(b"\x1ba");
            assert_manual(b"\x1bx");
            assert_manual(b"\x1b\r");
        }

        #[test]
        fn reply_classifier_sos_and_pm_strings_are_manual() {
            assert_manual(b"\x1bXhello\x1b\\");
            assert_manual(b"\x1b^hi\x1b\\");
        }

        #[test]
        fn reply_classifier_eight_bit_c1_bytes_are_manual() {
            assert_manual(&[0x9b]);
            assert_manual(&[0x9d]);
            assert_manual(&[0x90]);
            assert_manual(&[0x9f]);
            assert_manual(b"\x9b1;1R");
            assert_manual(b"\x9bI");
        }

        #[test]
        fn reply_classifier_overlong_csi_is_manual_at_the_byte_exceeding_the_limit() {
            let mut prefix = b"\x1b[".to_vec();
            prefix.extend(std::iter::repeat_n(b'1', CSI_PARAMETER_LIMIT));
            let mut classifier = classifier();
            let now = Instant::now();
            assert_eq!(classifier.feed(&prefix, now).manual, 0);
            assert!(classifier.pending());
            assert_eq!(
                classifier.feed(b"1", now).manual,
                prefix.len() + 1,
                "the parameter byte that exceeds the limit resolves the whole prefix"
            );
            assert!(!classifier.pending());
            assert_eq!(classifier.feed(b"R", now).manual, 1);
        }

        #[test]
        fn reply_classifier_overlong_csi_in_one_read_counts_every_byte() {
            let mut input = b"\x1b[?".to_vec();
            input.extend(joined(&["1"; 40]).bytes());
            input.push(b'c');
            assert!(input.len() - 3 > CSI_PARAMETER_LIMIT);
            assert_manual(&input);
        }

        #[test]
        fn reply_classifier_osc_body_over_the_limit_is_manual() {
            let mut prefix = b"\x1b]".to_vec();
            prefix.extend(std::iter::repeat_n(b'x', STRING_BODY_LIMIT));
            let mut classifier = classifier();
            let now = Instant::now();
            assert_eq!(classifier.feed(&prefix, now).manual, 0);
            assert!(classifier.pending());
            assert_eq!(
                classifier.feed(b"x", now).manual,
                prefix.len() + 1,
                "the body byte that exceeds the limit resolves the whole prefix"
            );
            assert!(!classifier.pending());
            assert_eq!(classifier.feed(b"\x07", now).manual, 1);
        }

        #[test]
        fn reply_classifier_osc_52_clipboard_read_over_the_limit_counts_every_byte() {
            let mut input = b"\x1b]52;c;".to_vec();
            input.extend(std::iter::repeat_n(b'A', STRING_BODY_LIMIT));
            input.push(0x07);
            assert_manual(&input);
        }

        #[test]
        fn reply_classifier_dcs_and_apc_bodies_over_the_limit_count_every_byte() {
            for introducer in [b"\x1bP", b"\x1b_"] {
                let mut input = introducer.to_vec();
                input.extend(std::iter::repeat_n(b'x', STRING_BODY_LIMIT + 1));
                input.extend(b"\x1b\\");
                assert_manual(&input);
            }
        }

        #[test]
        fn reply_classifier_mixed_read_counts_only_the_text_around_a_reply() {
            assert_eq!(feed_once(b"ab\x1b[Icd"), (4, false));
        }

        #[test]
        fn reply_classifier_bracketed_paste_counts_every_byte_including_reply_forms() {
            assert_manual(b"\x1b[200~\x1b[1;1R\x1b[Ipasted\x1b]11;x\x07\x1b[201~");
        }

        #[test]
        fn reply_classifier_bracketed_paste_split_across_reads_counts_every_byte() {
            let input: &[u8] = b"\x1b[200~one\x1b[1;1R\x1b[Itwo\x1b[201~";
            let mut classifier = classifier();
            let now = Instant::now();
            let manual: usize = input
                .chunks(3)
                .map(|chunk| classifier.feed(chunk, now).manual)
                .sum();
            assert_eq!(manual, input.len());
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_reply_after_bracketed_paste_counts_zero() {
            let paste: &[u8] = b"\x1b[200~text\x1b[201~";
            let mut classifier = classifier();
            let now = Instant::now();
            assert_eq!(classifier.feed(paste, now).manual, paste.len());
            assert_eq!(classifier.feed(b"\x1b[I", now).manual, 0);
            assert!(!classifier.pending());
        }
    }

    mod invalid_continuation {
        use super::*;

        #[test]
        fn reply_classifier_escape_inside_csi_restarts_a_reply() {
            assert_eq!(feed_once(b"\x1b[1;\x1b[I"), (4, false));
        }

        #[test]
        fn reply_classifier_escape_before_a_reply_counts_one() {
            assert_eq!(feed_once(b"\x1b\x1b[I"), (1, false));
        }

        #[test]
        fn reply_classifier_control_bytes_inside_csi_count_the_prefix_and_the_byte() {
            for byte in [0x18, 0x1a, 0x00, 0x03, 0x07, 0x0d, 0x7f, 0x80, 0xc3, 0xff] {
                let input = [0x1b, b'[', b'1', byte];
                assert_eq!(feed_once(&input), (4, false), "byte {byte:#04x}");
            }
        }

        #[test]
        fn reply_classifier_control_byte_inside_csi_is_followed_by_classification_from_ground() {
            assert_eq!(feed_once(b"\x1b[1\x18\x1b[I"), (4, false));
        }

        #[test]
        fn reply_classifier_parameter_byte_after_an_intermediate_is_manual() {
            assert_manual(b"\x1b[1$1y");
        }

        #[test]
        fn reply_classifier_bel_ends_an_osc_reply() {
            assert_reply(b"\x1b]11;x\x07");
        }

        #[test]
        fn reply_classifier_bel_inside_dcs_or_apc_is_manual() {
            assert_manual(b"\x1bP1$r\x07");
            assert_manual(b"\x1b_Gi=1\x07");
        }

        #[test]
        fn reply_classifier_bel_inside_dcs_resolves_before_a_following_reply() {
            assert_eq!(feed_once(b"\x1bP1$r\x07\x1b[I"), (6, false));
        }

        #[test]
        fn reply_classifier_escape_without_backslash_inside_osc_is_manual() {
            assert_manual(b"\x1b]11;x\x1ba");
            assert_manual(b"\x1b]1\x1b[I");
        }

        #[test]
        fn reply_classifier_escape_without_backslash_inside_osc_reclassifies_the_next_byte() {
            assert_eq!(feed_once(b"\x1b]1\x1b\x1b[I"), (4, false));
        }

        #[test]
        fn reply_classifier_escape_without_backslash_inside_dcs_or_apc_is_manual() {
            assert_manual(b"\x1bP1$r\x1bq");
            assert_manual(b"\x1b_G\x1bq");
        }

        #[test]
        fn reply_classifier_control_byte_inside_a_string_body_is_manual() {
            assert_manual(b"\x1b]11;x\x18");
            assert_manual(b"\x1bP1\r");
        }

        #[test]
        fn reply_classifier_escape_with_an_invalid_continuation_counts_both_bytes() {
            assert_manual(b"\x1b1");
            assert_manual(b"\x1b\x7f");
        }
    }

    mod pending {
        use super::*;

        const SPLIT_REPLIES: [&[u8]; 7] = [
            b"\x1b[I",
            b"\x1b[24;80R",
            b"\x1b[?62;22c",
            b"\x1b]11;rgb:0000/0000/0000\x07",
            b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\",
            b"\x1bP>|XTerm(380)\x1b\\",
            b"\x1b_Gi=31;OK\x1b\\",
        ];

        #[test]
        fn reply_classifier_reply_split_at_every_boundary_counts_zero() {
            let now = Instant::now();
            for reply in SPLIT_REPLIES {
                for split in 1..reply.len() {
                    let mut classifier = classifier();
                    assert_eq!(classifier.feed(&reply[..split], now).manual, 0);
                    assert!(classifier.pending(), "{reply:?} split at {split}");
                    assert_eq!(classifier.feed(&reply[split..], now).manual, 0);
                    assert!(!classifier.pending(), "{reply:?} split at {split}");
                }
            }
        }

        #[test]
        fn reply_classifier_reply_fed_one_byte_per_read_counts_zero() {
            let now = Instant::now();
            for reply in SPLIT_REPLIES {
                let mut classifier = classifier();
                for (index, byte) in reply.iter().enumerate() {
                    assert_eq!(classifier.feed(&[*byte], now).manual, 0);
                    assert_eq!(
                        classifier.pending(),
                        index + 1 < reply.len(),
                        "{reply:?} after byte {index}"
                    );
                }
            }
        }

        #[test]
        fn reply_classifier_reply_split_across_reads_at_different_instants_counts_zero() {
            let start = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"\x1b[24", start).manual, 0);
            let later = start + REPLY_PREFIX_BOUND / 2;
            assert_eq!(classifier.expire(later).manual, 0);
            assert_eq!(classifier.feed(b";80R", later).manual, 0);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_lone_escape_resolves_as_manual_at_the_bound() {
            let start = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"\x1b", start).manual, 0);
            assert!(classifier.pending());
            let before = start + REPLY_PREFIX_BOUND - Duration::from_millis(1);
            assert_eq!(classifier.expire(before).manual, 0);
            assert!(classifier.pending());
            assert_eq!(classifier.expire(start + REPLY_PREFIX_BOUND).manual, 1);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_csi_prefix_resolves_as_manual_at_the_bound() {
            let start = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"\x1b[12;", start).manual, 0);
            assert!(classifier.pending());
            let before = start + REPLY_PREFIX_BOUND - Duration::from_millis(1);
            assert_eq!(classifier.expire(before).manual, 0);
            assert!(classifier.pending());
            assert_eq!(classifier.expire(start + REPLY_PREFIX_BOUND).manual, 5);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_bound_restarts_at_each_new_prefix_byte() {
            let start = Instant::now();
            let second = start + Duration::from_millis(60);
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"\x1b", start).manual, 0);
            assert_eq!(classifier.feed(b"[", second).manual, 0);
            assert_eq!(classifier.expire(start + REPLY_PREFIX_BOUND).manual, 0);
            assert!(classifier.pending());
            assert_eq!(classifier.expire(second + REPLY_PREFIX_BOUND).manual, 2);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_bytes_after_an_expired_prefix_classify_from_ground() {
            let start = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"\x1b", start).manual, 0);
            let expired = start + REPLY_PREFIX_BOUND;
            assert_eq!(classifier.expire(expired).manual, 1);
            assert_eq!(classifier.feed(b"[I", expired).manual, 2);
            assert_eq!(classifier.feed(b"\x1b[I", expired).manual, 0);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_backdate_ages_a_pending_prefix_to_the_bound() {
            let now = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"\x1b[", now).manual, 0);
            assert_eq!(classifier.expire(now).manual, 0);
            classifier.backdate(REPLY_PREFIX_BOUND);
            assert_eq!(classifier.expire(now).manual, 2);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_expire_without_a_pending_prefix_counts_zero() {
            let now = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.expire(now + REPLY_PREFIX_BOUND).manual, 0);
            assert_eq!(classifier.feed(b"\x1b[I", now).manual, 0);
            assert_eq!(classifier.expire(now + REPLY_PREFIX_BOUND * 2).manual, 0);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_finish_resolves_a_pending_prefix_as_manual() {
            let now = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"\x1b[1;", now).manual, 0);
            assert_eq!(classifier.finish().manual, 4);
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_finish_resolves_a_pending_string_body_as_manual() {
            let prefix: &[u8] = b"\x1b]11;rgb:00";
            let mut classifier = classifier();
            assert_eq!(classifier.feed(prefix, Instant::now()).manual, 0);
            assert_eq!(classifier.finish().manual, prefix.len());
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_finish_resolves_a_pending_string_escape_as_manual() {
            let prefix: &[u8] = b"\x1bP1$r0m\x1b";
            let mut classifier = classifier();
            assert_eq!(classifier.feed(prefix, Instant::now()).manual, 0);
            assert!(classifier.pending());
            assert_eq!(classifier.finish().manual, prefix.len());
            assert!(!classifier.pending());
        }

        #[test]
        fn reply_classifier_finish_without_a_pending_prefix_counts_zero() {
            let mut classifier = classifier();
            assert_eq!(classifier.finish().manual, 0);
            assert_eq!(classifier.feed(b"ab", Instant::now()).manual, 2);
            assert_eq!(classifier.finish().manual, 0);
        }

        #[test]
        fn reply_classifier_text_before_a_pending_prefix_counts_in_the_same_read() {
            let now = Instant::now();
            let mut classifier = classifier();
            assert_eq!(classifier.feed(b"ab\x1b[", now).manual, 2);
            assert!(classifier.pending());
            assert_eq!(classifier.feed(b"O", now).manual, 0);
            assert!(!classifier.pending());
        }
    }
}
