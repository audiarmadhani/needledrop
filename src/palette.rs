//! Reads the terminal's color theme from Ghostty's config, so album art can
//! be drawn in the theme's own colors. (The rest of the UI doesn't need
//! this: it uses palette slots, which the terminal colors itself.)
//!
//! Follows Ghostty's rules: `config-file` includes (a leading `?` means
//! optional), `theme = Name` resolved in the user's themes folder and then
//! Ghostty's bundled ones, and `palette = N=#rrggbb` lines overriding the theme.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;

use crate::event::Event;

/// The 16 ANSI colors plus background and foreground, as RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub ansi: [[u8; 3]; 16],
    pub background: [u8; 3],
    pub foreground: [u8; 3],
}

impl Palette {
    /// Every color in the theme, for quantizing art.
    pub fn colors(&self) -> Vec<[u8; 3]> {
        let mut colors = self.ansi.to_vec();
        colors.extend([self.background, self.foreground]);
        colors
    }
}

/// How often to look for a theme switch.
const POLL: Duration = Duration::from_secs(2);

/// Sends the current palette now, then again whenever it changes.
pub fn watch(events: UnboundedSender<Event>) {
    tokio::spawn(async move {
        let mut last = None;
        loop {
            let palette = tokio::task::spawn_blocking(current).await.ok().flatten();
            if palette != last {
                last = palette;
                if events.send(Event::Palette(palette)).is_err() {
                    break;
                }
            }
            tokio::time::sleep(POLL).await;
        }
    });
}

/// The palette of the Ghostty we're running in, or `None` when that's not
/// Ghostty or the theme can't be resolved.
pub fn current() -> Option<Palette> {
    let in_ghostty = std::env::var("TERM_PROGRAM").is_ok_and(|t| t == "ghostty")
        || std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some();
    if !in_ghostty {
        return None;
    }
    let config_dir = config_dir()?;
    let mut settings = Settings::default();
    settings.read_config(&config_dir.join("config"), 0);
    settings.resolve(&theme_dirs(&config_dir))
}

#[derive(Default)]
struct Settings {
    theme: Option<String>,
    /// Explicit `palette`/`background`/`foreground` lines, in order.
    overrides: Vec<(String, String)>,
}

impl Settings {
    fn read_config(&mut self, path: &Path, depth: u8) {
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        for (key, value) in entries(&text) {
            match key {
                "theme" => self.theme = Some(value.to_owned()),
                "config-file" if depth < 4 => {
                    let include = value.trim_start_matches('?').trim_matches('"');
                    let include = path.parent().unwrap_or(Path::new("")).join(include);
                    self.read_config(&include, depth + 1);
                }
                "palette" | "background" | "foreground" => self.overrides.push((key.to_owned(), value.to_owned())),
                _ => {}
            }
        }
    }

    fn resolve(self, theme_dirs: &[PathBuf]) -> Option<Palette> {
        let mut builder = Builder::default();
        if let Some(theme) = &self.theme {
            let name = theme_name(theme);
            let file = theme_dirs
                .iter()
                .map(|dir| dir.join(name))
                .find(|path| path.is_file())?;
            let text = std::fs::read_to_string(file).ok()?;
            for (key, value) in entries(&text) {
                builder.set(key, value);
            }
        }
        for (key, value) in &self.overrides {
            builder.set(key, value);
        }
        builder.build()
    }
}

#[derive(Default)]
struct Builder {
    ansi: [Option<[u8; 3]>; 16],
    background: Option<[u8; 3]>,
    foreground: Option<[u8; 3]>,
}

impl Builder {
    fn set(&mut self, key: &str, value: &str) {
        match key {
            "palette" => {
                let Some((index, hex)) = value.split_once('=') else {
                    return;
                };
                if let (Ok(index), Some(color)) = (index.trim().parse::<usize>(), parse_hex(hex))
                    && let Some(slot) = self.ansi.get_mut(index)
                {
                    *slot = Some(color);
                }
            }
            "background" => self.background = parse_hex(value).or(self.background),
            "foreground" => self.foreground = parse_hex(value).or(self.foreground),
            _ => {}
        }
    }

    fn build(self) -> Option<Palette> {
        let mut ansi = [[0; 3]; 16];
        for (slot, color) in ansi.iter_mut().zip(self.ansi) {
            *slot = color?;
        }
        Some(Palette {
            ansi,
            background: self.background?,
            foreground: self.foreground?,
        })
    }
}

/// `key = value` lines, skipping comments and blanks.
fn entries(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
}

/// `theme = light:A,dark:B` picks per appearance; needledrop takes the dark one.
fn theme_name(value: &str) -> &str {
    let value = value.trim_matches('"');
    if !value.contains(':') {
        return value;
    }
    let pick = |prefix: &str| value.split(',').find_map(|part| part.trim().strip_prefix(prefix));
    pick("dark:").or_else(|| pick("light:")).unwrap_or(value).trim()
}

fn parse_hex(value: &str) -> Option<[u8; 3]> {
    let hex = value.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

fn config_dir() -> Option<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir).join("ghostty")),
        _ => Some(PathBuf::from(std::env::var_os("HOME")?).join(".config/ghostty")),
    }
}

fn theme_dirs(config_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![config_dir.join("themes")];
    if let Some(resources) = std::env::var_os("GHOSTTY_RESOURCES_DIR") {
        dirs.push(PathBuf::from(resources).join("themes"));
    }
    dirs.push(PathBuf::from(
        "/Applications/Ghostty.app/Contents/Resources/ghostty/themes",
    ));
    dirs.push(PathBuf::from("/usr/share/ghostty/themes"));
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme_file(overrides: &[(usize, &str)]) -> String {
        let mut text = String::from("# a theme\nbackground = #282a36\nforeground = #f8f8f2\n");
        for i in 0..16 {
            let hex = overrides.iter().find(|(n, _)| *n == i).map_or("#000000", |(_, h)| h);
            text.push_str(&format!("palette = {i}={hex}\n"));
        }
        text
    }

    #[test]
    fn follows_includes_and_applies_overrides_after_the_theme() {
        let dir = std::env::temp_dir().join(format!("needledrop-palette-test-{}", std::process::id()));
        let themes = dir.join("themes");
        std::fs::create_dir_all(&themes).unwrap();
        std::fs::write(themes.join("Test Theme"), theme_file(&[(5, "#ff79c6")])).unwrap();
        std::fs::write(
            dir.join("config"),
            "font-size = 12\nconfig-file = ?current-theme\nconfig-file = ?missing\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("current-theme"),
            "theme = \"Test Theme\"\npalette = 1=#ff5555\n",
        )
        .unwrap();

        let mut settings = Settings::default();
        settings.read_config(&dir.join("config"), 0);
        let palette = settings.resolve(&[themes]).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(palette.ansi[5], [0xff, 0x79, 0xc6]);
        assert_eq!(palette.ansi[1], [0xff, 0x55, 0x55]);
        assert_eq!(palette.background, [0x28, 0x2a, 0x36]);
    }

    #[test]
    fn missing_theme_means_no_palette() {
        let settings = Settings {
            theme: Some("Nope".into()),
            overrides: vec![],
        };
        assert_eq!(settings.resolve(&[]), None);
    }

    #[test]
    fn picks_the_dark_variant() {
        assert_eq!(theme_name("light:Day,dark:Night"), "Night");
        assert_eq!(theme_name("\"Dracula\""), "Dracula");
    }
}
