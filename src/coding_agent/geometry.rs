use crate::coding_agent::catalog::LiteralFraming;
use crate::coding_agent::catalog::TextWrap;
use unicode_width::UnicodeWidthChar;

/// How much text a composer shows at a pane size: its profile-level `composer.geometry` with a
/// rule's `text_geometry` fields applied.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ComposerGeometry {
    pub(crate) margin_columns: u16,
    pub(crate) first_row_margin_columns: u16,
    pub(crate) text_window: TextWindow,
    pub(crate) reserved_rows: ReservedRows,
    pub(crate) transient_rows: u16,
    pub(crate) wrap_slack_rows: u16,
    pub(crate) carried_space: bool,
    pub(crate) fit_wrap: TextWrap,
    pub(crate) collapsed_paste: Option<CollapsedPaste>,
    pub(crate) min_rows: u16,
    pub(crate) min_columns: u16,
}

/// The composer's internal window `W(rows)`, past which the native scrolls its text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TextWindow {
    Unbounded,
    /// `max(min, floor(fraction × rows) − offset)`.
    Fraction {
        fraction: f64,
        offset: u16,
        min: u16,
    },
    /// The window of the last `(min_rows, rows)` step at or below the pane height.
    Steps {
        steps: Vec<(u16, u16)>,
        uncapped_from: Option<u16>,
    },
}

/// The non-text rows extraction needs on screen at a pane height.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReservedRows {
    Fixed(u16),
    /// The reserved rows of the last `(min_rows, rows)` step at or below the pane height.
    Steps(Vec<(u16, u16)>),
}

/// A bracketed literal of at least `min_scalars` scalars renders as `rows` placeholder rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CollapsedPaste {
    pub(crate) min_scalars: u64,
    pub(crate) rows: u16,
}

/// Whether a fit keeps room for transient chrome: the full and compact rungs and prompt/exec
/// do, the short rungs do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Headroom {
    Zero,
    Transient,
}

/// A screen's live size and the effective geometry and wrap of the `composer.text` rule it
/// selects.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LiveComposer {
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) geometry: ComposerGeometry,
    pub(crate) wrap: TextWrap,
    pub(crate) measured_reserved_rows: Option<u16>,
    pub(crate) wrap_margin_columns: u16,
    pub(crate) visible_rows_cap: Option<u16>,
}

impl LiveComposer {
    /// Whether the rendered `literal` fits the visible text rows with `headroom`.
    pub(crate) fn fits(&self, literal: &str, framing: LiteralFraming, headroom: Headroom) -> bool {
        let extra = self
            .measured_reserved_rows
            .zip(self.geometry.reserved_rows_at(self.rows))
            .map_or(0, |(measured, baseline)| {
                usize::from(measured.saturating_sub(baseline))
            });
        let available = self
            .geometry
            .visible_text_rows(self.rows, headroom)
            .saturating_sub(extra);
        let available = self
            .visible_rows_cap
            .map_or(available, |cap| available.min(usize::from(cap)));
        self.geometry
            .fit_rows(
                literal,
                self.cols.saturating_sub(self.wrap_margin_columns),
                framing,
            )
            .is_some_and(|rows| rows <= available)
    }
}

/// The value of the last `(min_rows, value)` step at or below `rows`, or `None` below the
/// first step.
fn step_value(steps: &[(u16, u16)], rows: u16) -> Option<u16> {
    steps
        .iter()
        .rev()
        .find(|(min_rows, _)| *min_rows <= rows)
        .map(|(_, value)| *value)
}

impl ComposerGeometry {
    /// The text columns `T` at `cols`, or `None` when `T` or the first row's cells fall below
    /// one cell.
    pub(crate) fn text_columns(&self, cols: u16) -> Option<usize> {
        let text = usize::from(cols)
            .checked_sub(usize::from(self.margin_columns))
            .filter(|text| *text >= 1)?;
        (text > usize::from(self.first_row_margin_columns)).then_some(text)
    }

    fn reserved_rows_at(&self, rows: u16) -> Option<u16> {
        match &self.reserved_rows {
            ReservedRows::Fixed(reserved) => Some(*reserved),
            ReservedRows::Steps(steps) => step_value(steps, rows),
        }
    }

