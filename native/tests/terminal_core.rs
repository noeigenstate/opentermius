use opentermius::terminal::{Snapshot, Terminal};

fn line_of<'a>(snapshot: &'a Snapshot, text: &str) -> &'a opentermius::terminal::Run {
    snapshot
        .runs
        .iter()
        .find(|run| run.text.contains(text))
        .unwrap_or_else(|| panic!("no run containing {text:?} in {snapshot:?}"))
}

#[test]
fn wide_cjk_characters_use_two_columns_and_copy_correctly() {
    let mut terminal = Terminal::new(4, 20);
    terminal.feed("你好ab".as_bytes());
    let snapshot = terminal.snapshot();
    assert_eq!(snapshot.cursor, (0, 6));
    assert_eq!(snapshot.lines[0], "你好ab");

    let run = line_of(&snapshot, "你");
    assert_eq!((run.col, run.width), (0, 2));
    let run = line_of(&snapshot, "好");
    assert_eq!((run.col, run.width), (2, 2));

    assert_eq!(snapshot.selected_text((0, 0), (0, 3)), "你好");
    assert_eq!(snapshot.selected_text((0, 1), (0, 2)), "你好");
    assert_eq!(snapshot.selected_text((0, 4), (0, 5)), "ab");
    assert_eq!(snapshot.selected_text((0, 0), (0, 5)), "你好ab");
}

#[test]
fn emoji_combining_characters_and_variation_selectors_keep_columns() {
    let mut terminal = Terminal::new(4, 20);
    terminal.feed("😀e\u{301}x".as_bytes());
    let snapshot = terminal.snapshot();
    assert_eq!(snapshot.cursor, (0, 4));
    assert_eq!(snapshot.lines[0], "😀e\u{301}x");
    assert_eq!(line_of(&snapshot, "😀").width, 2);
    assert_eq!(line_of(&snapshot, "e\u{301}").width, 1);
    assert_eq!(snapshot.selected_text((0, 0), (0, 3)), "😀e\u{301}x");
    assert_eq!(snapshot.selected_text((0, 1), (0, 2)), "😀e\u{301}");

    let mut terminal = Terminal::new(4, 20);
    terminal.feed("❤️x".as_bytes());
    let snapshot = terminal.snapshot();
    assert_eq!(snapshot.cursor, (0, 3));
    assert_eq!(snapshot.selected_text((0, 0), (0, 2)), "❤️x");
}

#[test]
fn wide_character_that_does_not_fit_moves_to_the_next_row() {
    let mut terminal = Terminal::new(4, 5);
    terminal.feed("abcd你".as_bytes());
    let snapshot = terminal.snapshot();
    assert_eq!(snapshot.cursor, (1, 2));
    assert_eq!(snapshot.lines[0], "abcd");
    assert_eq!(snapshot.lines[1], "你");
    assert_eq!(snapshot.selected_text((0, 0), (1, 1)), "abcd\n你");
}

#[test]
fn sgr_16_color_256_color_and_truecolor_are_mapped() {
    let mut terminal = Terminal::new(6, 60);
    terminal.feed(
        b"\x1b[31mR\x1b[91mB\x1b[38;5;16m0\x1b[38;5;196mC\x1b[38;5;232mg\x1b[38;5;255mW\x1b[38;2;1;2;3mT\x1b[48;2;4;5;6mT\x1b[0mN"
    );
    let snapshot = terminal.snapshot();
    assert_eq!(line_of(&snapshot, "R").fg, 0xe2939c);
    assert_eq!(line_of(&snapshot, "B").fg, 0xf3a9b1);
    assert_eq!(line_of(&snapshot, "0").fg, 0x000000);
    assert_eq!(line_of(&snapshot, "C").fg, 0xff0000);
    assert_eq!(line_of(&snapshot, "g").fg, 0x080808);
    assert_eq!(line_of(&snapshot, "W").fg, 0xeeeeee);
    assert_eq!(line_of(&snapshot, "N").fg, 0xffffff);
    assert_eq!(line_of(&snapshot, "N").bg, 0x141729);

    let truecolor: Vec<_> = snapshot
        .runs
        .iter()
        .filter(|run| run.text == "T" && run.fg == 0x010203)
        .collect();
    assert_eq!(truecolor.len(), 2);
    assert!(truecolor.iter().any(|run| run.bg == 0x040506));
}

