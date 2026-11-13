#[cfg(test)]
mod tests {
    use crate::multiplexer::layout::*;

    const FLEET: i64 = 3;

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
    fn layout_constants_are_thirty_rows_and_eight_panes() {
        assert_eq!(MIN_PANE_HEIGHT, 30);
        assert_eq!(MAX_PANES_PER_COLUMN, 8);
    }

    #[test]
    fn capacity_is_the_window_height_over_thirty_rows_capped_at_eight() {
        for (height, expected) in [
            (0, 0),
            (29, 0),
            (30, 1),
            (50, 1),
            (59, 1),
            (60, 2),
            (120, 4),
            (239, 7),
            (240, 8),
            (1000, 8),
        ] {
            assert_eq!(capacity(height), expected, "height {height}");
        }
    }

    #[test]
    fn the_director_window_right_column_fills_up_to_its_capacity() {
        for members in 0..4 {
            assert_eq!(
                choose_slot(FLEET, &[director_window(120, members)], 0),
                existing(0, ColumnSide::Right),
                "{members} members"
            );
        }
        assert_eq!(
            choose_slot(FLEET, &[director_window(120, 4)], 0),
            new_window(1)
        );
        assert_eq!(
            choose_slot(FLEET, &[director_window(50, 1)], 0),
            new_window(1)
        );
    }

    #[test]
    fn a_director_window_shorter_than_one_slot_is_skipped() {
        assert_eq!(
            choose_slot(FLEET, &[director_window(29, 0)], 0),
            new_window(1)
        );
        assert_eq!(
            choose_slot(
                FLEET,
                &[director_window(20, 0), overflow_window(1, 120, 1, 0)],
                1
            ),
            existing(1, ColumnSide::Left)
        );
    }

    #[test]
    fn an_overflow_window_fills_its_left_column_then_its_right_column() {
        for (left, right, expected) in [
            (1, 0, Some(ColumnSide::Left)),
            (2, 0, Some(ColumnSide::Right)),
            (2, 1, Some(ColumnSide::Right)),
            (2, 2, None),
        ] {
            let choice = choose_slot(
                FLEET,
                &[director_window(60, 2), overflow_window(1, 60, left, right)],
                1,
            );
            let expected = match expected {
                Some(side) => existing(1, side),
                None => new_window(2),
            };
            assert_eq!(choice, expected, "left {left} right {right}");
        }
    }

    #[test]
    fn every_overflow_column_accepts_one_member_however_short_the_window() {
        for (left, right, expected) in [
            (0, 0, Some(ColumnSide::Left)),
            (1, 0, Some(ColumnSide::Right)),
            (1, 1, None),
        ] {
            let choice = choose_slot(
                FLEET,
                &[director_window(10, 0), overflow_window(1, 10, left, right)],
                1,
            );
            let expected = match expected {
                Some(side) => existing(1, side),
                None => new_window(2),
            };
            assert_eq!(choice, expected, "left {left} right {right}");
        }
    }

    #[test]
    fn a_new_window_takes_the_ordinal_after_the_last_allocated_one() {
        assert_eq!(
            choose_slot(
                FLEET,
                &[director_window(60, 2), overflow_window(2, 60, 2, 2)],
                5
            ),
            new_window(6)
        );
        assert_eq!(
            choose_slot(42, &[director_window(60, 2)], 0),
            SlotChoice::NewWindow {
                ordinal: 1,
                window_name: "cafleet-42-1".into(),
            }
        );
    }

    #[test]
    fn overflow_windows_are_scanned_in_ascending_ordinal_order() {
        assert_eq!(
            choose_slot(
                FLEET,
                &[
                    director_window(60, 2),
                    overflow_window(1, 60, 2, 2),
                    overflow_window(3, 60, 2, 1),
                    overflow_window(4, 60, 0, 0),
                ],
                4
            ),
            existing(3, ColumnSide::Right)
        );
    }

    #[test]
    fn a_freed_director_slot_is_reused_before_any_overflow_window() {
        assert_eq!(
            choose_slot(
                FLEET,
                &[director_window(120, 3), overflow_window(1, 120, 1, 0)],
                1
            ),
            existing(0, ColumnSide::Right)
        );
    }

    #[test]
    fn an_emptied_left_column_is_refilled_before_the_right_column() {
        assert_eq!(
            choose_slot(
                FLEET,
                &[director_window(60, 2), overflow_window(1, 60, 0, 1)],
                1
            ),
            existing(1, ColumnSide::Left)
        );
    }

    #[test]
    #[should_panic]
    fn choose_slot_requires_the_director_window_first() {
        choose_slot(FLEET, &[overflow_window(1, 60, 0, 0)], 1);
    }

    #[test]
    #[should_panic]
    fn choose_slot_requires_at_least_the_director_window() {
        choose_slot(FLEET, &[], 0);
    }

    #[test]
    #[should_panic]
    fn choose_slot_requires_strictly_ascending_ordinals() {
        choose_slot(
            FLEET,
            &[
                director_window(60, 2),
                overflow_window(2, 60, 0, 0),
                overflow_window(1, 60, 0, 0),
            ],
            2,
        );
    }

    #[test]
    #[should_panic]
    fn choose_slot_refuses_a_repeated_ordinal() {
        choose_slot(
            FLEET,
            &[
                director_window(60, 2),
                overflow_window(1, 60, 2, 2),
                overflow_window(1, 60, 0, 0),
            ],
            1,
        );
    }
}
