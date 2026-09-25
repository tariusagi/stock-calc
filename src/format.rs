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

/// Formats a number with a fixed number of decimals and `'` separators.
pub fn fmt_num(v: f64, decimals: usize) -> String {
    if !v.is_finite() {
        return "–".to_owned();
    }
    let s = format!("{:.*}", decimals, v.abs());
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    let mut out = String::new();
    // Avoid printing "-0.00".
    if v < 0.0 && s.chars().any(|c| c.is_ascii_digit() && c != '0') {
        out.push('-');
    }
    out += &group_digits(int);
    if !frac.is_empty() {
        out.push('.');
        out += frac;
    }
    out
}

/// Formats a money amount with 2 decimals.
pub fn fmt_money(v: f64) -> String {
    fmt_num(v, 2)
}

/// Formats a price for an input field: up to 4 decimals, trailing zeros trimmed.
pub fn fmt_input(v: f64) -> String {
    if v == 0.0 {
        return String::new();
    }
    let mut s = fmt_num(v, 4);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

pub fn fmt_signed_pct(v: f64) -> String {
    if !v.is_finite() {
        return "–".to_owned();
    }
    let sign = if v > 0.0 { "+" } else { "" };
    format!("{sign}{}%", fmt_num(v, 2))
}

/// Parses user text such as "1'234.5"; empty or invalid text yields `None`.
pub fn parse_num(s: &str) -> Option<f64> {
    let cleaned: String = s.chars().filter(|&c| c != SEP && !c.is_whitespace()).collect();
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse::<f64>().ok().filter(|v| v.is_finite())
}

pub fn parse_or_zero(s: &str) -> f64 {
    parse_num(s).unwrap_or(0.0)
}

/// Normalizes raw typed text into a formatted number string.
///
/// Returns the new text and the cursor position mapped from `cursor`
/// (a char index into `raw`), so typing feels natural while separators
/// appear and disappear.
fn reformat(raw: &str, cursor: usize, allow_decimals: bool) -> (String, usize) {
    // Keep digits and a single '.', remembering how many kept chars
    // precede the cursor.
    let mut int = String::new();
    let mut frac: Option<String> = None;
    let mut kept_before_cursor = 0;
    for (i, c) in raw.chars().enumerate() {
        let keep = if c.is_ascii_digit() {
            match &mut frac {
                Some(f) => f.push(c),
                None => int.push(c),
            }
            true
        } else if (c == '.' || c == ',') && allow_decimals && frac.is_none() {
            frac = Some(String::new());
            true
        } else {
            false
        };
        if keep && i < cursor {
            kept_before_cursor += 1;
        }
    }

    // Leading zeros like "007" collapse to "7" (but "0.5" stays).
    let trimmed = int.trim_start_matches('0');
    let removed_zeros = int.len() - trimmed.len();
    let mut int = trimmed.to_owned();
    let mut kept_before_cursor = kept_before_cursor - removed_zeros.min(kept_before_cursor);
    if int.is_empty() && (removed_zeros > 0 || frac.is_some()) {
        int.push('0');
        if cursor > 0 {
            kept_before_cursor += 1;
        }
    }

    let mut out = group_digits(&int);
    if let Some(f) = frac {
        out.push('.');
        out += &f;
    }

    // Place the cursor after the same number of significant chars.
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

/// A right-aligned numeric text field that formats with `'` separators as
/// the user types. Returns `true` when the text changed.
pub fn num_input(ui: &mut Ui, text: &mut String, width: f32, allow_decimals: bool, hint: &str) -> bool {
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

    let (formatted, pos) = reformat(&raw, cursor, allow_decimals);
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
        assert_eq!(fmt_num(1234567.891, 2), "1'234'567.89");
        assert_eq!(fmt_num(-1234.0, 0), "-1'234");
        assert_eq!(fmt_num(999.0, 2), "999.00");
        assert_eq!(fmt_num(-0.001, 2), "0.00");
        assert_eq!(fmt_input(1500.25), "1'500.25");
        assert_eq!(fmt_input(1000.0), "1'000");
    }

    #[test]
    fn parses_numbers() {
        assert_eq!(parse_num("1'234.5"), Some(1234.5));
        assert_eq!(parse_num(""), None);
        assert_eq!(parse_num("abc"), None);
    }

    #[test]
    fn reformats_while_typing() {
        // typed "1234" with cursor at end
        assert_eq!(reformat("1234", 4, true), ("1'234".into(), 5));
        // inserted a digit in the middle: "1'2534" cursor after '5' (idx 4)
        assert_eq!(reformat("1'2534", 4, true), ("12'534".into(), 4));
        // decimals are not grouped
        assert_eq!(reformat("12345.6789", 10, true), ("12'345.6789".into(), 11));
        // letters are dropped, decimals disallowed for quantities
        assert_eq!(reformat("1a2.3", 5, false), ("123".into(), 3));
        // leading zeros
        assert_eq!(reformat("0005", 4, true), ("5".into(), 1));
        assert_eq!(reformat(".5", 2, true), ("0.5".into(), 3));
    }
}
