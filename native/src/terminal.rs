use serde::Serialize;
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyMods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}
impl KeyMods {
    pub fn new(ctrl: bool, alt: bool, shift: bool) -> Self {
        Self { ctrl, alt, shift }
    }
    /// xterm modifier parameter: 1 + shift + 2 * alt + 4 * ctrl.
    fn parameter(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Insert,
    Delete,
    PageUp,
    PageDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Tab,
    BackTab,
    Enter,
    Escape,
    Backspace,
}

/// A terminal key, either one of the named keys, a single character, or
/// committed text (an IME commit or a multi-character key event).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key<'a> {
    Named(NamedKey),
    Char(char),
    Text(&'a str),
}
impl<'a> Key<'a> {
    /// Convert the `text` from a Slint key event into a terminal key.
    ///
    /// Slint encodes special keys as control characters or private use
    /// codepoints (see `Key::F1`, `Key::Home`, ...). Multi-character and
    /// non-ASCII text (IME commits, composed input) is kept verbatim.
    pub fn from_text(text: &'a str) -> Option<Self> {
        let mut chars = text.chars();
        let first = chars.next()?;
        if chars.next().is_some() {
            return Some(Self::Text(text));
        }
        let named = match first {
            '\u{8}' => Some(NamedKey::Backspace),
            '\u{9}' => Some(NamedKey::Tab),
            '\u{a}' => Some(NamedKey::Enter),
            '\u{1b}' => Some(NamedKey::Escape),
            '\u{19}' => Some(NamedKey::BackTab),
            '\u{7f}' => Some(NamedKey::Delete),
            '\u{f700}' => Some(NamedKey::Up),
            '\u{f701}' => Some(NamedKey::Down),
            '\u{f702}' => Some(NamedKey::Left),
            '\u{f703}' => Some(NamedKey::Right),
            '\u{f704}' => Some(NamedKey::F1),
            '\u{f705}' => Some(NamedKey::F2),
            '\u{f706}' => Some(NamedKey::F3),
            '\u{f707}' => Some(NamedKey::F4),
            '\u{f708}' => Some(NamedKey::F5),
            '\u{f709}' => Some(NamedKey::F6),
            '\u{f70a}' => Some(NamedKey::F7),
            '\u{f70b}' => Some(NamedKey::F8),
            '\u{f70c}' => Some(NamedKey::F9),
            '\u{f70d}' => Some(NamedKey::F10),
            '\u{f70e}' => Some(NamedKey::F11),
            '\u{f70f}' => Some(NamedKey::F12),
            '\u{f727}' => Some(NamedKey::Insert),
            '\u{f729}' => Some(NamedKey::Home),
            '\u{f72b}' => Some(NamedKey::End),
            '\u{f72c}' => Some(NamedKey::PageUp),
            '\u{f72d}' => Some(NamedKey::PageDown),
            _ => None,
        };
        if let Some(named) = named {
            Some(Self::Named(named))
        } else if first.is_control() {
            None
        } else if first.is_ascii() {
            Some(Self::Char(first))
        } else {
            Some(Self::Text(text))
        }
    }
}

fn with_alt(mut bytes: Vec<u8>, alt: bool) -> Vec<u8> {
    if alt {
        bytes.insert(0, 0x1b);
    }
    bytes
}

/// Encode a committed text run (IME commit, composed character) as UTF-8.
pub fn encode_text(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

fn encode_char(c: char, mods: KeyMods) -> Option<Vec<u8>> {
    if mods.ctrl {
        let byte = match c {
            ' ' | '@' => 0x00,
            'a'..='z' | 'A'..='Z' => (c as u8) & 0x1f,
            '[' => 0x1b,
            '\\' => 0x1c,
            ']' => 0x1d,
            '^' => 0x1e,
            '_' => 0x1f,
            '?' => 0x7f,
            _ => return None,
        };
        return Some(with_alt(vec![byte], mods.alt));
    }
    let mut buf = [0u8; 4];
    let mut bytes = Vec::with_capacity(5);
    if mods.alt {
        bytes.push(0x1b);
    }
    bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    Some(bytes)
}

fn encode_named(key: NamedKey, mods: KeyMods, app_cursor: bool) -> Option<Vec<u8>> {
    use NamedKey::*;
    let parameter = mods.parameter();
    let modified = parameter > 1;
    let cursor = |suffix: char, ss3: char| {
        if modified {
            format!("\x1b[1;{parameter}{suffix}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{ss3}").into_bytes()
        } else {
            format!("\x1b[{suffix}").into_bytes()
        }
    };
    let tilde = |number: u8| {
        if modified {
            format!("\x1b[{number};{parameter}~").into_bytes()
        } else {
            format!("\x1b[{number}~").into_bytes()
        }
    };
    let function = |letter: char| {
        if modified {
            format!("\x1b[1;{parameter}{letter}").into_bytes()
        } else {
            format!("\x1bO{letter}").into_bytes()
        }
    };
    let function_number = |number: u8| {
        if modified {
            format!("\x1b[{number};{parameter}~").into_bytes()
        } else {
            format!("\x1b[{number}~").into_bytes()
        }
    };
    Some(match key {
        Up => cursor('A', 'A'),
        Down => cursor('B', 'B'),
        Right => cursor('C', 'C'),
        Left => cursor('D', 'D'),
        Home => cursor('H', 'H'),
        End => cursor('F', 'F'),
        Insert => tilde(2),
        Delete => tilde(3),
        PageUp => tilde(5),
        PageDown => tilde(6),
        F1 => function('P'),
        F2 => function('Q'),
        F3 => function('R'),
        F4 => function('S'),
        F5 => function_number(15),
        F6 => function_number(17),
        F7 => function_number(18),
        F8 => function_number(19),
        F9 => function_number(20),
        F10 => function_number(21),
        F11 => function_number(23),
        F12 => function_number(24),
        Tab | BackTab => {
            let mut bytes = if key == Tab && !mods.shift {
                vec![b'\t']
            } else {
                b"\x1b[Z".to_vec()
            };
            if mods.alt {
                bytes.insert(0, 0x1b);
            }
            bytes
        }
        Enter => with_alt(vec![b'\r'], mods.alt),
        Escape => with_alt(vec![0x1b], mods.alt),
        Backspace => with_alt(vec![if mods.ctrl { 0x08 } else { 0x7f }], mods.alt),
    })
}

/// Encode a key using xterm/VT100 sequences.
///
/// `app_cursor` is DECCKM (application cursor keys); it changes unmodified
/// arrow and Home/End encoding to SS3. Modified keys always use CSI with the
/// xterm modifier parameter. Alt is an ESC prefix for keys that have no
/// modifier parameter (Enter, Tab, Backspace, ...).
pub fn encode_key(key: Key<'_>, mods: KeyMods, app_cursor: bool) -> Option<Vec<u8>> {
    match key {
        Key::Text(text) => {
            if text.is_empty() {
                None
            } else {
                Some(with_alt(encode_text(text), mods.alt))
            }
        }
        Key::Char(c) => encode_char(c, mods),
        Key::Named(named) => encode_named(named, mods, app_cursor),
    }
}

/// Encode a numeric-keypad key. Application keypad mode (DECPAM, `ESC =`)
/// switches digits and operators to SS3 sequences.
pub fn encode_keypad(key: char, mods: KeyMods, app_keypad: bool) -> Option<Vec<u8>> {
    if app_keypad {
        let suffix = match key {
            '0'..='9' => b'p' + (key as u8 - b'0'),
            '.' => b'n',
            ',' => b'l',
            '-' => b'm',
            '+' => b'k',
            '*' => b'j',
            '/' => b'o',
            '=' => b'X',
            '\r' | '\n' => b'M',
            _ => return encode_key(Key::Char(key), mods, false),
        };
        Some(with_alt(vec![0x1b, b'O', suffix], mods.alt))
    } else if key == '\r' || key == '\n' {
        Some(with_alt(vec![b'\r'], mods.alt))
    } else {
        encode_key(Key::Char(key), mods, false)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    /// No button (motion without a held button).
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Press,
    Release,
    Motion,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseMods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    pub kind: MouseKind,
    pub button: MouseButton,
    /// 1-based terminal cell.
    pub row: u16,
    /// 1-based terminal cell.
    pub col: u16,
    pub mods: MouseMods,
}

fn push_utf8(bytes: &mut Vec<u8>, value: u32) {
    let c = char::from_u32(value).unwrap_or('\u{fffd}');
    let mut buf = [0u8; 4];
    bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
}

/// Encode a mouse event for the protocol the remote enabled with
/// `CSI ? 1000/1002/1003 h` and `CSI ? 1005/1006 h`.
///
/// Returns `None` when mouse reporting is disabled or the event is not
/// reported in the active mode (for example a release in X10 mode).
pub fn encode_mouse(
    event: MouseEvent,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    if mode == MouseProtocolMode::None {
        return None;
    }
    if event.kind == MouseKind::Release && mode == MouseProtocolMode::Press {
        return None;
    }
    if event.kind == MouseKind::Motion {
        match mode {
            MouseProtocolMode::Press | MouseProtocolMode::PressRelease => return None,
            MouseProtocolMode::ButtonMotion if event.button == MouseButton::None => return None,
            _ => {}
        }
    }
    if matches!(event.kind, MouseKind::Press | MouseKind::Release)
        && event.button == MouseButton::None
    {
        return None;
    }
    let mut code: u16 = match event.kind {
        MouseKind::WheelUp => 64,
        MouseKind::WheelDown => 65,
        MouseKind::WheelLeft => 66,
        MouseKind::WheelRight => 67,
        MouseKind::Motion => {
            (match event.button {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
                MouseButton::None => 3,
            }) + 32
        }
        MouseKind::Press | MouseKind::Release => match event.button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::None => 3,
        },
    };
    if event.kind == MouseKind::Release && encoding != MouseProtocolEncoding::Sgr {
        // Legacy encodings cannot report which button was released.
        code = 3;
    }
    if event.mods.shift {
        code += 4;
    }
    if event.mods.alt {
        code += 8;
    }
    if event.mods.ctrl {
        code += 16;
    }
    let col = event.col.max(1);
    let row = event.row.max(1);
    match encoding {
        MouseProtocolEncoding::Sgr => {
            let suffix = if event.kind == MouseKind::Release {
                'm'
            } else {
                'M'
            };
            Some(format!("\x1b[<{code};{col};{row}{suffix}").into_bytes())
        }
        MouseProtocolEncoding::Default => {
            let col = col.min(223) as u8;
            let row = row.min(223) as u8;
            Some(vec![
                0x1b,
                b'[',
                b'M',
                (code as u8).saturating_add(32),
                col + 32,
                row + 32,
            ])
        }
        MouseProtocolEncoding::Utf8 => {
            let mut bytes = vec![0x1b, b'[', b'M'];
            push_utf8(&mut bytes, code as u32 + 32);
            push_utf8(&mut bytes, col.min(2015) as u32 + 32);
            push_utf8(&mut bytes, row.min(2015) as u32 + 32);
            Some(bytes)
        }
    }
}

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
    pub application_keypad: bool,
    pub bracketed_paste: bool,
    pub mouse_mode: MouseProtocolMode,
    pub mouse_encoding: MouseProtocolEncoding,
    pub rows: u16,
    pub cols: u16,
}
impl Snapshot {
    pub fn mouse_reporting(&self) -> bool {
        self.mouse_mode != MouseProtocolMode::None
    }
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
            application_keypad: screen.application_keypad(),
            bracketed_paste: screen.bracketed_paste(),
            mouse_mode: screen.mouse_protocol_mode(),
            mouse_encoding: screen.mouse_protocol_encoding(),
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