#[test]
fn sgr_bold_underline_inverse_and_resets_are_reported() {
    let mut terminal = Terminal::new(4, 40);
    terminal.feed(b"\x1b[1;4mBU\x1b[22;24mnormal\x1b[7mI\x1b[0mN");
    let snapshot = terminal.snapshot();
    let bu = line_of(&snapshot, "BU");
    assert!(bu.bold && bu.underline);
    let normal = line_of(&snapshot, "normal");
    assert!(!normal.bold && !normal.underline);
    let inverse = line_of(&snapshot, "I");
    assert_eq!((inverse.fg, inverse.bg), (0x141729, 0xffffff));
    let reset = line_of(&snapshot, "N");
    assert_eq!((reset.fg, reset.bg), (0xffffff, 0x141729));
    assert!(!reset.bold && !reset.underline);
}

#[test]
fn cursor_movement_erase_and_cursor_visibility() {
    let mut terminal = Terminal::new(4, 20);
    terminal.feed(b"abc\r\ndef\x1b[1;2H");
    assert_eq!(terminal.snapshot().cursor, (0, 1));
    terminal.feed(b"\x1b[A");
    assert_eq!(terminal.snapshot().cursor, (0, 1));
    terminal.feed(b"\x1b[B\x1b[C\x1b[D");
    assert_eq!(terminal.snapshot().cursor, (1, 1));
    terminal.feed(b"\x1b[2;3H");
    assert_eq!(terminal.snapshot().cursor, (1, 2));
    terminal.feed(b"\x1b[1G");
    assert_eq!(terminal.snapshot().cursor, (1, 0));
    terminal.feed(b"\x1b[2d");
    assert_eq!(terminal.snapshot().cursor, (1, 0));
    terminal.feed(b"\x1b[10;99H");
    assert_eq!(terminal.snapshot().cursor, (3, 19));

    terminal.feed(b"\x1b[2J\x1b[H");
    assert!(terminal.snapshot().lines.iter().all(String::is_empty));

    terminal.feed(b"visible\x1b[?25l");
    assert!(!terminal.snapshot().cursor_visible);
    terminal.feed(b"\x1b[?25h");
    assert!(terminal.snapshot().cursor_visible);
}

#[test]
fn alternate_screen_restores_the_main_screen() {
    let mut terminal = Terminal::new(4, 20);
    terminal.feed(b"main\x1b[?1049h");
    assert!(terminal.snapshot().lines.iter().all(String::is_empty));
    terminal.feed(b"alt");
    assert_eq!(terminal.snapshot().lines[0], "alt");
    terminal.feed(b"\x1b[?1049l");
    let snapshot = terminal.snapshot();
    assert_eq!(snapshot.lines[0], "main");
    assert_eq!(snapshot.cursor, (0, 4));
}

#[test]
fn bracketed_paste_and_application_cursor_modes_are_exposed() {
    let mut terminal = Terminal::new(4, 20);
    let snapshot = terminal.snapshot();
    assert!(!snapshot.application_cursor);
    assert!(!snapshot.bracketed_paste);

    terminal.feed(b"\x1b[?1h\x1b[?2004h");
    let snapshot = terminal.snapshot();
    assert!(snapshot.application_cursor);
    assert!(snapshot.bracketed_paste);

    terminal.feed(b"\x1b[?1l\x1b[?2004l");
    let snapshot = terminal.snapshot();
    assert!(!snapshot.application_cursor);
    assert!(!snapshot.bracketed_paste);
}

#[test]
fn scrollback_moves_the_viewport_and_hides_the_cursor() {
    let mut terminal = Terminal::new(5, 10);
    for index in 0..3100 {
        terminal.feed(format!("L{index}\r\n").as_bytes());
    }
    let live = terminal.snapshot();
    assert!(
        live.lines.iter().any(|line| line == "L3099"),
        "live view should show the newest line: {:?}",
        live.lines
    );
    assert!(live.cursor_visible);

    terminal.scroll(100000);
    let scrolled = terminal.snapshot();
    assert!(!scrolled.cursor_visible);
    assert!(scrolled.lines[0].starts_with('L'));
    assert_ne!(scrolled.lines, live.lines);
    assert!(!scrolled.lines.iter().any(|line| line == "L3099"));
    terminal.scroll(10);
    assert_eq!(
        terminal.snapshot().lines,
        scrolled.lines,
        "scroll clamps at the oldest row"
    );

    terminal.scroll(-100000);
    let live_again = terminal.snapshot();
    assert!(live_again.cursor_visible);
    assert!(live_again.lines.iter().any(|line| line == "L3099"));

    terminal.scroll(i32::MIN);
    assert!(terminal.snapshot().cursor_visible);
    terminal.scroll(i32::MAX);
    assert!(!terminal.snapshot().cursor_visible);
}

