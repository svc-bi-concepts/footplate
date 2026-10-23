//! What a session's pane shows, as styled lines: the ANSI of a
//! `SessionScreen` answer, parsed. Pane bytes are untrusted (anything an
//! agent printed, including malformed escapes), so the parsing is vte's,
//! the standard robust part, and not by hand. Only the text and its SGR
//! styling are kept: every other sequence is dropped, never shown and
//! never executed — no cursor moves, no clears, a static snapshot renders.

use vte::{Parser, Perform};

/// A colour a span is drawn in: one of the 256, or an RGB triple.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// One of the palette's 256: the sixteen, the cube, the greys.
    Ansi(u8),
    /// A truecolour triple.
    Rgb(u8, u8, u8),
}

impl Color {
    /// The triple the colour draws as. The sixteen are Campbell's, the
    /// Windows Terminal default set, drawn for dark grounds; the rest are
    /// the standard 256 algorithm: the 6×6×6 cube, then the greys.
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            Color::Rgb(r, g, b) => (r, g, b),
            Color::Ansi(n) => match n {
                0 => (0x0c, 0x0c, 0x0c),
                1 => (0xc5, 0x0f, 0x1f),
                2 => (0x13, 0xa1, 0x0e),
                3 => (0xc1, 0x9c, 0x00),
                4 => (0x00, 0x37, 0xda),
                5 => (0x88, 0x17, 0x98),
                6 => (0x3a, 0x96, 0xdd),
                7 => (0xcc, 0xcc, 0xcc),
                8 => (0x76, 0x76, 0x76),
                9 => (0xe7, 0x48, 0x56),
                10 => (0x16, 0xc6, 0x0c),
                11 => (0xf9, 0xf1, 0xa5),
                12 => (0x3b, 0x78, 0xff),
                13 => (0xb4, 0x00, 0x9e),
                14 => (0x61, 0xd6, 0xd6),
                15 => (0xf2, 0xf2, 0xf2),
                16..=231 => {
                    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
                    let cube = n - 16;
                    (
                        LEVELS[(cube / 36) as usize],
                        LEVELS[((cube % 36) / 6) as usize],
                        LEVELS[(cube % 6) as usize],
                    )
                }
                _ => {
                    let grey = 8 + 10 * (n - 232);
                    (grey, grey, grey)
                }
            },
        }
    }
}

/// How a span is drawn: the SGR attributes a pane's styling uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
}

/// One styled run of a line: text drawn whole in its style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

/// The screen parsed: its lines of spans, and the longest line's width
/// in characters, which the view lays out at. Width counts characters,
/// not cells: a wide glyph may overflow into the scroll.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screen {
    pub lines: Vec<Vec<Span>>,
    pub width: usize,
}

impl Screen {
    /// The lines' plain text, styling dropped: what a test reads the
    /// shape by.
    #[cfg(test)]
    fn plain(&self) -> Vec<String> {
        self.lines
            .iter()
            .map(|line| line.iter().map(|span| span.text.as_str()).collect())
            .collect()
    }
}

/// Parses the screen `text` into styled lines. Malformed and unknown
/// sequences are dropped; what is left is text.
pub fn parse(text: &str) -> Screen {
    let mut parser = Parser::new();
    let mut reading = Reading {
        lines: vec![Vec::new()],
        style: Style::default(),
        width: 0,
        widest: 0,
        carriage: false,
    };
    parser.advance(&mut reading, text.as_bytes());
    reading.finish()
}

/// The screen as vte reads it: the lines so far, the style the next text
/// draws in, the line's width, the widest yet, and whether a carriage
/// return stands before what comes next.
struct Reading {
    lines: Vec<Vec<Span>>,
    style: Style,
    width: usize,
    widest: usize,
    carriage: bool,
}

impl Reading {
    /// The screen read: the widest line's width stands with it.
    fn finish(mut self) -> Screen {
        self.widest = self.widest.max(self.width);
        Screen {
            lines: self.lines,
            width: self.widest,
        }
    }

