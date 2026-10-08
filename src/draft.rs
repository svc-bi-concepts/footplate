//! What is written and not sent yet: for each conversation the images
//! attached to it, and the text typed in it while another is open. An image
//! that is pasted or dropped is attached here and goes with Enter, in one
//! mail with the text.
//!
//! Nothing here calls the yard or draws: the window (window.rs) does both.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::model::{self, NO_IMAGES_OVER_THE_LINK, Target};

/// What an image from the clipboard is called: it has no name of its own.
const PASTED: &str = "what was pasted";

/// Something pasted or dropped, before it is attached.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// A file's name, or what stands for one.
    pub name: String,
    /// Its bytes. None for a file that was refused before it was read.
    pub bytes: Vec<u8>,
    pub size: u64,
    /// Why it cannot go, if it cannot.
    pub refused: Option<String>,
}

/// An image attached to a draft, or what was refused as one: that stays,
/// marked, until the person removes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    /// Its own among all the drafts of the window.
    pub id: u64,
    pub name: String,
    pub bytes: Vec<u8>,
    pub size: u64,
    /// Why it is marked, if it is.
    pub refused: Option<String>,
}

/// Why `name` is not attached as an image for mail to where what is typed
/// goes, if it is not: by its first bytes `head`, and by its size when the
/// yard's cap is known.
pub fn unattachable(
    target: &Target,
    name: &str,
    head: &[u8],
    size: u64,
    cap: Option<u64>,
) -> Option<String> {
    if matches!(target, Target::Peer { .. }) {
        return Some(NO_IMAGES_OVER_THE_LINK.to_string());
    }
    if model::image_type(head).is_none() {
        return Some(format!(
            "mail carries a PNG or a JPEG, and {name} is neither"
        ));
    }
    cap.and_then(|cap| model::over_cap(size as usize, cap))
}

/// An image from the clipboard. The yard's cap is not known here: the
/// window asks for it and marks the attachment then.
pub fn pasted(bytes: Vec<u8>, target: &Target) -> Found {
    let size = bytes.len() as u64;
    Found {
        name: PASTED.to_string(),
        refused: unattachable(target, PASTED, &bytes, size, None),
        bytes,
        size,
    }
}

/// A file dropped on the window. One that cannot go is refused by its first
/// bytes and its size: the rest of it is not read, and nothing of it leaves
/// the window.
pub fn dropped(path: &Path, target: &Target, cap: Option<u64>) -> Found {
    let name = match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.display().to_string(),
    };
    let read = || -> io::Result<(Vec<u8>, u64, Option<String>)> {
        let mut file = File::open(path)?;
        let size = file.metadata()?.len();
        let mut bytes = Vec::new();
        file.by_ref().take(8).read_to_end(&mut bytes)?;
        let refused = unattachable(target, &name, &bytes, size, cap);
        match refused {
            Some(_) => bytes.clear(),
            None => {
                file.read_to_end(&mut bytes)?;
            }
        }
        Ok((bytes, size, refused))
    };
    let (bytes, size, refused) = read().unwrap_or_else(|err| {
        let why = format!("{name} could not be read: {err}");
        (Vec::new(), 0, Some(why))
    });
    Found {
        name,
        bytes,
        size,
        refused,
    }
}

/// Why Enter sends nothing of a draft, if it does not: empty when there is
/// nothing to send. An image is a mail without text; a marked attachment
/// holds the whole draft back, so that what goes is what the draft shows.
pub fn unsendable(text: &str, attachments: &[Attachment]) -> Option<String> {
    if let Some(marked) = attachments.iter().find(|a| a.refused.is_some()) {
        let n = attachments.iter().filter(|a| a.refused.is_some()).count();
        return Some(match n {
            1 => format!(
                "{} is marked and cannot go: remove it, then send",
                marked.name
            ),
            n => format!("{n} attachments are marked and cannot go: remove them, then send"),
        });
    }
    match model::unsendable(text) {
        Some(why) if why.is_empty() && !attachments.is_empty() => None,
        other => other,
    }
}

/// A text that did not go, before what was typed since.
pub fn rejoin(unsent: String, typed: String) -> String {
    match (unsent.is_empty(), typed.is_empty()) {
        (_, true) => unsent,
        (true, false) => typed,
        (false, false) => format!("{unsent} {typed}"),
    }
}

