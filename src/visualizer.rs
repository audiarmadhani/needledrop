//! A spectrum visualizer that stays in sync with playback.
//!
//! mpv doesn't expose the samples it plays, so we decode the same stream a
//! second time with ffmpeg into memory, then analyze the window at mpv's
//! playback position. Decoding runs far faster than real time (a 5-minute
//! track takes about 2 seconds), and pauses and seeks need no special casing.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::task::AbortHandle;

pub const SAMPLE_RATE: u32 = 22_050;
pub const BANDS: usize = 64;
const FFT_SIZE: usize = 2048;
const MIN_FREQ: f32 = 40.0;
const MAX_FREQ: f32 = 10_000.0;
/// Levels at or below this many dB (relative to a full-scale sine) draw as empty.
const FLOOR_DB: f32 = -65.0;
/// Music has less energy in high frequencies; boost them by up to this many dB.
const TREBLE_TILT_DB: f32 = 18.0;
/// How much of the previous frame's height a bar keeps when falling.
const DECAY: f32 = 0.82;
/// Frames a peak cap hangs at its height before it starts to fall.
const PEAK_HOLD: u8 = 10;
/// How far a peak cap falls per frame (bar heights run 0–1).
const PEAK_FALL: f32 = 0.02;
/// Samples shown by the scope (about 93 ms), and the points they're averaged
/// into. About 43 samples per point keeps roughly the bottom 500 Hz: the
/// smooth, rolling bass line rather than treble zigzag.
const WAVE_SAMPLES: usize = 2048;
const WAVE_POINTS: usize = 48;

type Samples = Arc<Mutex<Vec<i16>>>;

/// The looks `V` cycles through, after Windows Media Player's visualizations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Smooth, wide spectrum bars.
    Bars,
    /// Thin bars with peak caps that hang, then fall (WMP's "Bars").
    Peaks,
    /// An oscilloscope trace (WMP's "Scope").
    Scope,
}

impl Style {
    pub fn next(self) -> Self {
        match self {
            Style::Bars => Style::Peaks,
            Style::Peaks => Style::Scope,
            Style::Scope => Style::Bars,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Style::Bars => "bars",
            Style::Peaks => "peaks",
            Style::Scope => "scope",
        }
    }
}

/// Owns the decoded audio for the current track and the animation state.
pub struct Visualizer {
    pub style: Style,
    analyzer: Analyzer,
    samples: Samples,
    decode: Option<AbortHandle>,
    levels: Vec<f32>,
    peaks: Vec<(f32, u8)>,
    wave: Vec<f32>,
    /// Smoothed auto-gain, so quiet passages still fill the scope.
    gain: f32,
}

impl Visualizer {
    pub fn new() -> Self {
        Self {
            style: Style::Bars,
            analyzer: Analyzer::new(),
            samples: Samples::default(),
            decode: None,
            levels: vec![0.0; BANDS],
            peaks: vec![(0.0, 0); BANDS],
            wave: vec![0.0; WAVE_POINTS],
            gain: 1.0,
        }
    }

    pub fn is_animating(&self) -> bool {
        let moving = |v: &f32| v.abs() > 0.001;
        self.levels.iter().any(moving)
            || self.peaks.iter().any(|(peak, _)| moving(peak))
            || self.wave.iter().any(moving)
    }

    /// Starts decoding a new stream, dropping the previous track's audio.
    pub fn start(&mut self, url: String) {
        self.clear();
        let samples = self.samples.clone();
        let task = tokio::spawn(async move {
            // A failed decode only means no bars; playback is unaffected.
            let _ = decode(&url, &samples).await;
        });
        self.decode = Some(task.abort_handle());
    }

    pub fn clear(&mut self) {
        if let Some(task) = self.decode.take() {
            task.abort();
        }
        // A fresh buffer, so an aborted decode can't append to the next track's.
        self.samples = Samples::default();
    }

    /// Advances the animation one frame. `position` is the playback position
    /// in seconds, or `None` while paused or stopped (bars fall to zero).
    pub fn tick(&mut self, position: Option<f64>) {
        let (target, wave) = match (position, self.samples.lock()) {
            (Some(seconds), Ok(samples)) => (self.analyzer.levels(&samples, seconds), window(&samples, seconds)),
            _ => (None, None),
        };
        for (i, level) in self.levels.iter_mut().enumerate() {
            let target = target.as_ref().map_or(0.0, |t| t[i]);
            *level = target.max(*level * DECAY);
        }
        for ((peak, hold), &level) in self.peaks.iter_mut().zip(&self.levels) {
            if level >= *peak {
                (*peak, *hold) = (level, PEAK_HOLD);
            } else if *hold > 0 {
                *hold -= 1;
            } else {
                *peak = (*peak - PEAK_FALL).max(level);
            }
        }
        self.wave = match wave {
            Some(wave) => {
                let loudest = wave.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                let target_gain = 0.85 / loudest.max(0.02);
                self.gain += (target_gain - self.gain) * 0.1;
                wave.iter().map(|v| (v * self.gain).clamp(-1.0, 1.0)).collect()
            }
            // Paused: let the trace flatten out.
            None => self.wave.iter().map(|v| v * DECAY).collect(),
        };
    }
}

