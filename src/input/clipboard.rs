//! The Mac's image pasteboard, including TIFF screenshots and Finder's
//! file URLs. Images enter the same Pasted event as gpui's clipboard.

use std::io::{Read, Seek};

use gpui::{Image, ImageFormat};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard};
use objc2_foundation::{NSData, NSDictionary, NSString, NSURL};

#[cfg(not(test))]
pub(super) fn image() -> Option<Image> {
    read(&NSPasteboard::generalPasteboard())
}

/// Prefer image data to its text alternative. A plain text path is never
/// opened: only a file URL explicitly offered by the pasteboard is a file.
fn read(board: &NSPasteboard) -> Option<Image> {
    for kind in ["public.png", "public.jpeg", "public.tiff"] {
        if let Some(data) = board.dataForType(&NSString::from_str(kind))
            && let Some(image) = from_bytes(data.to_vec())
        {
            return Some(image);
        }
    }
    let text = board.stringForType(&NSString::from_str("public.file-url"))?;
    let url = NSURL::URLWithString(&text)?;
    if !url.isFileURL() {
        return None;
    }
    let path = url.path()?.to_string();
    let mut file = std::fs::File::open(path).ok()?;
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
    file.rewind().ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    from_bytes(bytes)
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
        assert_eq!(read(&board).unwrap().bytes(), png);
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
        let pasted = read(&board).unwrap();
        assert_eq!(crate::model::image_type(pasted.bytes()), Some("image/png"));
        let decoded = image::load_from_memory(pasted.bytes()).unwrap();
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
        assert_eq!(read(&board).unwrap().bytes(), png);
        std::fs::write(&path, b"This is a text file").unwrap();
        assert!(read(&board).is_none());
        std::fs::remove_file(&path).unwrap();
        assert!(read(&board).is_none());
        board.clearContents();
    }

    #[test]
    fn text_and_non_image_files_are_left_to_gpui() {
        let board = board("public.utf8-plain-text", b"/tmp/a.png");
        assert!(read(&board).is_none());
        board.clearContents();
        assert!(from_bytes(b"some text".to_vec()).is_none());
        assert!(from_bytes(b"II*\0broken".to_vec()).is_none());
    }
}