    /// The composer window `W(rows)`, or `None` when it is unbounded.
    fn window(&self, rows: u16) -> Option<usize> {
        match &self.text_window {
            TextWindow::Unbounded => None,
            TextWindow::Fraction {
                fraction,
                offset,
                min,
            } => {
                // The epsilon keeps a product such as 0.3 × 30 from flooring one row short.
                let scaled = (fraction * f64::from(rows) + 1e-9).floor() as i64;
                Some(
                    usize::try_from((scaled - i64::from(*offset)).max(i64::from(*min)))
                        .expect("a window of at least min rows is positive"),
                )
            }
            TextWindow::Steps {
                steps,
                uncapped_from,
            } => match uncapped_from {
                Some(uncapped) if rows >= *uncapped => None,
                _ => Some(step_value(steps, rows).map_or(0, usize::from)),
            },
        }
    }

    /// The visible text rows `V` at `rows`: the window, bounded by the rows left beside the
    /// reserved rows and the headroom; 0 below the first step of either step list.
    pub(crate) fn visible_text_rows(&self, rows: u16, headroom: Headroom) -> usize {
        let Some(reserved) = self.reserved_rows_at(rows) else {
            return 0;
        };
        let headroom = match headroom {
            Headroom::Zero => 0,
            Headroom::Transient => self.transient_rows,
        };
        let room = usize::from(rows).saturating_sub(usize::from(reserved) + usize::from(headroom));
        self.window(rows).map_or(room, |window| window.min(room))
    }

    /// The rows `R` the rendered `literal` takes at `cols` under `fit_wrap`, or `None` when no
    /// text column remains.
    pub(crate) fn fit_rows(
        &self,
        literal: &str,
        cols: u16,
        framing: LiteralFraming,
    ) -> Option<usize> {
        let text = self.text_columns(cols)?;
        if framing == LiteralFraming::BracketedPaste
            && let Some(paste) = self.collapsed_paste
            && u64::try_from(literal.chars().count()).expect("a scalar count fits in u64")
                >= paste.min_scalars
        {
            return Some(usize::from(paste.rows));
        }
        let mut rows = Rows::new(text - usize::from(self.first_row_margin_columns), text);
        match self.fit_wrap {
            TextWrap::Char => {
                literal.chars().for_each(|scalar| rows.fill(width(scalar)));
                Some(rows.count)
            }
            TextWrap::Word => {
                for (index, word) in literal.split(' ').enumerate() {
                    rows.place_word(word, index > 0, self.carried_space);
                }
                Some(rows.count + usize::from(self.wrap_slack_rows))
            }
        }
    }
}

fn width(scalar: char) -> usize {
    scalar.width().unwrap_or(0)
}

/// Greedy row filling: the first row holds `first` cells and every later row `text` cells.
struct Rows {
    count: usize,
    used: usize,
    limit: usize,
    text: usize,
}

impl Rows {
    fn new(first: usize, text: usize) -> Self {
        Self {
            count: 1,
            used: 0,
            limit: first,
            text,
        }
    }

    fn break_row(&mut self) {
        self.count += 1;
        self.used = 0;
        self.limit = self.text;
    }

    /// Places one scalar of `cells` width, starting the next row when it does not fit.
    fn fill(&mut self, cells: usize) {
        if self.used > 0 && self.used + cells > self.limit {
            self.break_row();
        }
        self.used += cells;
    }

