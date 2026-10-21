//! The system's unlock in front of a weighty approval, before the window
//! sends `weighed`: on a Mac Touch ID, a paired watch or the login
//! password, through LocalAuthentication; on Linux the desktop's own
//! password or fingerprint prompt, through polkit and footplate's action
//! (`assets/linux/dev.yardr.footplate.policy`). One gate per approval,
//! with no reuse window: a hand pressing through twenty cards stops at
//! each weighty one.
//!
//! The reasons, and what footplate says to polkit, are plain functions,
//! tested on any system. Whether the system offers the gate
//! ([`available`]) and asking it ([`evaluate`]) are the system's: macOS
//! and Linux, and nowhere else. Where there is no gate the card asks the
//! ask's number back instead, as a terminal does, and sends `weighed`
//! where the number matches.

/// What the system shows with its prompt for a run ask's approval: what
/// is approved, and in which yard. Never empty: the call throws on an
/// empty reason.
pub fn approve_reason(command: &str, yard: &str) -> String {
    match command.is_empty() {
        true => format!("approve in {yard}"),
        false => format!("approve {command} in {yard}"),
    }
}

/// What the system shows with its prompt for a setup ask's approval: the
/// pack, and in which yard.
pub fn setup_reason(pack: &str, yard: &str) -> String {
    match pack.is_empty() {
        true => format!("approve the setup in {yard}"),
        false => format!("approve the setup of pack {pack} in {yard}"),
    }
}

/// What the system shows with its prompt for a secret ask's giving: the
/// names, and in which yard.
pub fn give_reason(names: &[String], yard: &str) -> String {
    match names.is_empty() {
        true => format!("give the secret in {yard}"),
        false => format!("give {} in {yard}", names.join(", ")),
    }
}

/// Whether the system offers the gate: macOS with a policy the person
/// can pass, Linux with polkit answering and footplate's action
/// installed, nothing elsewhere. Asked where a weighty card is drawn, and
/// where its control goes; it shows nothing of its own.
pub fn available() -> bool {
    #[cfg(target_os = "macos")]
    {
        mac::can_evaluate()
    }
    #[cfg(target_os = "linux")]
    {
        linux::available()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

/// Asks the system: answers true where the person passed, false where
/// they cancelled or failed. Never blocks the window: the answer comes
/// on the receiver, from whatever queue the system answers on. An empty
/// reason answers false without asking: the call throws on one.
pub fn evaluate(reason: &str) -> async_channel::Receiver<bool> {
    let refused = || {
        let (tell, told) = async_channel::bounded(1);
        let _ = tell.try_send(false);
        told
    };
    if reason.is_empty() {
        return refused();
    }
    #[cfg(target_os = "macos")]
    {
        mac::evaluate(reason)
    }
    #[cfg(target_os = "linux")]
    {
        linux::evaluate(reason)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        // No gate: refused, as a cancel is. Unreached, since
        // `available` is false wherever this builds.
        let _ = reason;
        refused()
    }
}

/// footplate's polkit action, as `assets/linux/dev.yardr.footplate.policy`
/// declares it: the person's own password, in an active session only.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const ACTION: &str = "dev.yardr.footplate.approve";

/// How long the answer of whether polkit offers the gate holds before it
/// is asked again: long enough that drawing a card asks the bus nothing,
/// short enough that an action installed while footplate runs is found.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const PROBE_HOLDS: std::time::Duration = std::time::Duration::from_secs(30);

/// The process polkit is asked about: footplate itself. polkit holds the
/// pid to its start time and uid, so a pid reused by another process
/// passes nothing.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, PartialEq)]
struct Subject {
    pid: u32,
    start_time: u64,
    uid: u32,
}

/// A process's start time, in clock ticks since boot, from its
/// `/proc/<pid>/stat`: the 22nd field. The name, the 2nd, is in
/// parentheses and may hold spaces and parentheses itself, so the fields
/// are counted from the last `)`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn start_time(stat: &str) -> Option<u64> {
    let (_, rest) = stat.rsplit_once(')')?;
    // The fields after the name start at the 3rd: the 22nd is the 20th.
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// What footplate passes polkit beside the action. The reason would be
/// `polkit.message`, the prompt's words, but polkit fails a check that
/// carries details from a caller not trusted with them (uid 0): there
/// the details are empty, and the prompt says the action file's message.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn details(reason: &str, uid: u32) -> Vec<(&'static str, String)> {
    match uid {
        0 => vec![("polkit.message", reason.to_string())],
        _ => vec![],
    }
}

/// Whether polkit's answer, asked without a prompt, offers the gate: the
/// person passes already (a rule of the system's says yes), or a prompt
/// would ask them. Neither is a session the action refuses, an inactive
/// one: a prompt there could pass nothing.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn offered(authorized: bool, challenge: bool) -> bool {
    authorized || challenge
}

