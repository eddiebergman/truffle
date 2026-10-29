//! Explicit native clipboard writes. No clipboard reads and no filesystem-path IPC.
#[cfg(target_os = "macos")]
pub fn text(value: &str) -> Result<(), String> {
    use objc2_app_kit::NSPasteboard;
    let board = NSPasteboard::generalPasteboard();
    text_to(&board, value)
}
#[cfg(target_os = "macos")]
fn text_to(board: &objc2_app_kit::NSPasteboard, value: &str) -> Result<(), String> {
    use objc2_app_kit::NSPasteboardTypeString;
    use objc2_foundation::NSString;
    board.clearContents();
    if board.setString_forType(&NSString::from_str(value), unsafe {
        NSPasteboardTypeString
    }) {
        Ok(())
    } else {
        Err("Could not write text to the clipboard.".into())
    }
}
#[cfg(target_os = "macos")]
pub fn image(bytes: &[u8]) -> Result<(), String> {
    image_to(&objc2_app_kit::NSPasteboard::generalPasteboard(), bytes)
}
#[cfg(target_os = "macos")]
fn image_to(board: &objc2_app_kit::NSPasteboard, bytes: &[u8]) -> Result<(), String> {
    use objc2::{runtime::ProtocolObject, AnyThread};
    use objc2_app_kit::{NSImage, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSData};
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes))
        .ok_or("This image could not be decoded for the clipboard.")?;
    let size = image.size();
    if size.width <= 0.0
        || size.height <= 0.0
        || !size.width.is_finite()
        || !size.height.is_finite()
        || size.width * size.height > 32_000_000.0
    {
        return Err("Image is too large to copy.".into());
    }
    let writer: &ProtocolObject<dyn NSPasteboardWriting> = ProtocolObject::from_ref(&*image);
    let objects = NSArray::from_slice(&[writer]);
    board.clearContents();
    if board.writeObjects(&objects) {
        Ok(())
    } else {
        Err("Could not write the image to the clipboard.".into())
    }
}
#[cfg(not(target_os = "macos"))]
pub fn text(_: &str) -> Result<(), String> {
    Err("Native clipboard support is currently available on macOS.".into())
}
#[cfg(not(target_os = "macos"))]
pub fn image(_: &[u8]) -> Result<(), String> {
    Err("Native image clipboard support is currently available on macOS.".into())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString, NSPasteboardTypeTIFF};
    #[test]
    #[ignore = "Needs the macOS pasteboard service; run explicitly outside the filesystem sandbox."]
    fn private_clipboard_roundtrip_keeps_text_and_copies_image_data_instead_of_paths() {
        objc2::rc::autoreleasepool(|_| {
            // A unique named pasteboard never reads or replaces the user's clipboard.
            let board = NSPasteboard::pasteboardWithUniqueName();
            text_to(&board, "a,b\nα,2\n").unwrap();
            assert_eq!(
                board
                    .stringForType(unsafe { NSPasteboardTypeString })
                    .unwrap()
                    .to_string(),
                "a,b\nα,2\n"
            );
            assert!(image_to(&board, b"not image bytes").is_err());
            assert_eq!(
                board
                    .stringForType(unsafe { NSPasteboardTypeString })
                    .unwrap()
                    .to_string(),
                "a,b\nα,2\n"
            );
            image_to(&board, include_bytes!("../icons/icon.png")).unwrap();
            let image = board
                .dataForType(unsafe { NSPasteboardTypeTIFF })
                .expect("Image clipboard must contain bitmap bytes");
            assert!(image.len() > 100);
            assert!(board
                .stringForType(unsafe { NSPasteboardTypeString })
                .is_none());
        });
    }
}
