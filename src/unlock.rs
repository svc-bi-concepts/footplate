//! The Mac's unlock in front of a weighty approval: Touch ID, a paired
//! watch or the login password, through LocalAuthentication, before the
//! window sends `weighed`. One gate per approval, with no reuse window:
//! a hand pressing through twenty cards stops at each weighty one.
//!
//! The reasons are plain functions, tested on any system. Whether the
//! system offers the gate ([`available`]) and asking it ([`evaluate`]) are
//! the system's: macOS alone, and nowhere else. Where there is no gate
//! the card shows where to approve instead, and no approval goes from
//! this window.

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
/// can pass, nothing elsewhere. Asked where a weighty card is drawn, and
/// where its control goes; it shows nothing of its own.
pub fn available() -> bool {
    #[cfg(target_os = "macos")]
    {
        mac::can_evaluate()
    }
    #[cfg(not(target_os = "macos"))]
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
    #[cfg(not(target_os = "macos"))]
    {
        // No gate: refused, as a cancel is. Unreached, since
        // `available` is false wherever this builds.
        let _ = reason;
        refused()
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
}
