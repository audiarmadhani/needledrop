//! `needledrop install-app`: wraps the binary in `~/Applications/needledrop.app`
//! and links `~/.local/bin/needledrop` to it. Once installed, any other copy
//! of needledrop (Homebrew's, a fresh `cargo build`) hands off to the app at
//! launch, updating the app's copy first, so the app is always what runs.
//!
//! A bare binary started from a terminal has no app identity. Control Center
//! shows it anyway, but notch and menu bar utilities that list music apps
//! (Vorssaint, for one) look players up as running apps by bundle and skip
//! anything without one. Running from a bundle gives needledrop a bundle id,
//! a name, and the music app category; `media::register_as_app` does the rest.

use std::fs;
use std::os::unix::fs::symlink;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub const BUNDLE_ID: &str = "io.github.audiarmadhani.needledrop";

fn app_path() -> Result<PathBuf> {
    Ok(home()?.join("Applications/needledrop.app"))
}

fn home() -> Result<PathBuf> {
    Ok(PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?))
}

pub fn install() -> Result<()> {
    let home = home()?;
    let app = app_path()?;
    let binary = app.join("Contents/MacOS/needledrop");
    let current = std::env::current_exe()?.canonicalize()?;

    fs::create_dir_all(binary.parent().expect("has a parent"))?;
    // Re-running from the installed copy just refreshes the bundle around it.
    if binary.canonicalize().ok().as_ref() != Some(&current) {
        copy_binary(&current, &binary, &app)?;
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
    println!(
        "From now on, any other copy of needledrop you run (Homebrew's, a new build) updates the app and runs from it."
    );
    Ok(())
}

/// If the app is installed and this isn't it, updates the app's copy of the
/// binary when it differs from this one, then replaces this process with the
/// app's. Returns only when there's nothing to do (no app installed, or we
/// already run from inside it).
pub fn run_from_app() -> Result<()> {
    let current = std::env::current_exe()?.canonicalize()?;
    if inside_app_bundle(&current) {
        return Ok(());
    }
    let app = app_path()?;
    let binary = app.join("Contents/MacOS/needledrop");
    if !binary.is_file() {
        return Ok(());
    }
    if fingerprint(&current)? != fs::read_to_string(fingerprint_path(&app)).unwrap_or_default() {
        copy_binary(&current, &binary, &app)?;
        sign(&app)?;
    }
    let error = Command::new(&binary).args(std::env::args_os().skip(1)).exec();
    Err(error).with_context(|| format!("could not start {}", binary.display()))
}

fn inside_app_bundle(path: &Path) -> bool {
    path.ancestors()
        .any(|dir| dir.extension().is_some_and(|ext| ext == "app"))
}

/// Where the bundle records which binary it was made from. Signing rewrites
/// the app's copy, so comparing the two binaries directly would never match.
fn fingerprint_path(app: &Path) -> PathBuf {
    app.join("Contents/Resources/source-sha1")
}

fn fingerprint(binary: &Path) -> Result<String> {
    use sha1::{Digest, Sha1};
    let bytes = fs::read(binary).with_context(|| format!("could not read {}", binary.display()))?;
    Ok(Sha1::digest(bytes).iter().map(|b| format!("{b:02x}")).collect())
}

/// Copies to a temporary name and renames, so a running copy is never
/// half-written, then records the source's fingerprint.
fn copy_binary(from: &Path, to: &Path, app: &Path) -> Result<()> {
    let staged = to.with_extension("new");
    fs::copy(from, &staged).with_context(|| format!("could not copy {}", from.display()))?;
    fs::rename(&staged, to)?;
    let record = fingerprint_path(app);
    fs::create_dir_all(record.parent().expect("has a parent"))?;
    fs::write(record, fingerprint(from)?)?;
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
    fn knows_when_it_runs_from_a_bundle() {
        assert!(inside_app_bundle(Path::new(
            "/Users/me/Applications/needledrop.app/Contents/MacOS/needledrop"
        )));
        assert!(!inside_app_bundle(Path::new(
            "/opt/homebrew/Cellar/needledrop/0.1.0/bin/needledrop"
        )));
    }

    #[test]
    fn copy_records_the_source_fingerprint() {
        let dir = std::env::temp_dir().join(format!("needledrop-copy-test-{}", std::process::id()));
        let app = dir.join("needledrop.app");
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let (source, copy) = (dir.join("needledrop"), app.join("Contents/MacOS/needledrop"));
        fs::write(&source, "binary v1").unwrap();
        copy_binary(&source, &copy, &app).unwrap();
        // Signing changes the copy; the record still identifies the source.
        fs::write(&copy, "binary v1 + signature").unwrap();
        let recorded = fs::read_to_string(fingerprint_path(&app)).unwrap();
        assert_eq!(recorded, fingerprint(&source).unwrap());
        fs::write(&source, "binary v2").unwrap();
        assert_ne!(recorded, fingerprint(&source).unwrap());
        fs::remove_dir_all(&dir).unwrap();
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
