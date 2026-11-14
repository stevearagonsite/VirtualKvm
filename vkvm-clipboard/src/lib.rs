//! Text clipboard for VirtualKvm (shared with the peer when the cursor moves over).
//!
//! On macOS, content marked as concealed or transient (password managers mark
//! copied passwords with `org.nspasteboard.ConcealedType`) is never read.

/// current clipboard text, if any (and if it may be shared)
pub fn get_text() -> Option<String> {
    if concealed() {
        log::debug!("clipboard content is concealed, not sharing it");
        return None;
    }
    let text = arboard::Clipboard::new().ok()?.get_text().ok()?;
    (!text.is_empty()).then_some(text)
}

/// replace the clipboard with `text`
pub fn set_text(text: &str) -> bool {
    match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_owned())) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("could not set the clipboard: {e}");
            false
        }
    }
}

#[cfg(target_os = "macos")]
fn concealed() -> bool {
    use objc2_app_kit::NSPasteboard;
    // http://nspasteboard.org: markers used by password managers & co.
    const PRIVATE: [&str; 3] = [
        "org.nspasteboard.ConcealedType",
        "org.nspasteboard.TransientType",
        "com.agilebits.onepassword",
    ];
    let pasteboard = NSPasteboard::generalPasteboard();
    let Some(types) = pasteboard.types() else {
        return false;
    };
    types
        .iter()
        .any(|t| PRIVATE.iter().any(|p| t.to_string() == *p))
}

#[cfg(not(target_os = "macos"))]
fn concealed() -> bool {
    false
}
