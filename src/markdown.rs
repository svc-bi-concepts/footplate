//! The small part of Markdown a message is drawn with, the part an agent's
//! mail uses: fenced code and inline code, bullets and numbered lists one
//! level deep, headings down to the third, bold and italic. Everything else
//! is text as it was written.
//!
//! A text becomes blocks, one under the other. A block is drawn without its
//! marks and knows where each of its characters stands in the text as it
//! was written, so that what is selected in it is copied with its marks.

use std::ops::Range;

/// What a block is drawn as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Lines as they were written, line breaks and blank lines included.
    Text,
    /// What stood between two fences, or after one that was never closed.
    Code,
    /// A heading of level 1 to 3.
    Heading(u8),
    /// An item of a list: `marker` is drawn before it, a dot for a bullet
    /// and the number as it was written. `depth` is 0, or 1 for an item
    /// under another.
    Item { marker: String, depth: u8 },
}

/// How a stretch of a block is drawn apart from the rest of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

/// A stretch of a block's text that stands unbroken in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Piece {
    shown: usize,
    source: usize,
    len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    /// What is drawn: the source without its marks.
    pub text: String,
    /// All of the source the block stands for, its marks included.
    pub source: Range<usize>,
    /// The stretches of `text` that are not plain, in order.
    pub marks: Vec<(Range<usize>, Marks)>,
    /// A blank line stood between this block and the one before it.
    pub apart: bool,
    pieces: Vec<Piece>,
}

impl Block {
    /// Where in the source the boundary `shown` of the drawn text is. Marks
    /// that are not drawn may stand there: the place is from before them to
    /// after them, and empty where there are none.
    pub fn source_at(&self, shown: usize) -> Range<usize> {
        let mut here = self
            .pieces
            .iter()
            .filter(|p| p.shown <= shown && shown <= p.shown + p.len)
            .map(|p| p.source + (shown - p.shown));
        let first = here.next();
        let last = here.next_back().or(first);
        let before = match first {
            Some(at) if shown > 0 => at,
            _ => self.source.start,
        };
        let after = match last {
            Some(at) if shown < self.text.len() => at,
            _ => self.source.end,
        };
        before..after
    }

    /// What a copy of this block puts on the clipboard: a code block's
    /// is the source between the fences, byte for byte, and no other block
    /// has a copy control.
    pub fn copy_text(&self) -> Option<&str> {
        matches!(self.kind, Kind::Code).then_some(&self.text)
    }

    /// What of the drawn text the stretch `source` of the source covers.
    pub fn shown(&self, source: Range<usize>) -> Range<usize> {
        let mut shown: Option<Range<usize>> = None;
        for p in &self.pieces {
            let start = source.start.max(p.source);
            let end = source.end.min(p.source + p.len);
            if start >= end {
                continue;
            }
            let (start, end) = (p.shown + start - p.source, p.shown + end - p.source);
            shown = Some(shown.map_or(start..end, |s| s.start.min(start)..s.end.max(end)));
        }
        shown.unwrap_or(0..0)
    }
}

/// A block as it is put together.
#[derive(Default)]
struct Out {
    text: String,
    pieces: Vec<Piece>,
    marks: Vec<(Range<usize>, Marks)>,
}

impl Out {
    /// `text` is drawn next, and stands at `source` in the source.
    fn push(&mut self, text: &str, source: usize, marks: Marks) {
        if text.is_empty() {
            return;
        }
        let shown = self.text.len();
        self.text.push_str(text);
        match self.pieces.last_mut() {
            Some(p) if p.shown + p.len == shown && p.source + p.len == source => {
                p.len += text.len()
            }
            _ => self.pieces.push(Piece {
                shown,
                source,
                len: text.len(),
            }),
        }
        if marks == Marks::default() {
            return;
        }
        match self.marks.last_mut() {
            Some((range, same)) if range.end == shown && *same == marks => {
                range.end = self.text.len()
            }
            _ => self.marks.push((shown..self.text.len(), marks)),
        }
    }

    fn block(self, kind: Kind, source: Range<usize>, apart: bool) -> Block {
        Block {
            kind,
            text: self.text,
            source,
            marks: self.marks,
            apart,
            pieces: self.pieces,
        }
    }
}

/// How many times `c` stands at the start of `text`.
fn run_of(text: &str, c: char) -> usize {
    text.chars().take_while(|&x| x == c).count()
}

