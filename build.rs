fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/app_icon.ico");
        res.set_icon_with_id("assets/tray_icon_rec.ico", "2");
        res.set("ProductName", "Wispr Flow Rust Edition");
        res.set("FileDescription", "High-performance native Rust AI voice dictation");
        res.set("LegalCopyright", "Wispr Flow Rust");
        if let Err(e) = res.compile() {
            eprintln!("Warning: Failed to compile Windows resource: {}", e);
        }
    }
}
