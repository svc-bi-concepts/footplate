//! footplate: a native chat window onto a yardr yard. It is an interface over
//! yardr's socket API, or over the same calls at a yard's address. Where
//! this machine has no yard it sets one up, with the yardr it fetches
//! (fetch.rs, setup.rs), and its head starts and stops the open yard with
//! the same yardr; beside those two it starts nothing of yardr's. On Windows
//! there is no yard of this machine to set up or reach, and the window is of
//! a yard at an address alone. The first slice is the
//! conversations a person has with this yard's crew and with the crew a
//! linked yard exposes, as `yardr chat` shows them.

// A release build on Windows is a window and no console; a debug build keeps
// the console, where what it prints is read.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
// The tests that start the socket fake, or a shell, are a unix system's and
// are left out on Windows: what only they use, helpers and imports, is unused
// there. Only that build allows it; the others still fail on dead code.
#![cfg_attr(all(test, not(unix)), allow(dead_code, unused_imports))]

mod api;
#[cfg(any(target_os = "macos", target_os = "linux", test))]
mod dock;
mod draft;
#[cfg(any(test, debug_assertions))]
#[cfg_attr(not(test), allow(dead_code))]
mod fake;
mod fetch;
mod input;
mod markdown;
mod model;
mod notify;
mod select;
mod session;
mod settings;
mod setup;
#[cfg(all(debug_assertions, unix))]
mod shot;
// A Linux desktop's: its tests name paths of a unix system's, so Windows
// has neither.
#[cfg(any(target_os = "linux", all(test, unix)))]
mod taskbar;
mod theme;
mod titlebar;
#[cfg(any(target_os = "macos", target_os = "linux", test))]
mod tray;
mod unlock;
mod window;
mod yard;

use std::cell::Cell;
use std::ffi::OsString;
use std::rc::Rc;

use gpui::{
    App, AppContext, Application, Bounds, Focusable, KeyBinding, Menu, MenuItem, WindowBounds,
    WindowOptions, actions, px, size,
};

use crate::window::Footplate;

/// The application's name: the window's title, and what a desktop groups
/// its windows under.
const NAME: &str = "footplate";

actions!(footplate, [Quit]);

/// Whether the program was run only to say its version. It reads no other
/// argument, since the window takes none.
fn asks_version(args: impl IntoIterator<Item = OsString>) -> bool {
    args.into_iter().any(|arg| arg == "--version")
}

/// What `--version` says: the name and Cargo.toml's version, the one a release
/// is tagged with (scripts/release.sh). The Windows workflow runs the exe it
/// zips for this line (.github/workflows/windows.yml), since a Mac cannot run
/// that build.
fn version_line() -> String {
    format!("{NAME} {}", env!("CARGO_PKG_VERSION"))
}

fn main() {
    // Before the environment is read or anything is drawn. A Windows release
    // has no console (windows_subsystem, above), so there the line reaches a
    // pipe or a file, which is how the workflow reads it, and not a console.
    if asks_version(std::env::args_os().skip(1)) {
        println!("{}", version_line());
        return;
    }
    let env = yard::Env::read();
    // In a debug build only, a screenshot state starts a fake yard of its
    // own and points this run's socket at it; held for the run, since
    // dropping it stops the yard. A release build does not read the
    // variable at all, nor does Windows, which has no socket to point.
    #[cfg(all(debug_assertions, unix))]
    let (env, _shot) = shot::start_if_asked(env);
    let name = session::login_name();

    let application = Application::new();
    // The system unhides a hidden footplate by itself on a click in the
    // dock and on cmd+tab. Started again while it is hidden, it is only
    // told so, here, and shows its window itself. The window is opened
    // once the application runs, and is known here from then on.
    let window = Rc::new(Cell::new(None));
    let opened_window = window.clone();
    application.on_reopen(move |cx| {
        let Some(window) = window.get() else {
            return;
        };
        unhide();
        cx.activate(true);
        cx.update_window(window, |_, window, _| window.activate_window())
            .ok();
    });
    application.run(move |cx: &mut App| {
        dock_icon();
        input::bind_keys(cx);
        window::bind_keys(cx);
        cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.set_menus(vec![Menu {
            name: NAME.into(),
            items: vec![
                MenuItem::action("About footplate", window::OpenAbout),
                MenuItem::separator(),
                MenuItem::action("Settings…", window::OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Quit footplate", Quit),
            ],
        }]);
        // The one window is the application: once it has closed, footplate
        // quits. Where closing hides instead (the window asks its view,
        // Footplate::closing_hides), it never closes, and only Quit quits.
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(960.), px(640.)),
                cx,
            ))),
            titlebar: Some(titlebar::options(cfg!(target_os = "macos"))),
            // The 560 the list and the pane have between them, and the rail
            // beside them: every pane keeps its size.
            window_min_size: Some(size(px(window::WINDOW_MIN), px(360.))),
            // On Linux the taskbar's icon and name are those of the desktop
            // entry with this id, which dock_icon puts there.
            app_id: Some(NAME.into()),
            ..Default::default()
        };
        let opened = cx.open_window(options, |window, cx| {
            let view = cx.new(|cx| Footplate::opened(env, name, settings::file(), window, cx));
            window.focus(&view.read(cx).line().focus_handle(cx));
            view
        });
        match opened {
            Ok(window) => {
                opened_window.set(Some(window.into()));
                cx.activate(true);
            }
            Err(err) => {
                eprintln!("footplate: the window could not be opened: {err}");
                cx.quit();
            }
        }
    });
}

/// gpui's window options have no icon. On macOS a program that is no bundle
/// gets the dock's icon by setting it on the application. In a bundle
/// (scripts/bundle-mac.sh) it is the same picture as the bundle's own.
#[cfg(target_os = "macos")]
fn dock_icon() {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(window::ICON);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    // SAFETY: called on the main thread with an image that was just made;
    // the application keeps its own reference to it.
    unsafe { NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&image)) };
}

/// Shows the application again where closing its window hid it. gpui can
/// hide but has no call that unhides, and activating alone does not promise
/// it.
#[cfg(target_os = "macos")]
fn unhide() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    if let Some(main_thread) = MainThreadMarker::new() {
        NSApplication::sharedApplication(main_thread).unhide(None);
    }
}

/// Only macOS hides footplate.
#[cfg(not(target_os = "macos"))]
fn unhide() {}

/// On Linux it is the desktop entry of the window's id, and under X11 a
/// property of the window too.
#[cfg(target_os = "linux")]
fn dock_icon() {
    taskbar::show(NAME, taskbar::ICON);
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn dock_icon() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_version_is_asked_by_its_flag_alone() {
        assert!(asks_version(args(&["--version"])));
        assert!(asks_version(args(&["--other", "--version"])));
        assert!(!asks_version(args(&[])));
        for near in ["version", "-V", "--versions", "--version=1"] {
            assert!(!asks_version(args(&[near])), "{near}");
        }
    }

    /// The Windows workflow holds the line to the version it reads from
    /// Cargo.toml as scripts/release.sh reads it (the first `version = "..."`
    /// line), so the line is the name and that version.
    #[test]
    fn the_version_line_is_the_name_and_the_version_a_release_reads() {
        let version = include_str!("../Cargo.toml")
            .lines()
            .find_map(|line| line.strip_prefix("version = \"")?.strip_suffix('"'))
            .expect("Cargo.toml has a version line");
        assert_eq!(version_line(), format!("footplate {version}"));
    }
}
