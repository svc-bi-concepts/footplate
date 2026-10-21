//! The rail's bits and what leaves the window: the images a mail
//! names, the tooltips under the pointer, the web view's probes, and
//! the calls that post a draft or a note.

use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, FocusHandle, Image, ImageFormat, Window, div, prelude::*};

use super::state::YardIdentity;
use crate::api::{self, BlobFile, BlobRef, Client};
use crate::draft::{self, Attachment};
use crate::model::{self, Target};
use crate::theme::Palette;

/// An image a mail or a note names, by its hash.
pub(crate) enum Picture {
    /// The yard is asked for it.
    Asked,
    Here(Arc<Image>),
    /// Why there is none to show, in a line.
    Not(String),
}

impl Picture {
    /// What the yard answered for the image `hash`.
    pub(crate) fn of(hash: &str, answer: api::Result<Option<Vec<u8>>>) -> Picture {
        let short = hash.get(..8).unwrap_or(hash);
        match answer {
            Ok(Some(bytes)) => {
                match model::image_type(&bytes).and_then(ImageFormat::from_mime_type) {
                    Some(format) => Picture::Here(Arc::new(Image::from_bytes(format, bytes))),
                    None => Picture::Not(format!(
                        "image {short}: neither a PNG nor a JPEG, and not shown"
                    )),
                }
            }
            Ok(None) => Picture::Not(format!("image {short}: the yard no longer has it")),
            Err(err) => Picture::Not(format!("image {short}: {err}")),
        }
    }
}

/// The image shown full size over the window: its hash, its bytes for
/// the line while they are not there yet, and the focus to give back
/// when it shuts.
pub(crate) struct ImageOpen {
    pub(crate) hash: String,
    pub(crate) bytes: u64,
    pub(crate) back: Option<FocusHandle>,
}

/// An image's tooltip: what a click does with it.
pub(crate) struct OpenTip;

impl Render for OpenTip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::now(window, cx);
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(palette.line)
            .bg(palette.ground)
            .text_sm()
            .text_color(palette.text)
            .child("Open")
    }
}

/// Text under the pointer: a rail icon's name and what answers it, a
/// word of the timetable, or several lines of working groups.
pub(crate) struct RailTip(pub(crate) String);

impl Render for RailTip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::now(window, cx);
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(palette.line)
            .bg(palette.ground)
            .text_sm()
            .text_color(palette.text)
            .child(self.0.clone())
    }
}

/// What a probe of the web view or the sandbox origin was asked of: the
/// yard it was asked for, and the address it was asked at. Asked once per
/// yard and address; the answer stands until the yard or the address
/// changes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Probe {
    pub(crate) yard: YardIdentity,
    pub(crate) base: String,
}

/// Whether the web view answers at the shown yard's base address: a page
/// carries Open when it does, and how to start it when it does not.
/// Asked once per yard and address while it answers; a "no" is asked
/// again on a timer while a file's or page's row is drawn. Switching
/// yards asks again.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ViewUp {
    /// No probe has gone for the yard's address.
    Unknown,
    /// A probe is underway for the yard and the address.
    Asked(Probe),
    /// The address answered, or did not.
    Answered(Probe, bool),
}

/// How long a "no" from the web view or the sandbox origin stands before
/// it is asked again: a view started after the first probe is found
/// without a restart.
pub(crate) const PROBE_AGAIN: Duration = Duration::from_secs(10);

/// The sandbox origin's base address for the settings: the run base
/// where one is set, else the web view's host on the sandbox port.
/// None where neither reads as an address.
pub(crate) fn run_address_of(run_base: &str, web_view: &str) -> Option<api::Address> {
    if run_base.trim().is_empty() {
        api::Address::parse_view(web_view)
            .ok()
            .map(|view| view.run_base())
    } else {
        api::Address::parse_run(run_base).ok()
    }
}

/// A draft that did not go: all of it, to be put back, with the attachment
/// that kept it, if it was one, and why.
pub(crate) struct Unsent {
    pub(crate) text: String,
    pub(crate) files: Vec<Attachment>,
    pub(crate) at: Option<u64>,
    pub(crate) why: String,
}

