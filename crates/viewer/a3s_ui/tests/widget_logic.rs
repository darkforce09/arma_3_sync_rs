//! Pure widget logic: percent text and the tri-state rule (spec B25, B34).

use a3s_ui::widgets::{CheckState, percent_text};

#[test]
fn a11_percent_text() {
    for ((value, min, max), expected) in [
        ((0, 0, 100), "0%"),
        ((50, 0, 100), "50%"),
        ((1, 0, 3), "33%"),
        ((2, 0, 3), "67%"),
        ((1, 0, 200), "1%"),
        ((150, 0, 100), "100%"),
        ((-5, 0, 100), "0%"),
        ((5, 10, 10), "0%"),
        ((7, 10, 5), "0%"),
    ] {
        assert_eq!(
            percent_text(value, min, max),
            expected,
            "{value}, {min}, {max}"
        );
    }
}

#[test]
fn a12_check_state_for_node() {
    use CheckState::{Checked, Indeterminate, Unchecked};
    let none: [bool; 0] = [];

    assert_eq!(CheckState::for_node(true, true, true, none), Checked);
    assert_eq!(CheckState::for_node(false, true, true, none), Unchecked);
    assert_eq!(
        CheckState::for_node(false, false, true, [true, false]),
        Indeterminate
    );
    assert_eq!(
        CheckState::for_node(true, false, true, [true, false]),
        Indeterminate
    );
    assert_eq!(
        CheckState::for_node(true, false, true, [true, true]),
        Checked
    );
    assert_eq!(
        CheckState::for_node(false, false, true, [true, true]),
        Unchecked
    );
    assert_eq!(
        CheckState::for_node(false, false, true, [false, false]),
        Unchecked
    );
    assert_eq!(
        CheckState::for_node(true, false, true, [false, false]),
        Checked
    );
    assert_eq!(CheckState::for_node(true, false, true, none), Checked);
    assert_eq!(CheckState::for_node(false, false, true, none), Unchecked);
    // The root (no parent) is never indeterminate.
    assert_eq!(
        CheckState::for_node(true, false, false, [true, false]),
        Checked
    );
    assert_eq!(
        CheckState::for_node(false, false, false, [true, false]),
        Unchecked
    );
    // Only direct children count: a single unselected child whose own children are mixed.
    assert_eq!(CheckState::for_node(true, false, true, [false]), Checked);
    assert_eq!(CheckState::for_node(false, false, true, [false]), Unchecked);
}