#[cfg(target_os = "linux")]
mod linux {
    //! polkit on the system bus, through zbus. Each call runs on a thread
    //! of its own: the window is never held for the bus, nor for a prompt.

    use super::{ACTION, PROBE_HOLDS, Subject, details, offered, start_time};
    use std::collections::HashMap;
    use std::sync::{Condvar, Mutex};
    use std::time::{Duration, Instant};
    use zbus::zvariant::Value;

    /// How long drawing a card waits for polkit's first answer, which
    /// takes milliseconds where polkit runs. Where it is slower the card
    /// asks for the number until the answer comes.
    const FIRST_WAIT: Duration = Duration::from_millis(250);

    /// The flag that lets polkit prompt: CheckAuthorization's
    /// AllowUserInteraction.
    const ALLOW_USER_INTERACTION: u32 = 1;

    /// polkit's last answer of whether it offers the gate, when it was
    /// given, whether a thread is asking it now, and whether a card has
    /// waited for the first answer already: one that never comes holds
    /// the window once, not on every frame.
    struct Probe {
        offered: Option<bool>,
        at: Option<Instant>,
        asking: bool,
        waited: bool,
    }

    static PROBE: Mutex<Probe> = Mutex::new(Probe {
        offered: None,
        at: None,
        asking: false,
        waited: false,
    });
    static ANSWERED: Condvar = Condvar::new();

    /// The last answer, asked again on a thread where it is older than
    /// [`PROBE_HOLDS`]. Drawing a card asks this each frame: the bus is
    /// asked from here at most once in that time.
    pub(super) fn available() -> bool {
        let Ok(mut probe) = PROBE.lock() else {
            return false;
        };
        let stale = probe.at.is_none_or(|at| at.elapsed() >= PROBE_HOLDS);
        if stale && !probe.asking {
            probe.asking = true;
            std::thread::spawn(|| {
                let answer = async_io::block_on(ask_offered()).unwrap_or(false);
                if let Ok(mut probe) = PROBE.lock() {
                    probe.offered = Some(answer);
                    probe.at = Some(Instant::now());
                    probe.asking = false;
                }
                ANSWERED.notify_all();
            });
        }
        if probe.offered.is_none() && !probe.waited {
            probe.waited = true;
            probe = match ANSWERED.wait_timeout_while(probe, FIRST_WAIT, |p| p.offered.is_none()) {
                Ok((probe, _)) => probe,
                Err(_) => return false,
            };
        }
        probe.offered.unwrap_or(false)
    }

    pub(super) fn evaluate(reason: &str) -> async_channel::Receiver<bool> {
        let (tell, told) = async_channel::bounded(1);
        let reason = reason.to_string();
        std::thread::spawn(move || {
            let passed = async_io::block_on(check(&reason, ALLOW_USER_INTERACTION))
                .is_ok_and(|(authorized, _)| authorized);
            let _ = tell.try_send(passed);
        });
        told
    }

    /// Asked without a prompt: an action polkit does not have is an
    /// error, and so is a bus or a polkit that is not there.
    async fn ask_offered() -> zbus::Result<bool> {
        let (authorized, challenge) = check("", 0).await?;
        Ok(offered(authorized, challenge))
    }

    /// CheckAuthorization of footplate's action for footplate itself:
    /// whether the person passed, and whether a prompt would ask them.
    /// With AllowUserInteraction it answers once the prompt is done with,
    /// passed, failed or cancelled.
    async fn check(reason: &str, flags: u32) -> zbus::Result<(bool, bool)> {
        let me = subject()?;
        let connection = zbus::Connection::system().await?;
        let mut kind = HashMap::new();
        kind.insert("pid", Value::from(me.pid));
        kind.insert("start-time", Value::from(me.start_time));
        // polkit reads the uid as int32.
        kind.insert("uid", Value::from(me.uid as i32));
        let details: HashMap<&str, String> = details(reason, me.uid).into_iter().collect();
        let reply = connection
            .call_method(
                Some("org.freedesktop.PolicyKit1"),
                "/org/freedesktop/PolicyKit1/Authority",
                Some("org.freedesktop.PolicyKit1.Authority"),
                "CheckAuthorization",
                &(("unix-process", kind), ACTION, details, flags, ""),
            )
            .await?;
        let (authorized, challenge, _): (bool, bool, HashMap<String, String>) =
            reply.body().deserialize()?;
        Ok((authorized, challenge))
    }

    fn subject() -> zbus::Result<Subject> {
        let stat = std::fs::read_to_string("/proc/self/stat")
            .map_err(|e| zbus::Error::Failure(e.to_string()))?;
        let start_time = start_time(&stat)
            .ok_or_else(|| zbus::Error::Failure("no start time in /proc/self/stat".into()))?;
        // SAFETY: getuid takes nothing and cannot fail.
        let uid = unsafe { libc::getuid() };
        Ok(Subject {
            pid: std::process::id(),
            start_time,
            uid,
        })
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::Bool;
    use objc2::{AllocAnyThread, msg_send};
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};

