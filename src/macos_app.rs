//! `needledrop install-app`: wraps the binary in `~/Applications/needledrop.app`
//! and links `~/.local/bin/needledrop` to it.
//!
//! A bare binary started from a terminal has no app identity. Control Center
//! shows it anyway, but notch and menu bar utilities that list music apps
//! (Vorssaint, for one) look players up as running apps by bundle and skip
//! anything without one. Running from a bundle gives needledrop a bundle id,
//! a name, and the music app category; `media::register_as_app` does the rest.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub const BUNDLE_ID: &str = "io.github.audiarmadhani.needledrop";

pub fn install() -> Result<()> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let app = home.join("Applications/needledrop.app");
    let binary = app.join("Contents/MacOS/needledrop");
    let current = std::env::current_exe()?.canonicalize()?;

    fs::create_dir_all(binary.parent().expect("has a parent"))?;
    // Re-running from the installed copy just refreshes the bundle around it.
    if binary.canonicalize().ok().as_ref() != Some(&current) {
        // Copy to a temporary name and rename, so a running copy is never half-written.
        let staged = binary.with_extension("new");
        fs::copy(&current, &staged).with_context(|| format!("could not copy {}", current.display()))?;
        fs::rename(&staged, &binary)?;
    }
    fs::write(app.join("Contents/Info.plist"), info_plist())?;
    sign(&app)?;

    let link = home.join(".local/bin/needledrop");
    fs::create_dir_all(link.parent().expect("has a parent"))?;
    replace_link(&link, &binary)?;

    println!("Installed {}", app.display());
    println!("Linked {} → the app's binary", link.display());
    let on_path = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir == link.parent().expect("has a parent")));
    if !on_path {
        println!("Add ~/.local/bin to your PATH to run it as `needledrop`.");
    }
    println!("After rebuilding needledrop, run `needledrop install-app` from the new build to update the app.");
    Ok(())
}

fn info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>{BUNDLE_ID}</string>
    <key>CFBundleName</key>
    <string>needledrop</string>
    <key>CFBundleDisplayName</key>
    <string>needledrop</string>
    <key>CFBundleExecutable</key>
    <string>needledrop</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.music</string>
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
"#,
        version = env!("CARGO_PKG_VERSION"),
    )
}

/// Ad-hoc signs the bundle, which seals the Info.plist to the binary.
fn sign(app: &Path) -> Result<()> {
    let status = Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(app)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("could not run codesign")?;
    if !status.success() {
        bail!("codesign failed to sign {}", app.display());
    }
    Ok(())
}

/// Points `link` at `target`, replacing an old link but never a real file.
fn replace_link(link: &Path, target: &Path) -> Result<()> {
    match fs::symlink_metadata(link) {
        Ok(meta) if meta.file_type().is_symlink() => fs::remove_file(link)?,
        Ok(_) => bail!(
            "{} exists and isn't a link; move it away and run this again",
            link.display()
        ),
        Err(_) => {}
    }
    symlink(target, link).with_context(|| format!("could not link {}", link.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_marks_needledrop_as_a_background_music_app() {
        let plist = info_plist();
        assert!(plist.contains(&format!("<string>{BUNDLE_ID}</string>")));
        assert!(plist.contains("<string>public.app-category.music</string>"));
        assert!(plist.contains("<key>LSUIElement</key>\n    <true/>"));
        assert!(plist.contains(&format!("<string>{}</string>", env!("CARGO_PKG_VERSION"))));
    }

    #[test]
    fn replaces_old_links_but_not_files() {
        let dir = std::env::temp_dir().join(format!("needledrop-link-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (target, link, file) = (dir.join("target"), dir.join("link"), dir.join("file"));
        fs::write(&target, "x").unwrap();
        symlink(dir.join("old"), &link).unwrap();
        replace_link(&link, &target).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), target);
        fs::write(&file, "keep me").unwrap();
        assert!(replace_link(&file, &target).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "keep me");
        fs::remove_dir_all(&dir).unwrap();
    }
}