/// What may close a sentence or a bracket after a link: as in the links a
/// text is searched for.
const AFTER_A_LINK: [char; 11] = ['.', ',', ';', ':', '!', '?', ')', ']', '>', '"', '\''];

/// Whether a `*` or `_` with `before` it and `after` it may open emphasis:
/// at the start of a word, and not in the middle of one or before a dot or
/// a slash, which is a glob or a path.
fn opens(before: Option<char>, after: Option<char>) -> bool {
    let free = before.is_none_or(|c| c.is_whitespace() || "([{\"'".contains(c));
    free && after.is_some_and(|c| c.is_alphanumeric() || "*_`\"'([".contains(c))
}

/// Where in `text`, which follows an opening run of `n` times `c`, the run
/// that closes it stands: of the same length, at the end of a word.
fn closer(text: &str, c: char, n: usize) -> Option<usize> {
    let mut at = 0;
    while let Some(found) = text[at..].find(c).map(|i| at + i) {
        let run = run_of(&text[found..], c);
        let before = text[..found].chars().next_back();
        let mut after = text[found + run..].chars();
        let ends = match after.next() {
            None => true,
            // A dot closes a sentence, and stands inside a file's name.
            Some('.') => after.next().is_none_or(|c| !c.is_alphanumeric()),
            Some(c) => c.is_whitespace() || ",;:!?)]}\"'".contains(c),
        };
        if run == n && before.is_some_and(|c| !c.is_whitespace()) && ends {
            return Some(found);
        }
        at = found + run;
    }
    None
}

/// Where in `text`, which follows an opening run of `n` backticks, a run of
/// exactly as many stands.
fn code_end(text: &str, n: usize) -> Option<usize> {
    let mut at = 0;
    while let Some(found) = text[at..].find('`').map(|i| at + i) {
        let run = run_of(&text[found..], '`');
        if run == n {
            return Some(found);
        }
        at = found + run;
    }
    None
}

/// One line, or what stands inside emphasis, which begins at `at` in the
/// source: inline code, bold and italic lose their marks, and a mark that
/// opens or closes nothing stays as it was written.
fn inline(out: &mut Out, line: &str, at: usize, marks: Marks) {
    // What was read and not yet put out: text without a mark in it.
    let mut from = 0;
    let mut i = 0;
    while i < line.len() {
        let rest = &line[i..];
        let Some(c) = rest.chars().next() else {
            break;
        };
        // A link is no place for a mark: its stars and underscores are its
        // own, except those after its end, which may close what is around it.
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let run = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let link =
                rest[..run].trim_end_matches(|c| AFTER_A_LINK.contains(&c) || "*_`".contains(c));
            i += link.len().max("http".len());
            continue;
        }
        if c == '`' {
            let n = run_of(rest, '`');
            if let Some(end) = code_end(&rest[n..], n) {
                out.push(&line[from..i], at + from, marks);
                let code = Marks {
                    code: true,
                    ..marks
                };
                out.push(&rest[n..n + end], at + i + n, code);
                i += n + end + n;
                from = i;
            } else {
                i += n;
            }
            continue;
        }
        if c == '*' || c == '_' {
            let n = run_of(rest, c);
            let before = line[..i].chars().next_back();
            let end = (n <= 3 && opens(before, rest[n..].chars().next()))
                .then(|| closer(&rest[n..], c, n))
                .flatten();
            if let Some(end) = end {
                out.push(&line[from..i], at + from, marks);
                let inner = Marks {
                    bold: marks.bold || n >= 2,
                    italic: marks.italic || n != 2,
                    ..marks
                };
                inline(out, &rest[n..n + end], at + i + n, inner);
                i += n + end + n;
                from = i;
            } else {
                i += n;
            }
            continue;
        }
        i += c.len_utf8();
    }
    out.push(&line[from..], at + from, marks);
}

/// The backticks of the fence `line` opens, if it opens one: three or more
/// at its start, and after them at most a word for the language.
fn fence(line: &str) -> Option<usize> {
    let line = line.trim_start_matches(' ');
    let n = run_of(line, '`');
    (n >= 3 && !line[n..].trim_end().contains(['`', ' '])).then_some(n)
}

/// Whether `line` closes a fence of `n` backticks: as many or more, alone.
fn closes(line: &str, n: usize) -> bool {
    let line = line.trim_matches(' ');
    line.len() >= n && line.chars().all(|c| c == '`')
}

