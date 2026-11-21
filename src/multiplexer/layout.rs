use crate::broker::stored_enum;

pub(crate) const MIN_PANE_HEIGHT: i64 = 15;
pub(crate) const MAX_PANES_PER_COLUMN: i64 = 8;

stored_enum!(ColumnSide, "member_placements.column_side", { Left => "left", Right => "right" });

pub(crate) fn capacity(height: i64) -> i64 {
    MAX_PANES_PER_COLUMN.min(height.div_euclid(MIN_PANE_HEIGHT))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WindowOccupancy {
    pub(crate) ordinal: i64,
    pub(crate) window_id: String,
    pub(crate) window_name: String,
    pub(crate) height: i64,
    pub(crate) left: Vec<String>,
    pub(crate) right: Vec<String>,
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

pub(crate) fn choose_slot(
    fleet_id: i64,
    windows: &[WindowOccupancy],
    last_window_ordinal: i64,
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
        let columns: &[(ColumnSide, &Vec<String>, i64)] = if window.ordinal == 0 {
            &[(ColumnSide::Right, &window.right, capacity(window.height))]
        } else {
            let overflow = capacity(window.height).max(1);
            &[
                (ColumnSide::Left, &window.left, overflow),
                (ColumnSide::Right, &window.right, overflow),
            ]
        };
        for (side, members, limit) in columns {
            if i64::try_from(members.len()).expect("a column's member count fits i64") < *limit {
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
    use crate::multiplexer::layout::*;

    const FLEET: i64 = 3;

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

    fn panes(prefix: &str, count: usize) -> Vec<String> {
        (0..count)
            .map(|index| format!("%{prefix}{index}"))
            .collect()
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

    #[test]
    fn layout_constants_are_fifteen_rows_and_eight_panes() {
        assert_eq!(MIN_PANE_HEIGHT, 15);
        assert_eq!(MAX_PANES_PER_COLUMN, 8);
    }

    #[test]
    fn capacity_is_the_window_height_over_fifteen_rows_capped_at_eight() {
        for (height, expected) in [
            (0, 0),
            (14, 0),
            (15, 1),
            (25, 1),
            (29, 1),
            (30, 2),
            (50, 3),
            (60, 4),
            (119, 7),
            (120, 8),
            (1000, 8),
        ] {
            assert_eq!(capacity(height), expected, "height {height}");
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
                choose_slot(case.fleet_id, &case.windows, case.last_window_ordinal),
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
                choose_slot(FLEET, &windows, last_window_ordinal);
            });
            assert!(refused.is_err(), "{name}");
        }
    }
}
