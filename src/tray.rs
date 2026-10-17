//! footplate's item in the Mac's menu bar: the track glyph, always
//! there, with the number of things that wait beside it while any do, the
//! number the dock's icon says (dock.rs). A click opens a menu of the yards
//! that are followed, each with what waits in it, and under them "Show
//! window" and "Quit footplate". It is the way back into a window that
//! closing hid, so it is there whatever the count. gpui has no call for a
//! status item, so it is Cocoa's own; it needs no bundle, and is there from
//! a bare binary too.
//!
//! On Linux it is an item in the desktop's tray, where a StatusNotifier
//! host draws one (a bar's tray, a panel's), with the same menu. A tray
//! draws the icon and no title, so the number is drawn on the mark, or a
//! dot where the icon is too small for one, and the tooltip says it in
//! words. A click on the item shows the window. It is spoken over D-Bus by
//! ksni; without a host, or without a session bus, there is none and
//! nothing else changes.
//!
//! What the item says is in plain functions, tested on any system; the
//! items are each for their own system.

/// The tray's mark: the track glyph, `tray-icon.svg` from yardr-design's
/// assets rendered at 128 pixels a side, single black on transparent. On
/// macOS it is a template image, drawn in the menu bar's foreground; on
/// Linux it is scaled to the sizes a tray draws.
pub const ICON: &[u8] = include_bytes!("../assets/tray-icon.png");

/// One row of the menu: the yard it opens, and what it says.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub yard: String,
    pub says: String,
}

/// What the item says: the title beside its mark, how many things wait and
/// that in words, and a row for each yard.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Said {
    pub title: String,
    pub waiting: usize,
    pub tip: String,
    pub rows: Vec<Row>,
}

/// What the item's icon shows of what waits, where the icon is all a tray
/// draws of the item.
#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Debug, PartialEq)]
pub enum Sign {
    /// The mark alone: nothing waits.
    Nothing,
    /// The number that waits, drawn on the mark.
    Number(String),
    /// A dot on the mark: something waits, and its number would not be
    /// read at this size.
    Dot,
}

/// What a person chose in the menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Chosen {
    /// The row of the yard of this name.
    Yard(String),
    /// "Show window".
    Show,
    /// "Quit footplate".
    Quit,
}

/// The title of the item while `waiting` things wait: their number, as the
/// dock's icon says it, and nothing where nothing waits. The item's mark is
/// there either way.
pub fn title(waiting: usize) -> String {
    crate::dock::label(waiting).unwrap_or_default()
}

/// What the icon of `size` pixels a side shows while `waiting` things wait:
/// their number where it is read, and a dot where it would not be. It is
/// read where a point of its digits is two pixels or more, and a sixteenth
/// of the icon or more: a tray that draws 20 pixels may take the largest
/// icon and scale it. A tray is given the icon in several sizes, and each
/// has its own sign.
#[cfg(any(target_os = "linux", test))]
pub fn sign(waiting: usize, size: u32) -> Sign {
    match crate::dock::label(waiting) {
        None => Sign::Nothing,
        Some(number) => match unit(number.len(), size) {
            unit if unit >= 2 && unit * 16 >= size => Sign::Number(number),
            _ => Sign::Dot,
        },
    }
}

/// How many pixels a point of a digit is, for a number of `digits` on an
/// icon of `size` pixels a side. A digit is 3 points wide and 5 high, with
/// a point between two and around them all; the number takes up to two
/// thirds of the icon's height and all of its width.
#[cfg(any(target_os = "linux", test))]
fn unit(digits: usize, size: u32) -> u32 {
    let digits = u32::try_from(digits).unwrap_or(u32::MAX);
    (size * 2 / 3 / 7).min(size / digits.saturating_mul(4).saturating_add(1))
}

/// The item's tooltip, in words: how many things wait, and `what` they are
/// as the switcher says it of a yard ("2 mail, 1 decision").
pub fn tip(waiting: usize, what: &str) -> String {
    match waiting {
        0 => "nothing waiting".to_string(),
        1 => format!("1 wait: {what}"),
        n => format!("{n} wait: {what}"),
    }
}

