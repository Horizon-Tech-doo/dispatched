//! Opening a web page and showing a file in the file manager. On macOS through `NSWorkspace`, which works
//! inside the Mac App Store sandbox (running `/usr/bin/open` from there is not something to rely on);
//! elsewhere through the platform's usual program.

/// Open `url` (already checked to be `https`) in the default browser.
pub fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::{NSString, NSURL};
        let parsed = NSURL::URLWithString(&NSString::from_str(url))
            .ok_or_else(|| std::io::Error::other("not a valid address"))?;
        if NSWorkspace::sharedWorkspace().openURL(&parsed) {
            Ok(())
        } else {
            Err(std::io::Error::other("the system did not open it"))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        open::that(url)
    }
}

/// Select `file` in Finder. Only macOS goes through here; the other platforms run their file manager.
#[cfg(target_os = "macos")]
pub fn reveal_file(file: &std::path::Path) {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSArray, NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
    NSWorkspace::sharedWorkspace()
        .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
}