impl Drop for Visualizer {
    fn drop(&mut self) {
        self.clear();
    }
}

/// The audio just before `seconds`, averaged down to `WAVE_POINTS` values in
/// -1..1. Averaging works as a low-pass filter.
fn window(samples: &[i16], seconds: f64) -> Option<Vec<f32>> {
    let end = (seconds.max(0.0) * SAMPLE_RATE as f64) as usize;
    let chunk = samples.get(end.checked_sub(WAVE_SAMPLES)?..end)?;
    let point = |p: usize| {
        let part = &chunk[p * WAVE_SAMPLES / WAVE_POINTS..(p + 1) * WAVE_SAMPLES / WAVE_POINTS];
        part.iter().map(|&s| s as f32).sum::<f32>() / (part.len() as f32 * 32768.0)
    };
    Some((0..WAVE_POINTS).map(point).collect())
}

/// Decodes `url` to mono 16-bit PCM, appending to `samples` as data arrives.
async fn decode(url: &str, samples: &Samples) -> Result<()> {
    let mut ffmpeg = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i", url, "-ac", "1", "-ar"])
        .arg(SAMPLE_RATE.to_string())
        .args(["-f", "s16le", "-"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("could not start ffmpeg")?;
    let mut stdout = ffmpeg.stdout.take().context("ffmpeg has no stdout")?;
    let mut chunk = vec![0u8; 64 * 1024];
    let mut pending = Vec::new();
    loop {
        let read = stdout.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        pending.extend_from_slice(&chunk[..read]);
        let whole = pending.len() / 2 * 2;
        let decoded = pending[..whole].as_chunks::<2>().0.iter().map(|&b| i16::from_le_bytes(b));
        samples.lock().map_err(|_| anyhow::anyhow!("sample buffer poisoned"))?.extend(decoded);
        pending.drain(..whole);
    }
    ffmpeg.wait().await?;
    Ok(())
}

struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// FFT bin range `[start, end)` for each band, log-spaced in frequency.
    bands: Vec<(usize, usize)>,
}

impl Analyzer {
    fn new() -> Self {
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32).cos())
            .collect();
        let bin = |freq: f32| (freq * FFT_SIZE as f32 / SAMPLE_RATE as f32) as usize;
        let edge = |i: usize| MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(i as f32 / BANDS as f32);
        let bands = (0..BANDS)
            .map(|i| {
                let start = bin(edge(i));
                (start, bin(edge(i + 1)).max(start + 1))
            })
            .collect();
        Self { fft: FftPlanner::new().plan_fft_forward(FFT_SIZE), window, bands }
    }

    /// Band levels in `0.0..=1.0` for the audio just before `seconds`, or
    /// `None` if that part hasn't been decoded yet.
    fn levels(&self, samples: &[i16], seconds: f64) -> Option<[f32; BANDS]> {
        let end = (seconds.max(0.0) * SAMPLE_RATE as f64) as usize;
        let start = end.checked_sub(FFT_SIZE)?;
        let window = samples.get(start..end)?;
        let mut buffer: Vec<Complex<f32>> = window
            .iter()
            .zip(&self.window)
            .map(|(&s, &w)| Complex::new(s as f32 / 32768.0 * w, 0.0))
            .collect();
        self.fft.process(&mut buffer);

        // A full-scale sine through a Hann window peaks at N/4.
        let reference = FFT_SIZE as f32 / 4.0;
        let mut levels = [0.0; BANDS];
        for (i, &(lo, hi)) in self.bands.iter().enumerate() {
            let magnitude = buffer[lo..hi].iter().map(|c| c.norm()).fold(0.0, f32::max);
            let db = 20.0 * (magnitude / reference + 1e-9).log10() + TREBLE_TILT_DB * i as f32 / BANDS as f32;
            levels[i] = ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0);
        }
        Some(levels)
    }
}

/// Draws the visualizer in its current style. `colors` run from the
/// bottom of the bars (or the middle of the scope) outward.
pub struct Visual<'a> {
    pub visualizer: &'a Visualizer,
    pub colors: &'a [Color],
    /// Peak caps.
    pub accent: Color,
}

