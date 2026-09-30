//! Replay captured terminal output through a VT100 emulator and print what
//! a user would see: the scrollback, then the screen. Handy for checking the
//! inline renderer end to end without a real terminal.
//!
//! Usage: vtdump <file> [cols] [rows]

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: vtdump <file> [cols] [rows]");
    let cols: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(100);
    let rows: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(30);
    let bytes = std::fs::read(path).expect("read capture");
    let mut vt = vt100::Parser::new(rows, cols, 10_000);
    vt.process(&bytes);
    let mut back = 0;
    loop {
        vt.screen_mut().set_scrollback(back + 1);
        if vt.screen().scrollback() == back {
            break;
        }
        back += 1;
    }
    for offset in (1..=back).rev() {
        vt.screen_mut().set_scrollback(offset);
        let row = vt.screen().rows(0, cols).next().unwrap_or_default();
        println!("{}", row.trim_end());
    }
    vt.screen_mut().set_scrollback(0);
    println!("{}", "=".repeat(cols as usize));
    for row in vt.screen().rows(0, cols) {
        println!("{}", row.trim_end());
    }
    let (r, c) = vt.screen().cursor_position();
    println!("{}\ncursor at row {r}, col {c}; hidden: {}", "=".repeat(cols as usize), vt.screen().hide_cursor());
}