#[test]
fn resize_grows_and_shrinks_within_limits() {
    let mut terminal = Terminal::new(3, 6);
    terminal.feed(b"hello");
    let before = terminal.snapshot();
    assert_eq!((before.rows, before.cols), (3, 6));

    terminal.resize(10, 30);
    let grown = terminal.snapshot();
    assert_eq!((grown.rows, grown.cols), (10, 30));
    assert_eq!(grown.lines[0], "hello");

    terminal.resize(2, 4);
    let shrunk = terminal.snapshot();
    assert_eq!((shrunk.rows, shrunk.cols), (2, 4));
    assert!(shrunk.lines[0].starts_with("hell"));
    assert!(shrunk.cursor.0 < 2 && shrunk.cursor.1 < 4);

    terminal.resize(0, 0);
    let minimal = terminal.snapshot();
    assert_eq!((minimal.rows, minimal.cols), (2, 2));

    terminal.resize(1000, 1000);
    let maximal = terminal.snapshot();
    assert_eq!((maximal.rows, maximal.cols), (300, 500));
}

#[test]
fn resize_that_splits_wide_characters_does_not_panic() {
    let mut terminal = Terminal::new(4, 20);
    terminal.feed("你好世界".as_bytes());
    terminal.resize(2, 3);
    let snapshot = terminal.snapshot();
    assert_eq!((snapshot.rows, snapshot.cols), (2, 3));
    terminal.resize(20, 20);
    terminal.feed("再".as_bytes());
    assert!(!terminal.snapshot().lines.join("").is_empty());
}

#[test]
fn query_replies_cover_cpr_dsr_da_and_osc_queries() {
    let mut terminal = Terminal::new(10, 60);
    terminal.feed(b"\x1b[6n");
    assert_eq!(terminal.take_responses(), b"\x1b[1;1R");

    terminal.feed(b"\x1b[3;7H\x1b[6n\x1b[?6n\x1b[5n");
    assert_eq!(terminal.take_responses(), b"\x1b[3;7R\x1b[?3;7R\x1b[0n");

    terminal.feed(b"\x1b[c\x1b[>c");
    assert_eq!(terminal.take_responses(), b"\x1b[?1;2c\x1b[>0;136;0c");

    terminal.feed(b"\x1b]10;?\x1b\\\x1b]11;?\x07\x1b]12;?\x1b\\");
    assert_eq!(
        terminal.take_responses(),
        b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\\x1b]11;rgb:1414/1717/2929\x1b\\"
    );
    assert!(terminal.take_responses().is_empty());
}

#[test]
fn repeated_queries_are_bounded_to_the_response_buffer() {
    let mut terminal = Terminal::new(10, 60);
    for _ in 0..20000 {
        terminal.feed(b"\x1b[6n");
    }
    let responses = terminal.take_responses();
    assert!(!responses.is_empty());
    assert!(
        responses.len() <= 65536,
        "response buffer grew to {}",
        responses.len()
    );
    assert_eq!(
        responses.len() % 6,
        0,
        "CPR replies are whole units (got {})",
        responses.len()
    );
}

#[test]
fn very_long_lines_and_large_output_do_not_panic() {
    let mut terminal = Terminal::new(10, 20);
    let long = "a".repeat(100_000);
    terminal.feed(long.as_bytes());
    let snapshot = terminal.snapshot();
    assert_eq!((snapshot.rows, snapshot.cols), (10, 20));
    assert!(snapshot.lines.iter().all(|line| line.len() <= 20));
    assert!(
        snapshot
            .lines
            .iter()
            .all(|line| line.chars().all(|c| c == 'a'))
    );

    terminal.feed(b"\r\nend");
    assert!(terminal.snapshot().lines[9].contains("end"));
}

