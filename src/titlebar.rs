//! The window's title bar. On macOS the content reaches up into it: the
//! title bar is transparent, the traffic lights stand over the rail's top
//! where the system puts them, and no strip of its own is drawn above the
//! window. gpui can move a window by a strip of its content on Windows and
//! under X11 and Wayland, but not on macOS, where the content view takes
//! every press, so the press on the window's top, and a double-click there,
//! are followed here. On Linux the desktop draws the title bar, or the
//! window has none, and its options are the plain ones.
//!
//! What the window does is a plain function or a small state, tested on
//! any system; the call into AppKit is for macOS only.

use gpui::{App, Pixels, Point, TitlebarOptions, Window};

/// The title bar's height on macOS, in pixels: the strip across the top of
/// the window that moves it, and what the rail leaves empty for the traffic
/// lights. Measured on macOS 26, as the window's frame less its content
/// layout rect. An older system's is a few pixels less, which a little room
/// to spare does not hurt.
pub const HEIGHT: f32 = 32.;

/// Where the traffic lights end, from the window's top, in pixels: on macOS
/// 26 they are 14 high and stand 9 below it, centred in the title bar, as
/// AppKit's standard window buttons say.
pub const LIGHTS: f32 = 23.;

const _: () = assert!(LIGHTS <= HEIGHT, "the title bar holds the traffic lights");

/// How far a press on the top strip moves before it is a drag and not a
/// click: past gpui's own 2, since a hand on a trackpad wanders while it
/// presses a control, and the window must not move under a click.
const SLOP: f32 = 3.;

/// What the window opens with as its title bar. `macos` says this is a Mac:
/// there the title bar is transparent, so the content reaches the top of
/// the window and the traffic lights stand over it. They are left where the
/// system puts them, which is right for its version, and in full screen,
/// where gpui's move of them does nothing. Elsewhere the options are the
/// plain ones, and the desktop draws what it likes. The title is set either
/// way: the Window menu, Mission Control and the accessibility tree read it
/// where no bar shows it.
pub fn options(macos: bool) -> TitlebarOptions {
    TitlebarOptions {
        title: Some(crate::NAME.into()),
        appears_transparent: macos,
        traffic_light_position: None,
    }
}

/// How much of the window's top the title bar covers, which the rail leaves
/// empty: [`HEIGHT`] where the content reaches up into it, and nothing where
/// the desktop draws the title bar above the window, or in full screen,
/// where the window has none of its own.
pub fn inset(reaches_up: bool, fullscreen: bool) -> f32 {
    if reaches_up && !fullscreen {
        HEIGHT
    } else {
        0.
    }
}

/// What the window does for a gesture on its top strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    /// Nothing: a click, and a press that is no title bar's.
    Nothing,
    /// The window follows the pointer from here.
    Move,
    /// A double-click: what the system's setting gives a title bar, which is
    /// to zoom, to minimise, or to do nothing.
    Zoom,
}

/// A left press on the top strip that has not yet moved far enough to be a
/// drag. The window moves on the drag and not on the press: a press on a
/// control in the strip must still be a click, and a click is a press that
/// stays.
#[derive(Debug, Default, PartialEq)]
pub struct Press(Option<Point<Pixels>>);

impl Press {
    /// The button went down at `at`, for the `clicks`th time in a row. A
    /// second click anywhere in the strip is a double-click; a first one in
    /// it waits to see whether it drags.
    pub fn down(&mut self, at: Point<Pixels>, clicks: usize) -> Gesture {
        self.0 = None;
        if f32::from(at.y) >= HEIGHT {
            return Gesture::Nothing;
        }
        match clicks {
            1 => {
                self.0 = Some(at);
                Gesture::Nothing
            }
            2 => Gesture::Zoom,
            _ => Gesture::Nothing,
        }
    }

    /// The pointer moved to `at`, with the left button down or not. The
    /// first move far enough from the press with the button still down is
    /// the drag; the press is spent by it.
    pub fn moved(&mut self, at: Point<Pixels>, left_down: bool) -> Gesture {
        let Some(from) = self.0 else {
            return Gesture::Nothing;
        };
        if !left_down {
            self.0 = None;
            return Gesture::Nothing;
        }
        let (dx, dy) = (f32::from(at.x - from.x), f32::from(at.y - from.y));
        if dx * dx + dy * dy <= SLOP * SLOP {
            return Gesture::Nothing;
        }
        self.0 = None;
        Gesture::Move
    }

    /// The button went up: whatever was waiting is a click.
    pub fn up(&mut self) {
        self.0 = None;
    }
}

/// What the window does for `gesture` on the top strip, on a Mac.
pub fn perform(gesture: Gesture, window: &mut Window, _: &mut App) {
    match gesture {
        Gesture::Nothing => {}
        // The setting is the system's: gpui reads it, and zooms, minimises
        // or leaves the window as it says.
        Gesture::Zoom => window.titlebar_double_click(),
        Gesture::Move => start_move(),
    }
}