    /// Prints `c` in the style, onto the line's last span where it draws
    /// alike, else a span of its own. A carriage return before it breaks
    /// the line: the snapshot lays out, it does not move a cursor.
    fn print(&mut self, c: char) {
        if c.is_control() {
            return;
        }
        if self.carriage {
            self.break_line();
        }
        let line = self.lines.last_mut().expect("the screen holds a line");
        match line.last_mut() {
            Some(span) if span.style == self.style => span.text.push(c),
            _ => line.push(Span {
                text: c.to_string(),
                style: self.style,
            }),
        }
        self.width += 1;
    }

    /// Breaks the line: what comes next stands on a line of its own.
    fn break_line(&mut self) {
        self.carriage = false;
        self.widest = self.widest.max(self.width);
        self.width = 0;
        self.lines.push(Vec::new());
    }

    /// Applies one SGR code: the subset panes style with. Any other code
    /// is ignored.
    fn code(&mut self, n: u16) {
        match n {
            0 => self.style = Style::default(),
            1 => self.style.bold = true,
            2 => self.style.dim = true,
            3 => self.style.italic = true,
            4 => self.style.underline = true,
            7 => self.style.inverse = true,
            22 => {
                self.style.bold = false;
                self.style.dim = false;
            }
            23 => self.style.italic = false,
            24 => self.style.underline = false,
            27 => self.style.inverse = false,
            30..=37 => self.style.fg = Some(Color::Ansi((n - 30) as u8)),
            39 => self.style.fg = None,
            40..=47 => self.style.bg = Some(Color::Ansi((n - 40) as u8)),
            49 => self.style.bg = None,
            90..=97 => self.style.fg = Some(Color::Ansi((n - 90) as u8 + 8)),
            100..=107 => self.style.bg = Some(Color::Ansi((n - 100) as u8 + 8)),
            _ => {}
        }
    }

    /// Applies an extended colour, `38` for the foreground or `48` for
    /// the background, with the groups after it: `5` and one palette
    /// index, or `2` and three triples. Anything else is dropped, with
    /// what it stood with: a malformed extended colour styles nothing.
    fn extended(&mut self, ground: bool, groups: &mut std::iter::Peekable<vte::ParamsIter<'_>>) {
        let set = |reading: &mut Reading, color: Color| match ground {
            true => reading.style.bg = Some(color),
            false => reading.style.fg = Some(color),
        };
        match groups.next() {
            Some([5]) => {
                if let Some([n]) = groups.next()
                    && let Ok(n) = u8::try_from(*n)
                {
                    set(self, Color::Ansi(n));
                }
            }
            Some([2]) => {
                if let (Some([r]), Some([g]), Some([b])) =
                    (groups.next(), groups.next(), groups.next())
                    && let (Ok(r), Ok(g), Ok(b)) =
                        (u8::try_from(*r), u8::try_from(*g), u8::try_from(*b))
                {
                    set(self, Color::Rgb(r, g, b));
                }
            }
            _ => {}
        }
    }
}

