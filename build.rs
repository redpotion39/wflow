use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/app_icon.ico");
    println!("cargo:rerun-if-changed=assets/tray_icon_rec.ico");
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(windows)]
    {
        let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
        let res_path = out_dir.join("resource.res");
        let rc_path = out_dir.join("app.rc");

        // Use absolute paths for icons so rc.exe always finds them
        let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        let app_icon_path = format!("{}\\assets\\app_icon.ico", manifest_dir).replace('\\', "\\\\");
        let rec_icon_path = format!("{}\\assets\\tray_icon_rec.ico", manifest_dir).replace('\\', "\\\\");

        let rc_content = format!(
            r#"
1 ICON "{}"
2 ICON "{}"
1 VERSIONINFO
FILEVERSION 0,1,0,0
PRODUCTVERSION 0,1,0,0
FILEFLAGSMASK 0x3f
FILEFLAGS 0x0
FILEOS 0x40004
FILETYPE 0x1
FILESUBTYPE 0x0
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "FileDescription", "Wispr Flow Rust Edition - AI Voice Studio"
            VALUE "FileVersion", "0.1.0"
            VALUE "ProductName", "Wispr Flow"
            VALUE "ProductVersion", "0.1.0"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#,
            app_icon_path, rec_icon_path
        );

        std::fs::write(&rc_path, rc_content).unwrap();

        // Find rc.exe or fallback
        let rc_candidates = [
            r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\rc.exe",
            r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64\rc.exe",
            "rc.exe",
        ];

        let mut compiled = false;
        for rc in &rc_candidates {
            if let Ok(status) = Command::new(rc)
                .args(&["/fo", res_path.to_str().unwrap(), rc_path.to_str().unwrap()])
                .status()
            {
                if status.success() {
                    compiled = true;
                    println!("cargo:rustc-link-arg={}", res_path.display());
                    break;
                }
            }
        }

        if !compiled {
            // Fallback to winres
            let mut res = winres::WindowsResource::new();
            res.set_icon("assets/app_icon.ico");
            res.set_icon_with_id("assets/tray_icon_rec.ico", "2");
            let _ = res.compile();
        }
    }
}
