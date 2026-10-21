//! The window's keys: the contexts its bindings answer in, and the tables
//! the cheatsheet names for each view.

pub(crate) const CONTEXT: &str = "Footplate";

/// Where no line is typed in: the yard row, and a decision with the window
/// focused, where a number key picks and `x` puts aside. With a
/// conversation open, with the line focused to type words in, or with the
/// history's filter focused, those keys are the line's text.
pub(crate) const NO_LINE_CONTEXT: &str = "FootplateNoLine";

/// Where the account of a setup ask is typed in, or the number of a
/// weighty card with no gate: its approval's key approves from the field
/// too, with what the field holds. Only that key is bound here: a
/// decline's words must not approve, and no other card's field approves
/// anything.
pub(crate) const SETUP_SECRET_CONTEXT: &str = "FootplateSetupSecret";

/// Where the switcher's list is open: the arrows move in it, Enter opens
/// the yard they are on, esc leaves it, + adds a yard, and no key is a
/// card's.
pub(crate) const SWITCH_CONTEXT: &str = "FootplateSwitch";

/// The key that is cmd on macOS and ctrl elsewhere, as the hints name it.
pub(crate) const SECONDARY: &str = if cfg!(target_os = "macos") {
    "cmd"
} else {
    "ctrl"
};

/// The open view, as the cheatsheet names its keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeysView {
    Switcher,
    Settings,
    Conversation,
    Card,
    Grants,
    Yard,
    Timetable,
    Away,
}

impl KeysView {
    /// The view in the cheatsheet's title.
    pub(crate) fn name(self) -> &'static str {
        match self {
            KeysView::Switcher => "the switcher",
            KeysView::Settings => "the settings",
            KeysView::Conversation => "this conversation",
            KeysView::Card => "this card",
            KeysView::Grants => "Grants",
            KeysView::Yard => "the yard row",
            KeysView::Timetable => "Lines",
            KeysView::Away => "this screen",
        }
    }
}

/// Every key the open `view` answers to, each with what it does: the
/// view's own keys first, then the window's, which answer everywhere.
/// Named as the footers and the README name them. With the bead drawer
/// open, the views it stands beside name esc shutting it.
pub(crate) fn keys_for(view: KeysView, drawer: bool) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = match view {
        KeysView::Switcher => vec![
            ("↑↓".into(), "move in the list".into()),
            ("enter".into(), "opens the yard they are on".into()),
            ("esc".into(), "leaves the list".into()),
            ("+".into(), "adds a yard".into()),
            ("?".into(), "this list".into()),
        ],
        KeysView::Settings => vec![("esc".into(), "back to what was open".into())],
        KeysView::Conversation => vec![
            ("enter".into(), "sends the draft as one mail".into()),
            ("shift+enter".into(), "breaks the line".into()),
            (
                "tab · shift+tab".into(),
                "the next, the previous row".into(),
            ),
            ("pageup · pagedown".into(), "scroll by a page".into()),
            (
                format!("{SECONDARY}+home · {SECONDARY}+end"),
                "the oldest loaded, the newest".into(),
            ),
            (format!("{SECONDARY}+c"), "copies what is selected".into()),
        ],
        KeysView::Card => vec![
            ("1–9".into(), "choose an option".into()),
            ("↑↓".into(), "choose, even while words are typed".into()),
            (
                "enter".into(),
                "sends the pick with the line's words".into(),
            ),
            ("shift+enter".into(), "breaks the line".into()),
            ("i".into(), "types the words in".into()),
            ("x".into(), "puts the ask aside".into()),
            ("d".into(), "declines with words".into()),
            ("a".into(), "asks about this".into()),
            ("e".into(), "expands to the bead whole, or folds it".into()),
            (
                "/".into(),
                "in decisions, the history with its filter typed in".into(),
            ),
            (
                format!("{SECONDARY}+enter"),
                "on a run or setup ask's card, approves it, after unlocking where the card is weighty"
                    .into(),
            ),
            (
                "esc".into(),
                "leaves the decline, the words, the pick, then the words".into(),
            ),
            (
                "tab · shift+tab".into(),
                "the next, the previous row".into(),
            ),
            ("pageup · pagedown".into(), "scroll by a page".into()),
            (
                format!("{SECONDARY}+home · {SECONDARY}+end"),
                "the oldest loaded, the newest".into(),
            ),
            ("?".into(), "this list".into()),
        ],
        KeysView::Grants => vec![
            (
                "tab · shift+tab".into(),
                "the next, the previous row".into(),
            ),
            ("pageup · pagedown".into(), "scroll by a page".into()),
            ("?".into(), "this list".into()),
        ],
        KeysView::Yard => vec![
            ("x".into(), "marks one notice read".into()),
            ("shift+x".into(), "marks every notice read".into()),
            (
                "tab · shift+tab".into(),
                "the next, the previous row".into(),
            ),
            ("pageup · pagedown".into(), "scroll by a page".into()),
            (
                format!("{SECONDARY}+home · {SECONDARY}+end"),
                "the first notice, the newest".into(),
            ),
            ("?".into(), "this list".into()),
        ],
        KeysView::Timetable => vec![
            (
                "tab · shift+tab".into(),
                "the next, the previous row".into(),
            ),
            ("pageup · pagedown".into(), "scroll by a page".into()),
            ("?".into(), "this list".into()),
        ],
        KeysView::Away => vec![
            (
                "enter".into(),
                "opens the details, where no fields stand open".into(),
            ),
            ("esc".into(), "leaves the way's fields".into()),
        ],
    };
    // With the bead drawer open, esc shuts it: the views whose reading
    // pane it stands beside name that key. A card's esc row says it with
    // the decline, ahead of the card's own leavings; the switcher's esc
    // leaves the list, the settings shut the drawer as they open, and
    // the away screen stands in place of the reading pane it shuts from.
    if drawer {
        match view {
            KeysView::Conversation | KeysView::Grants | KeysView::Yard | KeysView::Timetable => {
                rows.insert(0, ("esc".into(), "shuts the bead drawer".into()));
            }
            KeysView::Card => {
                if let Some(esc) = rows.iter_mut().find(|(key, _)| key == "esc") {
                    esc.1 = "leaves the decline and shuts the bead drawer, then the words, the pick, then the words".into();
                }
            }
            KeysView::Switcher | KeysView::Settings | KeysView::Away => {}
        }
    }
    // The window's keys, which answer in every view.
    rows.extend([
        (
            format!("{SECONDARY}+[ · {SECONDARY}+]"),
            "the modes back, forth".into(),
        ),
        (format!("{SECONDARY}+y"), "lists the yards".into()),
        (
            format!("{SECONDARY}+1–9"),
            "opens the yard at that place".into(),
        ),
        (
            format!("{SECONDARY}+,"),
            "the settings, and what was open again".into(),
        ),
        (
            format!("{SECONDARY}+= · {SECONDARY}+- · {SECONDARY}+0"),
            "the text larger, smaller, the usual size".into(),
        ),
        (
            format!("{SECONDARY}+shift+l"),
            "the next of auto, light, dark".into(),
        ),
    ]);
    rows
}

/// The key that approves a run ask, as its button and its keys name it.
pub(crate) const APPROVE_KEY: &str = if cfg!(target_os = "macos") {
    "\u{2318}\u{21b5}"
} else {
    "ctrl+enter"
};