#[test]
fn invalid_utf8_and_split_sequences_are_handled_without_losing_ascii() {
    let mut terminal = Terminal::new(4, 20);
    terminal.feed(b"a");
    terminal.feed(&[0xe4, 0xbd]);
    terminal.feed(&[0xa0]);
    terminal.feed(b"b\xffc");
    assert_eq!(terminal.snapshot().lines[0], "a你bc");

    terminal.feed(b"\x1b[2J\x1b[H");
    terminal.feed(&[0x80]);
    terminal.feed("😀".as_bytes().split_at(1).0);
    terminal.feed(&"😀".as_bytes()[1..]);
    terminal.feed(b"z");
    let snapshot = terminal.snapshot();
    assert!(snapshot.lines[0].contains('😀'));
    assert!(snapshot.lines[0].ends_with('z'));

    terminal.feed(b"\x1b[2J\x1b[H");
    terminal.feed(b"a\x00b\xc0\xafc");
    let snapshot = terminal.snapshot();
    assert!(snapshot.lines[0].contains('a'));
    assert!(snapshot.lines[0].contains('c'));
}

#[test]
fn selection_across_rows_trims_intermediate_trailing_spaces() {
    let mut terminal = Terminal::new(6, 20);
    terminal.feed(b"one  \r\ntwo  \r\nthree  ");
    let snapshot = terminal.snapshot();
    assert_eq!(snapshot.selected_text((0, 0), (1, 4)), "one\ntwo  ");
    assert_eq!(snapshot.selected_text((1, 4), (0, 0)), "one\ntwo  ");
    assert_eq!(snapshot.selected_text((0, 0), (2, 6)), "one\ntwo\nthree  ");
    assert_eq!(snapshot.selected_text((2, 0), (2, 6)), "three  ");
    assert_eq!(snapshot.selected_text((0, -5), (0, 2)), "one");
    assert_eq!(Snapshot::default().selected_text((0, 0), (1, 1)), "");
}

#[test]
fn selection_across_soft_wrapped_rows_has_no_line_break() {
    let mut terminal = Terminal::new(4, 20);
    terminal.feed(b"abcdefghijklmnopqrstuvwxy");
    let snapshot = terminal.snapshot();
    assert_eq!(snapshot.lines[0], "abcdefghijklmnopqrst");
    assert_eq!(snapshot.lines[1], "uvwxy");
    assert_eq!(
        snapshot.selected_text((0, 0), (1, 4)),
        "abcdefghijklmnopqrstuvwxy"
    );
}

#[test]
fn selection_across_soft_wrapped_rows_keeps_intentional_spaces() {
    let mut terminal = Terminal::new(4, 10);
    terminal.feed(b"abc       def");
    let snapshot = terminal.snapshot();
    assert_eq!(snapshot.lines[0], "abc");
    assert_eq!(snapshot.lines[1], "def");
    assert_eq!(snapshot.selected_text((0, 0), (1, 2)), "abc       def");
}

#[test]
fn selection_with_wide_and_combining_characters_uses_columns() {
    let mut terminal = Terminal::new(4, 20);
    terminal.feed("你e\u{301}好".as_bytes());
    let snapshot = terminal.snapshot();
    assert_eq!(snapshot.selected_text((0, 0), (0, 3)), "你e\u{301}好");
    assert_eq!(snapshot.selected_text((0, 1), (0, 2)), "你e\u{301}");
    assert_eq!(snapshot.selected_text((0, 3), (0, 4)), "好");
}

#[test]
fn snapshot_lines_trim_trailing_spaces_but_keep_leading_spaces() {
    let mut terminal = Terminal::new(4, 20);
    terminal.feed(b"  text   \r\nnext");
    let snapshot = terminal.snapshot();
    assert_eq!(snapshot.lines[0], "  text");
    assert_eq!(snapshot.lines[1], "next");
}

#[test]
fn snapshot_uses_clamped_dimensions_for_degenerate_input() {
    let terminal = Terminal::new(0, 0);
    let snapshot = terminal.snapshot();
    assert_eq!((snapshot.rows, snapshot.cols), (2, 2));
    assert!(snapshot.lines.iter().all(String::is_empty));
}