/// The ten digits, each 3 points wide and 5 high: a row a number, its
/// left point the highest bit of three.
#[cfg(any(target_os = "linux", test))]
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// What a sign is drawn in, red, green, blue and alpha: the red of a
/// badge, and the white of the number on it.
#[cfg(any(target_os = "linux", test))]
const BADGE: [u8; 4] = [0xe5, 0x39, 0x35, 0xff];
#[cfg(any(target_os = "linux", test))]
const NUMBER: [u8; 4] = [0xff, 0xff, 0xff, 0xff];

/// Draws `sign` on the mark in `rgba`, `size` pixels a side with four
/// bytes a pixel, in its lower right corner: the number in white on a red
/// badge with its corners cut, or a red dot two fifths of the icon across.
#[cfg(any(target_os = "linux", test))]
pub fn draw(sign: &Sign, size: u32, rgba: &mut [u8]) {
    let mut fill = |left: u32, top: u32, wide: u32, high: u32, colour: [u8; 4]| {
        for y in top..(top + high).min(size) {
            for x in left..(left + wide).min(size) {
                let at = ((y * size + x) * 4) as usize;
                if let Some(pixel) = rgba.get_mut(at..at + 4) {
                    pixel.copy_from_slice(&colour);
                }
            }
        }
    };
    match sign {
        Sign::Nothing => {}
        Sign::Dot => {
            let across = size * 2 / 5;
            let from = size - across;
            for y in 0..across {
                for x in 0..across {
                    // Twice the way from the middle, which is between two
                    // pixels where the dot is an even number across.
                    let (dx, dy) = ((2 * x + 1).abs_diff(across), (2 * y + 1).abs_diff(across));
                    if dx * dx + dy * dy <= across * across {
                        fill(from + x, from + y, 1, 1, BADGE);
                    }
                }
            }
        }
        Sign::Number(number) => {
            let unit = unit(number.len(), size);
            let places = number.len() as u32;
            let (wide, high) = ((4 * places + 1) * unit, 7 * unit);
            let (left, top) = (size.saturating_sub(wide), size.saturating_sub(high));
            // How far a pixel is into the point at an end of its row or
            // column: where both are, more than a point together, the
            // corner is cut.
            let into = |at: u32, of: u32| unit.saturating_sub(at.min(of - 1 - at));
            for y in 0..high {
                for x in 0..wide {
                    if into(x, wide) + into(y, high) <= unit {
                        fill(left + x, top + y, 1, 1, BADGE);
                    }
                }
            }
            let digits = number.chars().filter_map(|digit| digit.to_digit(10));
            for (place, digit) in (0u32..).zip(digits) {
                for (row, points) in (0u32..).zip(DIGITS[digit as usize]) {
                    for column in (0..3).filter(|column| points & (0b100 >> column) != 0) {
                        fill(
                            left + unit * (1 + 4 * place + column),
                            top + unit * (1 + row),
                            unit,
                            unit,
                            NUMBER,
                        );
                    }
                }
            }
        }
    }
}

/// A row's words as a menu over D-Bus takes them: an underscore there
/// marks the letter after it as the row's key and is not shown, and two
/// are one that is.
#[cfg(any(target_os = "linux", test))]
pub fn label(says: &str) -> String {
    says.replace('_', "__")
}

/// What a yard's row says: its name, and what the switcher says of it
/// (`state`: what waits in it, or that it is not reached and why).
pub fn row(yard: &str, state: &str) -> String {
    format!("{yard} · {state}")
}

/// What the item says while `waiting` things wait, which are `what` in
/// words, over `yards`, each a yard's name and its state, in the order of
/// the settings.
pub fn said<'a>(
    waiting: usize,
    what: &str,
    yards: impl IntoIterator<Item = (&'a str, String)>,
) -> Said {
    Said {
        title: title(waiting),
        waiting,
        tip: tip(waiting, what),
        rows: yards
            .into_iter()
            .map(|(yard, state)| Row {
                yard: yard.to_string(),
                says: row(yard, &state),
            })
            .collect(),
    }
}

