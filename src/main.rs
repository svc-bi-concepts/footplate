//! footplate: a native chat window onto a yardr yard. It is an interface over
//! yardr's socket API and starts nothing of yardr's. The first slice is the
//! conversations a person has with this yard's crew and with the crew a
//! linked yard exposes, as `yardr chat` shows them.

mod api;
mod draft;
#[cfg(test)]
mod fake;
mod input;
mod model;
mod session;
#[cfg(any(target_os = "linux", test))]
mod taskbar;
mod theme;
mod window;

use gpui::{
    App, AppContext, Application, Bounds, Focusable, KeyBinding, Menu, MenuItem, TitlebarOptions,
    WindowBounds, WindowOptions, actions, px, size,
};

use crate::api::Client;
use crate::window::Footplate;

/// The application's name: the window's title, and what a desktop groups
/// its windows under.
const NAME: &str = "footplate";

actions!(footplate, [Quit]);

fn main() {
    let client = Client::new(api::socket_path());
    let name = session::login_name();

    Application::new().run(move |cx: &mut App| {
        dock_icon();
        input::bind_keys(cx);
        window::bind_keys(cx);
        cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.set_menus(vec![Menu {
            name: NAME.into(),
            items: vec![MenuItem::action("Quit footplate", Quit)],
        }]);
        // The one window is the application.
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
            titlebar: Some(TitlebarOptions {
                title: Some(NAME.into()),
                ..Default::default()
            }),
            window_min_size: Some(size(px(560.), px(360.))),
            // On Linux the taskbar's icon and name are those of the desktop
            // entry with this id, which dock_icon puts there.
            app_id: Some(NAME.into()),
            ..Default::default()
        };
        let opened = cx.open_window(options, |window, cx| {
            let view = cx.new(|cx| Footplate::new(client, name, window, cx));
            window.focus(&view.read(cx).line().focus_handle(cx));
            view
        });
        match opened {
            Ok(_) => cx.activate(true),
            Err(err) => {
                eprintln!("footplate: the window could not be opened: {err}");
                cx.quit();
            }
        }
    });
}

/// gpui's window options have no icon. On macOS a program that is no bundle
/// gets the dock's icon by setting it on the application.
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

/// On Linux it is the desktop entry of the window's id, and under X11 a
/// property of the window too.
#[cfg(target_os = "linux")]
fn dock_icon() {
    taskbar::show(NAME, window::ICON);
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn dock_icon() {}
