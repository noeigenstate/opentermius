use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Run {
    pub row: u16,
    pub col: u16,
    pub width: u16,
    pub text: String,
    pub fg: u32,
    pub bg: u32,
    pub bold: bool,
    pub underline: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub runs: Vec<Run>,
    pub lines: Vec<String>,
    pub cursor: (u16, u16),
    pub cursor_visible: bool,
    pub application_cursor: bool,
    pub bracketed_paste: bool,
    pub rows: u16,
    pub cols: u16,
}
impl Snapshot {
    pub fn selected_text(&self, mut start: (i32, i32), mut end: (i32, i32)) -> String {
        if self.rows == 0 || self.cols == 0 {
            return String::new();
        }
        if start > end {
            std::mem::swap(&mut start, &mut end)
        }
        let top = start.0.clamp(0, self.rows as i32 - 1);
        let bottom = end.0.clamp(top, self.rows as i32 - 1);
        let mut lines = vec![String::new(); (bottom - top + 1) as usize];
        for run in &self.runs {
            let row = run.row as i32;
            if row < top || row > bottom {
                continue;
            }
            let from = if row == start.0 { start.1.max(0) } else { 0 };
            let to = if row == end.0 {
                end.1.min(self.cols as i32 - 1)
            } else {
                self.cols as i32 - 1
            };
            let first = run.col as i32;
            let last = first + run.width as i32 - 1;
            if last < from || first > to {
                continue;
            }
            let line = &mut lines[(row - top) as usize];
            if run.text.is_ascii() && run.text.len() == run.width as usize {
                let a = (from - first).max(0) as usize;
                let b = (to - first + 1).min(run.width as i32).max(0) as usize;
                line.push_str(&run.text[a..b]);
            } else {
                line.push_str(&run.text)
            }
        }
        let last = lines.len() - 1;
        for (i, line) in lines.iter_mut().enumerate() {
            if i != last {
                *line = line.trim_end().into()
            }
        }
        lines.join("\n")
    }
}
pub struct Terminal {
    parser: vt100::Parser<Responses>,
}
#[derive(Default)]
struct Responses {
    bytes: Vec<u8>,
}
impl Responses {
    fn push(&mut self, value: &[u8]) {
        if self.bytes.len() + value.len() <= 65536 {
            self.bytes.extend_from_slice(value)
        }
    }
}
impl vt100::Callbacks for Responses {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let param = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        if c == 'n' && param == 6 {
            let (row, col) = screen.cursor_position();
            self.push(
                format!(
                    "\x1b[{}{};{}R",
                    if i1 == Some(b'?') { "?" } else { "" },
                    row + 1,
                    col + 1
                )
                .as_bytes(),
            );
        } else if c == 'n' && param == 5 {
            self.push(b"\x1b[0n");
        } else if c == 'c' && param == 0 {
            self.push(if i1 == Some(b'>') {
                b"\x1b[>0;136;0c"
            } else {
                b"\x1b[?1;2c"
            });
        }
    }
    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        if params.len() > 1 && params[1] == b"?" {
            match params[0] {
                b"10" => self.push(b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\"),
                b"11" => self.push(b"\x1b]11;rgb:1414/1717/2929\x1b\\"),
                _ => {}
            }
        }
    }
}
impl Terminal {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: vt100::Parser::new_with_callbacks(
                rows.clamp(2, 300).try_into().unwrap(),
                cols.clamp(2, 500).try_into().unwrap(),
                3000,
                Responses::default(),
            ),
        }
    }
    pub fn feed(&mut self, data: &[u8]) {
        self.parser.process(data)
    }
    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.parser.callbacks_mut().bytes)
    }
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.set_size(
            rows.clamp(2, 300).try_into().unwrap(),
            cols.clamp(2, 500).try_into().unwrap(),
        )
    }
    pub fn scroll(&mut self, delta: i32) {
        let current = self.parser.screen().scrollback() as i64;
        self.parser
            .screen_mut()
            .set_scrollback((current + delta as i64).max(0) as usize)
    }
    pub fn snapshot(&self) -> Snapshot {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let (rows, cols) = (rows.get(), cols.get());
        let mut runs: Vec<Run> = Vec::new();
        let mut lines = Vec::new();
        for row in 0..rows {
            let mut line = String::new();
            for col in 0..cols {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let text = if cell.contents().is_empty() {
                    " "
                } else {
                    cell.contents()
                };
                line.push_str(text);
                let mut fg = color(cell.fgcolor(), 0xffffff);
                let mut bg = color(cell.bgcolor(), 0x141729);
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg)
                }
                let width = if cell.is_wide() { 2 } else { 1 };
                if let Some(last) = runs.last_mut().filter(|last| {
                    last.row == row
                        && last.col + last.width == col
                        && last.fg == fg
                        && last.bg == bg
                        && last.bold == cell.bold()
                        && last.underline == cell.underline()
                        && width == 1
                        && last.text.is_ascii()
                        && text.is_ascii()
                }) {
                    last.text.push_str(text);
                    last.width += width;
                } else {
                    runs.push(Run {
                        row,
                        col,
                        width,
                        text: text.into(),
                        fg,
                        bg,
                        bold: cell.bold(),
                        underline: cell.underline(),
                    })
                }
            }
            lines.push(line.trim_end().into());
        }
        Snapshot {
            runs,
            lines,
            cursor: screen.cursor_position(),
            cursor_visible: !screen.hide_cursor() && screen.scrollback() == 0,
            application_cursor: screen.application_cursor(),
            bracketed_paste: screen.bracketed_paste(),
            rows,
            cols,
        }
    }
}
fn color(value: vt100::Color, default: u32) -> u32 {
    match value {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => (r as u32) << 16 | (g as u32) << 8 | b as u32,
        vt100::Color::Idx(i) => {
            const COLORS: [u32; 16] = [
                0x1a202a, 0xe2939c, 0x92ccac, 0xe5c58b, 0x8baedb, 0xb7a0d7, 0x84c9cf, 0xd3deec,
                0x687c93, 0xf3a9b1, 0xb2e3bf, 0xf0d6ab, 0xaccaf0, 0xd3baed, 0xa5e0df, 0xf2f6fc,
            ];
            if i < 16 {
                COLORS[i as usize]
            } else if i >= 232 {
                let c = 8 + (i as u32 - 232) * 10;
                c << 16 | c << 8 | c
            } else {
                let n = i as u32 - 16;
                let channel = |v| if v == 0 { 0 } else { 55 + v * 40 };
                channel(n / 36) << 16 | channel(n / 6 % 6) << 8 | channel(n % 6)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copy_uses_terminal_columns_for_wide_characters() {
        let mut t = Terminal::new(4, 20);
        t.feed("你好world".as_bytes());
        let s = t.snapshot();
        assert_eq!(s.selected_text((0, 4), (0, 8)), "world");
        assert_eq!(s.selected_text((0, 3), (0, 0)), "你好");
    }
    #[test]
    fn terminal_query_replies() {
        let mut t = Terminal::new(10, 60);
        t.feed(b"abc\x1b[6n\x1b[5n\x1b[c");
        assert_eq!(t.take_responses(), b"\x1b[1;4R\x1b[0n\x1b[?1;2c");
        assert!(t.take_responses().is_empty());
    }
    #[test]
    fn ansi_unicode_and_cursor() {
        let mut t = Terminal::new(10, 60);
        t.feed("\x1b[31m红色\x1b[0m\r\nhello".as_bytes());
        let s = t.snapshot();
        assert!(s.lines[0].starts_with("红色"));
        assert!(
            s.runs
                .iter()
                .any(|r| r.fg == 0xe2939c && r.text.contains('红'))
        );
        assert_eq!(s.cursor, (1, 5));
    }
    #[test]
    fn alternate_screen_resize_and_history() {
        let mut t = Terminal::new(4, 20);
        t.feed(b"original\x1b[?1049hfull screen\x1b[?1049l");
        assert!(t.snapshot().lines[0].starts_with("original"));
        t.resize(8, 40);
        assert_eq!((t.snapshot().rows, t.snapshot().cols), (8, 40));
        for _ in 0..10000 {
            t.feed(b"line\r\n")
        }
        t.scroll(100000);
        assert!(t.parser.screen().scrollback() <= 3000);
    }
    #[test]
    fn split_utf8_and_clear() {
        let mut t = Terminal::new(4, 20);
        let bytes = "你好".as_bytes();
        t.feed(&bytes[..2]);
        t.feed(&bytes[2..]);
        assert!(t.snapshot().lines[0].starts_with("你好"));
        t.feed(b"\x1b[2J\x1b[H");
        assert!(t.snapshot().lines.iter().all(|l| l.is_empty()));
    }
}