    /// Whether the person can pass the device owner's policy: Touch ID,
    /// a paired watch or the login password, whichever the Mac offers.
    pub(super) fn can_evaluate() -> bool {
        let context: Retained<LAContext> = context();
        // SAFETY: the context is valid and the policy is one the
        // framework names; the call throws none.
        unsafe {
            context
                .canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthentication)
                .is_ok()
        }
    }

    pub(super) fn evaluate(reason: &str) -> async_channel::Receiver<bool> {
        let (tell, told) = async_channel::bounded(1);
        let context: Retained<LAContext> = context();
        // Held until the answer: without a strong reference the context
        // is released and the evaluation cancelled with it.
        let held = context.clone();
        let answered = RcBlock::new(move |passed: Bool, _error: *mut NSError| {
            let _ = &held;
            let _ = tell.try_send(passed.as_bool());
        });
        // SAFETY: the reason is checked non-empty by the caller, and the
        // block is sendable: it holds a channel's sender and a retained
        // context alone, and the framework copies it for its own queue.
        unsafe {
            context.evaluatePolicy_localizedReason_reply(
                LAPolicy::DeviceOwnerAuthentication,
                &NSString::from_str(reason),
                &answered,
            )
        };
        told
    }

    fn context() -> Retained<LAContext> {
        // SAFETY: NSObject's init, on a context just allocated.
        unsafe { msg_send![LAContext::alloc(), init] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_s_prompt_names_what_is_approved_and_in_which_yard() {
        assert_eq!(
            approve_reason("yardr peer allow airy --mail-limit 300", "plantime"),
            "approve yardr peer allow airy --mail-limit 300 in plantime"
        );
        assert_eq!(
            setup_reason("courier", "plantime"),
            "approve the setup of pack courier in plantime"
        );
        assert_eq!(
            give_reason(&["BOOKS_TOKEN".to_string()], "plantime"),
            "give BOOKS_TOKEN in plantime"
        );
        assert_eq!(
            give_reason(
                &[
                    "LEDGER_TOKEN".to_string(),
                    "LEDGER_UPDATE_TOKEN".to_string()
                ],
                "south"
            ),
            "give LEDGER_TOKEN, LEDGER_UPDATE_TOKEN in south"
        );
        // Never empty: the call throws on an empty reason.
        assert_eq!(approve_reason("", "plantime"), "approve in plantime");
        assert_eq!(
            setup_reason("", "plantime"),
            "approve the setup in plantime"
        );
        assert_eq!(give_reason(&[], "plantime"), "give the secret in plantime");
    }

    #[test]
    fn polkit_is_asked_about_footplate_itself() {
        // A name with spaces and parentheses of its own: the fields are
        // counted from the last parenthesis.
        let stat = "4242 (foot (plate) x) S 1 4242 4242 0 -1 4194560 2711 0 0 0 \
                    12 5 0 0 20 0 9 0 873451 1210000000 9000 18446744073709551615";
        assert_eq!(start_time(stat), Some(873451));
        assert_eq!(start_time("4242 (footplate) S 1 2 3"), None);
        assert_eq!(start_time("no name here"), None);
        assert_eq!(ACTION, "dev.yardr.footplate.approve");
    }

    #[test]
    fn polkit_takes_the_reason_only_from_a_trusted_caller() {
        let reason = approve_reason("yardr peer allow airy", "plantime");
        // A person's footplate passes no details: polkit fails the check
        // otherwise, and its prompt says the action file's message.
        assert_eq!(details(&reason, 501), vec![]);
        assert_eq!(
            details(&reason, 0),
            vec![(
                "polkit.message",
                "approve yardr peer allow airy in plantime".to_string()
            )]
        );
    }

    #[test]
    fn polkit_offers_the_gate_where_it_passes_or_would_prompt() {
        assert!(offered(false, true));
        assert!(offered(true, false));
        // An inactive session, which the action refuses outright.
        assert!(!offered(false, false));
    }

    /// Needs Linux with polkit on the system bus, so ignored: run by hand
    /// as `FOOTPLATE_POLKIT=<answer> cargo test -- --ignored
    /// polkit_answers`, where the answer is what the system is set to
    /// give: `absent` (no action installed), `pass` or `refuse` (the
    /// action installed, and a rule of polkit's saying yes or no to it).
    /// A prompt of the desktop's is no part of it: that is tried by hand.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn polkit_answers() {
        let expected = std::env::var("FOOTPLATE_POLKIT").expect("FOOTPLATE_POLKIT is set");
        let passed = async_io::block_on(evaluate("approve in plantime").recv()).unwrap();
        match expected.as_str() {
            "absent" | "refuse" => {
                assert!(!available());
                assert!(!passed);
            }
            "pass" => {
                assert!(available());
                assert!(passed);
            }
            other => panic!("FOOTPLATE_POLKIT={other}: absent, pass or refuse"),
        }
    }
}