#[cfg(all(target_os = "linux", not(test)))]
use linux::Item;
#[cfg(all(target_os = "macos", not(test)))]
use mac::Item;

/// The item as the window has it: what it was last told to say, and what a
/// person chooses in its menu. It is in the menu bar, or the tray, while it
/// has something to say, and out of it while it has not.
pub struct Tray {
    chosen: (
        async_channel::Sender<Chosen>,
        async_channel::Receiver<Chosen>,
    ),
    said: Option<Said>,
    /// A test shows none: it runs where a person works, and the menu bar
    /// is theirs.
    #[cfg(not(test))]
    item: Option<Item>,
}

impl Default for Tray {
    fn default() -> Self {
        Tray {
            chosen: async_channel::unbounded(),
            said: None,
            #[cfg(not(test))]
            item: None,
        }
    }
}

impl Tray {
    /// What a person chooses in the menu, as they do.
    pub fn chosen(&self) -> async_channel::Receiver<Chosen> {
        self.chosen.1.clone()
    }

    /// Has the item say `said`, where that is something else than it says
    /// now, and takes it out of the menu bar for none.
    pub fn say(&mut self, said: Option<Said>) {
        if said == self.said {
            return;
        }
        #[cfg(not(test))]
        match &said {
            Some(said) => {
                if self.item.is_none() {
                    self.item = Item::new(self.chosen.0.clone());
                }
                if let Some(item) = &self.item {
                    item.say(said);
                }
            }
            None => self.item = None,
        }
        self.said = said;
    }

    /// What the item was last told to say.
    #[cfg(test)]
    pub fn told(&self) -> Option<&Said> {
        self.said.as_ref()
    }

    /// A person chooses `chosen` in the menu.
    #[cfg(test)]
    pub fn choose(&self, chosen: Chosen) {
        self.chosen.0.try_send(chosen).unwrap();
    }
}

