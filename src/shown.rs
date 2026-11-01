//! Whether the window shows, on Linux. gpui tells of neither a minimized
//! window nor a hidden one there, and a voice session does not go on under a
//! window that shows nothing (design 3.2), so it asks while it stands. What a
//! desktop can answer differs:
//!
//! - Under X11 the window manager puts `_NET_WM_STATE_HIDDEN` in the window's
//!   `_NET_WM_STATE` when it has minimized it. footplate reads it on a
//!   connection of its own, since gpui gives no handle to its X11 window. A
//!   window that others cover shows still, as it does on a Mac.
//! - A Wayland compositor tells a client nothing of it, but it sends no frame
//!   callbacks to a surface that shows nowhere. So the window stamps its
//!   frames (`Means::frame`, kept by the voice session while it stands), and
//!   one with no frame for two seconds is out of view. That is wider than a
//!   Mac's answer: a window that others cover wholly, or that stands on a
//!   workspace out of view, gets no frames either, and the session ends with
//!   it. The microphone off wherever nothing shows is the design's direction,
//!   and Wayland has no narrower sign.
//!
//! The rule is a plain function, tested on any system; the read of the X
//! server is for Linux only.

use std::time::Duration;

/// How long a window may go with no frame before the compositor is taken to
/// show it nowhere. A window that shows is asked for a frame at every
/// refresh, drawing or not (gpui commits its surface after each), so a gap
/// this long is not its idleness.
pub const STALLED: Duration = Duration::from_secs(2);

/// Whether the window is out of view on the desktop `compositor` names, as
/// gpui guesses it: under X11 where the window manager has it hidden,
/// elsewhere where `frame_age`, the time since its last frame, has reached
/// [`STALLED`]. `window_manager` is the read of the manager's word, made
/// under X11 alone, so that a Wayland session with an X server beside it
/// never connects to that. It gives none where the word could not be had,
/// and that says no: a window that nothing can be said of shows.
pub fn hidden(
    compositor: &str,
    window_manager: impl FnOnce() -> Option<bool>,
    frame_age: Duration,
) -> bool {
    match compositor {
        "X11" => window_manager() == Some(true),
        _ => frame_age >= STALLED,
    }
}

/// Whether the X server's window manager has footplate's window hidden, or
/// none where that could not be asked.
#[cfg(target_os = "linux")]
#[cfg_attr(test, allow(dead_code))]
pub fn x11_hidden() -> Option<bool> {
    x11::hidden()
}

#[cfg(target_os = "linux")]
mod x11 {
    use std::cell::RefCell;
    use std::error::Error;
    use std::sync::atomic::{AtomicBool, Ordering};

    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, Window};
    use x11rb::rust_connection::RustConnection;

    use crate::taskbar::x11::find;

    /// The connection and what it has learned of the window. A session asks
    /// every 200 ms, and a connection made and the server's windows walked for
    /// each ask would be most of the cost.
    struct Watch {
        connection: RustConnection,
        root: Window,
        /// `_NET_WM_PID`, which the window is found by, `_NET_WM_STATE`, and
        /// `_NET_WM_STATE_HIDDEN`, which stands in it while the window is
        /// minimized.
        pid: Atom,
        state: Atom,
        hidden: Atom,
        /// The window, once found.
        window: Option<Window>,
    }

    thread_local! {
        /// Made at the first ask, on the thread that asks (the window's), and
        /// kept for the process.
        static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) };
    }

    impl Watch {
        fn connect() -> Result<Watch, Box<dyn Error>> {
            let (connection, screen) = x11rb::connect(None)?;
            let root = connection.setup().roots[screen].root;
            // Asked together, answered together.
            let pid = connection.intern_atom(false, b"_NET_WM_PID")?;
            let state = connection.intern_atom(false, b"_NET_WM_STATE")?;
            let hidden = connection.intern_atom(false, b"_NET_WM_STATE_HIDDEN")?;
            let (pid, state, hidden) =
                (pid.reply()?.atom, state.reply()?.atom, hidden.reply()?.atom);
            Ok(Watch {
                connection,
                root,
                pid,
                state,
                hidden,
                window: None,
            })
        }

        /// Whether `_NET_WM_STATE_HIDDEN` stands in the window's state. A
        /// window with no state at all (no window manager) is not hidden.
        fn ask(&mut self) -> Result<bool, Box<dyn Error>> {
            let window = match self.window {
                Some(window) => window,
                None => {
                    let window = find(&self.connection, self.root, crate::NAME, self.pid)?
                        .ok_or("the window was not found on the X server")?;
                    self.window = Some(window);
                    window
                }
            };
            let state = self
                .connection
                .get_property(false, window, self.state, AtomEnum::ATOM, 0, 64)?
                .reply()?;
            Ok(state
                .value32()
                .is_some_and(|mut atoms| atoms.any(|atom| atom == self.hidden)))
        }
    }

    pub fn hidden() -> Option<bool> {
        WATCH.with_borrow_mut(|kept| {
            let asked = match kept.as_mut() {
                Some(watch) => watch.ask(),
                None => match Watch::connect() {
                    Ok(watch) => kept.insert(watch).ask(),
                    Err(err) => Err(err),
                },
            };
            match asked {
                Ok(hidden) => Some(hidden),
                // Whatever went wrong answers no, and the next ask starts
                // over: a window not found yet may be there by then.
                Err(err) => {
                    *kept = None;
                    // Said once, not at every tick.
                    static SAID: AtomicBool = AtomicBool::new(false);
                    if !SAID.swap(true, Ordering::Relaxed) {
                        eprintln!("footplate: no word of whether the window is minimized: {err}");
                    }
                    None
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// Under X11 the window manager's word is the whole answer, whatever the
    /// frames say: its refresh loop stops for a window that others cover, and
    /// that window shows.
    #[test]
    fn under_x11_the_window_is_hidden_where_the_window_manager_says_so() {
        for age in [Duration::ZERO, STALLED, Duration::from_secs(600)] {
            assert!(
                hidden("X11", || Some(true), age),
                "hidden stands in the state"
            );
            assert!(!hidden("X11", || Some(false), age), "it does not");
            // No connection, no window, no property read: no word, no.
            assert!(!hidden("X11", || None, age), "no word");
        }
    }

    /// Anywhere else the frames are the only sign, and the X server is not
    /// asked: a Wayland session may have one beside it that knows nothing of
    /// the window.
    #[test]
    fn elsewhere_the_window_is_hidden_where_no_frame_has_come_for_two_seconds() {
        let asked = Cell::new(false);
        let manager = || {
            asked.set(true);
            Some(true)
        };
        for compositor in ["Wayland", "Headless", ""] {
            assert!(
                !hidden(compositor, manager, Duration::ZERO),
                "a fresh frame"
            );
            assert!(
                !hidden(compositor, manager, STALLED - Duration::from_millis(1)),
                "a frame just short of two seconds old"
            );
            assert!(
                hidden(compositor, manager, STALLED),
                "a frame two seconds old"
            );
            assert!(
                hidden(compositor, manager, Duration::from_secs(600)),
                "no frame since"
            );
        }
        assert_eq!(STALLED, Duration::from_secs(2));
        assert!(!asked.get(), "the window manager was asked");
    }
}