/// A note that did not go: the words and the files, to be put back, with
/// the file that kept it, if it was one, and why.
pub(crate) struct UnsentNote {
    pub(crate) words: String,
    pub(crate) files: Vec<Attachment>,
    pub(crate) at: Option<u64>,
    pub(crate) why: String,
}

/// Sends a draft as one mail from `me`: `PutBlob` for each file in the
/// order attached, then `Mail` with the text and the hashes they answered,
/// each with its base name, `MailPeer` to one of a linked yard. The cap
/// and the count are asked for first, so that a file over the cap is
/// refused in footplate's words and none travels, and more than the letter
/// carries is refused whole. Whether the link carries a file is the yard's
/// to say: it refuses the mail when not, in its words. Answers the files
/// as the yard has them, or the attachment that kept the mail, if it was
/// one, and why.
pub(crate) fn post(
    client: &Client,
    me: &str,
    target: &Target,
    text: &str,
    files: &[Attachment],
) -> Result<Vec<BlobRef>, (Option<u64>, String)> {
    let said = |err: api::Error| (None, err.to_string());
    let mut blobs = Vec::with_capacity(files.len());
    if !files.is_empty() {
        let settings = client.settings().map_err(said)?;
        if files.len() as u64 > settings.files_cap() {
            return Err((None, draft::over_count(files.len(), settings.files_cap())));
        }
        let cap = settings.blob_cap();
        if let Some((file, why)) = files
            .iter()
            .find_map(|a| Some((a, model::over_cap_for(&a.bytes, a.bytes.len(), cap)?)))
        {
            return Err((Some(file.id), why));
        }
        for file in files {
            let blob = client
                .put_blob(&file.bytes)
                .map_err(|err| (Some(file.id), err.to_string()))?;
            blobs.push(blob);
        }
    }
    let carried: Vec<BlobFile> = files
        .iter()
        .zip(&blobs)
        .map(|(file, blob)| BlobFile {
            hash: blob.hash.clone(),
            name: file.name.clone(),
        })
        .collect();
    match target {
        Target::Crew(name) => client.mail(name, text, me, &carried).map_err(said)?,
        Target::Peer { name, peer } => {
            client
                .mail_peer(name, peer, text, me, &carried)
                .map_err(said)?;
        }
    }
    Ok(blobs)
}

/// Sends the words as one note on `bead` from `me`: `PutBlob` for each
/// file in the order attached, then `AddNote` with the text and the hashes
/// they answered, each with its base name. The cap and the count are asked
/// for first, so that a file over the cap is refused in footplate's words
/// and none travels, and more than the note carries is refused whole. A
/// yard from before notes carried files takes the blobs and refuses the
/// note: that is said as what it is. Answers the attachment that kept the
/// note, if it was one, and why.
pub(crate) fn post_note(
    client: &Client,
    me: &str,
    bead: &str,
    words: &str,
    files: &[Attachment],
) -> Result<(), (Option<u64>, String)> {
    let said = |err: api::Error| (None, err.to_string());
    let mut carried = Vec::with_capacity(files.len());
    if !files.is_empty() {
        let settings = client.settings().map_err(said)?;
        if files.len() as u64 > settings.files_cap() {
            return Err((
                None,
                draft::note_over_count(files.len(), settings.files_cap()),
            ));
        }
        let cap = settings.blob_cap();
        if let Some((file, why)) = files
            .iter()
            .find_map(|a| Some((a, model::over_cap_for(&a.bytes, a.bytes.len(), cap)?)))
        {
            return Err((Some(file.id), why));
        }
        for file in files {
            let blob = client
                .put_blob(&file.bytes)
                .map_err(|err| (Some(file.id), err.to_string()))?;
            carried.push(BlobFile {
                hash: blob.hash,
                name: file.name.clone(),
            });
        }
    }
    client.add_note(bead, words, me, &carried).map_err(|err| {
        if model::note_files_too_old(&err) {
            (
                None,
                "this yard's yardr is too old for a note with a file".to_string(),
            )
        } else {
            said(err)
        }
    })?;
    Ok(())
}
