//! The number on the dock's icon: how many things wait for the person, over
//! every yard that is followed. It is a sign outside the window, there
//! while the window is behind another or hidden. gpui has no call for it,
//! so it is Cocoa's own, on the application's dock tile.
//!
//! What the icon says is a plain function, tested on any system; the call
//! is for macOS only. A Linux taskbar has no count.

/// What the icon says of `waiting` things: their number, and nothing where
/// nothing waits, which leaves the icon without a badge.
pub fn label(waiting: usize) -> Option<String> {
    (waiting > 0).then(|| waiting.to_string())
}

/// Shows on the dock's icon that `waiting` things wait. A test shows none:
/// it runs where a person works, and the dock is theirs.
#[cfg(all(target_os = "macos", not(test)))]
pub fn badge(waiting: usize) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSString;

    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let label = label(waiting).map(|label| NSString::from_str(&label));
    NSApplication::sharedApplication(main_thread)
        .dockTile()
        .setBadgeLabel(label.as_deref());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_says_the_number_and_nothing_where_nothing_waits() {
        assert_eq!(label(0), None);
        assert_eq!(label(1).as_deref(), Some("1"));
        assert_eq!(label(128).as_deref(), Some("128"));
    }
}