/// Has AppKit move the window with the pointer, from the drag being
/// handled. The window server takes the move over, and AppKit says (in
/// NSWindow.h) that the call returns right away and a mouse-up may never be
/// sent. gpui, which repeats the drag it was last told of every 16
/// milliseconds until it hears the button go up, would repeat this one
/// for good, and hover would hold at the place the window was taken by
/// until the next click; so it is told of the release here, off the window,
/// where it clicks nothing. A test moves nothing: it runs where a person
/// works, and the windows are theirs.
#[cfg(all(target_os = "macos", not(test)))]
pub fn start_move() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_foundation::NSPoint;

    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    // The event being handled. Where it is not a press or a drag of the
    // left button, there is nothing to move from; the next drag asks again.
    let Some(event) = NSApplication::sharedApplication(main_thread).currentEvent() else {
        return;
    };
    if !matches!(
        event.r#type(),
        NSEventType::LeftMouseDown | NSEventType::LeftMouseDragged
    ) {
        return;
    }
    let Some(window) = event.window(main_thread) else {
        return;
    };
    window.performWindowDragWithEvent(&event);
    let up = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::LeftMouseUp,
        NSPoint::new(-1., -1.),
        NSEventModifierFlags::empty(),
        event.timestamp(),
        event.windowNumber(),
        None,
        event.eventNumber(),
        1,
        0.,
    );
    // The view that has the window's mouse events is gpui's.
    if let (Some(view), Some(up)) = (window.firstResponder(), up) {
        view.mouseUp(&up);
    }
}

#[cfg(not(all(target_os = "macos", not(test))))]
pub fn start_move() {}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px};

    fn at(x: f32, y: f32) -> Point<Pixels> {
        point(px(x), px(y))
    }

    #[test]
    fn a_mac_window_has_a_transparent_title_bar_and_the_lights_where_the_system_puts_them() {
        let bar = options(true);
        assert!(bar.appears_transparent);
        assert_eq!(bar.traffic_light_position, None);
    }

    #[test]
    fn any_other_window_keeps_the_plain_options() {
        // What the window opened with before the title bar was a Mac's
        // concern: the desktop's own bar, and no say in where its buttons are.
        let bar = options(false);
        assert!(!bar.appears_transparent);
        assert_eq!(bar.traffic_light_position, None);
    }

    #[test]
    fn the_title_is_set_where_no_bar_shows_it() {
        for macos in [true, false] {
            assert_eq!(
                options(macos).title.map(|title| title.to_string()),
                Some("footplate".to_string()),
                "macos {macos}"
            );
        }
    }

    #[test]
    fn the_rail_leaves_the_title_bar_empty_only_where_the_content_reaches_it() {
        assert_eq!(inset(true, false), HEIGHT);
        // The desktop's bar is above the window, not in it.
        assert_eq!(inset(false, false), 0.);
        // Full screen has none of its own: the system slides one over.
        assert_eq!(inset(true, true), 0.);
        assert_eq!(inset(false, true), 0.);
    }

    #[test]
    fn a_press_on_the_strip_that_stays_is_a_click() {
        let mut press = Press::default();
        assert_eq!(press.down(at(200., 10.), 1), Gesture::Nothing);
        // A hand wanders a pixel or two.
        assert_eq!(press.moved(at(202., 11.), true), Gesture::Nothing);
        press.up();
        assert_eq!(press.moved(at(260., 10.), true), Gesture::Nothing);
    }

    #[test]
    fn a_press_on_the_strip_that_travels_moves_the_window_once() {
        let mut press = Press::default();
        assert_eq!(press.down(at(200., 10.), 1), Gesture::Nothing);
        assert_eq!(press.moved(at(204., 10.), true), Gesture::Move);
        // The window has the drag from here: no second move for each step.
        assert_eq!(press.moved(at(230., 40.), true), Gesture::Nothing);
    }

    #[test]
    fn a_drag_may_leave_the_strip_before_it_is_one() {
        let mut press = Press::default();
        press.down(at(200., 30.), 1);
        // A quick drag downward has left the strip by its first move.
        assert_eq!(press.moved(at(200., 60.), true), Gesture::Move);
    }

    #[test]
    fn a_press_below_the_strip_moves_nothing() {
        let mut press = Press::default();
        assert_eq!(press.down(at(200., HEIGHT), 1), Gesture::Nothing);
        assert_eq!(press.moved(at(300., 90.), true), Gesture::Nothing);
        assert_eq!(press.down(at(200., 400.), 2), Gesture::Nothing);
    }

    #[test]
    fn a_pointer_that_moves_with_the_button_up_is_not_a_drag() {
        let mut press = Press::default();
        press.down(at(200., 10.), 1);
        // The release was missed, say outside the window: a move without
        // the button ends the wait.
        assert_eq!(press.moved(at(300., 10.), false), Gesture::Nothing);
        assert_eq!(press.moved(at(400., 10.), true), Gesture::Nothing);
    }

    #[test]
    fn a_second_click_in_the_strip_is_a_double_click() {
        let mut press = Press::default();
        assert_eq!(press.down(at(500., 12.), 1), Gesture::Nothing);
        press.up();
        assert_eq!(press.down(at(500., 12.), 2), Gesture::Zoom);
        // It does not wait to be dragged on: the window is zooming.
        assert_eq!(press.moved(at(560., 12.), true), Gesture::Nothing);
        // Nor does a third.
        press.up();
        assert_eq!(press.down(at(500., 12.), 3), Gesture::Nothing);
        assert_eq!(press.moved(at(560., 12.), true), Gesture::Nothing);
    }

    #[test]
    fn a_new_press_forgets_the_last() {
        let mut press = Press::default();
        press.down(at(200., 10.), 1);
        // The next press is below the strip: the first no longer waits.
        press.down(at(200., 200.), 1);
        assert_eq!(press.moved(at(260., 10.), true), Gesture::Nothing);
    }
}