/// The level of the heading `line` is and where its words begin, if it is
/// one: one to three `#` at its start and a space.
fn heading(line: &str) -> Option<(u8, usize)> {
    let n = run_of(line, '#');
    let rest = &line[n..];
    let words = rest.trim_start_matches(' ');
    ((1..=3).contains(&n) && rest.starts_with(' ') && !words.is_empty())
        .then(|| (n as u8, line.len() - words.len()))
}

/// The item `line` is, if it is one: how far it is indented, its marker,
/// and where its words begin. A marker is `-`, `*` or a number of up to
/// three digits with a dot or a bracket, and a space follows it.
fn item(line: &str) -> Option<(usize, Range<usize>, usize)> {
    let indent = run_of(line, ' ');
    let rest = &line[indent..];
    let marker = if rest.starts_with(['-', '*']) {
        1
    } else {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if !(1..=3).contains(&digits) || !rest[digits..].starts_with(['.', ')']) {
            return None;
        }
        digits + 1
    };
    let after = &rest[marker..];
    let words = after.trim_start_matches(' ');
    (after.starts_with(' ') && !words.is_empty())
        .then(|| (indent, indent..indent + marker, line.len() - words.len()))
}

/// The blocks `text` is drawn as, from top to bottom. A text without a mark
/// is one block with the text as it is.
pub fn blocks(text: &str) -> Vec<Block> {
    let mut lines = Vec::new();
    let mut start = 0;
    for line in text.split('\n') {
        lines.push((start, line));
        start += line.len() + 1;
    }
    let end_of = |i: usize| lines[i].0 + lines[i].1.len();
    let blank = |i: usize| lines[i].1.trim().is_empty();

    let mut blocks: Vec<Block> = Vec::new();
    // A blank line was passed since the last block.
    let mut apart = false;
    // The lines since the last block that are text and nothing else.
    let mut plain: Option<Range<usize>> = None;
    // They become a block. The blank lines between it and its neighbours
    // are the space between blocks, and no lines of it.
    let flush = |plain: &mut Option<Range<usize>>,
                 blocks: &mut Vec<Block>,
                 apart: &mut bool,
                 more: bool| {
        let Some(mut run) = plain.take() else {
            return;
        };
        while !blocks.is_empty() && !run.is_empty() && blank(run.start) {
            run.start += 1;
            *apart = true;
        }
        let mut after = false;
        while more && !run.is_empty() && blank(run.end - 1) {
            run.end -= 1;
            after = true;
        }
        if !run.is_empty() {
            let mut out = Out::default();
            for i in run.clone() {
                let (at, line) = lines[i];
                if i > run.start {
                    out.push("\n", at - 1, Marks::default());
                }
                inline(&mut out, line, at, Marks::default());
            }
            let source = lines[run.start].0..end_of(run.end - 1);
            blocks.push(out.block(Kind::Text, source, *apart));
            *apart = false;
        }
        *apart |= after;
    };

    let mut i = 0;
    while i < lines.len() {
        let (at, line) = lines[i];
        if let Some(n) = fence(line) {
            flush(&mut plain, &mut blocks, &mut apart, true);
            // A fence that is never closed has the rest of the text.
            let close = (i + 1..lines.len()).find(|&j| closes(lines[j].1, n));
            let last = close.unwrap_or(lines.len());
            let mut out = Out::default();
            if last > i + 1 {
                let code = lines[i + 1].0..end_of(last - 1);
                out.push(&text[code.clone()], code.start, Marks::default());
            }
            let end = end_of(close.unwrap_or(lines.len() - 1));
            blocks.push(out.block(Kind::Code, at..end, apart));
            apart = false;
            i = last + 1;
            continue;
        }
        if let Some((level, words)) = heading(line) {
            flush(&mut plain, &mut blocks, &mut apart, true);
            let mut out = Out::default();
            inline(
                &mut out,
                line[words..].trim_end(),
                at + words,
                Marks::default(),
            );
            blocks.push(out.block(Kind::Heading(level), at..at + line.len(), apart));
            apart = false;
            i += 1;
            continue;
        }
        if let Some((indent, marker, words)) = item(line) {
            flush(&mut plain, &mut blocks, &mut apart, true);
            let under = matches!(blocks.last(), Some(b) if matches!(b.kind, Kind::Item { .. }));
            let marker = match &line[marker] {
                "-" | "*" => "•".to_string(),
                number => number.to_string(),
            };
            let mut out = Out::default();
            inline(&mut out, &line[words..], at + words, Marks::default());
            // An indented line under an item goes on with it.
            let mut last = i;
            while let Some(&(next_at, next)) = lines.get(last + 1) {
                let goes_on = next.starts_with(' ')
                    && !next.trim().is_empty()
                    && fence(next).is_none()
                    && item(next).is_none();
                if !goes_on {
                    break;
                }
                out.push("\n", next_at - 1, Marks::default());
                let words = next.len() - next.trim_start_matches(' ').len();
                inline(&mut out, &next[words..], next_at + words, Marks::default());
                last += 1;
            }
            let kind = Kind::Item {
                marker,
                depth: (indent >= 2 && under) as u8,
            };
            blocks.push(out.block(kind, at..end_of(last), apart));
            apart = false;
            i = last + 1;
            continue;
        }
        plain = Some(plain.map_or(i..i + 1, |run| run.start..i + 1));
        i += 1;
    }
    flush(&mut plain, &mut blocks, &mut apart, false);
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOLD: Marks = Marks {
        bold: true,
        italic: false,
        code: false,
    };
    const ITALIC: Marks = Marks {
        bold: false,
        italic: true,
        code: false,
    };
    const CODE: Marks = Marks {
        bold: false,
        italic: false,
        code: true,
    };

    /// The blocks of `text` as their kind and what is drawn.
    fn drawn(text: &str) -> Vec<(Kind, String)> {
        blocks(text).into_iter().map(|b| (b.kind, b.text)).collect()
    }

    fn text(words: &str) -> (Kind, String) {
        (Kind::Text, words.to_string())
    }

    fn bullet(depth: u8, words: &str) -> (Kind, String) {
        let marker = "•".to_string();
        (Kind::Item { marker, depth }, words.to_string())
    }

    /// The one block of `text`: what is drawn, and the stretches that are
    /// not plain.
    fn marked(text: &str) -> (String, Vec<(String, Marks)>) {
        let mut all = blocks(text);
        assert_eq!(all.len(), 1, "{text:?}");
        let block = all.remove(0);
        let marks = block
            .marks
            .iter()
            .map(|(range, marks)| (block.text[range.clone()].to_string(), *marks))
            .collect();
        (block.text, marks)
    }

    /// `text` is one block of text, drawn as it was written.
    fn untouched(text: &str) {
        let all = blocks(text);
        assert_eq!(all.len(), 1, "{text:?}");
        assert_eq!((&all[0].kind, all[0].text.as_str()), (&Kind::Text, text));
        assert!(all[0].marks.is_empty(), "{text:?}: {:?}", all[0].marks);
        assert_eq!(all[0].source, 0..text.len());
        assert!(!all[0].apart);
    }

    #[test]
    fn a_text_without_a_mark_is_one_block_as_it_was_written() {
        untouched("");
        untouched("Land it today, José.");
        untouched("one\ntwo\n\nthree\n");
        untouched("\n\n  indented, after two blank lines");
        untouched("see https://example.org/a_b_c/**/x_y now");
    }

    #[test]
    fn what_only_looks_like_a_mark_is_left_alone() {
        // A # in the middle of a line, and one without a space after it.
        untouched("issue #12 and a # alone");
        untouched("#12 is the issue");
        untouched("#");
        // Deeper than the third level is no heading here.
        untouched("#### four");
        // Paths and names with underscores.
        untouched("src/my_file_name.rs and snake_case_name");
        untouched("__init__.py and _private.rs");
        // Stars in globs and in sums.
        untouched("*.rs and src/*.rs, **/*.md");
        untouched("*.*");
        untouched("2 * 3 * 4 and a*b*c");
        // A mark that is never closed.
        untouched("a *star, a **pair, a `tick and an _underscore");
        untouched("``` is three backticks");
        // No list without a space after the marker, or with nothing in it.
        untouched("-dash and 1.5 and 3.14");
        untouched("- ");
        untouched("---");
        untouched("2026. That was the year.");
        // Not on the list: a table, an image, HTML, a footnote, a quote.
        untouched("| a | b |\n|---|---|\n| 1 | 2 |");
        untouched("![fence](fence.png) and <b>bold</b>[^1]");
        untouched("> quoted");
        untouched("a \\*star\\*");
    }

    #[test]
    fn a_fence_is_code_as_it_was_written() {
        let src = "Run:\n```sh\n  cd yard\n1. not an item\n# not a heading\n- *nor* this\n```\nThen wait.";
        assert_eq!(
            drawn(src),
            [
                text("Run:"),
                (
                    Kind::Code,
                    "  cd yard\n1. not an item\n# not a heading\n- *nor* this".to_string()
                ),
                text("Then wait."),
            ]
        );
        let code = &blocks(src)[1];
        assert!(code.marks.is_empty());
        // The block stands for its fences too.
        assert_eq!(
            &src[code.source.clone()],
            &src[5..src.len() - "\nThen wait.".len()]
        );

        // Without a language word, and with more backticks than three.
        assert_eq!(drawn("```\nls\n```"), [(Kind::Code, "ls".to_string())]);
        assert_eq!(
            drawn("````\n```\nls\n```\n````"),
            [(Kind::Code, "```\nls\n```".to_string())]
        );
        // Blank lines in it are its own.
        assert_eq!(
            drawn("```\n\na\n\nb\n\n```"),
            [(Kind::Code, "\na\n\nb\n".to_string())]
        );
        // An empty one is still a block.
        assert_eq!(drawn("```\n```"), [(Kind::Code, String::new())]);
    }

    #[test]
    fn a_copy_of_a_code_block_is_the_source_between_the_fences() {
        let code = format!(
            "  indented\n\ttabbed\n\n{}",
            "echo a very long line that wraps in the window ".repeat(20)
        );
        let src = format!("Run:\n```sh\n{code}\n```\nThen wait.");
        let all = blocks(&src);
        assert_eq!(all.len(), 3);
        assert_eq!(all[1].copy_text(), Some(code.as_str()));
        // The fences and the language word are not in it, and no other
        // block has a copy control.
        assert_eq!(all[0].copy_text(), None);
        assert_eq!(all[2].copy_text(), None);
        assert!(!all[1].copy_text().unwrap().contains("```"));
    }

    #[test]
    fn an_unclosed_fence_has_the_rest_as_code_and_hides_nothing() {
        let src = "Look:\n```\nfirst\n\n- second\n**third**";
        assert_eq!(
            drawn(src),
            [
                text("Look:"),
                (Kind::Code, "first\n\n- second\n**third**".to_string()),
            ]
        );
        assert_eq!(blocks(src)[1].source, 6..src.len());
        assert_eq!(drawn("```"), [(Kind::Code, String::new())]);
    }

    #[test]
    fn inline_code_bold_and_italic_lose_their_marks() {
        assert_eq!(
            marked("run `yardr prime` now"),
            (
                "run yardr prime now".to_string(),
                vec![("yardr prime".to_string(), CODE)]
            )
        );
        assert_eq!(
            marked("a **bold** and an *italic* and an _other_ and __more__."),
            (
                "a bold and an italic and an other and more.".to_string(),
                vec![
                    ("bold".to_string(), BOLD),
                    ("italic".to_string(), ITALIC),
                    ("other".to_string(), ITALIC),
                    ("more".to_string(), BOLD),
                ]
            )
        );
        let both = Marks {
            bold: true,
            italic: true,
            code: false,
        };
        assert_eq!(
            marked("***both*** and **bold with _italic_ in it**"),
            (
                "both and bold with italic in it".to_string(),
                vec![
                    ("both".to_string(), both),
                    ("bold with ".to_string(), BOLD),
                    ("italic".to_string(), both),
                    (" in it".to_string(), BOLD),
                ]
            )
        );
        // What is in code is code: no mark in it is read.
        assert_eq!(
            marked("`**/*.rs` and ``a ` b``"),
            (
                "**/*.rs and a ` b".to_string(),
                vec![("**/*.rs".to_string(), CODE), ("a ` b".to_string(), CODE)]
            )
        );
        let bold_code = Marks {
            bold: true,
            italic: false,
            code: true,
        };
        assert_eq!(
            marked("**`cargo test`**"),
            (
                "cargo test".to_string(),
                vec![("cargo test".to_string(), bold_code)]
            )
        );
        // A link keeps its own stars and underscores, and what closes after
        // it closes.
        assert_eq!(
            marked("**https://example.org/a_b_** (_see https://example.org/*x*_)"),
            (
                "https://example.org/a_b_ (see https://example.org/*x*)".to_string(),
                vec![
                    ("https://example.org/a_b_".to_string(), BOLD),
                    ("see https://example.org/*x*".to_string(), ITALIC),
                ]
            )
        );
        // Emphasis does not go over a line break.
        untouched("*one\ntwo*");
    }

    #[test]
    fn bullets_and_numbers_are_items_one_level_deep() {
        let src = "To do:\n- one\n* two\n  - under two\n      - and deeper\n1. first\n10) tenth";
        let number = |marker: &str, words: &str| {
            let marker = marker.to_string();
            (Kind::Item { marker, depth: 0 }, words.to_string())
        };
        assert_eq!(
            drawn(src),
            [
                text("To do:"),
                bullet(0, "one"),
                bullet(0, "two"),
                bullet(1, "under two"),
                bullet(1, "and deeper"),
                number("1.", "first"),
                number("10)", "tenth"),
            ]
        );
        // An indented line goes on with its item, without its indent; one
        // that is not indented is text again.
        assert_eq!(
            drawn("- one\n  goes on\nand ends\n\n- two"),
            [
                bullet(0, "one\ngoes on"),
                text("and ends"),
                bullet(0, "two")
            ]
        );
        // An item's words are read as a line's are.
        let item = &blocks("- **Done**: see `x`")[0];
        assert_eq!(item.text, "Done: see x");
        assert_eq!(item.marks, [(0..4, BOLD), (10..11, CODE)]);
        // An indented item under no other is at the first level.
        assert_eq!(drawn("  - alone"), [bullet(0, "alone")]);
    }

    #[test]
    fn headings_go_to_the_third_level() {
        assert_eq!(
            drawn("# One\n## Two\n### Three ###\ntext"),
            [
                (Kind::Heading(1), "One".to_string()),
                (Kind::Heading(2), "Two".to_string()),
                (Kind::Heading(3), "Three ###".to_string()),
                text("text"),
            ]
        );
        let heading = &blocks("## The `new` stage")[0];
        assert_eq!(heading.text, "The new stage");
        assert_eq!(heading.marks, [(4..7, CODE)]);
    }

    #[test]
    fn blank_lines_between_blocks_are_space_and_no_lines() {
        let all = blocks("# Head\n\nwords\n\n\n- item\n- next\n\n```\ncode\n```\n\n");
        let apart: Vec<(&str, bool)> = all.iter().map(|b| (b.text.as_str(), b.apart)).collect();
        assert_eq!(
            apart,
            [
                ("Head", false),
                ("words", true),
                ("item", true),
                ("next", false),
                ("code", true),
            ]
        );
        // Inside text they stay lines.
        assert_eq!(
            drawn("one\n\ntwo\n# Head"),
            [text("one\n\ntwo"), (Kind::Heading(1), "Head".to_string())]
        );
    }

    #[test]
    fn a_place_in_a_block_is_a_place_in_the_source() {
        let src = "a **bold** b";
        let block = &blocks(src)[0];
        assert_eq!(block.text, "a bold b");
        assert_eq!(block.source_at(0), 0..0);
        assert_eq!(block.source_at(1), 1..1);
        // Before the first letter of the bold word the stars stand, and
        // after its last.
        assert_eq!(block.source_at(2), 2..4);
        assert_eq!(block.source_at(4), 6..6);
        assert_eq!(block.source_at(6), 8..10);
        assert_eq!(block.source_at(8), 12..12);
        // What is selected of the source, as it is drawn.
        assert_eq!(block.shown(0..src.len()), 0..8);
        assert_eq!(block.shown(4..8), 2..6);
        assert_eq!(block.shown(2..5), 2..3);
        assert_eq!(block.shown(2..4), 0..0);
        assert_eq!(block.shown(11..40), 7..8);

        // The start and the end of a block are where its marks begin and
        // end.
        let src = "one\n```sh\nls -la\n```\n## Two";
        let all = blocks(src);
        let (code, heading) = (&all[1], &all[2]);
        assert_eq!(code.source_at(0), 4..10);
        assert_eq!(code.source_at(3), 13..13);
        assert_eq!(code.source_at(6), 16..20);
        assert_eq!(heading.source_at(0), 21..24);
        assert_eq!(heading.source_at(3), 27..27);
        assert_eq!(code.shown(0..src.len()), 0..6);
        assert_eq!(all[0].shown(4..src.len()), 0..0);

        // An item that goes on: the indent of its second line is not drawn.
        let src = "- one\n  two";
        let item = &blocks(src)[0];
        assert_eq!(item.text, "one\ntwo");
        assert_eq!(item.source_at(0), 0..2);
        assert_eq!(item.source_at(3), 5..5);
        assert_eq!(item.source_at(4), 6..8);
        assert_eq!(item.source_at(7), 11..11);
    }
}