impl Perform for Reading {
    fn print(&mut self, c: char) {
        self.print(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            // A pane ends its lines carriage-return newline; the return
            // alone breaks the line before what comes next, and one
            // before a newline breaks it once.
            b'\n' | 0x0b | 0x0c => self.break_line(),
            b'\r' => self.carriage = true,
            // A tab goes to the next stop of eight, as the terminal does.
            b'\t' => {
                if self.carriage {
                    self.break_line();
                }
                for _ in 0..8 - self.width % 8 {
                    self.print(' ');
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        // SGR alone styles; any other CSI (a cursor move, a clear, a
        // scroll) is dropped, as is one vte or the terminal would not
        // take: SGR takes no intermediates.
        if action != 'm' || ignore || !intermediates.is_empty() {
            return;
        }
        let mut groups = params.iter().peekable();
        // No codes is a reset, as `\x1b[m` is.
        if groups.peek().is_none() {
            self.style = Style::default();
            return;
        }
        while let Some(group) = groups.next() {
            match group {
                [38] => self.extended(false, &mut groups),
                [48] => self.extended(true, &mut groups),
                // A colon group (vte splits semicolons apart): only the
                // extended colours come in that form.
                [38, 5, n] => {
                    if let Ok(n) = u8::try_from(*n) {
                        self.style.fg = Some(Color::Ansi(n));
                    }
                }
                [48, 5, n] => {
                    if let Ok(n) = u8::try_from(*n) {
                        self.style.bg = Some(Color::Ansi(n));
                    }
                }
                [38, 2, r, g, b] => {
                    if let (Ok(r), Ok(g), Ok(b)) =
                        (u8::try_from(*r), u8::try_from(*g), u8::try_from(*b))
                    {
                        self.style.fg = Some(Color::Rgb(r, g, b));
                    }
                }
                [48, 2, r, g, b] => {
                    if let (Ok(r), Ok(g), Ok(b)) =
                        (u8::try_from(*r), u8::try_from(*g), u8::try_from(*b))
                    {
                        self.style.bg = Some(Color::Rgb(r, g, b));
                    }
                }
                [n] => self.code(*n),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real Claude Code pane: the yardmaster's, done, read with the
    /// `SessionScreen` form (`herdr pane read --source visible --format
    /// ansi`).
    const CLAUDE: &str = include_str!("fixtures/screen-claude.ansi");
    /// A real Muse pane: a builder working, read the same way.
    const MUSE: &str = include_str!("fixtures/screen-muse.ansi");

    /// The spans' styles over the screen: what a test finds a styling by.
    fn styles(screen: &Screen) -> Vec<Style> {
        screen
            .lines
            .iter()
            .flatten()
            .map(|span| span.style)
            .collect()
    }

    #[test]
    fn the_claude_pane_reads_as_its_lines_with_its_256_colours() {
        let screen = parse(CLAUDE);
        let plain = screen.plain();
        assert_eq!(plain.len(), 51);
        assert_eq!(
            plain[0],
            "⏺ vvy2 (yardtest runs kept away from the real yard) landed at"
        );
        assert_eq!(
            plain[50],
            "  ⏵⏵ bypass permissions on (shift+tab to cycle) · ← 1 agent"
        );
        assert_eq!(screen.width, 62);
        // No escape and no carriage return shows: the styling is spans.
        for line in &plain {
            assert!(!line.contains('\x1b'), "{line:?}");
            assert!(!line.contains('\r'), "{line:?}");
        }
        let styles = styles(&screen);
        // The 256-colour foregrounds and the grey ground of the inbox
        // lines, and the bold bead ids.
        assert!(styles.contains(&Style {
            fg: Some(Color::Ansi(15)),
            ..Style::default()
        }));
        assert!(styles.contains(&Style {
            fg: Some(Color::Ansi(15)),
            bg: Some(Color::Ansi(8)),
            ..Style::default()
        }));
        assert!(styles.iter().any(|style| style.bold));
        // The update line's bright yellow, on no ground.
        assert!(styles.contains(&Style {
            fg: Some(Color::Ansi(11)),
            ..Style::default()
        }));
    }

    #[test]
    fn the_muse_pane_reads_with_its_truecolours_bold_and_dim() {
        let screen = parse(MUSE);
        let plain = screen.plain();
        assert_eq!(plain.len(), 50);
        assert_eq!(screen.width, 61);
        for line in &plain {
            assert!(!line.contains('\x1b'), "{line:?}");
            assert!(!line.contains('\r'), "{line:?}");
        }
        let styles = styles(&screen);
        assert!(styles.contains(&Style {
            fg: Some(Color::Rgb(204, 211, 219)),
            ..Style::default()
        }));
        assert!(styles.iter().any(|style| style.bold));
        assert!(styles.iter().any(|style| style.dim));
        // The spinner's per-letter shades: several truecolours stand.
        let fg: Vec<Color> = styles.iter().filter_map(|style| style.fg).collect();
        assert!(fg.contains(&Color::Rgb(103, 108, 116)));
        assert!(fg.contains(&Color::Rgb(90, 160, 255)));
    }

    #[test]
    fn the_sgr_subset_styles_and_unstyles() {
        let styled = |text: &str| parse(text).lines[0].clone();
        // Bold, dim, italic, underline, inverse, each on and off.
        assert!(styled("\x1b[1mx").iter().all(|span| span.style.bold));
        assert!(
            styled("\x1b[1m\x1b[22mx")
                .iter()
                .all(|span| !span.style.bold)
        );
        assert!(styled("\x1b[2mx").iter().all(|span| span.style.dim));
        assert!(
            styled("\x1b[2m\x1b[22mx")
                .iter()
                .all(|span| !span.style.dim)
        );
        assert!(styled("\x1b[3mx").iter().all(|span| span.style.italic));
        assert!(
            styled("\x1b[3m\x1b[23mx")
                .iter()
                .all(|span| !span.style.italic)
        );
        assert!(styled("\x1b[4mx").iter().all(|span| span.style.underline));
        assert!(
            styled("\x1b[4m\x1b[24mx")
                .iter()
                .all(|span| !span.style.underline)
        );
        assert!(styled("\x1b[7mx").iter().all(|span| span.style.inverse));
        assert!(
            styled("\x1b[7m\x1b[27mx")
                .iter()
                .all(|span| !span.style.inverse)
        );
        // The sixteen, plain and bright, each ground, and back to none.
        let fg = |text: &str| styled(text)[0].style.fg;
        assert_eq!(fg("\x1b[31mx"), Some(Color::Ansi(1)));
        assert_eq!(fg("\x1b[91mx"), Some(Color::Ansi(9)));
        assert_eq!(fg("\x1b[31m\x1b[39mx"), None);
        let bg = |text: &str| styled(text)[0].style.bg;
        assert_eq!(bg("\x1b[44mx"), Some(Color::Ansi(4)));
        assert_eq!(bg("\x1b[104mx"), Some(Color::Ansi(12)));
        assert_eq!(bg("\x1b[44m\x1b[49mx"), None);
        // The 256 and the truecolours, each ground, in both forms.
        assert_eq!(fg("\x1b[38;5;200mx"), Some(Color::Ansi(200)));
        assert_eq!(bg("\x1b[48;5;8mx"), Some(Color::Ansi(8)));
        assert_eq!(fg("\x1b[38:5:200mx"), Some(Color::Ansi(200)));
        assert_eq!(fg("\x1b[38;2;1;2;3mx"), Some(Color::Rgb(1, 2, 3)));
        assert_eq!(bg("\x1b[48;2;1;2;3mx"), Some(Color::Rgb(1, 2, 3)));
        assert_eq!(fg("\x1b[38:2:1:2:3mx"), Some(Color::Rgb(1, 2, 3)));
        // Codes join with semicolons, and a reset clears all of it. No
        // codes is a reset too.
        let all = styled("\x1b[1;3;38;5;7;48;2;1;2;3mx")[0].style;
        assert_eq!(
            all,
            Style {
                bold: true,
                italic: true,
                fg: Some(Color::Ansi(7)),
                bg: Some(Color::Rgb(1, 2, 3)),
                ..Style::default()
            }
        );
        assert_eq!(styled("\x1b[1;31mx\x1b[0my")[1].style, Style::default());
        assert_eq!(styled("\x1b[1mx\x1b[my")[1].style, Style::default());
        // Text either side of a style change is spans of its own.
        let spans = styled("a\x1b[1mb\x1b[22mc");
        assert_eq!(
            spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(
            spans.iter().map(|span| span.style.bold).collect::<Vec<_>>(),
            [false, true, false]
        );
    }

    #[test]
    fn everything_but_sgr_is_dropped_never_shown_and_never_run() {
        let plain = |text: &str| parse(text).plain().join("\n");
        // Cursor moves, clears, scrolls, modes, titles, device control:
        // none shows, and the text stands in order.
        assert_eq!(plain("a\x1b[2Jb"), "ab");
        assert_eq!(plain("a\x1b[1;1Hb"), "ab");
        assert_eq!(plain("a\x1b[Kb"), "ab");
        assert_eq!(plain("a\x1b[1Ab"), "ab");
        assert_eq!(plain("a\x1b[?25lb"), "ab");
        assert_eq!(plain("a\x1b]0;title\x07b"), "ab");
        assert_eq!(plain("a\x1bP1$r0$q\x1b\\b"), "ab");
        assert_eq!(plain("a\x1bMX"), "aX");
        // A truncated sequence styles nothing, and takes no text with it.
        assert_eq!(plain("a\x1b["), "a");
        assert_eq!(plain("a\x1b"), "a");
        assert_eq!(plain("a\x1b[38;5"), "a");
        assert_eq!(plain("a\x1b[38;2;1;2"), "a");
        // Unknown SGR is ignored: out of range, no triple of its own, or
        // a colon styling this draws none of.
        let style = |text: &str| parse(text).lines[0][0].style;
        assert_eq!(style("\x1b[999mx"), Style::default());
        assert_eq!(style("\x1b[21mx"), Style::default());
        assert_eq!(style("\x1b[4:3mx"), Style::default());
        assert_eq!(style("\x1b[38;5;999mx"), Style::default());
        assert_eq!(style("\x1b[38;2;1;2;999mx"), Style::default());
        assert_eq!(style("\x1b[38;1mx"), Style::default());
        // An SGR with intermediates is none this takes.
        assert_eq!(style("\x1b[?1mx"), Style::default());
    }

    #[test]
    fn lines_break_tabs_stop_and_width_is_the_longest() {
        let screen = parse("ab\r\nc\nd\re");
        assert_eq!(screen.plain(), ["ab", "c", "d", "e"]);
        // A tab goes to the next stop of eight.
        assert_eq!(parse("a\tb").plain(), ["a       b"]);
        assert_eq!(parse("abcdefgh\ti").plain(), ["abcdefgh        i"]);
        // Width counts characters of the longest line.
        assert_eq!(parse("ab\r\ncdef\ng").width, 4);
        assert_eq!(parse("").width, 0);
        assert_eq!(parse("").plain(), [""]);
        // A style runs past a line break, as the terminal keeps it.
        let screen = parse("\x1b[1ma\nb");
        assert_eq!(screen.plain(), ["a", "b"]);
        assert!(screen.lines[1][0].style.bold);
    }

    #[test]
    fn the_sixteen_are_campbell_s_and_the_rest_are_standard() {
        assert_eq!(Color::Ansi(0).rgb(), (0x0c, 0x0c, 0x0c));
        assert_eq!(Color::Ansi(1).rgb(), (0xc5, 0x0f, 0x1f));
        assert_eq!(Color::Ansi(7).rgb(), (0xcc, 0xcc, 0xcc));
        assert_eq!(Color::Ansi(8).rgb(), (0x76, 0x76, 0x76));
        assert_eq!(Color::Ansi(15).rgb(), (0xf2, 0xf2, 0xf2));
        assert_eq!(Color::Ansi(16).rgb(), (0, 0, 0));
        assert_eq!(Color::Ansi(21).rgb(), (0, 0, 255));
        assert_eq!(Color::Ansi(231).rgb(), (255, 255, 255));
        assert_eq!(Color::Ansi(232).rgb(), (8, 8, 8));
        assert_eq!(Color::Ansi(255).rgb(), (238, 238, 238));
        assert_eq!(Color::Rgb(1, 2, 3).rgb(), (1, 2, 3));
    }
}