    fn mods(ctrl: bool, alt: bool, shift: bool) -> KeyMods {
        KeyMods { ctrl, alt, shift }
    }
    const PLAIN: KeyMods = KeyMods {
        ctrl: false,
        alt: false,
        shift: false,
    };
    fn key(named: NamedKey, mods: KeyMods, app_cursor: bool) -> Vec<u8> {
        encode_key(Key::Named(named), mods, app_cursor).unwrap()
    }
    #[test]
    fn function_keys_and_modifiers() {
        let plain = [
            (NamedKey::F1, "\x1bOP"),
            (NamedKey::F2, "\x1bOQ"),
            (NamedKey::F3, "\x1bOR"),
            (NamedKey::F4, "\x1bOS"),
            (NamedKey::F5, "\x1b[15~"),
            (NamedKey::F6, "\x1b[17~"),
            (NamedKey::F7, "\x1b[18~"),
            (NamedKey::F8, "\x1b[19~"),
            (NamedKey::F9, "\x1b[20~"),
            (NamedKey::F10, "\x1b[21~"),
            (NamedKey::F11, "\x1b[23~"),
            (NamedKey::F12, "\x1b[24~"),
        ];
        for (named, expected) in plain {
            assert_eq!(key(named, PLAIN, false), expected.as_bytes(), "{named:?}");
            assert_eq!(key(named, PLAIN, true), expected.as_bytes(), "{named:?}");
        }
        assert_eq!(
            key(NamedKey::F1, mods(false, false, true), false),
            b"\x1b[1;2P"
        );
        assert_eq!(
            key(NamedKey::F4, mods(true, false, true), false),
            b"\x1b[1;6S"
        );
        assert_eq!(
            key(NamedKey::F5, mods(false, true, false), false),
            b"\x1b[15;3~"
        );
        assert_eq!(
            key(NamedKey::F12, mods(true, true, true), false),
            b"\x1b[24;8~"
        );
    }
    #[test]
    fn all_xterm_modifier_combinations_for_named_keys() {
        let csi_keys = [
            (NamedKey::Up, 'A'),
            (NamedKey::Down, 'B'),
            (NamedKey::Right, 'C'),
            (NamedKey::Left, 'D'),
            (NamedKey::Home, 'H'),
            (NamedKey::End, 'F'),
            (NamedKey::F1, 'P'),
            (NamedKey::F2, 'Q'),
            (NamedKey::F3, 'R'),
            (NamedKey::F4, 'S'),
        ];
        let tilde_keys = [
            (NamedKey::Insert, 2),
            (NamedKey::Delete, 3),
            (NamedKey::PageUp, 5),
            (NamedKey::PageDown, 6),
            (NamedKey::F5, 15),
            (NamedKey::F6, 17),
            (NamedKey::F7, 18),
            (NamedKey::F8, 19),
            (NamedKey::F9, 20),
            (NamedKey::F10, 21),
            (NamedKey::F11, 23),
            (NamedKey::F12, 24),
        ];
        for (modifier, parameter) in [
            (mods(false, false, true), 2),
            (mods(false, true, false), 3),
            (mods(false, true, true), 4),
            (mods(true, false, false), 5),
            (mods(true, false, true), 6),
            (mods(true, true, false), 7),
            (mods(true, true, true), 8),
        ] {
            for application in [false, true] {
                for (named, suffix) in csi_keys {
                    assert_eq!(
                        key(named, modifier, application),
                        format!("\x1b[1;{parameter}{suffix}").as_bytes()
                    );
                }
                for (named, number) in tilde_keys {
                    assert_eq!(
                        key(named, modifier, application),
                        format!("\x1b[{number};{parameter}~").as_bytes()
                    );
                }
            }
        }
    }
    #[test]
    fn cursor_and_editing_keys() {
        assert_eq!(key(NamedKey::Up, PLAIN, false), b"\x1b[A");
        assert_eq!(key(NamedKey::Down, PLAIN, true), b"\x1bOB");
        assert_eq!(key(NamedKey::Right, PLAIN, false), b"\x1b[C");
        assert_eq!(key(NamedKey::Left, PLAIN, true), b"\x1bOD");
        assert_eq!(key(NamedKey::Home, PLAIN, false), b"\x1b[H");
        assert_eq!(key(NamedKey::End, PLAIN, true), b"\x1bOF");
        assert_eq!(
            key(NamedKey::Up, mods(true, false, false), false),
            b"\x1b[1;5A"
        );
        assert_eq!(
            key(NamedKey::Down, mods(false, false, true), false),
            b"\x1b[1;2B"
        );
        assert_eq!(
            key(NamedKey::Left, mods(false, true, false), false),
            b"\x1b[1;3D"
        );
        assert_eq!(
            key(NamedKey::Right, mods(false, true, true), false),
            b"\x1b[1;4C"
        );
        assert_eq!(
            key(NamedKey::Home, mods(true, false, true), true),
            b"\x1b[1;6H"
        );
        assert_eq!(
            key(NamedKey::End, mods(true, true, true), false),
            b"\x1b[1;8F"
        );
        assert_eq!(key(NamedKey::Insert, PLAIN, false), b"\x1b[2~");
        assert_eq!(key(NamedKey::Delete, PLAIN, false), b"\x1b[3~");
        assert_eq!(key(NamedKey::PageUp, PLAIN, false), b"\x1b[5~");
        assert_eq!(key(NamedKey::PageDown, PLAIN, false), b"\x1b[6~");
        assert_eq!(
            key(NamedKey::Insert, mods(false, false, true), false),
            b"\x1b[2;2~"
        );
        assert_eq!(
            key(NamedKey::Delete, mods(true, false, false), false),
            b"\x1b[3;5~"
        );
        assert_eq!(
            key(NamedKey::PageUp, mods(false, true, false), false),
            b"\x1b[5;3~"
        );
        assert_eq!(
            key(NamedKey::PageDown, mods(true, true, true), false),
            b"\x1b[6;8~"
        );
    }
    #[test]
    fn ctrl_letters_and_punctuation() {
        for letter in b'a'..=b'z' {
            let lower = letter as char;
            let upper = lower.to_ascii_uppercase();
            let expected = vec![letter & 0x1f];
            assert_eq!(
                encode_key(Key::Char(lower), mods(true, false, false), false),
                Some(expected.clone())
            );
            assert_eq!(
                encode_key(Key::Char(upper), mods(true, false, false), false),
                Some(expected)
            );
        }
        for (c, expected) in [
            (' ', 0x00),
            ('@', 0x00),
            ('[', 0x1b),
            ('\\', 0x1c),
            (']', 0x1d),
            ('^', 0x1e),
            ('_', 0x1f),
            ('?', 0x7f),
        ] {
            assert_eq!(
                encode_key(Key::Char(c), mods(true, false, false), false),
                Some(vec![expected]),
                "{c:?}"
            );
        }
        assert_eq!(
            encode_key(Key::Char('c'), mods(true, true, false), false),
            Some(vec![0x1b, 0x03])
        );
    }
    #[test]
    fn alt_prefix_tab_backspace_and_enter() {
        assert_eq!(
            encode_key(Key::Char('a'), mods(false, true, false), false),
            Some(b"\x1ba".to_vec())
        );
        assert_eq!(
            encode_key(Key::Char('中'), PLAIN, false),
            Some("中".as_bytes().to_vec())
        );
        assert_eq!(key(NamedKey::Backspace, PLAIN, false), b"\x7f");
        assert_eq!(
            key(NamedKey::Backspace, mods(true, false, false), false),
            b"\x08"
        );
        assert_eq!(
            key(NamedKey::Backspace, mods(false, true, false), false),
            b"\x1b\x7f"
        );
        assert_eq!(key(NamedKey::Tab, PLAIN, false), b"\t");
        assert_eq!(
            key(NamedKey::Tab, mods(false, false, true), false),
            b"\x1b[Z"
        );
        assert_eq!(key(NamedKey::BackTab, PLAIN, false), b"\x1b[Z");
        assert_eq!(
            key(NamedKey::BackTab, mods(false, true, false), false),
            b"\x1b\x1b[Z"
        );
        assert_eq!(key(NamedKey::Enter, PLAIN, false), b"\r");
        assert_eq!(
            key(NamedKey::Enter, mods(false, true, false), true),
            b"\x1b\r"
        );
        assert_eq!(key(NamedKey::Escape, PLAIN, false), b"\x1b");
        assert_eq!(
            key(NamedKey::Escape, mods(false, true, false), false),
            b"\x1b\x1b"
        );
    }
    #[test]
    fn slint_key_text_mapping() {
        let named = [
            ('\u{8}', NamedKey::Backspace),
            ('\u{9}', NamedKey::Tab),
            ('\u{a}', NamedKey::Enter),
            ('\u{1b}', NamedKey::Escape),
            ('\u{19}', NamedKey::BackTab),
            ('\u{7f}', NamedKey::Delete),
            ('\u{f700}', NamedKey::Up),
            ('\u{f701}', NamedKey::Down),
            ('\u{f702}', NamedKey::Left),
            ('\u{f703}', NamedKey::Right),
            ('\u{f704}', NamedKey::F1),
            ('\u{f705}', NamedKey::F2),
            ('\u{f706}', NamedKey::F3),
            ('\u{f707}', NamedKey::F4),
            ('\u{f708}', NamedKey::F5),
            ('\u{f709}', NamedKey::F6),
            ('\u{f70a}', NamedKey::F7),
            ('\u{f70b}', NamedKey::F8),
            ('\u{f70c}', NamedKey::F9),
            ('\u{f70d}', NamedKey::F10),
            ('\u{f70e}', NamedKey::F11),
            ('\u{f70f}', NamedKey::F12),
            ('\u{f727}', NamedKey::Insert),
            ('\u{f729}', NamedKey::Home),
            ('\u{f72b}', NamedKey::End),
            ('\u{f72c}', NamedKey::PageUp),
            ('\u{f72d}', NamedKey::PageDown),
        ];
        for (text, expected) in named {
            assert_eq!(
                Key::from_text(&text.to_string()),
                Some(Key::Named(expected)),
                "{text:?}"
            );
        }
        assert_eq!(Key::from_text("a"), Some(Key::Char('a')));
        assert_eq!(Key::from_text(" "), Some(Key::Char(' ')));
        assert_eq!(Key::from_text(""), None);
        assert_eq!(Key::from_text("\u{1}"), None);
        assert_eq!(Key::from_text("中"), Some(Key::Text("中")));
        assert_eq!(Key::from_text("你好ab"), Some(Key::Text("你好ab")));
        assert_eq!(
            encode_key(Key::from_text("\u{f704}").unwrap(), PLAIN, false),
            Some(b"\x1bOP".to_vec())
        );
    }
    #[test]
    fn ime_text_is_utf8() {
        assert_eq!(encode_text("中文"), "中文".as_bytes());
        assert_eq!(
            encode_key(Key::Text("你好"), PLAIN, false),
            Some("你好".as_bytes().to_vec())
        );
        assert_eq!(
            encode_key(Key::Text("中文abc"), mods(false, true, false), false),
            Some([b"\x1b".as_slice(), "中文abc".as_bytes()].concat())
        );
        let key = Key::from_text("中文abc").unwrap();
        assert_eq!(key, Key::Text("中文abc"));
        assert_eq!(
            encode_key(key, PLAIN, false),
            Some("中文abc".as_bytes().to_vec())
        );
    }
    #[test]
    fn application_keypad() {
        assert_eq!(encode_keypad('0', PLAIN, true), Some(b"\x1bOp".to_vec()));
        assert_eq!(encode_keypad('9', PLAIN, true), Some(b"\x1bOy".to_vec()));
        assert_eq!(encode_keypad('.', PLAIN, true), Some(b"\x1bOn".to_vec()));
        assert_eq!(encode_keypad(',', PLAIN, true), Some(b"\x1bOl".to_vec()));
        assert_eq!(encode_keypad('-', PLAIN, true), Some(b"\x1bOm".to_vec()));
        assert_eq!(encode_keypad('+', PLAIN, true), Some(b"\x1bOk".to_vec()));
        assert_eq!(encode_keypad('*', PLAIN, true), Some(b"\x1bOj".to_vec()));
        assert_eq!(encode_keypad('/', PLAIN, true), Some(b"\x1bOo".to_vec()));
        assert_eq!(encode_keypad('\r', PLAIN, true), Some(b"\x1bOM".to_vec()));
        assert_eq!(encode_keypad('7', PLAIN, false), Some(b"7".to_vec()));
        assert_eq!(encode_keypad('\r', PLAIN, false), Some(b"\r".to_vec()));
    }
    fn mouse(kind: MouseKind, button: MouseButton, row: u16, col: u16) -> MouseEvent {
        MouseEvent {
            kind,
            button,
            row,
            col,
            mods: MouseMods::default(),
        }
    }
    #[test]
    fn mouse_sgr_encoding() {
        let sgr = MouseProtocolEncoding::Sgr;
        let mode = MouseProtocolMode::PressRelease;
        assert_eq!(
            encode_mouse(mouse(MouseKind::Press, MouseButton::Left, 5, 10), mode, sgr),
            Some(b"\x1b[<0;10;5M".to_vec())
        );
        assert_eq!(
            encode_mouse(
                mouse(MouseKind::Release, MouseButton::Left, 5, 10),
                mode,
                sgr
            ),
            Some(b"\x1b[<0;10;5m".to_vec())
        );
        assert_eq!(
            encode_mouse(mouse(MouseKind::Press, MouseButton::Right, 2, 3), mode, sgr),
            Some(b"\x1b[<2;3;2M".to_vec())
        );
        assert_eq!(
            encode_mouse(
                mouse(MouseKind::WheelDown, MouseButton::None, 2, 3),
                mode,
                sgr
            ),
            Some(b"\x1b[<65;3;2M".to_vec())
        );
        let mut event = mouse(MouseKind::Press, MouseButton::Left, 5, 10);
        event.mods = MouseMods {
            ctrl: true,
            alt: false,
            shift: true,
        };
        assert_eq!(
            encode_mouse(event, mode, sgr),
            Some(b"\x1b[<20;10;5M".to_vec())
        );
        assert_eq!(
            encode_mouse(
                mouse(MouseKind::Motion, MouseButton::Left, 5, 10),
                MouseProtocolMode::ButtonMotion,
                sgr
            ),
            Some(b"\x1b[<32;10;5M".to_vec())
        );
        assert_eq!(
            encode_mouse(
                mouse(MouseKind::Motion, MouseButton::None, 5, 10),
                MouseProtocolMode::AnyMotion,
                sgr
            ),
            Some(b"\x1b[<35;10;5M".to_vec())
        );
    }
    #[test]
    fn mouse_legacy_and_utf8_encoding() {
        assert_eq!(
            encode_mouse(
                mouse(MouseKind::Press, MouseButton::Left, 5, 10),
                MouseProtocolMode::Press,
                MouseProtocolEncoding::Default
            ),
            Some(vec![0x1b, b'[', b'M', 32, 42, 37])
        );
        assert_eq!(
            encode_mouse(
                mouse(MouseKind::Release, MouseButton::Left, 5, 10),
                MouseProtocolMode::PressRelease,
                MouseProtocolEncoding::Default
            ),
            Some(vec![0x1b, b'[', b'M', 35, 42, 37])
        );
        assert_eq!(
            encode_mouse(
                mouse(MouseKind::WheelUp, MouseButton::None, 2, 3),
                MouseProtocolMode::Press,
                MouseProtocolEncoding::Default
            ),
            Some(vec![0x1b, b'[', b'M', 96, 35, 34])
        );
        assert_eq!(
            encode_mouse(
                mouse(MouseKind::Press, MouseButton::Left, 200, 100),
                MouseProtocolMode::Press,
                MouseProtocolEncoding::Utf8
            ),
            Some(vec![0x1b, b'[', b'M', 32, 0xc2, 0x84, 0xc3, 0xa8])
        );
        assert_eq!(
            encode_mouse(
                mouse(MouseKind::Press, MouseButton::Left, 300, 300),
                MouseProtocolMode::Press,
                MouseProtocolEncoding::Default
            ),
            Some(vec![0x1b, b'[', b'M', 32, 255, 255])
        );
    }
    #[test]
    fn mouse_mode_filtering() {
        let sgr = MouseProtocolEncoding::Sgr;
        assert_eq!(
            encode_mouse(
                mouse(MouseKind::Press, MouseButton::Left, 1, 1),
                MouseProtocolMode::None,
                sgr
            ),
            None
        );
        assert_eq!(
            encode_mouse(
                mouse(MouseKind::Release, MouseButton::Left, 1, 1),
                MouseProtocolMode::Press,
                sgr
            ),
            None
        );
        assert_eq!(
            encode_mouse(
                mouse(MouseKind::Motion, MouseButton::Left, 1, 1),
                MouseProtocolMode::PressRelease,
                sgr
            ),
            None
        );
        assert_eq!(
            encode_mouse(
                mouse(MouseKind::Motion, MouseButton::None, 1, 1),
                MouseProtocolMode::ButtonMotion,
                sgr
            ),
            None
        );
        assert_eq!(
            encode_mouse(
                mouse(MouseKind::Motion, MouseButton::Left, 1, 1),
                MouseProtocolMode::AnyMotion,
                sgr
            ),
            Some(b"\x1b[<32;1;1M".to_vec())
        );
    }
    #[test]
    fn snapshot_reports_terminal_modes() {
        let mut t = Terminal::new(10, 60);
        assert!(!t.snapshot().mouse_reporting());
        t.feed(b"\x1b[?1002h\x1b[?1006h\x1b[?1h\x1b=");
        let s = t.snapshot();
        assert_eq!(s.mouse_mode, MouseProtocolMode::ButtonMotion);
        assert_eq!(s.mouse_encoding, MouseProtocolEncoding::Sgr);
        assert!(s.application_cursor);
        assert!(s.application_keypad);
        assert!(s.mouse_reporting());
        t.feed(b"\x1b[?1003h\x1b[?1005h");
        let s = t.snapshot();
        assert_eq!(s.mouse_mode, MouseProtocolMode::AnyMotion);
        assert_eq!(s.mouse_encoding, MouseProtocolEncoding::Utf8);
        t.feed(b"\x1b[?1002l\x1b[?1006l\x1b[?1l\x1b>");
        // Resetting an inactive mode must not disable the active protocol.
        assert_eq!(t.snapshot().mouse_mode, MouseProtocolMode::AnyMotion);
        assert_eq!(t.snapshot().mouse_encoding, MouseProtocolEncoding::Utf8);
        t.feed(b"\x1b[?1003l\x1b[?1005l");
        let s = t.snapshot();
        assert!(!s.mouse_reporting());
        assert_eq!(s.mouse_mode, MouseProtocolMode::None);
        assert_eq!(s.mouse_encoding, MouseProtocolEncoding::Default);
        assert!(!s.application_cursor);
        assert!(!s.application_keypad);
    }
}
