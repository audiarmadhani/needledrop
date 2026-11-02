//! OS media controls: the Now Playing widget, media keys, and headphone
//! buttons (macOS Now Playing, MPRIS on Linux), via souvlaki.
//!
//! On macOS, the controls deliver key presses through the main thread's run
//! loop, which a terminal app doesn't otherwise run. So the main thread owns
//! the controls and pumps the run loop, while the app runs on a worker thread
//! and sends it `Update`s over a channel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::event::Event;

/// What the OS asked the player to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    /// Seek relative to the current position, in seconds.
    SeekBy(f64),
    /// Seek to an absolute position, in seconds.
    SeekTo(f64),
    SetShuffle(bool),
}

/// What the app tells the OS.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Track {
        title: String,
        artist: String,
        album: Option<String>,
        cover_url: Option<String>,
        duration: Option<Duration>,
    },
    Playing {
        position: f64,
    },
    Paused {
        position: f64,
    },
    Stopped,
    Shuffle(bool),
}

/// The app's side of the channel. Sending never blocks or fails loudly:
/// media controls are a nicety, never a reason to stop playback.
#[derive(Clone)]
pub struct Handle(Sender<Update>);

impl Handle {
    pub fn new(sender: Sender<Update>) -> Self {
        Self(sender)
    }

    pub fn send(&self, update: Update) {
        let _ = self.0.send(update);
    }
}

/// Creates the controls and forwards OS commands to the app as events.
/// Returns `None` if the platform refuses (no D-Bus session, for example).
/// Call on the main thread.
pub fn create(events: UnboundedSender<Event>) -> Option<MediaControls> {
    #[cfg(target_os = "macos")]
    register_as_app();
    let config = PlatformConfig {
        dbus_name: "needledrop",
        display_name: "needledrop",
        hwnd: None,
    };
    let mut controls = MediaControls::new(config).ok()?;
    controls
        .attach(move |event| {
            if let Some(command) = command(event) {
                let _ = events.send(Event::Media(command));
            }
        })
        .ok()?;
    Some(controls)
}

fn command(event: MediaControlEvent) -> Option<Command> {
    const SEEK_STEP: f64 = 10.0;
    let signed = |direction, seconds: f64| {
        if direction == SeekDirection::Forward {
            seconds
        } else {
            -seconds
        }
    };
    Some(match event {
        MediaControlEvent::Play => Command::Play,
        MediaControlEvent::Pause => Command::Pause,
        MediaControlEvent::Toggle => Command::Toggle,
        MediaControlEvent::Next => Command::Next,
        MediaControlEvent::Previous => Command::Previous,
        MediaControlEvent::Stop => Command::Stop,
        MediaControlEvent::Seek(direction) => Command::SeekBy(signed(direction, SEEK_STEP)),
        MediaControlEvent::SeekBy(direction, by) => Command::SeekBy(signed(direction, by.as_secs_f64())),
        MediaControlEvent::SetPosition(MediaPosition(at)) => Command::SeekTo(at.as_secs_f64()),
        MediaControlEvent::SetShuffle(on) => Command::SetShuffle(on),
        _ => return None,
    })
}

/// Runs on the main thread until `done` is set: applies updates and lets the
/// OS deliver media key presses.
pub fn run(mut controls: Option<MediaControls>, updates: Receiver<Update>, done: &AtomicBool) {
    while !done.load(Ordering::Relaxed) {
        pump_events(Duration::from_millis(50));
        while let Ok(update) = updates.try_recv() {
            if let Some(controls) = controls.as_mut() {
                apply(controls, update);
            }
        }
    }
}

fn apply(controls: &mut MediaControls, update: Update) {
    let progress = |seconds: f64| Some(MediaPosition(Duration::from_secs_f64(seconds.max(0.0))));
    let _ = match update {
        Update::Track {
            title,
            artist,
            album,
            cover_url,
            duration,
        } => controls.set_metadata(MediaMetadata {
            title: Some(&title),
            artist: Some(&artist),
            album: album.as_deref(),
            cover_url: cover_url.as_deref(),
            duration,
        }),
        Update::Playing { position } => controls.set_playback(MediaPlayback::Playing {
            progress: progress(position),
        }),
        Update::Paused { position } => controls.set_playback(MediaPlayback::Paused {
            progress: progress(position),
        }),
        Update::Stopped => controls.set_playback(MediaPlayback::Stopped),
        Update::Shuffle(on) => controls.set_shuffle(Some(on)),
    };
}

/// Checks the process in with macOS as an (invisible) app.
///
/// A program started from a terminal is not a registered app, so apps that
/// read Now Playing and look players up as running apps (Vorssaint and other
/// notch utilities) can't find it, even though Control Center can. Creating
/// the shared `NSApplication` registers it; the "prohibited" activation policy
/// keeps it out of the Dock and the app switcher and never takes focus. When
/// needledrop runs from its `.app` bundle, those apps then see its name, icon,
/// and music category.
#[cfg(target_os = "macos")]
fn register_as_app() {
    use std::ffi::{c_char, c_void};
    type Id = *mut c_void;
    /// `NSApplicationActivationPolicyProhibited`.
    const PROHIBITED: isize = 2;

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}
    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Id;
        fn objc_msgSend();
    }
    // SAFETY: on the main thread (see `create`). `objc_msgSend` is called
    // through function types matching each method's real signature:
    // `+[NSApplication sharedApplication] -> NSApplication*` and
    // `-[NSApplication setActivationPolicy:(NSInteger)] -> BOOL`.
    unsafe {
        let class = objc_getClass(c"NSApplication".as_ptr());
        if class.is_null() {
            return;
        }
        let shared: unsafe extern "C" fn(Id, Id) -> Id = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let app = shared(class, sel_registerName(c"sharedApplication".as_ptr()));
        if app.is_null() {
            return;
        }
        let set_policy: unsafe extern "C" fn(Id, Id, isize) -> bool =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        set_policy(app, sel_registerName(c"setActivationPolicy:".as_ptr()), PROHIBITED);
    }
}

#[cfg(target_os = "macos")]
fn pump_events(timeout: Duration) {
    use std::ffi::c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source_handled: u8) -> i32;
    }
    // SAFETY: called on the main thread with the default mode constant
    // CoreFoundation exports; the run loop returns after `timeout` at most.
    unsafe {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, timeout.as_secs_f64(), 0);
    }
}

#[cfg(not(target_os = "macos"))]
fn pump_events(timeout: Duration) {
    std::thread::sleep(timeout);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_os_events_to_commands() {
        assert_eq!(command(MediaControlEvent::Toggle), Some(Command::Toggle));
        assert_eq!(
            command(MediaControlEvent::Seek(SeekDirection::Backward)),
            Some(Command::SeekBy(-10.0))
        );
        assert_eq!(
            command(MediaControlEvent::SeekBy(
                SeekDirection::Forward,
                Duration::from_secs(15)
            )),
            Some(Command::SeekBy(15.0))
        );
        assert_eq!(
            command(MediaControlEvent::SetPosition(MediaPosition(Duration::from_secs(42)))),
            Some(Command::SeekTo(42.0))
        );
        assert_eq!(command(MediaControlEvent::Raise), None);
    }
}
