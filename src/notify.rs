//! Telling a person of what arrives while the window does not show it: a
//! notification of the operating system, for mail to them, a new decision,
//! a notice of the yard. Off until the settings switch it on.
//!
//! The rules are plain functions, tested on any system: what is told of
//! ([`wanted`]), and that a burst is one notification ([`Burst`]). How one
//! is shown is the system's ([`System`]):
//!
//! - macOS: `UNUserNotificationCenter`, the one way the system asks a
//!   person's permission for and lists in its settings. It knows an
//!   application by its bundle, so footplate notifies from `Footplate.app`
//!   (scripts/bundle-mac.sh) and not from a bare binary (`cargo run`).
//!   Activating a notification brings the window forward on its row.
//! - Linux: `notify-send`, which hands it to the desktop's notification
//!   service. No permission is asked there, and activating one opens
//!   nothing: `notify-send` has no way back into a running program.
//! - elsewhere: nothing is shown, and the settings say so.
//!
//! What a notification says was written by a crew member or in another
//! yard. It is text: no control character reaches the system, the body goes
//! to macOS as a string that is never markup, and to a Linux desktop with
//! its markup's characters escaped.

use std::time::Duration;

use unicode_segmentation::UnicodeSegmentation;

use crate::model::{self, Kind, News, Selected};
use crate::settings::Notify;

/// What arrives within this long of the first is told of with it, in one
/// notification: a crew member that writes three mails in a row does not
/// ring three times. A notification is this late at most.
pub const BURST: Duration = Duration::from_secs(3);

/// As much of a first line as a notification says. The system cuts what its
/// banner does not hold, and the list shows more; nothing needs all of a
/// mail.
const LINE: usize = 140;

/// Whether the system tells of `news`, with `notify` as the settings have
/// it. `in_view` is whether the window is in front and shows its rows (not
/// the settings in their place): what arrives on the open row is then read
/// where it stands, and nothing more tells of it.
pub fn wanted(notify: &Notify, news: &News, in_view: bool) -> bool {
    notify.tells(news.kind) && !(in_view && news.open)
}

/// One notification: two lines, and the row it leads to.
#[derive(Debug, Clone, PartialEq)]
pub struct Telling {
    pub title: String,
    pub body: String,
    /// Read where activating a notification leads somewhere: on macOS.
    #[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
    pub target: Selected,
}

/// What arrived since the last notification and is still to be told of.
#[derive(Debug, Default)]
pub struct Burst(Vec<News>);

impl Burst {
    /// Takes `news` in, and says whether it is the first: the window waits
    /// [`BURST`] from then and tells of all there is.
    pub fn add(&mut self, news: News) -> bool {
        self.0.push(news);
        self.0.len() == 1
    }

    /// Drops news that was waiting when the person changed what they want
    /// told of. The window also invalidates that batch's timer.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// All there is as one notification, or none when there is nothing. It
    /// leads to the row of the first.
    pub fn take(&mut self) -> Option<Telling> {
        let news = std::mem::take(&mut self.0);
        let first = news.first()?;
        let target = first.target.clone();
        if let [one] = news.as_slice() {
            let title = match one.kind {
                Kind::Mail => line(&one.from),
                Kind::Decision if one.from.is_empty() => "a decision waits".to_string(),
                Kind::Decision => format!("{} asks", line(&one.from)),
                Kind::Notice => model::YARD.to_string(),
            };
            return Some(Telling {
                title,
                body: line(&one.text),
                target,
            });
        }
        let mail_of_one = news
            .iter()
            .all(|n| n.kind == Kind::Mail && n.from == first.from);
        if mail_of_one {
            // The newest says most of where the sender stands.
            let last = news.last().unwrap_or(first);
            return Some(Telling {
                title: format!("{} from {}", news.len(), line(&first.from)),
                body: line(&last.text),
                target,
            });
        }
        // Of several kinds or senders: how many of each, mail by sender in
        // the order it came.
        let mut parts: Vec<(String, usize)> = Vec::new();
        let mut count = |part: String| match parts.iter_mut().find(|(have, _)| *have == part) {
            Some((_, n)) => *n += 1,
            None => parts.push((part, 1)),
        };
        for n in news.iter().filter(|n| n.kind == Kind::Mail) {
            count(format!("from {}", line(&n.from)));
        }
        let mut body: Vec<String> = parts
            .into_iter()
            .map(|(part, n)| format!("{n} {part}"))
            .collect();
        for (kind, one, several) in [
            (Kind::Decision, "decision", "decisions"),
            (Kind::Notice, "notice", "notices"),
        ] {
            match news.iter().filter(|n| n.kind == kind).count() {
                0 => {}
                1 => body.push(format!("1 {one}")),
                n => body.push(format!("{n} {several}")),
            }
        }
        Some(Telling {
            title: format!("{} new in the yard", news.len()),
            body: body.join(", "),
            target,
        })
    }
}

