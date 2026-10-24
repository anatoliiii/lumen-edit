//! Virtual rows preserve the editor's fixed line-height layout without touching Rope.
#[derive(Default, Clone)]
pub struct Layout {
    pub line: usize,
    pub column: usize,
    pub extra: usize,
    pub text: String,
}
impl Layout {
    pub fn new(line: usize, column: usize, text: &str) -> Self {
        Self { line, column, extra: text.bytes().filter(|b| *b == b'\n').count(), text: text.into() }
    }
    pub fn row(&self, line: usize) -> usize {
        line + if line > self.line { self.extra } else { 0 }
    }
    pub fn line_at(&self, row: usize) -> usize {
        if row > self.line { if row <= self.line + self.extra { self.line } else { row - self.extra } } else { row }
    }
    pub fn in_ghost_rows(&self, row: usize) -> bool {
        self.extra > 0 && row > self.line && row <= self.line + self.extra
    }
    pub fn expanded_lines(&self, tab: usize) -> Vec<(usize, String)> {
        self.text
            .split('\n')
            .enumerate()
            .map(|(i, line)| {
                let start = if i == 0 { self.column } else { 0 };
                let mut column = start;
                let mut expanded = String::new();
                for c in line.chars() {
                    if c == '\t' {
                        let width = tab.max(1) - column % tab.max(1);
                        expanded.push_str(&" ".repeat(width));
                        column += width;
                    } else {
                        expanded.push(c);
                        column += 1;
                    }
                }
                (start, expanded)
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn virtual_rows_and_tabs() {
        let l = Layout::new(3, 7, "send(\n\tvalue\n)");
        assert_eq!(l.extra, 2);
        assert_eq!(l.row(4), 6);
        assert_eq!(l.line_at(5), 3);
        assert_eq!(l.line_at(6), 4);
        assert_eq!(l.expanded_lines(4)[1], (0, "    value".into()));
    }
}
