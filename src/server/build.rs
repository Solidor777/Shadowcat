//! Embeds the application icon (`assets/icon.ico`) into the Windows executable's
//! Win32 resource section. Every other target is a no-op, so the Linux/macOS
//! binaries are byte-for-byte unaffected.
//!
//! The `#[cfg(windows)]` gate reflects the build *host* (build scripts run on
//! the host); it matches the host-scoped `winresource` build-dependency, so the
//! crate is neither compiled nor referenced off Windows. The inner
//! `CARGO_CFG_TARGET_OS` check reflects the build *target*, skipping the rare
//! Windows-host → non-Windows-target cross-compile.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon.ico");

    #[cfg(windows)]
    {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
            let mut res = winresource::WindowsResource::new();
            res.set_icon("../../assets/icon.ico");
            if let Err(e) = res.compile() {
                // Don't fail the build over the icon; warn so it is visible.
                println!("cargo:warning=failed to embed Windows icon: {e}");
            }
        }
    }
}
