use crate::broker::stored_enum;
use crate::coding_agent::CodingAgent;
use crate::coding_agent::ProfileCatalog;
use crate::multiplexer::MuxBackend;

/// The slot floor: a member's pane keeps at least this many rows whatever its profile's
/// minimum, so delivery degrades only after a resize.
pub(crate) const DEFAULT_MIN_ROWS: i64 = 14;
pub(crate) const MAX_PANES_PER_COLUMN: i64 = 8;

stored_enum!(ColumnSide, "member_placements.column_side", { Left => "left", Right => "right" });

/// The rows a pane's border or separator takes on `multiplexer`.
fn pane_chrome_rows(multiplexer: MuxBackend) -> i64 {
    match multiplexer {
        MuxBackend::Tmux => 1,
        MuxBackend::Herdr => 2,
    }
}

/// The rows a member of `backend` takes in a column: the larger of the floor and its profile's
/// minimum, plus the pane chrome.
pub(crate) fn slot_rows(
    catalog: &ProfileCatalog,
    backend: CodingAgent,
    multiplexer: MuxBackend,
) -> i64 {
    let minimum = catalog
        .composer_minimum(backend)
        .map_or(DEFAULT_MIN_ROWS, |(min_rows, _)| i64::from(min_rows));
    DEFAULT_MIN_ROWS.max(minimum) + pane_chrome_rows(multiplexer)
}

