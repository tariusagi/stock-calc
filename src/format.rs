//! Number formatting with `'` as the thousand separator, plus a text input
//! that inserts the separators live while the user types.

use eframe::egui::{
    self, Align, Key, TextEdit, Ui,
    text::{CCursor, CCursorRange},
};

pub const SEP: char = '\'';

/// Groups a string of ASCII digits in threes: "1234567" -> "1'234'567".
fn group_digits(digits: &str) -> String {
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(SEP);
        }
        out.push(c);
    }
    out
}

/// Formats a number as a whole number (rounded half away from zero) with
/// `'` separators. The app shows no decimals anywhere.
pub fn fmt_int(v: f64) -> String {
    if !v.is_finite() {
        return "–".to_owned();
    }
    let r = v.round();
    let digits = format!("{:.0}", r.abs());
    let mut out = String::new();
    // `-0.0 < 0.0` is false, so "-0" is never printed.
    if r < 0.0 {
        out.push('-');
    }
    out += &group_digits(&digits);
    out
}

/// Formats a value for an input field (empty for zero).
pub fn fmt_input(v: f64) -> String {
    if v.round() == 0.0 { String::new() } else { fmt_int(v) }
}

pub fn fmt_signed_pct(v: f64) -> String {
    if !v.is_finite() {
        return "–".to_owned();
    }
    let sign = if v.round() > 0.0 { "+" } else { "" };
    format!("{sign}{}%", fmt_int(v))
}

/// Parses user text such as "1'234" into a whole number; empty or invalid
/// text yields `None`.
pub fn parse_num(s: &str) -> Option<f64> {
    let cleaned: String = s.chars().filter(|&c| c != SEP && !c.is_whitespace()).collect();
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse::<f64>().ok().filter(|v| v.is_finite()).map(f64::round)
}

pub fn parse_or_zero(s: &str) -> f64 {
    parse_num(s).unwrap_or(0.0)
}

/// Normalizes raw typed text into a formatted whole number.
///
/// Returns the new text and the cursor position mapped from `cursor`
/// (a char index into `raw`), so typing feels natural while separators
/// appear and disappear. A pasted decimal like "245.7" is rounded to "246".
fn reformat(raw: &str, cursor: usize) -> (String, usize) {
    if let Some((int_part, frac_part)) = raw.split_once('.') {
        let digits = |s: &str| s.chars().filter(char::is_ascii_digit).collect::<String>();
        let int: f64 = digits(int_part).parse().unwrap_or(0.0);
        let round_up = digits(frac_part).chars().next().is_some_and(|c| c >= '5');
        let v = int + if round_up { 1.0 } else { 0.0 };
        let out = if digits(int_part).is_empty() && !round_up { String::new() } else { fmt_int(v) };
        let len = out.chars().count();
        return (out, len);
    }

    // Keep digits only, remembering how many precede the cursor.
    let mut int = String::new();
    let mut kept_before_cursor = 0;
    for (i, c) in raw.chars().enumerate() {
        if c.is_ascii_digit() {
            int.push(c);
            if i < cursor {
                kept_before_cursor += 1;
            }
        }
    }

    // Leading zeros like "007" collapse to "7".
    let trimmed = int.trim_start_matches('0');
    let removed_zeros = int.len() - trimmed.len();
    let mut int = trimmed.to_owned();
    let mut kept_before_cursor = kept_before_cursor - removed_zeros.min(kept_before_cursor);
    if int.is_empty() && removed_zeros > 0 {
        int.push('0');
        if cursor > 0 {
            kept_before_cursor += 1;
        }
    }

    let out = group_digits(&int);

    // Place the cursor after the same number of digits.
    let mut pos = 0;
    let mut seen = 0;
    for c in out.chars() {
        if seen == kept_before_cursor {
            break;
        }
        pos += 1;
        if c != SEP {
            seen += 1;
        }
    }
    (out, pos)
}

/// A right-aligned whole-number text field that formats with `'`
/// separators as the user types. Returns `true` when the text changed.
pub fn num_input(ui: &mut Ui, text: &mut String, width: f32, hint: &str) -> bool {
    let before = text.clone();
    let mut output = TextEdit::singleline(text)
        .desired_width(width)
        .horizontal_align(Align::RIGHT)
        .hint_text(hint)
        .show(ui);

    if !output.response.response.changed() {
        return false;
    }

    let mut raw = text.clone();
    let mut cursor = output
        .cursor_range
        .map(|r| r.primary.index.0)
        .unwrap_or_else(|| raw.chars().count());

    // If the user only deleted a separator, delete the neighboring digit
    // instead — otherwise the separator would just reappear.
    let strip = |s: &str| s.chars().filter(|&c| c != SEP).collect::<String>();
    if raw.chars().count() + 1 == before.chars().count() && strip(&raw) == strip(&before) {
        let mut chars: Vec<char> = raw.chars().collect();
        if ui.input(|i| i.key_pressed(Key::Delete)) {
            if cursor < chars.len() {
                chars.remove(cursor);
            }
        } else if cursor > 0 {
            chars.remove(cursor - 1);
            cursor -= 1;
        }
        raw = chars.into_iter().collect();
    }

    let (formatted, pos) = reformat(&raw, cursor);
    *text = formatted;
    output
        .state
        .cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(pos))));
    output.state.store(ui.ctx(), output.response.response.id);
    ui.ctx().request_repaint();
    *text != before
}

/// Plain uppercase text input for ticker symbols.
pub fn symbol_input(ui: &mut Ui, text: &mut String, width: f32) -> egui::Response {
    let resp = ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(width)
            .char_limit(12)
            .hint_text("SYMBOL"),
    );
    if resp.changed() {
        *text = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_uppercase)
            .collect();
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_numbers() {
        assert_eq!(fmt_int(1234567.891), "1'234'568");
        assert_eq!(fmt_int(-1234.0), "-1'234");
        assert_eq!(fmt_int(2.5), "3");
        assert_eq!(fmt_int(-0.4), "0");
        assert_eq!(fmt_input(1500.25), "1'500");
        assert_eq!(fmt_input(0.0), "");
        assert_eq!(fmt_signed_pct(6.28), "+6%");
        assert_eq!(fmt_signed_pct(-6.5), "-7%");
        assert_eq!(fmt_signed_pct(0.2), "0%");
    }

    #[test]
    fn parses_numbers() {
        assert_eq!(parse_num("1'234"), Some(1234.0));
        assert_eq!(parse_num("245.7"), Some(246.0));
        assert_eq!(parse_num(""), None);
        assert_eq!(parse_num("abc"), None);
    }

    #[test]
    fn reformats_while_typing() {
        // typed "1234" with cursor at end
        assert_eq!(reformat("1234", 4), ("1'234".into(), 5));
        // inserted a digit in the middle: "1'2534" cursor after '5' (idx 4)
        assert_eq!(reformat("1'2534", 4), ("12'534".into(), 4));
        // letters are dropped
        assert_eq!(reformat("1a23", 4), ("123".into(), 3));
        // leading zeros
        assert_eq!(reformat("0005", 4), ("5".into(), 1));
        // decimals are rounded away
        assert_eq!(reformat("12345.6789", 10), ("12'346".into(), 6));
        assert_eq!(reformat("245.", 4), ("245".into(), 3));
        assert_eq!(reformat(".4", 2), ("".into(), 0));
    }
}