#[cfg(all(target_os = "macos", not(test)))]
mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObject, NSObjectProtocol, Sel};
    use objc2::{
        AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
    };
    use objc2_app_kit::{
        NSCellImagePosition, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem,
        NSVariableStatusItemLength,
    };
    use objc2_foundation::{NSData, NSSize, NSString};

    use super::{Chosen, Said};

    /// The mark's side in the menu bar, in points: what the bar's own
    /// items have.
    const MARK: f64 = 18.;

    define_class!(
        // A menu's row tells this object that it was chosen.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "FootplateTray"]
        #[ivars = async_channel::Sender<Chosen>]
        struct Target;

        unsafe impl NSObjectProtocol for Target {}

        impl Target {
            #[unsafe(method(yard:))]
            fn yard(&self, row: &NSMenuItem) {
                let yard = row
                    .representedObject()
                    .and_then(|yard| yard.downcast::<NSString>().ok());
                if let Some(yard) = yard {
                    let _ = self.ivars().try_send(Chosen::Yard(yard.to_string()));
                }
            }

            #[unsafe(method(show:))]
            fn show(&self, _row: &NSMenuItem) {
                let _ = self.ivars().try_send(Chosen::Show);
            }

            #[unsafe(method(quit:))]
            fn quit(&self, _row: &NSMenuItem) {
                let _ = self.ivars().try_send(Chosen::Quit);
            }
        }
    );

    impl Target {
        fn new(
            chosen: async_channel::Sender<Chosen>,
            main_thread: MainThreadMarker,
        ) -> Retained<Self> {
            let this = Self::alloc(main_thread).set_ivars(chosen);
            // SAFETY: NSObject's init, on an object just allocated with
            // its ivars set.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// The item in the menu bar, for as long as this is held.
    pub struct Item {
        item: Retained<NSStatusItem>,
        menu: Retained<NSMenu>,
        /// A menu's row keeps no reference of its own to its target.
        target: Retained<Target>,
        main_thread: MainThreadMarker,
    }

    impl Item {
        /// Puts the item in the menu bar, with the track glyph and
        /// no title yet. What is chosen in its menu goes to `chosen`.
        pub fn new(chosen: async_channel::Sender<Chosen>) -> Option<Item> {
            let main_thread = MainThreadMarker::new()?;
            let item =
                NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
            if let Some(button) = item.button(main_thread) {
                // The track glyph, a template: the menu bar draws it in
                // its own foreground, light or dark.
                let data = NSData::with_bytes(super::ICON);
                if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
                    image.setTemplate(true);
                    image.setSize(NSSize::new(MARK, MARK));
                    button.setImage(Some(&image));
                }
                button.setImagePosition(NSCellImagePosition::ImageLeading);
            }
            let menu = NSMenu::new(main_thread);
            item.setMenu(Some(&menu));
            Some(Item {
                item,
                menu,
                target: Target::new(chosen, main_thread),
                main_thread,
            })
        }

        /// Has the item say `said`: the title beside the mark, and the
        /// menu's rows anew.
        pub fn say(&self, said: &Said) {
            if let Some(button) = self.item.button(self.main_thread) {
                button.setTitle(&NSString::from_str(&said.title));
            }
            self.menu.removeAllItems();
            for row in &said.rows {
                let item = self.row(&row.says, sel!(yard:));
                // SAFETY: a string, which is what Target::yard takes it
                // for.
                unsafe { item.setRepresentedObject(Some(&NSString::from_str(&row.yard))) };
                self.menu.addItem(&item);
            }
            if !said.rows.is_empty() {
                self.menu
                    .addItem(&NSMenuItem::separatorItem(self.main_thread));
            }
            self.menu.addItem(&self.row("Show window", sel!(show:)));
            self.menu.addItem(&self.row("Quit footplate", sel!(quit:)));
        }

        /// A row of the menu that says `says` and tells the target
        /// `chosen` when it is chosen.
        fn row(&self, says: &str, chosen: Sel) -> Retained<NSMenuItem> {
            // SAFETY: the selector is one of Target's, which takes the
            // row; the target is an object that answers it, and is held
            // for as long as the menu is.
            unsafe {
                let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(self.main_thread),
                    &NSString::from_str(says),
                    Some(chosen),
                    &NSString::new(),
                );
                item.setTarget(Some(&self.target));
                item
            }
        }
    }

    impl Drop for Item {
        fn drop(&mut self) {
            NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
        }
    }
}

#[cfg(all(target_os = "linux", not(test)))]
mod linux {
    use std::sync::atomic::{AtomicBool, Ordering};

    use image::imageops::{self, FilterType};
    use ksni::blocking::{Handle, TrayMethods};
    use ksni::menu::StandardItem;

    use super::{Chosen, Said};

    /// The sizes the mark is given in, for a host to take the one it
    /// draws: what a bar's tray and a panel's have, at one and two device
    /// pixels a point.
    const SIZES: [u32; 6] = [16, 22, 24, 32, 48, 64];

    /// Whether the item could not be had once: it is not tried again,
    /// and said once.
    static NONE: AtomicBool = AtomicBool::new(false);

    /// What a host asks the item for, and tells it of.
    struct Shown {
        said: Said,
        chosen: async_channel::Sender<Chosen>,
        /// The mark in each of [`SIZES`], four bytes a pixel, red first.
        marks: Vec<(u32, Vec<u8>)>,
    }

    impl Shown {
        /// A row of the menu that says `says` and hands on `chosen` when
        /// it is chosen.
        fn row(says: &str, chosen: Chosen) -> ksni::MenuItem<Self> {
            StandardItem {
                label: super::label(says),
                activate: Box::new(move |this: &mut Self| {
                    let _ = this.chosen.try_send(chosen.clone());
                }),
                ..Default::default()
            }
            .into()
        }
    }

    impl ksni::Tray for Shown {
        fn id(&self) -> String {
            crate::NAME.into()
        }

        fn title(&self) -> String {
            crate::NAME.into()
        }

        fn category(&self) -> ksni::Category {
            ksni::Category::Communications
        }

