mod api;
mod app;
mod art;
mod auth;
mod event;
mod lyrics;
#[cfg(target_os = "macos")]
mod macos_app;
mod media;
mod minimal;
mod model;
mod palette;
mod player;
mod retro;
mod streams;
mod theme;
mod ui;
mod visualizer;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, anyhow, bail};
use clap::{ArgGroup, Parser, Subcommand};
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::{self, UnboundedReceiver};

use crate::app::App;
use crate::auth::{Browser, Credentials};
use crate::event::Event;
use crate::player::Player;

/// A good-looking terminal client for YouTube Music.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Sign in using a browser where you're logged in to YouTube Music.
    #[command(group = ArgGroup::new("source").required(true))]
    Login {
        /// Read the login cookies from this browser.
        #[arg(long, value_enum, group = "source")]
        browser: Option<Browser>,
        /// Paste a `Cookie` request header copied from your browser's devtools instead.
        #[arg(long, group = "source")]
        paste: bool,
        /// Which signed-in Google account to use, as numbered in the list `login` shows.
        /// Asked interactively when the browser has more than one.
        #[arg(long)]
        account: Option<u32>,
    },
    /// Forget the saved login.
    Logout,
    /// Install needledrop as ~/Applications/needledrop.app and link ~/.local/bin/needledrop to it,
    /// so notch and menu bar apps (like Vorssaint) recognize it as a music player.
    #[cfg(target_os = "macos")]
    InstallApp,
}

fn main() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    match Cli::parse().command {
        None => run_tui(runtime),
        Some(Command::Login {
            browser,
            paste,
            account,
        }) => runtime.block_on(login(browser, paste, account)),
        #[cfg(target_os = "macos")]
        Some(Command::InstallApp) => macos_app::install(),
        Some(Command::Logout) => {
            if Credentials::delete()? {
                println!("Signed out. Saved login removed.");
            } else {
                println!("You weren't signed in.");
            }
            Ok(())
        }
    }
}

async fn login(browser: Option<Browser>, paste: bool, choice: Option<u32>) -> Result<()> {
    let credentials = if paste {
        println!("Open music.youtube.com, then devtools → Network. Pick any request to music.youtube.com");
        println!("and copy its `Cookie` request header. Paste it here and press Enter:");
        let mut header = String::new();
        std::io::stdin().read_line(&mut header)?;
        Credentials::from_cookie_header(&header)?
    } else {
        let browser = browser.expect("clap requires --browser or --paste");
        println!("Reading YouTube cookies from {browser:?}. macOS may ask for Keychain access.");
        Credentials::from_browser(browser)?
    };

    let accounts = signed_in_accounts(&credentials).await?;
    if accounts.is_empty() {
        bail!(
            "YouTube treated these cookies as signed out. They may be stale: open music.youtube.com in that \
             browser, make sure you're signed in, then run `needledrop login` again."
        );
    }
    let index = match choice {
        Some(n) if (1..=accounts.len() as u32).contains(&n) => n - 1,
        Some(n) => bail!("there is no account {n}; this browser has {}", accounts.len()),
        None if accounts.len() == 1 => 0,
        None => ask_account(&accounts)?,
    };
    let credentials = credentials.with_account(index);
    let path = credentials.save()?;
    println!(
        "Signed in as {}. Login saved to {}",
        accounts[index as usize],
        path.display()
    );
    Ok(())
}

/// The browser's signed-in Google accounts, in `X-Goog-AuthUser` order.
async fn signed_in_accounts(credentials: &Credentials) -> Result<Vec<api::Account>> {
    let mut accounts = Vec::new();
    for index in 0..10 {
        let client = api::Client::new(Some(credentials.with_account(index)))?;
        match client.account().await {
            Ok(Some(account)) => accounts.push(account),
            Ok(None) => break,
            // YouTube answers past the last account with an error (404).
            Err(_) if index > 0 => break,
            Err(error) => return Err(error),
        }
    }
    Ok(accounts)
}

fn ask_account(accounts: &[api::Account]) -> Result<u32> {
    println!("This browser is signed in to {} Google accounts:", accounts.len());
    for (i, account) in accounts.iter().enumerate() {
        println!("  {}. {account}", i + 1);
    }
    loop {
        print!("Which one has your YouTube Music library? [1-{}] ", accounts.len());
        std::io::Write::flush(&mut std::io::stdout())?;
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line)? == 0 {
            bail!("no account chosen; pass --account N");
        }
        match line.trim().parse::<u32>() {
            Ok(n) if (1..=accounts.len() as u32).contains(&n) => return Ok(n - 1),
            _ => println!("Enter a number from 1 to {}.", accounts.len()),
        }
    }
}

/// Runs the TUI on a worker thread, because the main thread has to serve
/// the OS media controls (see `media`).
fn run_tui(runtime: tokio::runtime::Runtime) -> Result<()> {
    let (events, rx) = mpsc::unbounded_channel();
    let (updates, update_rx) = std::sync::mpsc::channel();
    let controls = media::create(events.clone());
    let done = Arc::new(AtomicBool::new(false));
    let worker = {
        let done = SetOnDrop(done.clone());
        std::thread::spawn(move || {
            let _done = done;
            runtime.block_on(tui(events, rx, media::Handle::new(updates)))
        })
    };
    media::run(controls, update_rx, &done);
    worker.join().map_err(|_| anyhow!("needledrop crashed"))?
}

/// Sets the flag when dropped, so the main thread stops even if the TUI panics.
struct SetOnDrop(Arc<AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

async fn tui(
    events: mpsc::UnboundedSender<Event>,
    mut rx: UnboundedReceiver<Event>,
    media: media::Handle,
) -> Result<()> {
    let credentials = Credentials::load()?.map(Credentials::refreshed);
    let api = api::Client::new(credentials)?;
    // Start mpv before taking over the terminal so a startup error prints cleanly.
    let player = Player::spawn(events.clone()).await?;

    let mut terminal = ratatui::init();
    let mut app = App::new(api, player, media, events.clone());
    palette::watch(events.clone());
    spawn_ticker(events.clone());
    event::spawn_input_reader(events);
    let result = run(&mut terminal, &mut app, &mut rx).await;
    ratatui::restore();
    result
}

const FRAME: std::time::Duration = std::time::Duration::from_millis(33);

fn spawn_ticker(events: mpsc::UnboundedSender<Event>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(FRAME);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if events.send(Event::Tick).is_err() {
                break;
            }
        }
    });
}

async fn run(terminal: &mut DefaultTerminal, app: &mut App, rx: &mut UnboundedReceiver<Event>) -> Result<()> {
    let mut redraw = true;
    while !app.should_quit {
        if redraw {
            terminal.draw(|frame| ui::render(app, frame))?;
        }
        let Some(event) = rx.recv().await else { break };
        // Ticks only matter while something is moving; skip idle redraws.
        redraw = !matches!(event, Event::Tick) || app.is_animating();
        app.handle(event).await?;
        // Drain whatever else is queued so bursts (mpv position updates) cost one redraw.
        while let Ok(event) = rx.try_recv() {
            redraw |= !matches!(event, Event::Tick) || app.is_animating();
            app.handle(event).await?;
        }
    }
    Ok(())
}
