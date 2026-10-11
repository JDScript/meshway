//! A small aligned table: rounded box lines on a TTY, plain columns elsewhere.
use super::term::{Role, Term, pad, width};
use std::io::{self, Write};

pub(crate) struct Cell {
    pub text: String,
    pub role: Role,
}

impl Cell {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            role: Role::Plain,
        }
    }
    pub fn styled(text: impl Into<String>, role: Role) -> Self {
        Self {
            text: text.into(),
            role,
        }
    }
}

pub(crate) struct Table {
    headers: Vec<&'static str>,
    rows: Vec<Vec<Cell>>,
}

impl Table {
    pub fn new(headers: Vec<&'static str>) -> Self {
        Self {
            headers,
            rows: Vec::new(),
        }
    }

    pub fn row(&mut self, cells: Vec<Cell>) {
        debug_assert_eq!(cells.len(), self.headers.len());
        self.rows.push(cells);
    }

    pub fn render(&self, output: &mut impl Write, term: &Term) -> io::Result<()> {
        let widths: Vec<usize> = (0..self.headers.len())
            .map(|column| {
                self.rows
                    .iter()
                    .map(|row| width(&row[column].text))
                    .chain([width(self.headers[column])])
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let cells = |row: &[Cell], header: bool| -> Vec<String> {
            row.iter()
                .zip(&widths)
                .map(|(cell, width)| {
                    let role = if header { Role::Bold } else { cell.role };
                    term.paint(role, pad(&cell.text, *width))
                })
                .collect()
        };
        let headers: Vec<Cell> = self.headers.iter().map(|h| Cell::new(*h)).collect();
        if !term.rich {
            for row in std::iter::once(&headers).chain(&self.rows) {
                let line = cells(row, false).join("  ");
                writeln!(output, "{}", line.trim_end())?;
            }
            return Ok(());
        }
        let rule = |left: &str, middle: &str, right: &str| {
            let parts: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
            term.paint(Role::Dim, format!("{left}{}{right}", parts.join(middle)))
        };
        let bar = term.paint(Role::Dim, "│");
        let line = |values: Vec<String>| {
            let inner = values.join(&format!(" {bar} "));
            format!("{bar} {inner} {bar}")
        };
        writeln!(output, "{}", rule("╭", "┬", "╮"))?;
        writeln!(output, "{}", line(cells(&headers, true)))?;
        writeln!(output, "{}", rule("├", "┼", "┤"))?;
        for row in &self.rows {
            writeln!(output, "{}", line(cells(row, false)))?;
        }
        writeln!(output, "{}", rule("╰", "┴", "╯"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Table {
        let mut table = Table::new(vec!["NAME", "STATE"]);
        table.row(vec![
            Cell::new("节点-一"),
            Cell::styled("direct", Role::Good),
        ]);
        table.row(vec![Cell::new("b"), Cell::new("idle")]);
        table
    }

    #[test]
    fn plain_columns_align_by_display_width_without_escapes() {
        let mut output = Vec::new();
        table().render(&mut output, &Term::plain()).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert_eq!(text, "NAME     STATE\n节点-一  direct\nb        idle\n");
        assert!(!text.contains('\u{1b}'));
    }

    #[test]
    fn rich_tables_draw_boxes_with_equal_display_widths() {
        let mut output = Vec::new();
        let term = Term {
            rich: true,
            color: false,
        };
        table().render(&mut output, &term).unwrap();
        let text = String::from_utf8(output).unwrap();
        let widths: Vec<usize> = text.lines().map(width).collect();
        assert!(widths.iter().all(|w| *w == widths[0]), "{text}");
        assert!(text.starts_with('╭') && text.contains("│ 节点-一 │ direct │"));
        let mut colored = Vec::new();
        table().render(&mut colored, &Term::rich()).unwrap();
        assert!(String::from_utf8(colored).unwrap().contains('\u{1b}'));
    }
}
