use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn find_rc_exe() -> Option<PathBuf> {
    if let Ok(path_var) = env::var("PATH") {
        for dir in env::split_paths(&path_var) {
            let candidate = dir.join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    let kits_bin = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    if let Ok(entries) = fs::read_dir(kits_bin) {
        let mut candidates: Vec<PathBuf> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path().join("x64").join("rc.exe");
                if path.is_file() {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();
        candidates.sort();
        if let Some(latest) = candidates.pop() {
            return Some(latest);
        }
    }

    None
}

fn parse_version_quad(version: &str) -> (u16, u16, u16, u16) {
    let mut parts = version
        .split(['.', '-', '+'])
        .filter_map(|p| p.parse::<u16>().ok());
    (
        parts.next().unwrap_or(1),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../hybrid-player/assets/icons/icon.ico");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }

    let Ok(manifest_dir) = env::var("CARGO_MANIFEST_DIR") else {
        return;
    };
    let Ok(out_dir) = env::var("OUT_DIR") else {
        return;
    };

    let icon_path = Path::new(&manifest_dir)
        .join("../../hybrid-player/assets/icons/icon.ico")
        .canonicalize()
        .unwrap_or_else(|_| {
            Path::new(&manifest_dir).join("../../hybrid-player/assets/icons/icon.ico")
        });
    if !icon_path.is_file() {
        println!(
            "cargo:warning=hybrid-app icon.ico not found at {}",
            icon_path.display()
        );
        return;
    }

    let Some(rc_exe) = find_rc_exe() else {
        println!("cargo:warning=Windows SDK rc.exe not found; skipping EXE icon/version resource");
        return;
    };

    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "12.6.0".to_string());
    let (v1, v2, v3, v4) = parse_version_quad(&version);
    let version_dot = format!("{v1}.{v2}.{v3}.{v4}");
    let icon_rc = icon_path
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('\\', "/");

    let rc_content = format!(
        r#"1 ICON "{icon_rc}"

1 VERSIONINFO
FILEVERSION {v1},{v2},{v3},{v4}
PRODUCTVERSION {v1},{v2},{v3},{v4}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "CompanyName", "Hybrid Player"
            VALUE "FileDescription", "Hybrid Player"
            VALUE "FileVersion", "{version_dot}"
            VALUE "InternalName", "hybrid-app"
            VALUE "OriginalFilename", "hybrid-app.exe"
            VALUE "ProductName", "Hybrid Player"
            VALUE "ProductVersion", "{version_dot}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"#
    );

    let out_path = Path::new(&out_dir);
    let rc_file = out_path.join("hybrid-app.rc");
    let res_file = out_path.join("hybrid-app.res");

    if let Err(err) = fs::write(&rc_file, rc_content) {
        println!("cargo:warning=failed writing {}: {err}", rc_file.display());
        return;
    }

    let status = Command::new(&rc_exe)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res_file)
        .arg(&rc_file)
        .status();

    match status {
        Ok(st) if st.success() && res_file.is_file() => {
            println!("cargo:rustc-link-arg-bins={}", res_file.display());
        }
        Ok(st) => {
            println!(
                "cargo:warning=rc.exe exited with status {} while compiling {}",
                st,
                rc_file.display()
            );
        }
        Err(err) => {
            println!("cargo:warning=failed running rc.exe: {err}");
        }
    }
}
