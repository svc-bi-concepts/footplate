//! footplate's item in the Mac's menu bar: the application's mark, always
//! there, with the number of things that wait beside it while any do, the
//! number the dock's icon says (dock.rs). A click opens a menu of the yards
//! that are followed, each with what waits in it, and under them "Show
//! window" and "Quit footplate". It is the way back into a window that
//! closing hid, so it is there whatever the count. gpui has no call for a
//! status item, so it is Cocoa's own; it needs no bundle, and is there from
//! a bare binary too.
//!
//! What the item says is in plain functions, tested on any system; the
//! item is for macOS only. Linux has none.

/// One row of the menu: the yard it opens, and what it says.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub yard: String,
    pub says: String,
}

/// What the item says: the title beside its mark, and a row for each yard.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Said {
    pub title: String,
    pub rows: Vec<Row>,
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

/// What a yard's row says: its name, and what the switcher says of it
/// (`state`: what waits in it, or that it is not reached and why).
pub fn row(yard: &str, state: &str) -> String {
    format!("{yard} · {state}")
}

/// What the item says while `waiting` things wait over `yards`, each a
/// yard's name and its state, in the order of the settings.
pub fn said<'a>(waiting: usize, yards: impl IntoIterator<Item = (&'a str, String)>) -> Said {
    Said {
        title: title(waiting),
        rows: yards
            .into_iter()
            .map(|(yard, state)| Row {
                yard: yard.to_string(),
                says: row(yard, &state),
            })
            .collect(),
    }
}

/// The item as the window has it: what it was last told to say, and what a
/// person chooses in its menu. It is in the menu bar while it has something
/// to say, and out of it while it has not.
pub struct Tray {
    chosen: (
        async_channel::Sender<Chosen>,
        async_channel::Receiver<Chosen>,
    ),
    said: Option<Said>,
    /// A test shows none: it runs where a person works, and the menu bar
    /// is theirs.
    #[cfg(all(target_os = "macos", not(test)))]
    item: Option<mac::Item>,
}

impl Default for Tray {
    fn default() -> Self {
        Tray {
            chosen: async_channel::unbounded(),
            said: None,
            #[cfg(all(target_os = "macos", not(test)))]
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
        #[cfg(all(target_os = "macos", not(test)))]
        match &said {
            Some(said) => {
                if self.item.is_none() {
                    self.item = mac::Item::new(self.chosen.0.clone());
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
        /// Puts the item in the menu bar, with the application's mark and
        /// no title yet. What is chosen in its menu goes to `chosen`.
        pub fn new(chosen: async_channel::Sender<Chosen>) -> Option<Item> {
            let main_thread = MainThreadMarker::new()?;
            let item =
                NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
            if let Some(button) = item.button(main_thread) {
                // The mark as it is in the dock, small, and not a template:
                // masked to one colour the icon would be a filled square.
                let data = NSData::with_bytes(crate::window::ICON);
                if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
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
        assert_eq!(said(0, []), Said::default());

        // One yard.
        let one = said(1, [("mac", "1 mail".to_string())]);
        assert_eq!(one.title, "1");
        assert_eq!(rows(&one), [("mac", "mac · 1 mail")]);

        // Several, in the order they are given in.
        let several = said(
            3,
            [
                ("mac", "1 mail".to_string()),
                ("plantime", "1 mail, 1 decision".to_string()),
            ],
        );
        assert_eq!(several.title, "3");
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
        let nothing = said(0, [("mac", "nothing waiting".to_string())]);
        assert_eq!(nothing.title, "");
        assert_eq!(rows(&nothing), [("mac", "mac · nothing waiting")]);
    }

    #[test]
    fn the_item_is_told_only_what_is_new_and_hands_on_what_is_chosen() {
        let mut tray = Tray::default();
        assert_eq!(tray.told(), None);
        let one = said(1, [("mac", "1 mail".to_string())]);
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