/// The first line of `text` that says something, as a notification has it:
/// no control character, and cut with an ellipsis where it is long, as a
/// row of the list cuts a name.
pub fn line(text: &str) -> String {
    let text = model::plain(text);
    let first = text
        .lines()
        .map(|line| line.replace('\t', " ").trim().to_string())
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let mut letters = first.graphemes(true);
    let mut cut: String = letters.by_ref().take(LINE).collect();
    if letters.next().is_some() {
        cut = format!("{}…", cut.trim_end());
    }
    cut
}

/// A row as the name a notification carries, so that activating it finds
/// the row again: [`row`] reads it back.
#[cfg(any(target_os = "macos", test))]
pub fn name(target: &Selected) -> String {
    match target {
        Selected::Conversation(with) => format!("conversation:{with}"),
        Selected::Decision(bead) | Selected::Decided(bead, _) => format!("decision:{bead}"),
        Selected::Yard => "yard".to_string(),
    }
}

/// The row a notification's name stands for, if it is one of ours.
#[cfg(any(target_os = "macos", test))]
pub fn row(name: &str) -> Option<Selected> {
    if name == "yard" {
        return Some(Selected::Yard);
    }
    let (kind, of) = name.split_once(':')?;
    match kind {
        "conversation" => Some(Selected::Conversation(of.to_string())),
        "decision" => Some(Selected::Decision(of.to_string())),
        _ => None,
    }
}

/// Whether the system lets footplate notify, as it answered when asked.
#[derive(Debug, Clone, PartialEq)]
pub enum Permission {
    Allowed,
    /// It does not, or cannot: why, in a line for the settings.
    Refused(String),
}

/// The operating system's notifications, as the window uses them.
pub trait System {
    /// Asks whether footplate may notify, the way the system wants it
    /// asked: the first time, macOS asks the person. The answer comes when
    /// they gave it, once.
    fn ask(&self) -> async_channel::Receiver<Permission>;

    /// Shows one notification. One the system does not take is dropped:
    /// [`System::ask`] said why it would be.
    fn show(&self, telling: &Telling);

    /// The rows of the notifications a person activates, as they do. On a
    /// system with no way back into the program nothing ever comes.
    fn activated(&self) -> async_channel::Receiver<Selected>;
}