#[derive(Debug, Default)]
struct Draft {
    text: String,
    attachments: Vec<Attachment>,
}

/// The drafts of the window, by whom the conversation is with.
#[derive(Debug, Default)]
pub struct Drafts {
    of: HashMap<String, Draft>,
    /// The id of the attachment made last.
    last: u64,
}

impl Drafts {
    /// What is attached to the draft for `with`, in the order it came.
    pub fn attachments(&self, with: &str) -> &[Attachment] {
        self.of.get(with).map_or(&[], |d| &d.attachments)
    }

    /// Why attachments of the draft for `with` are marked, each reason once.
    pub fn reasons(&self, with: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for why in self
            .attachments(with)
            .iter()
            .filter_map(|a| a.refused.as_ref())
        {
            if !out.contains(why) {
                out.push(why.clone());
            }
        }
        out
    }

    /// Attaches what was found to the draft for `with`, after what is there.
    pub fn attach(&mut self, with: &str, found: Found) -> u64 {
        self.last += 1;
        let draft = self.of.entry(with.to_string()).or_default();
        draft.attachments.push(Attachment {
            id: self.last,
            name: found.name,
            bytes: found.bytes,
            size: found.size,
            refused: found.refused,
        });
        self.last
    }

    /// Takes the attachment `id` off whichever draft has it.
    pub fn remove(&mut self, id: u64) -> bool {
        self.of.values_mut().any(|d| {
            let before = d.attachments.len();
            d.attachments.retain(|a| a.id != id);
            d.attachments.len() < before
        })
    }

    /// Takes the last attachment off the draft for `with`, and says which.
    pub fn remove_last(&mut self, with: &str) -> Option<u64> {
        let draft = self.of.get_mut(with)?;
        draft.attachments.pop().map(|a| a.id)
    }

    /// Marks the attachment `id`: it cannot go, and why.
    pub fn refuse(&mut self, id: u64, why: &str) {
        let all = self.of.values_mut().flat_map(|d| &mut d.attachments);
        for a in all.filter(|a| a.id == id) {
            a.refused = Some(why.to_string());
        }
    }

    /// The attachments of the draft for `with`, to be sent in this order.
    /// The draft has none from then on.
    pub fn take(&mut self, with: &str) -> Vec<Attachment> {
        self.of
            .get_mut(with)
            .map(|d| std::mem::take(&mut d.attachments))
            .unwrap_or_default()
    }

    /// Attachments that did not go are back in the draft for `with`, before
    /// what was attached since.
    pub fn restore(&mut self, with: &str, mut unsent: Vec<Attachment>) {
        let draft = self.of.entry(with.to_string()).or_default();
        unsent.append(&mut draft.attachments);
        draft.attachments = unsent;
    }

    /// Keeps what is typed to `with` while another conversation is open.
    pub fn put_text(&mut self, with: &str, text: String) {
        self.of.entry(with.to_string()).or_default().text = text;
    }

    /// What was typed to `with`, for the line: the conversation is open.
    pub fn take_text(&mut self, with: &str) -> String {
        self.of
            .get_mut(with)
            .map(|d| std::mem::take(&mut d.text))
            .unwrap_or_default()
    }

