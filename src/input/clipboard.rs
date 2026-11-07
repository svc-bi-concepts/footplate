//! The Mac's image pasteboard, including TIFF screenshots and Finder's
//! file URLs. Images enter the same Pasted event as gpui's clipboard.

use std::io::Read;
use std::path::PathBuf;

use gpui::{Image, ImageFormat};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard};
use objc2_foundation::{NSData, NSDictionary, NSString, NSURL};

#[cfg(not(test))]
pub(super) fn image(images: bool) -> Option<Content> {
    read(&NSPasteboard::generalPasteboard(), images)
}

/// Prefer image data to its text alternative. A plain text path is never
/// opened: only a file URL explicitly offered by the pasteboard is a file.
fn read(board: &NSPasteboard, images: bool) -> Option<Content> {
    if !images {
        return None;
    }
    for kind in ["public.png", "public.jpeg", "public.tiff"] {
        if let Some(data) = board.dataForType(&NSString::from_str(kind)) {
            return Some(Content::Bytes(data.to_vec()));
        }
    }
    let text = board.stringForType(&NSString::from_str("public.file-url"))?;
    let url = NSURL::URLWithString(&text)?;
    if !url.isFileURL() {
        return None;
    }
    let path = url.path()?.to_string();
    let mut file = std::fs::File::open(&path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    // Finder can copy any file. Read its signature before bringing a
    // potentially large non-image into the line's foreground thread.
    let mut head = [0; 8];
    file.read_exact(&mut head).ok()?;
    if crate::model::image_type(&head).is_none() && !is_tiff(&head) {
        return None;
    }
    Some(Content::File(PathBuf::from(path)))
}

#[derive(Clone)]
pub(crate) enum Content {
    Bytes(Vec<u8>),
    File(PathBuf),
}

impl Content {
    pub(crate) fn read(self, cap: Option<u64>) -> Option<crate::draft::Found> {
        let mut found = match self {
            Self::Bytes(bytes) => crate::draft::pasted(bytes),
            Self::File(path) => crate::draft::dropped(&path, cap, u64::MAX, 0).pop()?,
        };
        if found.refused.is_none() {
            let image = from_bytes(std::mem::take(&mut found.bytes))?;
            found.bytes = image.bytes().to_vec();
            found.size = found.bytes.len() as u64;
            found.refused = cap
                .and_then(|cap| crate::model::over_cap_for(&found.bytes, found.bytes.len(), cap));
        }
        Some(found)
    }
}

fn is_tiff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*")
}

fn from_bytes(bytes: Vec<u8>) -> Option<Image> {
    let format = match crate::model::image_type(&bytes) {
        Some("image/png") => ImageFormat::Png,
        Some("image/jpeg") => ImageFormat::Jpeg,
        // The yard takes TIFF as a file, not an image. AppKit converts it
        // without adding a codec dependency to the other platforms.
        _ if is_tiff(&bytes) => {
            let data = NSData::with_bytes(&bytes);
            let bitmap = NSBitmapImageRep::imageRepWithData(&data)?;
            // An empty properties dictionary has no typed values to get wrong.
            let png = unsafe {
                bitmap.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::new(),
                )
            }?;
            return Some(Image::from_bytes(ImageFormat::Png, png.to_vec()));
        }
        _ => return None,
    };
    Some(Image::from_bytes(format, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_foundation::NSArray;

    fn board(kind: &str, bytes: &[u8]) -> objc2::rc::Retained<NSPasteboard> {
        let board = NSPasteboard::pasteboardWithUniqueName();
        let kind = NSString::from_str(kind);
        let text = NSString::from_str("public.utf8-plain-text");
        // No owner: these tests supply all data immediately.
        unsafe {
            board.declareTypes_owner(&NSArray::from_slice(&[&*kind, &*text]), None);
        }
        assert!(board.setData_forType(Some(&NSData::with_bytes(bytes)), &kind));
        board
    }

    #[test]
    fn screenshots_are_images_even_with_a_text_alternative() {
        let png = include_bytes!("../fixtures/shot-page.png");
        let board = board("public.png", png);
        board.setString_forType(
            &NSString::from_str("screenshot"),
            &NSString::from_str("public.utf8-plain-text"),
        );
        assert!(read(&board, false).is_none());
        assert_eq!(
            read(&board, true)
                .unwrap()
                .read(Some(crate::api::BLOB_MAX_BYTES))
                .unwrap()
                .bytes,
            png
        );
        board.clearContents();
    }

    #[test]
    fn a_tiff_screenshot_becomes_a_png() {
        let data = NSData::with_bytes(include_bytes!("../fixtures/shot-page.png"));
        let bitmap = NSBitmapImageRep::imageRepWithData(&data).unwrap();
        let tiff = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::TIFF,
                &NSDictionary::new(),
            )
        }
        .unwrap();
        let board = board("public.tiff", &tiff.to_vec());
        let pasted = read(&board, true)
            .unwrap()
            .read(Some(crate::api::BLOB_MAX_BYTES))
            .unwrap();
        assert_eq!(crate::model::image_type(&pasted.bytes), Some("image/png"));
        let decoded = image::load_from_memory(&pasted.bytes).unwrap();
        let original = image::load_from_memory(&data.to_vec()).unwrap();
        assert_eq!(decoded.to_rgba8(), original.to_rgba8());
        board.clearContents();
    }

    #[test]
    fn a_finder_png_is_read_from_its_file_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a screenshot #1.png");
        let png = include_bytes!("../fixtures/shot-page.png");
        std::fs::write(&path, png).unwrap();
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str().unwrap()));
        let board = board(
            "public.file-url",
            url.absoluteString().unwrap().to_string().as_bytes(),
        );
        board.setString_forType(
            &NSString::from_str("a screenshot #1.png"),
            &NSString::from_str("public.utf8-plain-text"),
        );
        assert_eq!(
            read(&board, true)
                .unwrap()
                .read(Some(crate::api::BLOB_MAX_BYTES))
                .unwrap()
                .bytes,
            png
        );
        std::fs::write(&path, b"This is a text file").unwrap();
        assert!(read(&board, true).is_none());
        std::fs::remove_file(&path).unwrap();
        assert!(read(&board, true).is_none());
        board.clearContents();
    }

    #[test]
    fn finder_images_over_the_cap_are_refused_without_buffering() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.png");
        let png = include_bytes!("../fixtures/shot-page.png");
        std::fs::write(&path, png).unwrap();
        let found = Content::File(path).read(Some(1)).unwrap();
        assert!(found.bytes.is_empty());
        assert_eq!(found.size, png.len() as u64);
        assert!(found.refused.unwrap().contains("at most"));
    }

    #[test]
    fn unknown_limits_do_not_refuse_native_images() {
        let mut png = include_bytes!("../fixtures/shot-page.png").to_vec();
        png.resize(crate::api::BLOB_MAX_BYTES as usize + 1, 0);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.png");
        std::fs::write(&path, &png).unwrap();
        for content in [Content::Bytes(png.clone()), Content::File(path)] {
            let found = content.read(None).unwrap();
            assert_eq!(found.refused, None);
            assert_eq!(found.bytes, png);
        }
    }

    #[test]
    fn text_and_non_image_files_are_left_to_gpui() {
        let board = board("public.utf8-plain-text", b"/tmp/a.png");
        assert!(read(&board, true).is_none());
        board.clearContents();
        assert!(from_bytes(b"some text".to_vec()).is_none());
        assert!(from_bytes(b"II*\0broken".to_vec()).is_none());
    }
}
