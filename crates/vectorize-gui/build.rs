fn main() {
    // Embed Windows icon and version info into the executable.
    // Requires the `winresource` crate (build dependency).
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        // Use .ico if available, otherwise skip icon embedding
        let ico_path = std::path::Path::new("assets/vectorize.ico");
        if ico_path.exists() {
            res.set_icon(ico_path.to_str().unwrap());
        }
        res.set("ProductName", "Vectorize");
        res.set("FileDescription", "High-quality raster-to-vector conversion");
        res.set("LegalCopyright", "Copyright © 2026");
        res.set_version_info(winresource::VersionInfo::PRODUCTVERSION, 0x0001000000000000);
        if let Err(e) = res.compile() {
            eprintln!("winresource warning: {e}");
        }
    }
}