        /// A click on the item itself is "Show window".
        fn activate(&mut self, _x: i32, _y: i32) {
            let _ = self.chosen.try_send(Chosen::Show);
        }

        /// The mark in each size, with what that size shows of what
        /// waits. No icon is named: a host would take a named one from
        /// the desktop's theme, which has the mark and no number.
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.marks
                .iter()
                .map(|(size, mark)| {
                    let mut data = mark.clone();
                    super::draw(&super::sign(self.said.waiting, *size), *size, &mut data);
                    // A host takes alpha first.
                    for pixel in data.as_chunks_mut::<4>().0 {
                        pixel.rotate_right(1);
                    }
                    ksni::Icon {
                        width: *size as i32,
                        height: *size as i32,
                        data,
                    }
                })
                .collect()
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: crate::NAME.into(),
                description: self.said.tip.clone(),
                ..Default::default()
            }
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            let rows = self.said.rows.iter();
            let mut menu: Vec<ksni::MenuItem<Self>> = rows
                .map(|row| Self::row(&row.says, Chosen::Yard(row.yard.clone())))
                .collect();
            if !menu.is_empty() {
                menu.push(ksni::MenuItem::Separator);
            }
            menu.push(Self::row("Show window", Chosen::Show));
            menu.push(Self::row("Quit footplate", Chosen::Quit));
            menu
        }
    }

    /// The item in the tray, for as long as this is held.
    pub struct Item {
        handle: Handle<Shown>,
    }

    impl Item {
        /// Offers the item to the desktop's tray, with the track glyph
        /// and nothing said yet. What is chosen in its menu goes to
        /// `chosen`. A desktop without a tray shows none, until it has one:
        /// a bar that starts later, or again, gets the item then. Without
        /// a session bus there is none to offer it on, and no item.
        pub fn new(chosen: async_channel::Sender<Chosen>) -> Option<Item> {
            if NONE.load(Ordering::Relaxed) {
                return None;
            }
            let shown = || {
                let mark =
                    image::load_from_memory_with_format(super::ICON, image::ImageFormat::Png)
                        .map_err(|err| err.to_string())?
                        .to_rgba8();
                let marks = SIZES
                    .into_iter()
                    .map(|size| {
                        let scaled = imageops::resize(&mark, size, size, FilterType::Lanczos3);
                        (size, scaled.into_raw())
                    })
                    .collect();
                let shown = Shown {
                    said: Said::default(),
                    chosen,
                    marks,
                };
                shown
                    .assume_sni_available(true)
                    .spawn()
                    .map_err(|err| err.to_string())
            };
            match shown() {
                Ok(handle) => Some(Item { handle }),
                Err(err) => {
                    NONE.store(true, Ordering::Relaxed);
                    eprintln!("footplate: no item in the tray: {err}");
                    None
                }
            }
        }

        /// Has the item say `said`: the sign on the mark, the tooltip and
        /// the menu's rows anew. The host is told of what changed.
        pub fn say(&self, said: &Said) {
            let said = said.clone();
            self.handle.update(move |shown| shown.said = said);
        }
    }

    impl Drop for Item {
        fn drop(&mut self) {
            // Gone from the tray before this returns: an item that is
            // switched on again is another.
            self.handle.shutdown().wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(said: &Said) -> Vec<(&str, &str)> {
        said.rows
            .iter()
            .map(|row| (row.yard.as_str(), row.says.as_str()))
            .collect()
    }

    #[test]
    fn the_tray_icon_is_the_rendered_track_glyph() {
        // The PNG's header: its width and height after the IHDR chunk's name.
        let number = |at: usize| u32::from_be_bytes(ICON[at..at + 4].try_into().unwrap());
        assert_eq!(&ICON[12..16], b"IHDR");
        assert_eq!((number(16), number(20)), (128, 128));
    }

    #[test]
    fn the_title_is_the_number_that_waits_and_empty_where_nothing_does() {
        assert_eq!(title(0), "");
        assert_eq!(title(1), "1");
        assert_eq!(title(128), "128");
        // It is what the dock's icon says, whatever that is.
        for waiting in [0, 3, 42] {
            assert_eq!(
                title(waiting),
                crate::dock::label(waiting).unwrap_or_default()
            );
        }
    }

    #[test]
    fn the_menu_has_a_row_for_each_yard_with_what_waits_in_it() {
        // No yard: no row, and no title. The mark, "Show window" and
        // "Quit footplate" are the item's own, and there all the same.
        assert_eq!(
            said(0, "nothing waiting", []),
            Said {
                tip: "nothing waiting".to_string(),
                ..Said::default()
            }
        );

        // One yard.
        let one = said(1, "1 mail", [("mac", "1 mail".to_string())]);
        assert_eq!(one.title, "1");
        assert_eq!((one.waiting, one.tip.as_str()), (1, "1 wait: 1 mail"));
        assert_eq!(rows(&one), [("mac", "mac · 1 mail")]);

        // Several, in the order they are given in.
        let several = said(
            3,
            "2 mail, 1 decision",
            [
                ("mac", "1 mail".to_string()),
                ("plantime", "1 mail, 1 decision".to_string()),
            ],
        );
        assert_eq!(several.title, "3");
        assert_eq!(
            (several.waiting, several.tip.as_str()),
            (3, "3 wait: 2 mail, 1 decision")
        );
        assert_eq!(
            rows(&several),
            [
                ("mac", "mac · 1 mail"),
                ("plantime", "plantime · 1 mail, 1 decision")
            ]
        );

        // One that is not reached says so, with its why.
        let unreached = said(
            2,
            "2 mail",
            [
                ("plantime", "2 mail".to_string()),
                ("shop", "not reached: nothing answers".to_string()),
            ],
        );
        assert_eq!(
            rows(&unreached),
            [
                ("plantime", "plantime · 2 mail"),
                ("shop", "shop · not reached: nothing answers")
            ]
        );

        // Nothing waits: the rows say so, and the title is empty.
        let nothing = said(
            0,
            "nothing waiting",
            [("mac", "nothing waiting".to_string())],
        );
        assert_eq!(nothing.title, "");
        assert_eq!(rows(&nothing), [("mac", "mac · nothing waiting")]);
    }

    #[test]
    fn the_icon_shows_nothing_a_number_or_a_dot_by_what_waits_and_its_size() {
        let number = |n: &str| Sign::Number(n.to_string());
        // Nothing waits: the mark alone, at every size.
        for size in [16, 22, 24, 32, 48, 64] {
            assert_eq!(sign(0, size), Sign::Nothing);
        }
        // One digit and two are read from 22 pixels up, and are a dot on
        // the 16 of a small tray.
        for waiting in [1, 9, 10, 99] {
            assert_eq!(sign(waiting, 16), Sign::Dot);
            for size in [22, 24, 32, 48, 64] {
                assert_eq!(sign(waiting, size), number(&waiting.to_string()));
            }
        }
        // Three digits need 32 pixels, and more are a dot wherever a tray
        // asks: they would be too small a part of the icon.
        for size in [16, 22, 24] {
            assert_eq!(sign(128, size), Sign::Dot);
        }
        for size in [32, 48, 64] {
            assert_eq!(sign(128, size), number("128"));
        }
        for size in [16, 22, 24, 32, 48, 64] {
            assert_eq!(sign(1000, size), Sign::Dot);
            assert_eq!(sign(usize::MAX, size), Sign::Dot);
        }
        // No size at all asks for nothing that cannot be had.
        assert_eq!(sign(3, 0), Sign::Dot);
    }

    #[test]
    fn the_tooltip_says_in_words_how_many_wait_and_what_they_are() {
        assert_eq!(tip(0, "nothing waiting"), "nothing waiting");
        assert_eq!(tip(1, "1 decision"), "1 wait: 1 decision");
        assert_eq!(tip(3, "2 mail, 1 decision"), "3 wait: 2 mail, 1 decision");
    }

    /// An icon of `size` pixels a side, all of one colour that no sign is
    /// drawn in, with `sign` drawn on it: a letter a pixel, row by row,
    /// `.` what was there, `#` the badge and `o` the number.
    fn drawn(sign: &Sign, size: u32) -> Vec<String> {
        let mark = [1, 2, 3, 4];
        let mut rgba = mark.repeat((size * size) as usize);
        draw(sign, size, &mut rgba);
        let pixels: Vec<char> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&pixel| match pixel {
                pixel if pixel == mark => '.',
                BADGE => '#',
                NUMBER => 'o',
                other => panic!("a colour that is no sign's: {other:?}"),
            })
            .collect();
        pixels
            .chunks(size as usize)
            .map(|row| row.iter().collect())
            .collect()
    }

    #[test]
    fn the_sign_is_drawn_in_the_lower_right_corner_of_the_mark() {
        // Nothing: the mark as it was.
        assert!(
            drawn(&Sign::Nothing, 22)
                .iter()
                .all(|row| row == &".".repeat(22))
        );

        // A number: its digits two pixels a point at 22, white on the
        // badge, whose corners are cut. The rest is the mark's.
        assert_eq!(
            drawn(&sign(7, 22), 22)[8..],
            [
                "..............######..",
                ".............########.",
                "............##oooooo##",
                "............##oooooo##",
                "............######oo##",
                "............######oo##",
                "............######oo##",
                "............######oo##",
                "............######oo##",
                "............######oo##",
                "............######oo##",
                "............######oo##",
                ".............########.",
                "..............######..",
            ]
        );
        assert!(
            drawn(&sign(7, 22), 22)[..8]
                .iter()
                .all(|row| row == &".".repeat(22))
        );

        // Every digit is its own, and two stand side by side.
        let digits: Vec<Vec<String>> = (0..10)
            .map(|digit| drawn(&sign(digit + 10, 22), 22))
            .collect();
        for (at, one) in digits.iter().enumerate() {
            assert!(one.iter().any(|row| row.contains('o')));
            assert!(digits[at + 1..].iter().all(|other| other != one));
        }
        assert_eq!(drawn(&sign(42, 22), 22)[10], "....##oo##oo##oooooo##");

        // A dot: two fifths of the icon across, its corners off, and no
        // number.
        assert_eq!(
            drawn(&sign(7, 16), 16)[10..],
            [
                "...........####.",
                "..........######",
                "..........######",
                "..........######",
                "..........######",
                "...........####.",
            ]
        );

        // A sign is drawn within the icon whatever is asked: a number
        // too long for its size, one that is none, and no pixels at all.
        for (sign, size) in [
            (Sign::Number("123456".to_string()), 16),
            (Sign::Number("1x".to_string()), 22),
            (Sign::Dot, 1),
        ] {
            assert_eq!(drawn(&sign, size).len(), size as usize);
        }
        draw(&Sign::Dot, 22, &mut []);
        draw(&Sign::Number("7".to_string()), 22, &mut [0; 8]);
    }

    #[test]
    fn a_menu_row_keeps_its_underscores() {
        assert_eq!(label("mac · 1 mail"), "mac · 1 mail");
        assert_eq!(label("my_yard · 1 mail"), "my__yard · 1 mail");
    }

    #[test]
    fn the_item_is_told_only_what_is_new_and_hands_on_what_is_chosen() {
        let mut tray = Tray::default();
        assert_eq!(tray.told(), None);
        let one = said(1, "1 mail", [("mac", "1 mail".to_string())]);
        tray.say(Some(one.clone()));
        assert_eq!(tray.told(), Some(&one));
        tray.say(None);
        assert_eq!(tray.told(), None);

        let chosen = tray.chosen();
        tray.choose(Chosen::Yard("mac".to_string()));
        tray.choose(Chosen::Show);
        tray.choose(Chosen::Quit);
        assert_eq!(chosen.try_recv(), Ok(Chosen::Yard("mac".to_string())));
        assert_eq!(chosen.try_recv(), Ok(Chosen::Show));
        assert_eq!(chosen.try_recv(), Ok(Chosen::Quit));
        assert!(chosen.try_recv().is_err());
    }
}
