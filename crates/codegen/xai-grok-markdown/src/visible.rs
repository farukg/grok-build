/// C0 controls other than tab and newline are terminal commands. A literal ESC
/// starts a sequence the terminal consumes, so the rest of the line vanishes.
/// Show those bytes as text; tab and newline stay, they structure the line.
pub(crate) fn visible_text(segment: &str) -> String {
    segment
        .chars()
        .map(|character| match character {
            '\t' | '\n' => character,
            control if control.is_control() => '\u{FFFD}',
            other => other,
        })
        .collect()
}