/// One concrete member pane of a column and the backend it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ColumnMember {
    pub(crate) pane: String,
    pub(crate) backend: CodingAgent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WindowOccupancy {
    pub(crate) ordinal: i64,
    pub(crate) window_id: String,
    pub(crate) window_name: String,
    pub(crate) height: i64,
    pub(crate) left: Vec<ColumnMember>,
    pub(crate) right: Vec<ColumnMember>,
    pub(crate) director_pane: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SlotChoice {
    Existing {
        ordinal: i64,
        window_id: String,
        window_name: String,
        side: ColumnSide,
    },
    NewWindow {
        ordinal: i64,
        window_name: String,
    },
}

pub(crate) fn fleet_window_name(fleet_id: i64, ordinal: i64) -> String {
    format!("cafleet-{fleet_id}-{ordinal}")
}

/// Whether a column `height` rows high holding `members` admits one more member of `creating`:
/// at most [`MAX_PANES_PER_COLUMN`] panes, each keeping the largest slot among them.
fn admits(
    height: i64,
    members: &[ColumnMember],
    catalog: &ProfileCatalog,
    creating: CodingAgent,
    multiplexer: MuxBackend,
) -> bool {
    let count = i64::try_from(members.len()).expect("a column's member count fits i64") + 1;
    let largest = members
        .iter()
        .map(|member| slot_rows(catalog, member.backend, multiplexer))
        .chain(std::iter::once(slot_rows(catalog, creating, multiplexer)))
        .max()
        .expect("the creating member has a slot");
    count <= MAX_PANES_PER_COLUMN && height.div_euclid(count) >= largest
}

pub(crate) fn choose_slot(
    fleet_id: i64,
    windows: &[WindowOccupancy],
    last_window_ordinal: i64,
    catalog: &ProfileCatalog,
    creating: CodingAgent,
    multiplexer: MuxBackend,
) -> SlotChoice {
    assert_eq!(
        windows.first().map(|window| window.ordinal),
        Some(0),
        "slot selection starts at the Director window"
    );
    assert!(
        windows
            .windows(2)
            .all(|pair| pair[0].ordinal < pair[1].ordinal),
        "window ordinals are strictly ascending"
    );
    for window in windows {
        let columns: &[(ColumnSide, &Vec<ColumnMember>)] = if window.ordinal == 0 {
            &[(ColumnSide::Right, &window.right)]
        } else {
            &[
                (ColumnSide::Left, &window.left),
                (ColumnSide::Right, &window.right),
            ]
        };
        for (side, members) in columns {
            let first_overflow = window.ordinal > 0 && members.is_empty();
            if first_overflow || admits(window.height, members, catalog, creating, multiplexer) {
                return SlotChoice::Existing {
                    ordinal: window.ordinal,
                    window_id: window.window_id.clone(),
                    window_name: window.window_name.clone(),
                    side: *side,
                };
            }
        }
    }
    let ordinal = last_window_ordinal + 1;
    SlotChoice::NewWindow {
        ordinal,
        window_name: fleet_window_name(fleet_id, ordinal),
    }
}

#[cfg(test)]
mod tests {
    use crate::coding_agent::CodingAgent;
    use crate::coding_agent::ProfileCatalog;
    use crate::multiplexer::MuxBackend;
    use crate::multiplexer::layout::*;
    use serde_json::json;

    const FLEET: i64 = 3;

    /// A catalog whose codex profile declares `min_rows`; every other backend has no profile.
    fn codex_minimum(min_rows: u64) -> ProfileCatalog {
        let mut value =
            crate::coding_agent::catalog::test_support::definition(Some("0d"), Some("0d"));
        value["codex"]["profile"]["capabilities"]["composer"]["geometry"]["min_rows"] =
            json!(min_rows);
        crate::coding_agent::catalog::test_support::load(&value).unwrap()
    }

    #[test]
    fn a_slot_is_the_larger_of_the_floor_and_the_profile_minimum_plus_the_pane_chrome() {
        assert_eq!(DEFAULT_MIN_ROWS, 14);
        for (name, min_rows, backend, multiplexer, expected) in [
            (
                "a backend without a profile on tmux",
                9,
                CodingAgent::Claude,
                MuxBackend::Tmux,
                15,
            ),
            (
                "a backend without a profile on herdr",
                9,
                CodingAgent::Claude,
                MuxBackend::Herdr,
                16,
            ),
            (
                "a minimum below the floor on tmux",
                9,
                CodingAgent::Codex,
                MuxBackend::Tmux,
                15,
            ),
            (
                "a minimum below the floor on herdr",
                9,
                CodingAgent::Codex,
                MuxBackend::Herdr,
                16,
            ),
            (
                "a minimum at the floor",
                14,
                CodingAgent::Codex,
                MuxBackend::Tmux,
                15,
            ),
            (
                "a minimum above the floor on tmux",
                20,
                CodingAgent::Codex,
                MuxBackend::Tmux,
                21,
            ),
            (
                "a minimum above the floor on herdr",
                20,
                CodingAgent::Codex,
                MuxBackend::Herdr,
                22,
            ),
        ] {
            assert_eq!(
                slot_rows(&codex_minimum(min_rows), backend, multiplexer),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn fleet_window_names_cover_the_director_and_overflow_ordinals() {
        for (fleet_id, ordinal, expected) in [
            (3, 0, "cafleet-3-0"),
            (3, 1, "cafleet-3-1"),
            (42, 12, "cafleet-42-12"),
            (
                i64::MAX,
                i64::MAX,
                "cafleet-9223372036854775807-9223372036854775807",
            ),
        ] {
            assert_eq!(fleet_window_name(fleet_id, ordinal), expected);
        }
    }

    fn members_of(prefix: &str, backends: &[CodingAgent]) -> Vec<ColumnMember> {
        backends
            .iter()
            .enumerate()
            .map(|(index, backend)| ColumnMember {
                pane: format!("%{prefix}{index}"),
                backend: *backend,
            })
            .collect()
    }

    fn panes(prefix: &str, count: usize) -> Vec<ColumnMember> {
        members_of(prefix, &vec![CodingAgent::Codex; count])
    }

    fn director_window(height: i64, members: usize) -> WindowOccupancy {
        WindowOccupancy {
            ordinal: 0,
            window_id: "@1".into(),
            window_name: "main".into(),
            height,
            left: Vec::new(),
            right: panes("1", members),
            director_pane: Some("%0".into()),
        }
    }

    fn overflow_window(ordinal: i64, height: i64, left: usize, right: usize) -> WindowOccupancy {
        WindowOccupancy {
            ordinal,
            window_id: format!("@{}", 10 + ordinal),
            window_name: format!("cafleet-{FLEET}-{ordinal}"),
            height,
            left: panes(&format!("{ordinal}0"), left),
            right: panes(&format!("{ordinal}9"), right),
            director_pane: None,
        }
    }

    fn existing(ordinal: i64, side: ColumnSide) -> SlotChoice {
        let (window_id, window_name) = if ordinal == 0 {
            ("@1".to_string(), "main".to_string())
        } else {
            (
                format!("@{}", 10 + ordinal),
                format!("cafleet-{FLEET}-{ordinal}"),
            )
        };
        SlotChoice::Existing {
            ordinal,
            window_id,
            window_name,
            side,
        }
    }

    fn new_window(ordinal: i64) -> SlotChoice {
        SlotChoice::NewWindow {
            ordinal,
            window_name: format!("cafleet-{FLEET}-{ordinal}"),
        }
    }

    /// The slot choice for a codex member on tmux under a catalog whose codex minimum is below
    /// the floor, so every slot is the 15-row default.
    fn tmux_slot(
        fleet_id: i64,
        windows: &[WindowOccupancy],
        last_window_ordinal: i64,
    ) -> SlotChoice {
        choose_slot(
            fleet_id,
            windows,
            last_window_ordinal,
            &codex_minimum(9),
            CodingAgent::Codex,
            MuxBackend::Tmux,
        )
    }

    #[test]
    fn a_column_holds_at_most_eight_members() {
        assert_eq!(MAX_PANES_PER_COLUMN, 8);
        for (members, expected) in [(7, existing(0, ColumnSide::Right)), (8, new_window(1))] {
            assert_eq!(
                tmux_slot(FLEET, &[director_window(1000, members)], 0),
                expected,
                "{members} members in a 1000-row column"
            );
        }
    }

    #[test]
    fn a_herdr_column_admits_a_member_only_with_room_for_its_wider_chrome() {
        let catalog = codex_minimum(9);
        for (name, height, members, multiplexer, expected) in [
            (
                "tmux at 15 rows each",
                30,
                1,
                MuxBackend::Tmux,
                existing(0, ColumnSide::Right),
            ),
            (
                "herdr below 16 rows each",
                30,
                1,
                MuxBackend::Herdr,
                new_window(1),
            ),
            (
                "herdr at 16 rows each",
                32,
                1,
                MuxBackend::Herdr,
                existing(0, ColumnSide::Right),
            ),
            (
                "herdr with one slot",
                16,
                0,
                MuxBackend::Herdr,
                existing(0, ColumnSide::Right),
            ),
            (
                "herdr below one slot",
                15,
                0,
                MuxBackend::Herdr,
                new_window(1),
            ),
        ] {
            assert_eq!(
                choose_slot(
                    FLEET,
                    &[director_window(height, members)],
                    0,
                    &catalog,
                    CodingAgent::Codex,
                    multiplexer
                ),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn a_mixed_column_admits_a_member_only_with_room_for_the_largest_slot() {
        let catalog = codex_minimum(20);
        for (name, height, column, creating, expected) in [
            (
                "a codex member joining a claude member with room for 21 rows",
                42,
                vec![CodingAgent::Claude],
                CodingAgent::Codex,
                existing(0, ColumnSide::Right),
            ),
            (
                "a codex member joining a claude member without room for 21 rows",
                40,
                vec![CodingAgent::Claude],
                CodingAgent::Codex,
                new_window(1),
            ),
            (
                "a claude member joining a codex member without room for its 21 rows",
                40,
                vec![CodingAgent::Codex],
                CodingAgent::Claude,
                new_window(1),
            ),
            (
                "a claude member joining a claude member at 20 rows each",
                40,
                vec![CodingAgent::Claude],
                CodingAgent::Claude,
                existing(0, ColumnSide::Right),
            ),
            (
                "a codex member alone in a 21-row column",
                21,
                Vec::new(),
                CodingAgent::Codex,
                existing(0, ColumnSide::Right),
            ),
            (
                "a codex member alone in a 20-row column",
                20,
                Vec::new(),
                CodingAgent::Codex,
                new_window(1),
            ),
        ] {
            let mut window = director_window(height, 0);
            window.right = members_of("1", &column);
            assert_eq!(
                choose_slot(FLEET, &[window], 0, &catalog, creating, MuxBackend::Tmux),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn an_overflow_column_always_admits_its_first_member() {
        let catalog = codex_minimum(20);
        for multiplexer in [MuxBackend::Tmux, MuxBackend::Herdr] {
            assert_eq!(
                choose_slot(
                    FLEET,
                    &[director_window(10, 0), overflow_window(1, 10, 0, 0)],
                    1,
                    &catalog,
                    CodingAgent::Codex,
                    multiplexer
                ),
                existing(1, ColumnSide::Left),
                "{multiplexer:?}: an empty overflow column below one slot"
            );
            assert_eq!(
                choose_slot(
                    FLEET,
                    &[director_window(10, 0), overflow_window(1, 10, 1, 1)],
                    1,
                    &catalog,
                    CodingAgent::Codex,
                    multiplexer
                ),
                new_window(2),
                "{multiplexer:?}: a second member needs the slot budget"
            );
        }
    }

    struct SlotCase {
        name: String,
        fleet_id: i64,
        windows: Vec<WindowOccupancy>,
        last_window_ordinal: i64,
        expected: SlotChoice,
    }

    fn slot_case(
        name: &str,
        windows: Vec<WindowOccupancy>,
        last_window_ordinal: i64,
        expected: SlotChoice,
    ) -> SlotCase {
        SlotCase {
            name: name.into(),
            fleet_id: FLEET,
            windows,
            last_window_ordinal,
            expected,
        }
    }

    #[test]
    fn choose_slot_takes_the_first_column_below_capacity_or_the_next_window_ordinal() {
        let director_fill = (0..4).map(|members| {
            slot_case(
                &format!("director window holding {members} of 4 members"),
                vec![director_window(60, members)],
                0,
                existing(0, ColumnSide::Right),
            )
        });
        let overflow_fill = [
            ("overflow 1 left 0 right", 1, 0, Some(ColumnSide::Left)),
            ("overflow 2 left 0 right", 2, 0, Some(ColumnSide::Right)),
            ("overflow 2 left 1 right", 2, 1, Some(ColumnSide::Right)),
            ("overflow 2 left 2 right", 2, 2, None),
        ]
        .map(|(name, left, right, side)| {
            slot_case(
                name,
                vec![director_window(30, 2), overflow_window(1, 30, left, right)],
                1,
                side.map_or_else(|| new_window(2), |side| existing(1, side)),
            )
        });
        let short_overflow = [
            (
                "short overflow 0 left 0 right",
                0,
                0,
                Some(ColumnSide::Left),
            ),
            (
                "short overflow 1 left 0 right",
                1,
                0,
                Some(ColumnSide::Right),
            ),
            ("short overflow 1 left 1 right", 1, 1, None),
        ]
        .map(|(name, left, right, side)| {
            slot_case(
                name,
                vec![director_window(10, 0), overflow_window(1, 10, left, right)],
                1,
                side.map_or_else(|| new_window(2), |side| existing(1, side)),
            )
        });
        let cases = director_fill
            .chain(overflow_fill)
            .chain(short_overflow)
            .chain([
                slot_case(
                    "full director window",
                    vec![director_window(60, 4)],
                    0,
                    new_window(1),
                ),
                slot_case(
                    "one-slot director window holding one member",
                    vec![director_window(25, 1)],
                    0,
                    new_window(1),
                ),
                slot_case(
                    "director window shorter than one slot",
                    vec![director_window(14, 0)],
                    0,
                    new_window(1),
                ),
                slot_case(
                    "short director window skipped for an overflow window",
                    vec![director_window(10, 0), overflow_window(1, 30, 1, 0)],
                    1,
                    existing(1, ColumnSide::Left),
                ),
                slot_case(
                    "new window after the last allocated ordinal",
                    vec![director_window(30, 2), overflow_window(2, 30, 2, 2)],
                    5,
                    new_window(6),
                ),
                SlotCase {
                    name: "new window named for its fleet".into(),
                    fleet_id: 42,
                    windows: vec![director_window(30, 2)],
                    last_window_ordinal: 0,
                    expected: SlotChoice::NewWindow {
                        ordinal: 1,
                        window_name: "cafleet-42-1".into(),
                    },
                },
                slot_case(
                    "overflow windows scanned in ascending ordinal order",
                    vec![
                        director_window(30, 2),
                        overflow_window(1, 30, 2, 2),
                        overflow_window(3, 30, 2, 1),
                        overflow_window(4, 30, 0, 0),
                    ],
                    4,
                    existing(3, ColumnSide::Right),
                ),
                slot_case(
                    "freed director slot reused before an overflow window",
                    vec![director_window(60, 3), overflow_window(1, 30, 1, 0)],
                    1,
                    existing(0, ColumnSide::Right),
                ),
                slot_case(
                    "emptied left column refilled before the right column",
                    vec![director_window(30, 2), overflow_window(1, 30, 0, 1)],
                    1,
                    existing(1, ColumnSide::Left),
                ),
            ]);
        for case in cases {
            assert_eq!(
                tmux_slot(case.fleet_id, &case.windows, case.last_window_ordinal),
                case.expected,
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn choose_slot_refuses_windows_that_do_not_start_at_the_director_in_ascending_order() {
        for (name, windows, last_window_ordinal) in [
            (
                "no director window first",
                vec![overflow_window(1, 30, 0, 0)],
                1,
            ),
            ("no windows", Vec::new(), 0),
            (
                "descending ordinals",
                vec![
                    director_window(30, 2),
                    overflow_window(2, 30, 0, 0),
                    overflow_window(1, 30, 0, 0),
                ],
                2,
            ),
            (
                "repeated ordinal",
                vec![
                    director_window(30, 2),
                    overflow_window(1, 30, 2, 2),
                    overflow_window(1, 30, 0, 0),
                ],
                1,
            ),
        ] {
            let refused = std::panic::catch_unwind(|| {
                tmux_slot(FLEET, &windows, last_window_ordinal);
            });
            assert!(refused.is_err(), "{name}");
        }
    }
}
