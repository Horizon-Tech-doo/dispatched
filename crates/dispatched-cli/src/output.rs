//! Plain aligned tables for humans; JSON for machines.

/// Bidi controls, line/paragraph separators and zero-width/mark characters
/// that render as nothing (or reorder text) in a terminal.
fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{200B}'..='\u{200F}'
            | '\u{061C}'
            | '\u{FEFF}'
    )
}

/// Replace control characters and invisible formatting characters with a
/// space, so untrusted text cannot inject terminal escapes or reorder the output.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() || is_invisible_format(c) {
                ' '
            } else {
                c
            }
        })
        .collect()
}

pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let headers: Vec<String> = headers.iter().map(|h| sanitize(h)).collect();
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| r.iter().map(|c| sanitize(c)).collect())
        .collect();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |cells: &[String]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<w$}", w = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let mut out = vec![line(&headers)];
    out.extend(rows.iter().map(|r| line(r)));
    out.join("\n")
}

/// `10000` as `10,000`.
pub fn thousands(n: u64) -> String {
    let digits: Vec<char> = n.to_string().chars().rev().collect();
    let groups: Vec<String> = digits
        .chunks(3)
        .map(|g| g.iter().rev().collect::<String>())
        .collect();
    groups.iter().rev().cloned().collect::<Vec<_>>().join(",")
}

pub fn ellipsize(s: &str, max: usize) -> String {
    let flat = sanitize(s);
    if flat.chars().count() <= max {
        flat
    } else {
        format!(
            "{}…",
            flat.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn thousands_groups_digits() {
        use super::thousands;
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(10_000), "10,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    use super::*;

    #[test]
    fn aligns_columns() {
        let t = table(
            &["NAME", "N"],
            &[
                vec!["orders".into(), "3".into()],
                vec!["q".into(), "10".into()],
            ],
        );
        assert_eq!(t, "NAME    N\norders  3\nq       10");
    }

    #[test]
    fn ellipsize_flattens_newlines() {
        assert_eq!(ellipsize("a\nb", 10), "a b");
        assert_eq!(ellipsize("abcdef", 4), "abc…");
    }

    #[test]
    fn invisible_formatting_characters_are_neutralised() {
        for c in ['\u{200B}', '\u{200E}', '\u{200F}', '\u{061C}', '\u{FEFF}'] {
            assert_eq!(sanitize(&format!("a{c}b")), "a b");
        }
    }

    #[test]
    fn table_neutralises_control_and_bidi_characters() {
        let hostile = [
            "\x1b[31mred\x1b[0m",
            "\u{9b}31m",
            "a\x07b",
            "\x1b]8;;http://x\x07t\x1b]8;;\x07",
            "x\u{202e}y\u{2066}z\u{2028}w",
        ];
        for h in hostile {
            let out = table(&["H\x1b[1m"], &[vec![h.to_string()]]);
            assert!(
                !out.chars().any(|c| (c.is_control() && c != '\n') || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}')),
                "not neutralised: {out:?}"
            );
            assert_eq!(out.lines().count(), 2, "{out:?}");
        }
    }
}
