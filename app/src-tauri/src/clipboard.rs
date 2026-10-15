//! The system clipboard, written from Rust only (no clipboard command or permission in the webview).

use crate::error::CommandError;
use std::sync::Mutex;

/// One clipboard for the app's lifetime. On Linux the app serves what it copied for as long as its
/// `Clipboard` lives, so it is kept rather than made per copy; elsewhere this only saves setup.
static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

/// Put a secret (a connection string) on the clipboard, asking clipboard history to skip it where
/// the platform has a way to: macOS marks it concealed (nspasteboard.org convention, honoured by
/// third-party clipboard managers), Windows keeps it out of clipboard history and the cloud
/// clipboard, Linux adds the `x-kde-passwordManagerHint` that desktop clipboard managers honour.
pub fn write_secret_text(text: String) -> Result<(), CommandError> {
    let failed =
        |e: arboard::Error| CommandError::new("io", format!("Couldn't write the clipboard: {e}"));
    let mut slot = CLIPBOARD
        .lock()
        .map_err(|_| CommandError::new("io", "Couldn't write the clipboard: it is busy."))?;
    if slot.is_none() {
        *slot = Some(arboard::Clipboard::new().map_err(failed)?);
    }
    let clipboard = slot.as_mut().expect("clipboard set above");
    let set = clipboard.set();
    #[cfg(target_os = "macos")]
    let set = {
        use arboard::SetExtApple;
        set.exclude_from_history()
    };
    #[cfg(windows)]
    let set = {
        use arboard::SetExtWindows;
        set.exclude_from_history().exclude_from_cloud()
    };
    #[cfg(all(
        unix,
        not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))
    ))]
    let set = {
        use arboard::SetExtLinux;
        set.exclude_from_history()
    };
    set.text(text).map_err(failed)
}