    /// A text to `with` that did not go while another conversation is open:
    /// it is back in the draft, before what was typed there since.
    pub fn unsend(&mut self, with: &str, text: String) {
        let draft = self.of.entry(with.to_string()).or_default();
        draft.text = rejoin(text, std::mem::take(&mut draft.text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nthe rest";
    const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0, 16];

    fn crew() -> Target {
        Target::Crew("yardmaster".into())
    }

    fn peer() -> Target {
        Target::Peer {
            name: "clerk".into(),
            peer: "north".into(),
        }
    }

    fn image(bytes: &[u8]) -> Found {
        pasted(bytes.to_vec(), &crew())
    }

    fn ids(drafts: &Drafts, with: &str) -> Vec<u64> {
        drafts.attachments(with).iter().map(|a| a.id).collect()
    }

    #[test]
    fn what_is_attached_stays_in_the_draft_in_the_order_it_came() {
        let mut drafts = Drafts::default();
        assert_eq!(drafts.attachments("yardmaster"), []);
        let one = drafts.attach("yardmaster", image(PNG));
        let two = drafts.attach("yardmaster", image(JPEG));
        assert_eq!(ids(&drafts, "yardmaster"), [one, two]);
        let first = &drafts.attachments("yardmaster")[0];
        assert_eq!(
            (first.name.as_str(), first.bytes.as_slice(), first.size),
            ("what was pasted", PNG, 16)
        );
        assert_eq!(first.refused, None);
        assert_eq!(drafts.reasons("yardmaster"), Vec::<String>::new());
    }

    #[test]
    fn an_attachment_is_removed_by_its_mark_or_as_the_last() {
        let mut drafts = Drafts::default();
        let one = drafts.attach("yardmaster", image(PNG));
        let two = drafts.attach("yardmaster", image(PNG));
        let three = drafts.attach("yardmaster", image(PNG));
        assert!(drafts.remove(two));
        assert_eq!(ids(&drafts, "yardmaster"), [one, three]);
        // One that is gone is not removed again.
        assert!(!drafts.remove(two));
        assert_eq!(drafts.remove_last("yardmaster"), Some(three));
        assert_eq!(drafts.remove_last("yardmaster"), Some(one));
        assert_eq!(drafts.remove_last("yardmaster"), None);
        assert_eq!(drafts.remove_last("nobody"), None);
        // An id is never given twice, whatever was removed.
        assert!(drafts.attach("yardmaster", image(PNG)) > three);
    }

    #[test]
    fn each_conversation_has_its_own_draft() {
        let mut drafts = Drafts::default();
        let one = drafts.attach("yardmaster", image(PNG));
        let two = drafts.attach("assembly", image(JPEG));
        drafts.put_text("yardmaster", "Like this.".into());
        assert_eq!(ids(&drafts, "yardmaster"), [one]);
        assert_eq!(ids(&drafts, "assembly"), [two]);
        assert_eq!(drafts.remove_last("assembly"), Some(two));
        assert_eq!(ids(&drafts, "yardmaster"), [one]);
        // The text is the line's once the conversation is open again.
        assert_eq!(drafts.take_text("assembly"), "");
        assert_eq!(drafts.take_text("yardmaster"), "Like this.");
        assert_eq!(drafts.take_text("yardmaster"), "");
        // Sending one leaves the other.
        drafts.attach("assembly", image(JPEG));
        assert_eq!(drafts.take("yardmaster").len(), 1);
        assert_eq!(drafts.attachments("assembly").len(), 1);
    }

    #[test]
    fn a_draft_is_sent_in_the_order_attached_and_what_did_not_go_is_back_first() {
        let mut drafts = Drafts::default();
        let one = drafts.attach("yardmaster", image(PNG));
        let two = drafts.attach("yardmaster", image(JPEG));
        let sending = drafts.take("yardmaster");
        assert_eq!(sending.iter().map(|a| a.id).collect::<Vec<_>>(), [one, two]);
        assert_eq!(drafts.attachments("yardmaster"), []);
        assert_eq!(drafts.take("yardmaster"), []);

        // It did not go, and another was attached meanwhile.
        let three = drafts.attach("yardmaster", image(PNG));
        drafts.restore("yardmaster", sending);
        assert_eq!(ids(&drafts, "yardmaster"), [one, two, three]);
        // The yard refused the second: it is marked, with the yard's words.
        drafts.refuse(two, "an image is a PNG or a JPEG");
        assert_eq!(
            drafts.reasons("yardmaster"),
            ["an image is a PNG or a JPEG"]
        );
        assert_eq!(
            unsendable("Like this.", drafts.attachments("yardmaster")).as_deref(),
            Some("what was pasted is marked and cannot go: remove it, then send")
        );
        assert!(drafts.remove(two));
        assert_eq!(unsendable("", drafts.attachments("yardmaster")), None);

        // The text of a conversation that is not open is back before what
        // was typed there since.
        drafts.put_text("assembly", "And this.".into());
        drafts.unsend("assembly", "First this.".into());
        assert_eq!(drafts.take_text("assembly"), "First this. And this.");
        assert_eq!(rejoin("a".into(), String::new()), "a");
        assert_eq!(rejoin(String::new(), "b".into()), "b");
    }

    #[test]
    fn enter_sends_text_images_or_both_and_nothing_of_a_marked_draft() {
        let mut drafts = Drafts::default();
        assert_eq!(unsendable("  ", &[]), Some(String::new()));
        assert_eq!(unsendable("hello", &[]), None);
        drafts.attach("yardmaster", image(PNG));
        let attached = drafts.attachments("yardmaster");
        // An image is a mail without text.
        assert_eq!(unsendable("", attached), None);
        assert_eq!(unsendable("Like this.", attached), None);
        let long = unsendable(&"x".repeat(4097), attached).unwrap();
        assert!(long.contains("this is 4097"), "{long}");

        drafts.attach("yardmaster", image(b"II*\0"));
        drafts.attach("yardmaster", image(b"GIF89a"));
        assert_eq!(
            unsendable("Like this.", drafts.attachments("yardmaster")).as_deref(),
            Some("2 attachments are marked and cannot go: remove them, then send")
        );
        // Each reason is said once.
        assert_eq!(
            drafts.reasons("yardmaster"),
            ["mail carries a PNG or a JPEG, and what was pasted is neither"]
        );
    }

    #[test]
    fn an_image_is_for_this_yard_s_crew_a_png_or_a_jpeg_and_under_the_cap() {
        assert_eq!(unattachable(&crew(), "a.png", PNG, 16, None), None);
        assert_eq!(unattachable(&crew(), "a.jpg", JPEG, 6, Some(6)), None);
        assert_eq!(
            unattachable(&peer(), "a.png", PNG, 16, Some(1024)).as_deref(),
            Some("images do not cross the link yet")
        );
        assert_eq!(
            unattachable(&crew(), "notes.pdf", b"%PDF-1.7", 900, Some(1024)).as_deref(),
            Some("mail carries a PNG or a JPEG, and notes.pdf is neither")
        );
        assert_eq!(
            unattachable(&crew(), "a.png", PNG, 2048, Some(1024)).as_deref(),
            Some(
                "the image is 2 KiB, and this yard takes one of 1 KiB at most (yardr settings: blob_max_bytes)"
            )
        );
        // What is pasted is marked by its kind and by where it goes.
        assert_eq!(image(PNG).refused, None);
        assert!(image(b"II*\0").refused.is_some());
        assert_eq!(
            pasted(PNG.to_vec(), &peer()).refused.as_deref(),
            Some("images do not cross the link yet")
        );
    }

    #[test]
    fn a_dropped_file_is_read_when_it_can_go_and_refused_in_words_when_not() {
        let dir = tempfile::tempdir().unwrap();
        let at = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };

        let found = dropped(&at("fence.png", PNG), &crew(), Some(1024));
        assert_eq!(
            found,
            Found {
                name: "fence.png".into(),
                bytes: PNG.to_vec(),
                size: 16,
                refused: None
            }
        );
        // The cap could not be asked for: Enter asks again.
        assert_eq!(dropped(&at("fence.jpg", JPEG), &crew(), None).bytes, JPEG);

        // No image by its bytes, whatever its name: refused, and not read.
        let found = dropped(&at("notes.png", b"%PDF-1.7 and more"), &crew(), Some(1024));
        assert_eq!(
            found.refused.as_deref(),
            Some("mail carries a PNG or a JPEG, and notes.png is neither")
        );
        assert_eq!((found.bytes.len(), found.size), (0, 17));
        assert!(
            dropped(&at("empty.png", b""), &crew(), None)
                .refused
                .is_some()
        );

        let found = dropped(&at("big.png", PNG), &crew(), Some(15));
        assert!(
            found
                .refused
                .as_deref()
                .unwrap()
                .starts_with("the image is 16 bytes"),
            "{found:?}"
        );
        assert!(found.bytes.is_empty());

        let found = dropped(&at("fence.png", PNG), &peer(), Some(1024));
        assert_eq!(
            found.refused.as_deref(),
            Some("images do not cross the link yet")
        );

        let found = dropped(&dir.path().join("gone.png"), &crew(), None);
        let why = found.refused.unwrap();
        assert!(why.starts_with("gone.png could not be read: "), "{why}");
        // A folder is no image either.
        assert!(dropped(dir.path(), &crew(), None).refused.is_some());
    }
}