/// A system whose notifications footplate does not send, which says so.
pub struct Nowhere(pub &'static str);

impl System for Nowhere {
    fn ask(&self) -> async_channel::Receiver<Permission> {
        let (tell, told) = async_channel::bounded(1);
        let _ = tell.try_send(Permission::Refused(self.0.to_string()));
        told
    }

    fn show(&self, _: &Telling) {}

    fn activated(&self) -> async_channel::Receiver<Selected> {
        async_channel::bounded(1).1
    }
}

/// This system's notifications. A test shows none: it runs where a person
/// works, and has a system of its own to look at.
pub fn system() -> Box<dyn System> {
    #[cfg(all(target_os = "macos", not(test)))]
    return Box::new(mac::Center::default());
    #[cfg(all(target_os = "linux", not(test)))]
    return Box::new(linux::Desktop);
    #[allow(unreachable_code)]
    Box::new(Nowhere(
        "footplate shows no system notifications on this system.",
    ))
}

/// The arguments `notify-send` shows `telling` with. The summary is plain
/// text to every notification service; the body is markup to most, so the
/// characters that would start markup are escaped. After `--` nothing is an
/// option, whatever a mail begins with. The icon is the one the taskbar's
/// desktop entry names (taskbar.rs).
#[cfg(any(target_os = "linux", test))]
pub fn notify_send(telling: &Telling) -> Vec<String> {
    let body = telling
        .body
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    vec![
        format!("--app-name={}", crate::NAME),
        format!("--icon={}", crate::NAME),
        "--".to_string(),
        telling.title.clone(),
        body,
    ]
}

#[cfg(all(target_os = "linux", not(test)))]
mod linux {
    use std::process::{Command, Stdio};
    use std::thread;

    use super::{Nowhere, Permission, Selected, System, Telling};

    /// The desktop's notification service, by way of `notify-send`.
    pub struct Desktop;

    fn notify_send() -> Command {
        let mut command = Command::new("notify-send");
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    impl System for Desktop {
        /// A Linux desktop asks no permission; what can keep a
        /// notification from showing is that `notify-send` is not there.
        fn ask(&self) -> async_channel::Receiver<Permission> {
            let (tell, told) = async_channel::bounded(1);
            thread::spawn(move || {
                let there = notify_send().arg("--version").status().is_ok();
                let _ = tell.try_send(match there {
                    true => Permission::Allowed,
                    false => Permission::Refused(
                        "notify-send was not found: footplate notifies through it (libnotify's, in most desktops' packages)."
                            .to_string(),
                    ),
                });
            });
            told
        }

        fn show(&self, telling: &Telling) {
            let arguments = super::notify_send(telling);
            // On a thread: it waits for the program to end, and the window
            // does not.
            thread::spawn(move || {
                if let Err(err) = notify_send().args(arguments).status() {
                    eprintln!("footplate: no notification: notify-send: {err}");
                }
            });
        }

        fn activated(&self) -> async_channel::Receiver<Selected> {
            Nowhere("").activated()
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::cell::OnceCell;

    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
        UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
        UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };

    use super::{Permission, Selected, System, Telling};

    const NO_BUNDLE: &str = "macOS notifies for an application, not for a bare program: build Footplate.app (scripts/bundle-mac.sh) and open that.";
    const DENIED: &str = "macOS does not let footplate notify: allow it in System Settings, Notifications, Footplate.";

    define_class!(
        // The center tells this object what a person does with a
        // notification, and asks it whether one is shown while footplate
        // is in front.
        #[unsafe(super(NSObject))]
        #[name = "FootplateNotifications"]
        #[ivars = async_channel::Sender<Selected>]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            // Without this macOS shows nothing while the application is in
            // front. Whether the window shows the row already is decided
            // before a notification is sent (`wanted`), so one that got
            // here is shown.
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                done: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                done.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
            }

            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                done: &DynBlock<dyn Fn()>,
            ) {
                let name = response.notification().request().identifier().to_string();
                if let Some(row) = super::row(&name) {
                    let _ = self.ivars().try_send(row);
                }
                done.call(());
            }
        }
    );

    impl Delegate {
        fn new(tell: async_channel::Sender<Selected>) -> Retained<Self> {
            let this = Self::alloc().set_ivars(tell);
            // SAFETY: NSObject's init, on an object just allocated with
            // its ivars set.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// The notification center of the bundle footplate runs in. Nothing of
    /// the system's is touched before a person switched notifications on.
    pub struct Center {
        activated: (
            async_channel::Sender<Selected>,
            async_channel::Receiver<Selected>,
        ),
        /// The center keeps no reference of its own to its delegate.
        delegate: OnceCell<Retained<Delegate>>,
    }

    impl Default for Center {
        fn default() -> Self {
            Center {
                activated: async_channel::unbounded(),
                delegate: OnceCell::new(),
            }
        }
    }

    impl Center {
        /// The center, with the delegate set, or none for a program that
        /// is in no bundle: asking for the center there raises an
        /// exception, which would end the program.
        fn center(&self) -> Option<Retained<UNUserNotificationCenter>> {
            NSBundle::mainBundle().bundleIdentifier()?;
            let center = UNUserNotificationCenter::currentNotificationCenter();
            let delegate = self
                .delegate
                .get_or_init(|| Delegate::new(self.activated.0.clone()));
            center.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
            Some(center)
        }
    }

    /// How the system has footplate's permission, without asking the
    /// person: 0 not asked yet, 1 denied, 2 allowed, 3 provisional. For
    /// the test that goes through the center in a bundle.
    #[cfg(test)]
    pub fn probe(center: &Center) -> Option<isize> {
        use std::ptr::NonNull;

        use objc2_user_notifications::UNNotificationSettings;

        let center = center.center()?;
        let (tell, told) = async_channel::bounded(1);
        let answered = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
            // SAFETY: the center hands valid settings for this call.
            let status = unsafe { settings.as_ref() }.authorizationStatus();
            let _ = tell.try_send(status.0);
        });
        center.getNotificationSettingsWithCompletionHandler(&answered);
        told.recv_blocking().ok()
    }

    impl System for Center {
        fn ask(&self) -> async_channel::Receiver<Permission> {
            let (tell, told) = async_channel::bounded(1);
            let Some(center) = self.center() else {
                let _ = tell.try_send(Permission::Refused(NO_BUNDLE.to_string()));
                return told;
            };
            // The first time this asks the person; afterwards it answers
            // what they said, or what they set in System Settings since.
            let answered = RcBlock::new(move |granted: Bool, error: *mut NSError| {
                // SAFETY: the center hands a valid error or none, for the
                // length of this call.
                let error =
                    unsafe { error.as_ref() }.map(|error| error.localizedDescription().to_string());
                let _ = tell.try_send(match (granted.as_bool(), error) {
                    (true, _) => Permission::Allowed,
                    (false, None) => Permission::Refused(DENIED.to_string()),
                    (false, Some(why)) => Permission::Refused(format!("{DENIED} ({why})")),
                });
            });
            center.requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert,
                &answered,
            );
            told
        }

        fn show(&self, telling: &Telling) {
            let Some(center) = self.center() else {
                return;
            };
            // Strings, set as the title and the body: the system reads no
            // markup in them and runs nothing of them.
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(&telling.title));
            content.setBody(&NSString::from_str(&telling.body));
            // Named by its row: activating it leads there, and a later one
            // of the same row takes its place in the system's list.
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &NSString::from_str(&super::name(&telling.target)),
                &content,
                None,
            );
            center.addNotificationRequest_withCompletionHandler(&request, None);
        }

        fn activated(&self) -> async_channel::Receiver<Selected> {
            self.activated.1.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(from: &str, text: &str) -> News {
        News {
            kind: Kind::Mail,
            from: from.into(),
            text: text.into(),
            target: Selected::Conversation(from.into()),
            open: false,
        }
    }

    fn ask(from: &str, text: &str, bead: &str) -> News {
        News {
            kind: Kind::Decision,
            from: from.into(),
            text: text.into(),
            target: Selected::Decision(bead.into()),
            open: false,
        }
    }

    fn notice(text: &str) -> News {
        News {
            kind: Kind::Notice,
            from: model::YARD.into(),
            text: text.into(),
            target: Selected::Yard,
            open: false,
        }
    }

    fn on() -> Notify {
        Notify {
            on: true,
            ..Notify::default()
        }
    }

    #[test]
    fn nothing_tells_until_it_is_switched_on_and_then_the_kinds_that_are_chosen() {
        let news = [
            mail("yardmaster", "Landed."),
            ask("yardmaster", "Land it?", "yd-1"),
            notice("yd-1 closed"),
        ];
        // Off, as it is for a person who never chose.
        for n in &news {
            assert!(!wanted(&Notify::default(), n, false), "{:?}", n.kind);
        }
        // On: mail and decisions, and the yard's notices once chosen.
        let told = |notify: Notify| {
            news.iter()
                .map(|n| wanted(&notify, n, false))
                .collect::<Vec<_>>()
        };
        assert_eq!(told(on()), [true, true, false]);
        assert_eq!(told(on().with(Kind::Notice, true)), [true, true, true]);
        assert_eq!(
            told(on().with(Kind::Mail, false).with(Kind::Decision, false)),
            [false, false, false]
        );
    }

    #[test]
    fn nothing_tells_of_what_arrives_on_the_open_row_of_a_window_in_front() {
        let shown = News {
            open: true,
            ..mail("yardmaster", "Landed.")
        };
        let elsewhere = mail("assembly", "Built.");
        // In front on that conversation: it is read where it stands.
        assert!(!wanted(&on(), &shown, true));
        // In front on another conversation, or behind another window, or
        // with the settings where the conversation would be.
        assert!(wanted(&on(), &elsewhere, true));
        assert!(wanted(&on(), &shown, false));
        assert!(wanted(&on(), &elsewhere, false));
    }

    #[test]
    fn one_arrival_names_who_it_is_from_and_its_first_line() {
        let mut burst = Burst::default();
        assert_eq!(burst.take(), None);
        assert!(burst.add(mail(
            "yardmaster@mac",
            "\n  Landed on main.\nThe tests held."
        )));
        assert_eq!(
            burst.take(),
            Some(Telling {
                title: "yardmaster@mac".into(),
                body: "Landed on main.".into(),
                target: Selected::Conversation("yardmaster@mac".into()),
            })
        );
        // Taken: the next is the first again.
        assert_eq!(burst.take(), None);
        assert!(burst.add(ask("assembly", "Land it?", "yd-1")));
        assert_eq!(
            burst.take(),
            Some(Telling {
                title: "assembly asks".into(),
                body: "Land it?".into(),
                target: Selected::Decision("yd-1".into()),
            })
        );
        burst.add(ask("", "Land it?", "yd-1"));
        assert_eq!(burst.take().unwrap().title, "a decision waits");
        burst.add(notice("yd-1 closed"));
        assert_eq!(
            burst.take(),
            Some(Telling {
                title: "yard".into(),
                body: "yd-1 closed".into(),
                target: Selected::Yard,
            })
        );
    }

    #[test]
    fn a_burst_is_one_notification() {
        let mut burst = Burst::default();
        assert!(burst.add(mail("yardmaster@mac", "One.")));
        assert!(!burst.add(mail("yardmaster@mac", "Two.")));
        assert!(!burst.add(mail("yardmaster@mac", "Three.")));
        assert_eq!(
            burst.take(),
            Some(Telling {
                title: "3 from yardmaster@mac".into(),
                body: "Three.".into(),
                target: Selected::Conversation("yardmaster@mac".into()),
            })
        );
        assert_eq!(burst.take(), None);

        // Of several senders and kinds: how many of each, and it leads to
        // the first.
        for news in [
            ask("yardmaster", "Land it?", "yd-1"),
            mail("assembly", "Built."),
            mail("yardmaster", "One."),
            mail("assembly", "Tested."),
            ask("assembly", "And this?", "yd-2"),
            notice("yd-1 closed"),
        ] {
            burst.add(news);
        }
        assert_eq!(
            burst.take(),
            Some(Telling {
                title: "6 new in the yard".into(),
                body: "2 from assembly, 1 from yardmaster, 2 decisions, 1 notice".into(),
                target: Selected::Decision("yd-1".into()),
            })
        );
    }

    #[test]
    fn a_line_is_the_first_that_says_something_cut_where_it_is_long() {
        assert_eq!(line(""), "");
        assert_eq!(line("\n\n"), "");
        assert_eq!(line("\r\n \t\nSecond\tline. \nThird."), "Second line.");
        // No control character reaches the system: an escape sequence of a
        // terminal is letters.
        assert_eq!(line("\u{1b}[31mred\u{7}\u{0}"), "[31mred");
        let long = "é".repeat(LINE + 1);
        let cut = line(&long);
        assert_eq!(cut.graphemes(true).count(), LINE + 1);
        assert!(cut.ends_with('…') && cut.starts_with('é'));
        assert_eq!(line(&"é".repeat(LINE)), "é".repeat(LINE));
        // A letter of several code points is cut whole.
        let family = "👨‍👩‍👧".repeat(LINE + 5);
        assert_eq!(line(&family), format!("{}…", "👨‍👩‍👧".repeat(LINE)));
    }

    #[test]
    fn a_row_is_found_again_by_the_name_its_notification_carries() {
        for target in [
            Selected::Conversation("yardmaster".into()),
            // A name is whatever another yard says it is.
            Selected::Conversation("clerk:of@north".into()),
            Selected::Decision("yardr-n8ta".into()),
            Selected::Yard,
        ] {
            assert_eq!(row(&name(&target)), Some(target));
        }
        // A crew member named yard is a conversation, not the yard row.
        assert_eq!(
            row("conversation:yard"),
            Some(Selected::Conversation("yard".into()))
        );
        for other in ["", "yards", "bead:yd-1", "com.apple.something"] {
            assert_eq!(row(other), None, "{other}");
        }
    }

    #[test]
    fn what_another_yard_wrote_reaches_notify_send_as_text() {
        let telling = Telling {
            title: "--urgency=critical".into(),
            body: "<a href=\"https://example.net\">look</a> & <b>see</b>".into(),
            target: Selected::Yard,
        };
        assert_eq!(
            notify_send(&telling),
            [
                "--app-name=footplate",
                "--icon=footplate",
                "--",
                "--urgency=critical",
                "&lt;a href=\"https://example.net\"&gt;look&lt;/a&gt; &amp; &lt;b&gt;see&lt;/b&gt;",
            ]
        );
    }

    #[test]
    fn a_system_footplate_does_not_notify_on_says_so_and_shows_nothing() {
        let nowhere = Nowhere("not here");
        assert_eq!(
            nowhere.ask().try_recv(),
            Ok(Permission::Refused("not here".into()))
        );
        nowhere.show(&Telling {
            title: "yard".into(),
            body: String::new(),
            target: Selected::Yard,
        });
        assert!(nowhere.activated().try_recv().is_err());
        // A test's own system is that one: no test rings where a person
        // works.
        assert!(matches!(
            system().ask().try_recv(),
            Ok(Permission::Refused(_))
        ));
    }

    /// What macOS itself answers, which no other test asks: run in a
    /// bundle, this goes through the calls a notification takes, short of
    /// asking the person and of showing one. In no bundle, as under
    /// `cargo test`, it holds that nothing of the center is touched.
    ///
    /// For a bundle, put the test binary in a copy of Footplate.app in
    /// place of `Contents/MacOS/footplate` and run it from there with
    /// `the_center_answers --nocapture`.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_center_answers_in_a_bundle_and_is_left_alone_outside_one() {
        use objc2_foundation::NSBundle;

        let center = mac::Center::default();
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            assert!(matches!(
                center.ask().try_recv(),
                Ok(Permission::Refused(why)) if why.contains("Footplate.app")
            ));
            center.show(&Telling {
                title: "yard".into(),
                body: String::new(),
                target: Selected::Yard,
            });
            return;
        }
        println!("in a bundle: {:?}", mac::probe(&center));
    }
}