const EIGHTHS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

impl Widget for Visual<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.colors.is_empty() || area.is_empty() {
            return;
        }
        let v = self.visualizer;
        match v.style {
            Style::Bars => draw_bars(&v.levels, None, 2, area, buf, self.colors, self.accent),
            Style::Peaks => {
                let peaks: Vec<f32> = v.peaks.iter().map(|p| p.0).collect();
                draw_bars(&v.levels, Some(&peaks), 1, area, buf, self.colors, self.accent);
            }
            Style::Scope => draw_scope(&v.wave, area, buf, self.colors),
        }
    }
}

/// Bars `bar_width` cells wide with one-cell gaps; each bar shows the
/// loudest band it covers. With `peaks`, a cap marks each bar's recent high.
fn draw_bars(
    levels: &[f32],
    peaks: Option<&[f32]>,
    bar_width: u16,
    area: Rect,
    buf: &mut Buffer,
    colors: &[Color],
    cap: Color,
) {
    const GAP: u16 = 1;
    let bars = (area.width + GAP) / (bar_width + GAP);
    if bars == 0 || levels.is_empty() {
        return;
    }
    let used = bars * (bar_width + GAP) - GAP;
    let left = area.x + (area.width - used) / 2;
    let loudest = |values: &[f32], bar: usize| {
        let from = bar * values.len() / bars as usize;
        let to = ((bar + 1) * values.len() / bars as usize).max(from + 1).min(values.len());
        values[from..to].iter().copied().fold(0.0, f32::max)
    };
    let color_at = |row: u16| colors[row as usize * colors.len() / area.height as usize];
    for bar in 0..bars as usize {
        let x = left + bar as u16 * (bar_width + GAP);
        let mut eighths = (loudest(levels, bar) * area.height as f32 * 8.0).round() as i32;
        for row in 0..area.height {
            if eighths <= 0 {
                break;
            }
            let symbol = EIGHTHS[eighths.min(8) as usize - 1];
            for dx in 0..bar_width {
                buf[(x + dx, area.bottom() - 1 - row)].set_symbol(symbol).set_fg(color_at(row));
            }
            eighths -= 8;
        }
        if let Some(peaks) = peaks {
            let peak = loudest(peaks, bar);
            if peak > 0.02 {
                let row = ((peak * area.height as f32) as u16).min(area.height - 1);
                for dx in 0..bar_width {
                    buf[(x + dx, area.bottom() - 1 - row)].set_symbol("▔").set_fg(cap);
                }
            }
        }
    }
}

/// An oscilloscope trace in braille: each cell is a 2×4 grid of dots, so
/// the line is drawn at four times the vertical resolution of text.
fn draw_scope(wave: &[f32], area: Rect, buf: &mut Buffer, colors: &[Color]) {
    let (width, height) = (area.width as usize, area.height as usize);
    let mut cells = vec![0u8; width * height];
    plot(wave, width, height, &mut cells);
    let middle = (height as f32 - 1.0) / 2.0;
    for row in 0..height {
        // Brighter colors further from the center line, like a glowing trace.
        let distance = ((row as f32 - middle).abs() / (middle + 0.5)).min(0.999);
        let color = colors[(distance * colors.len() as f32) as usize];
        for col in 0..width {
            let dots = cells[row * width + col];
            if dots != 0 {
                let symbol = char::from_u32(0x2800 + dots as u32).expect("braille block").to_string();
                buf[(area.x + col as u16, area.y + row as u16)].set_symbol(&symbol).set_fg(color);
            }
        }
    }
}