    /// Places `word`, after the separating space when `spaced`: on the current row when it
    /// fits, else at the start of the next, hard-broken when longer than a row. With
    /// `carried_space`, a space that follows a row filled to its last cell starts the next row.
    fn place_word(&mut self, word: &str, spaced: bool, carried_space: bool) {
        let cells = word.chars().map(width).sum::<usize>();
        let space = usize::from(spaced && self.used > 0);
        if self.used > 0 && self.used + space + cells <= self.limit {
            self.used += space + cells;
            return;
        }
        let carried = carried_space && space == 1 && self.used == self.limit;
        if self.used > 0 {
            self.break_row();
        }
        if carried {
            self.used = 1;
        }
        if self.used + cells <= self.limit {
            self.used += cells;
        } else {
            word.chars().for_each(|scalar| self.fill(width(scalar)));
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::coding_agent::CodingAgent;
    use crate::coding_agent::catalog::test_support::SYNTHETIC_TEXT_RULE;
    use crate::coding_agent::catalog::test_support::definition;
    use crate::coding_agent::catalog::test_support::load;
    use crate::coding_agent::catalog::test_support::synthetic_geometry;
    use crate::coding_agent::detector::Detector;
    use crate::coding_agent::geometry::ComposerGeometry;
    use crate::coding_agent::geometry::LiveComposer;
    use crate::runtime::TerminalSize;
    use crate::runtime::TerminalState;
    use serde_json::Value;
    use serde_json::json;

    /// The geometry the catalog parses from the profile-level `composer.geometry` object.
    pub(crate) fn parsed(geometry: Value) -> ComposerGeometry {
        let mut value = definition(Some("0d"), Some("0d"));
        value["codex"]["profile"]["capabilities"]["composer"]["geometry"] = geometry.clone();
        load(&value)
            .unwrap_or_else(|error| panic!("{geometry}: {}", error.message()))
            .composer_geometry(CodingAgent::Codex, SYNTHETIC_TEXT_RULE)
            .expect("the synthetic composer text rule has a geometry")
    }

    /// The synthetic geometry with each field of `overrides` replaced.
    pub(crate) fn synthetic_with(overrides: Value) -> ComposerGeometry {
        let mut geometry = synthetic_geometry();
        for (key, field) in overrides.as_object().expect("geometry overrides") {
            geometry[key] = field.clone();
        }
        parsed(geometry)
    }

    const LIVE_RULE: &str = "live-box-empty";

    /// The live composer the detector reads from an empty `rows` x `cols` box screen, a `> `
    /// row above a `-- footer` row, under a profile fitting its composer text with `geometry`
    /// and `wrap`.
    pub(crate) fn live_composer(rows: u16, cols: u16, geometry: Value, wrap: &str) -> LiveComposer {
        let mut value = definition(Some("0d"), Some("0d"));
        value["codex"]["profile"]["rules"] = json!([{
            "id":LIVE_RULE,"priority":50,"observation":"composer_empty",
            "region":{"screen":"either","start_pattern":"^> ","end_pattern":"^-- footer$","max_rows":6,"cursor":"any"},
            "matchers":[{"id":"prompt","scope":"line","pattern":"^> *$"}],
            "extract":{"operation":null,"choices":[]},
            "verified_noninteraction":true
        }]);
        let composer = &mut value["codex"]["profile"]["capabilities"]["composer"];
        composer["rule_ids"] = json!([LIVE_RULE]);
        composer["text"] = json!({
            "rule_ids":[LIVE_RULE],"first_prefix":"^> ","continuation_prefix":"^  ",
            "wrap":wrap,"placeholders":[]
        });
        composer["geometry"] = geometry.clone();
        let catalog =
            load(&value).unwrap_or_else(|error| panic!("{geometry}: {}", error.message()));
        let mut state = TerminalState::new(TerminalSize { rows, cols }).unwrap();
        state
            .feed(format!("\x1b[2J\x1b[{};1H> \x1b[{};1H-- footer", rows - 1, rows).as_bytes())
            .unwrap();
        Detector::new(&catalog, CodingAgent::Codex, None)
            .composer_geometry(&state.snapshot())
            .unwrap()
            .unwrap_or_else(|| panic!("{rows}x{cols}: the box composer selects its rule"))
    }
}

#[cfg(test)]
mod tests {
    use crate::coding_agent::catalog::LiteralFraming;
    use crate::coding_agent::catalog::TextWrap;
    use crate::coding_agent::geometry::test_support::*;
    use crate::coding_agent::geometry::*;
    use serde_json::Value;
    use serde_json::json;

    mod visible_text_rows {
        use crate::coding_agent::geometry::tests::*;

        fn measured(
            margin: u64,
            text_window: Value,
            reserved_rows: Value,
            transient_rows: u64,
        ) -> Value {
            json!({
                "margin_columns":margin,"first_row_margin_columns":0,
                "text_window":text_window,"reserved_rows":reserved_rows,
                "transient_rows":transient_rows,"wrap_slack_rows":0,"carried_space":false,"fit_wrap":"char","collapsed_paste":null,
                "min_rows":5,"min_columns":40
            })
        }

        fn claude() -> Value {
            measured(
                4,
                json!({"fraction":0.5,"offset":5,"min":3}),
                json!({"steps":[[9,8],[12,4]]}),
                4,
            )
        }

        fn codex() -> Value {
            measured(
                3,
                json!({"steps":[[5,3],[6,4],[7,5],[8,6],[14,7],[15,8],[18,10]],"uncapped_from":21}),
                json!(2),
                2,
            )
        }

        fn opencode() -> Value {
            measured(4, json!({"fraction":0,"offset":0,"min":6}), json!(2), 4)
        }

        fn pi() -> Value {
            measured(
                1,
                json!({"fraction":0.3,"offset":0,"min":5}),
                json!({"steps":[[5,3],[6,4],[7,5]]}),
                4,
            )
        }

        fn hermes_classic() -> Value {
            measured(
                0,
                json!({"fraction":0,"offset":0,"min":8}),
                json!({"steps":[[5,1],[6,2]]}),
                4,
            )
        }

        fn hermes_tui() -> Value {
            measured(6, Value::Null, json!(3), 4)
        }

        fn capped_steps() -> Value {
            measured(
                2,
                json!({"steps":[[5,3],[8,6]],"uncapped_from":null}),
                json!(1),
                1,
            )
        }

        struct RowsCase {
            name: &'static str,
            geometry: fn() -> Value,
            rows: u16,
            zero: usize,
            transient: usize,
        }

        const ROWS_CASES: &[RowsCase] = &[
            RowsCase {
                name: "claude below its first reserved step at 2 rows",
                geometry: claude,
                rows: 2,
                zero: 0,
                transient: 0,
            },
            RowsCase {
                name: "claude below its first reserved step at 5 rows",
                geometry: claude,
                rows: 5,
                zero: 0,
                transient: 0,
            },
            RowsCase {
                name: "claude one row below its first reserved step",
                geometry: claude,
                rows: 8,
                zero: 0,
                transient: 0,
            },
            RowsCase {
                name: "claude at 9 rows keeps one row above the bottom rule",
                geometry: claude,
                rows: 9,
                zero: 1,
                transient: 0,
            },
            RowsCase {
                name: "claude at 12 rows takes its 3-row minimum window",
                geometry: claude,
                rows: 12,
                zero: 3,
                transient: 3,
            },
            RowsCase {
                name: "claude at 18 rows takes a 4-row window",
                geometry: claude,
                rows: 18,
                zero: 4,
                transient: 4,
            },
            RowsCase {
                name: "claude at 24 rows takes a 7-row window",
                geometry: claude,
                rows: 24,
                zero: 7,
                transient: 7,
            },
            RowsCase {
                name: "codex below its first window step",
                geometry: codex,
                rows: 4,
                zero: 0,
                transient: 0,
            },
            RowsCase {
                name: "codex at 2 rows",
                geometry: codex,
                rows: 2,
                zero: 0,
                transient: 0,
            },
            RowsCase {
                name: "codex at its first window step",
                geometry: codex,
                rows: 5,
                zero: 3,
                transient: 1,
            },
            RowsCase {
                name: "codex at 9 rows",
                geometry: codex,
                rows: 9,
                zero: 6,
                transient: 5,
            },
            RowsCase {
                name: "codex at 12 rows",
                geometry: codex,
                rows: 12,
                zero: 6,
                transient: 6,
            },
            RowsCase {
                name: "codex at a middle window step",
                geometry: codex,
                rows: 14,
                zero: 7,
                transient: 7,
            },
            RowsCase {
                name: "codex one row below uncapped_from",
                geometry: codex,
                rows: 20,
                zero: 10,
                transient: 10,
            },
            RowsCase {
                name: "codex at uncapped_from",
                geometry: codex,
                rows: 21,
                zero: 19,
                transient: 17,
            },
            RowsCase {
                name: "codex above uncapped_from",
                geometry: codex,
                rows: 24,
                zero: 22,
                transient: 20,
            },
            RowsCase {
                name: "opencode with no row beside its reserved rows",
                geometry: opencode,
                rows: 2,
                zero: 0,
                transient: 0,
            },
            RowsCase {
                name: "opencode at 4 rows",
                geometry: opencode,
                rows: 4,
                zero: 2,
                transient: 0,
            },
            RowsCase {
                name: "opencode at 5 rows",
                geometry: opencode,
                rows: 5,
                zero: 3,
                transient: 0,
            },
            RowsCase {
                name: "opencode at 9 rows takes its fixed window",
                geometry: opencode,
                rows: 9,
                zero: 6,
                transient: 3,
            },
            RowsCase {
                name: "opencode at 12 rows",
                geometry: opencode,
                rows: 12,
                zero: 6,
                transient: 6,
            },
            RowsCase {
                name: "pi below its first reserved step",
                geometry: pi,
                rows: 4,
                zero: 0,
                transient: 0,
            },
            RowsCase {
                name: "pi at 2 rows",
                geometry: pi,
                rows: 2,
                zero: 0,
                transient: 0,
            },
            RowsCase {
                name: "pi at 5 rows",
                geometry: pi,
                rows: 5,
                zero: 2,
                transient: 0,
            },
            RowsCase {
                name: "pi at 6 rows",
                geometry: pi,
                rows: 6,
                zero: 2,
                transient: 0,
            },
            RowsCase {
                name: "pi at 9 rows overflows its minimum window",
                geometry: pi,
                rows: 9,
                zero: 4,
                transient: 0,
            },
            RowsCase {
                name: "pi at 12 rows",
                geometry: pi,
                rows: 12,
                zero: 5,
                transient: 3,
            },
            RowsCase {
                name: "pi at 20 rows takes its pane fraction",
                geometry: pi,
                rows: 20,
                zero: 6,
                transient: 6,
            },
            RowsCase {
                name: "hermes classic below its first reserved step",
                geometry: hermes_classic,
                rows: 2,
                zero: 0,
                transient: 0,
            },
            RowsCase {
                name: "hermes classic at 5 rows",
                geometry: hermes_classic,
                rows: 5,
                zero: 4,
                transient: 0,
            },
            RowsCase {
                name: "hermes classic at 9 rows",
                geometry: hermes_classic,
                rows: 9,
                zero: 7,
                transient: 3,
            },
            RowsCase {
                name: "hermes classic at 12 rows takes its fixed window",
                geometry: hermes_classic,
                rows: 12,
                zero: 8,
                transient: 6,
            },
            RowsCase {
                name: "hermes TUI with fewer rows than it reserves",
                geometry: hermes_tui,
                rows: 2,
                zero: 0,
                transient: 0,
            },
            RowsCase {
                name: "hermes TUI at 5 rows",
                geometry: hermes_tui,
                rows: 5,
                zero: 2,
                transient: 0,
            },
            RowsCase {
                name: "hermes TUI at 9 rows has no window",
                geometry: hermes_tui,
                rows: 9,
                zero: 6,
                transient: 2,
            },
            RowsCase {
                name: "hermes TUI at 12 rows has no window",
                geometry: hermes_tui,
                rows: 12,
                zero: 9,
                transient: 5,
            },
            RowsCase {
                name: "a step window capped at every larger size",
                geometry: capped_steps,
                rows: 30,
                zero: 6,
                transient: 6,
            },
            RowsCase {
                name: "a step window between its steps",
                geometry: capped_steps,
                rows: 7,
                zero: 3,
                transient: 3,
            },
        ];

        #[test]
        fn visible_text_rows_evaluate_the_window_and_reserved_rows_at_the_live_size() {
            for case in ROWS_CASES {
                let geometry = parsed((case.geometry)());
                assert_eq!(
                    geometry.visible_text_rows(case.rows, Headroom::Zero),
                    case.zero,
                    "{}: no headroom",
                    case.name
                );
                assert_eq!(
                    geometry.visible_text_rows(case.rows, Headroom::Transient),
                    case.transient,
                    "{}: transient headroom",
                    case.name
                );
            }
        }
    }

    mod text_columns {
        use crate::coding_agent::geometry::tests::*;

        #[test]
        fn text_columns_subtract_the_margin_and_nothing_fits_below_one_cell() {
            for (name, margin, first_row, cols, expected) in [
                ("80 columns less a 4-cell margin", 4, 0, 80, Some(76)),
                ("120 columns less a 4-cell margin", 4, 0, 120, Some(116)),
                ("one text cell", 4, 0, 5, Some(1)),
                ("a margin as wide as the pane", 4, 0, 4, None),
                ("a margin wider than the pane", 12, 0, 10, None),
                ("a first row with one cell", 0, 2, 3, Some(3)),
                ("a first row with no cell", 0, 2, 2, None),
                ("a first row narrower than its margin", 1, 4, 4, None),
            ] {
                let geometry = synthetic_with(
                    json!({"margin_columns":margin,"first_row_margin_columns":first_row}),
                );
                assert_eq!(geometry.text_columns(cols), expected, "{name}");
            }
        }
    }

    mod fit_rows {
        use crate::coding_agent::geometry::tests::*;

        struct FitCase {
            name: &'static str,
            geometry: Value,
            literal: String,
            cols: u16,
            wrap: TextWrap,
            framing: LiteralFraming,
            rows: Option<usize>,
        }

        fn case(
            name: &'static str,
            geometry: Value,
            literal: impl Into<String>,
            cols: u16,
            wrap: TextWrap,
            rows: Option<usize>,
        ) -> FitCase {
            FitCase {
                name,
                geometry,
                literal: literal.into(),
                cols,
                wrap,
                framing: LiteralFraming::Plain,
                rows,
            }
        }

        fn margin(margin: u64) -> Value {
            json!({"margin_columns":margin,"first_row_margin_columns":0})
        }

        fn first_row(first_row: u64) -> Value {
            json!({"margin_columns":0,"first_row_margin_columns":first_row})
        }

        fn slack() -> Value {
            json!({"margin_columns":4,"first_row_margin_columns":0,"wrap_slack_rows":1})
        }

        fn carried(carried_space: bool) -> Value {
            json!({"margin_columns":4,"first_row_margin_columns":0,"carried_space":carried_space})
        }

        fn collapsing() -> Value {
            json!({"margin_columns":4,"first_row_margin_columns":0,"collapsed_paste":{"min_scalars":100,"rows":1}})
        }

        fn bracketed(mut case: FitCase) -> FitCase {
            case.framing = LiteralFraming::BracketedPaste;
            case
        }

        fn cases() -> Vec<FitCase> {
            use TextWrap::Char;
            use TextWrap::Word;
            vec![
                case("a one-row literal", margin(4), "hello", 80, Char, Some(1)),
                case(
                    "a literal filling the text columns",
                    margin(4),
                    "a".repeat(76),
                    80,
                    Char,
                    Some(1),
                ),
                case(
                    "one cell past the text columns",
                    margin(4),
                    "a".repeat(77),
                    80,
                    Char,
                    Some(2),
                ),
                case(
                    "a literal at 120 columns",
                    margin(4),
                    "a".repeat(233),
                    120,
                    Char,
                    Some(3),
                ),
                case(
                    "the operation prefix takes cells",
                    margin(4),
                    "! abcde",
                    10,
                    Char,
                    Some(2),
                ),
                case(
                    "a first row narrowed by its prefix",
                    first_row(2),
                    "a".repeat(8),
                    10,
                    Char,
                    Some(1),
                ),
                case(
                    "one cell past the narrowed first row",
                    first_row(2),
                    "a".repeat(9),
                    10,
                    Char,
                    Some(2),
                ),
                case(
                    "continuation rows keep the full width",
                    first_row(2),
                    "a".repeat(18),
                    10,
                    Char,
                    Some(2),
                ),
                case(
                    "one cell past the second row",
                    first_row(2),
                    "a".repeat(19),
                    10,
                    Char,
                    Some(3),
                ),
                case(
                    "wide scalars filling the row",
                    margin(4),
                    "あああ",
                    10,
                    Char,
                    Some(1),
                ),
                case(
                    "a wide scalar at the row edge starts the next row",
                    margin(4),
                    "aaaaaあ",
                    10,
                    Char,
                    Some(2),
                ),
                case(
                    "a wide scalar after an odd cell count",
                    margin(4),
                    "aあああ",
                    10,
                    Char,
                    Some(2),
                ),
                case("one text cell", margin(4), "ab", 5, Char, Some(2)),
                case(
                    "a margin as wide as the pane",
                    margin(4),
                    "a",
                    4,
                    Char,
                    None,
                ),
                case(
                    "a margin wider than the pane",
                    margin(12),
                    "a",
                    10,
                    Word,
                    None,
                ),
                case("a first row with no cell", first_row(2), "a", 2, Char, None),
                case(
                    "char wrap splits words",
                    margin(4),
                    "aaaaaa bbbbbb cccccc",
                    14,
                    Char,
                    Some(2),
                ),
                case(
                    "word wrap breaks at the last space that fits",
                    margin(4),
                    "aaaaaa bbbbbb cccccc",
                    14,
                    Word,
                    Some(3),
                ),
                case(
                    "two words filling the row exactly",
                    margin(4),
                    "aaaa bbbbb",
                    14,
                    Word,
                    Some(1),
                ),
                case(
                    "two words one cell past the row",
                    margin(4),
                    "aaaaa bbbbb",
                    14,
                    Word,
                    Some(2),
                ),
                case(
                    "word wrap on a narrowed first row",
                    first_row(2),
                    "aaaa bbbb",
                    10,
                    Word,
                    Some(2),
                ),
                case(
                    "a row filled to its last column drops its separating space",
                    carried(false),
                    "aaaaaaaaaa bbbb ccccc",
                    14,
                    Word,
                    Some(2),
                ),
                case(
                    "a row filled to its last column carries its separating space",
                    carried(true),
                    "aaaaaaaaaa bbbb ccccc",
                    14,
                    Word,
                    Some(3),
                ),
                case(
                    "a row ending short of its last column carries no space",
                    carried(true),
                    "aaaaaaaaa bbbb ccccc",
                    14,
                    Word,
                    Some(2),
                ),
                case(
                    "char wrap carries no space",
                    carried(true),
                    "aaaaaaaaaa bbbbbbbbb",
                    14,
                    Char,
                    Some(2),
                ),
                case(
                    "two words filling the narrowed first row",
                    first_row(2),
                    "aaa bbbb",
                    10,
                    Word,
                    Some(1),
                ),
                case(
                    "a word longer than its row is hard-broken",
                    margin(4),
                    "a".repeat(25),
                    14,
                    Word,
                    Some(3),
                ),
                case(
                    "word wrap adds the slack row",
                    slack(),
                    "a".repeat(25),
                    14,
                    Word,
                    Some(4),
                ),
                case(
                    "word wrap adds the slack row to a one-row literal",
                    slack(),
                    "hello",
                    14,
                    Word,
                    Some(2),
                ),
                case(
                    "char wrap takes no slack row",
                    slack(),
                    "a".repeat(25),
                    14,
                    Char,
                    Some(3),
                ),
                bracketed(case(
                    "a bracketed literal at the collapse threshold",
                    collapsing(),
                    "a".repeat(100),
                    14,
                    Char,
                    Some(1),
                )),
                bracketed(case(
                    "a bracketed literal below the collapse threshold",
                    collapsing(),
                    "a".repeat(99),
                    14,
                    Char,
                    Some(10),
                )),
                case(
                    "a plain literal at the collapse threshold",
                    collapsing(),
                    "a".repeat(100),
                    14,
                    Char,
                    Some(10),
                ),
                bracketed(case(
                    "a bracketed literal without collapsed paste",
                    margin(4),
                    "a".repeat(100),
                    14,
                    Char,
                    Some(10),
                )),
            ]
        }

        #[test]
        fn fit_rows_wrap_the_rendered_literal_into_the_text_columns() {
            for case in cases() {
                let mut overrides = case.geometry.clone();
                overrides["fit_wrap"] = json!(match case.wrap {
                    TextWrap::Char => "char",
                    TextWrap::Word => "word",
                });
                let geometry = synthetic_with(overrides);
                assert_eq!(
                    geometry.fit_rows(&case.literal, case.cols, case.framing),
                    case.rows,
                    "{}",
                    case.name
                );
            }
        }
    }

    mod fit_wrap {
        use crate::coding_agent::geometry::tests::*;

        /// Takes 3 rows char-wrapped and 4 word-wrapped in 10 text columns.
        const LITERAL: &str = "aaaaaaaa bbbbbbbb cccccccc dd";

        #[test]
        fn the_fit_wraps_by_fit_wrap_whatever_the_rule_extracts_with() {
            for (fit_wrap, text_wrap, fits) in [
                ("char", "word", true),
                ("word", "word", false),
                ("char", "char", true),
                ("word", "char", false),
            ] {
                let live = live_composer(
                    4,
                    12,
                    json!({
                        "margin_columns":2,"first_row_margin_columns":0,"text_window":null,
                        "reserved_rows":1,"transient_rows":0,"wrap_slack_rows":0,
                        "carried_space":false,"fit_wrap":fit_wrap,"collapsed_paste":null,
                        "min_rows":2,"min_columns":10
                    }),
                    text_wrap,
                );
                assert_eq!(
                    live.fits(LITERAL, LiteralFraming::Plain, Headroom::Zero),
                    fits,
                    "fit_wrap {fit_wrap} under {text_wrap} extraction"
                );
            }
        }
    }
}
