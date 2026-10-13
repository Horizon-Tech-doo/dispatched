//! Plain aligned tables for humans; JSON for machines.

pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |cells: Vec<String>| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<w$}", w = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let mut out = vec![line(headers.iter().map(|h| h.to_string()).collect())];
    out.extend(rows.iter().map(|r| line(r.clone())));
    out.join("\n")
}

pub fn ellipsize(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
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
}