/// Plots `wave` (values in -1..1) as a connected line into braille cells.
fn plot(wave: &[f32], width: usize, height: usize, cells: &mut [u8]) {
    // Bit for the dot at (column within cell, row within cell).
    const DOT: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
    let (dots_x, dots_y) = (width * 2, height * 4);
    if wave.is_empty() || dots_x == 0 || dots_y == 0 {
        return;
    }
    let y_at = |x: usize| {
        // Linear interpolation between points, so the line curves instead of stepping.
        let at = x as f32 * (wave.len() - 1) as f32 / (dots_x - 1).max(1) as f32;
        let (i, t) = (at.floor() as usize, at.fract());
        let value = wave[i] + (wave[(i + 1).min(wave.len() - 1)] - wave[i]) * t;
        ((1.0 - (value + 1.0) / 2.0) * (dots_y - 1) as f32).round() as usize
    };
    let mut previous = y_at(0);
    for x in 0..dots_x {
        let y = y_at(x);
        // Fill the vertical gap to the previous point so the line stays connected.
        for dot_y in y.min(previous)..=y.max(previous) {
            cells[(dot_y / 4) * width + x / 2] |= DOT[x % 2][dot_y % 4];
        }
        previous = y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, seconds: f32) -> Vec<i16> {
        (0..(seconds * SAMPLE_RATE as f32) as usize)
            .map(|i| ((2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin() * 16000.0) as i16)
            .collect()
    }

    fn loudest_band(levels: &[f32]) -> usize {
        levels.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0
    }

    #[test]
    fn a_tone_peaks_in_its_own_band() {
        let analyzer = Analyzer::new();
        for freq in [100.0, 440.0, 3000.0] {
            let levels = analyzer.levels(&sine(freq, 1.0), 0.5).unwrap();
            let (lo, hi) = analyzer.bands[loudest_band(&levels)];
            let bin = (freq * FFT_SIZE as f32 / SAMPLE_RATE as f32) as usize;
            assert!((lo.saturating_sub(1)..=hi).contains(&bin), "{freq} Hz peaked in bins {lo}..{hi}");
        }
    }

    #[test]
    fn silence_is_flat_and_undecoded_audio_is_none() {
        let analyzer = Analyzer::new();
        let levels = analyzer.levels(&vec![0; SAMPLE_RATE as usize], 0.5).unwrap();
        assert!(levels.iter().all(|&l| l == 0.0));
        assert_eq!(analyzer.levels(&sine(440.0, 1.0), 5.0), None);
        assert_eq!(analyzer.levels(&sine(440.0, 1.0), 0.01), None);
    }

    #[test]
    fn bars_fall_gradually_when_paused() {
        let mut visualizer = Visualizer::new();
        visualizer.levels = vec![1.0; BANDS];
        visualizer.tick(None);
        assert!(visualizer.levels.iter().all(|&l| (l - DECAY).abs() < 1e-6));
        assert!(visualizer.is_animating());
    }

    #[test]
    fn peak_caps_hang_then_fall() {
        let mut visualizer = Visualizer::new();
        visualizer.levels = vec![1.0; BANDS];
        visualizer.tick(None);
        let (caught, hold) = visualizer.peaks[0];
        assert_eq!((caught, hold), (visualizer.levels[0], PEAK_HOLD), "the cap catches the bar's height");
        for _ in 0..PEAK_HOLD {
            visualizer.tick(None);
        }
        assert_eq!(visualizer.peaks[0].0, caught, "still hanging while the bar falls away");
        assert!(visualizer.levels[0] < caught);
        visualizer.tick(None);
        assert!(visualizer.peaks[0].0 < caught, "falling");
    }

    #[test]
    fn scope_draws_a_connected_line() {
        let (width, height) = (4, 2);
        let mut cells = vec![0u8; width * height];
        // A full-height ramp from bottom to top.
        let wave: Vec<f32> = (0..8).map(|i| -1.0 + 2.0 * i as f32 / 7.0).collect();
        plot(&wave, width, height, &mut cells);
        // Every one of the 8 dot rows is lit somewhere: no gaps in the line.
        for dot_y in 0..height * 4 {
            let lit = (0..width * 2).any(|x| {
                const DOT: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
                cells[(dot_y / 4) * width + x / 2] & DOT[x % 2][dot_y % 4] != 0
            });
            assert!(lit, "dot row {dot_y} is empty");
        }
    }

    #[test]
    fn scope_window_is_averaged_and_normalized_to_full_scale() {
        let samples = vec![i16::MAX; SAMPLE_RATE as usize];
        let wave = window(&samples, 0.5).unwrap();
        assert_eq!(wave.len(), WAVE_POINTS);
        assert!(wave.iter().all(|&v| (v - 1.0).abs() < 0.001));
        assert_eq!(window(&samples, 0.01), None);
    }

    #[test]
    fn renders_full_and_partial_bars() {
        let area = Rect::new(0, 0, 5, 2);
        let mut buf = Buffer::empty(area);
        let levels = [1.0, 0.25];
        draw_bars(&levels, None, 2, area, &mut buf, &[Color::Blue, Color::Magenta], Color::White);
        let rows: Vec<String> = (0..2).map(|y| (0..5).map(|x| buf[(x, y)].symbol().to_owned()).collect()).collect();
        assert_eq!(rows, vec!["██   ", "██ ▄▄"]);
        // Bottom row in the first color, top row in the second.
        assert_eq!(buf[(0, 1)].fg, Color::Blue);
        assert_eq!(buf[(0, 0)].fg, Color::Magenta);
    }
}
