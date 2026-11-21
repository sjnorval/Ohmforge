//! EILIi startup animation: frame timing, skip/hold/fade, and egui painting.
//!
//! The packed frames in `assets/startup/splash.ofsp` are JPEG stills made by
//! `scripts/gen-startup-frames.sh` (or `.ps1`). The app never links a video
//! decoder. Audio from the source clip is dropped.

use std::time::{Duration, Instant};

use egui::{
    Color32, ColorImage, Context, Event, Id, LayerId, Order, Rect, Sense, TextureHandle, TextureOptions, Vec2, pos2,
};

/// Fade from the last splash frame into the main window.
pub const FADE_DURATION: Duration = Duration::from_millis(300);

/// True when `OF_NO_SPLASH` is set to a truthy value (`1`, `true`, `yes`, or `on`).
///
/// The desktop binary also honours `--no-splash`.
pub fn blocked_by_env() -> bool {
    match std::env::var("OF_NO_SPLASH") {
        Ok(value) => matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => false,
    }
}

/// Phase of the load-screen state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SplashPhase {
    /// Playing the animation in real time.
    Playing,
    /// Last frame held until the app has finished initialising.
    Holding,
    /// Cross-fading into the main window.
    Fading,
    /// Splash is finished; the main window has full control.
    Done,
}

/// Pure timing state for the load screen. Independent of egui so it can be unit-tested.
#[derive(Clone, Debug)]
pub struct SplashState {
    play_duration: Duration,
    fade_duration: Duration,
    frame_count: u32,
    fps_num: u32,
    fps_den: u32,
    elapsed: Duration,
    fade_elapsed: Duration,
    phase: SplashPhase,
}

impl SplashState {
    /// Build a machine for `frame_count` frames at `fps_num/fps_den` frames per second.
    pub fn new(frame_count: u32, fps_num: u32, fps_den: u32, fade_duration: Duration) -> Self {
        let frame_count = frame_count.max(1);
        let fps_num = fps_num.max(1);
        let fps_den = fps_den.max(1);
        Self {
            play_duration: play_duration(frame_count, fps_num, fps_den),
            fade_duration,
            frame_count,
            fps_num,
            fps_den,
            elapsed: Duration::ZERO,
            fade_elapsed: Duration::ZERO,
            phase: SplashPhase::Playing,
        }
    }

    /// Current phase.
    pub fn phase(&self) -> SplashPhase {
        self.phase
    }

    /// `true` until the fade has finished.
    pub fn is_active(&self) -> bool {
        self.phase != SplashPhase::Done
    }

    /// `true` while the splash should cover the main UI completely.
    pub fn is_covering(&self) -> bool {
        matches!(self.phase, SplashPhase::Playing | SplashPhase::Holding)
    }

    /// `true` during the cross-fade.
    pub fn is_fading(&self) -> bool {
        self.phase == SplashPhase::Fading
    }

    /// Advance wall-clock time. `init_ready` is true once the main UI can be shown.
    pub fn advance_to(&mut self, elapsed: Duration, init_ready: bool) {
        if self.phase == SplashPhase::Done {
            return;
        }
        if self.phase == SplashPhase::Playing {
            self.elapsed = elapsed;
            if self.elapsed >= self.play_duration {
                self.elapsed = self.play_duration;
                if init_ready {
                    self.start_fade();
                } else {
                    self.phase = SplashPhase::Holding;
                }
            }
            return;
        }
        if self.phase == SplashPhase::Holding && init_ready {
            self.start_fade();
        }
    }

    /// Advance the fade by `dt`. Ignored in other phases.
    pub fn advance_fade(&mut self, dt: Duration) {
        if self.phase != SplashPhase::Fading {
            return;
        }
        self.fade_elapsed = self.fade_elapsed.saturating_add(dt);
        if self.fade_elapsed >= self.fade_duration {
            self.fade_elapsed = self.fade_duration;
            self.phase = SplashPhase::Done;
        }
    }

    /// Jump to the last frame. Fades if the app is ready, otherwise holds.
    ///
    /// A skip during the fade finishes immediately.
    pub fn skip(&mut self, init_ready: bool) {
        match self.phase {
            SplashPhase::Playing | SplashPhase::Holding => {
                self.elapsed = self.play_duration;
                if init_ready {
                    self.start_fade();
                } else {
                    self.phase = SplashPhase::Holding;
                }
            }
            SplashPhase::Fading => {
                self.fade_elapsed = self.fade_duration;
                self.phase = SplashPhase::Done;
            }
            SplashPhase::Done => {}
        }
    }

    /// Frame to show, in `0..frame_count`.
    pub fn frame_index(&self) -> u32 {
        if self.phase != SplashPhase::Playing {
            return self.frame_count - 1;
        }
        let ticks = self.elapsed.as_secs_f64() * f64::from(self.fps_num) / f64::from(self.fps_den);
        (ticks.floor() as u32).min(self.frame_count - 1)
    }

    /// Opacity of the splash overlay: 1 while playing or holding, 0 when done.
    pub fn fade_alpha(&self) -> f32 {
        match self.phase {
            SplashPhase::Playing | SplashPhase::Holding => 1.0,
            SplashPhase::Fading => {
                let denom = self.fade_duration.as_secs_f32().max(f32::EPSILON);
                (1.0 - self.fade_elapsed.as_secs_f32() / denom).clamp(0.0, 1.0)
            }
            SplashPhase::Done => 0.0,
        }
    }

    fn start_fade(&mut self) {
        self.elapsed = self.play_duration;
        self.fade_elapsed = Duration::ZERO;
        self.phase = SplashPhase::Fading;
    }
}

fn play_duration(frame_count: u32, fps_num: u32, fps_den: u32) -> Duration {
    let nanos = u128::from(frame_count) * u128::from(fps_den) * 1_000_000_000 / u128::from(fps_num);
    Duration::from_nanos(nanos.min(u128::from(u64::MAX)) as u64)
}

/// Packed splash frames (`OFSP` v1) embedded from `assets/startup/splash.ofsp`.
pub struct SplashFrames {
    width: u32,
    height: u32,
    fps_num: u32,
    fps_den: u32,
    frames: Vec<Vec<u8>>,
}

impl SplashFrames {
    /// Parse a packed splash blob.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        const HEADER: usize = 4 + 4 * 6;
        if bytes.len() < HEADER || &bytes[..4] != b"OFSP" {
            return Err("splash pack is missing the OFSP header".into());
        }
        let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        if version != 1 {
            return Err(format!("unsupported splash pack version {version}"));
        }
        let width = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let height = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let fps_num = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let fps_den = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        let count = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        if width == 0 || height == 0 || fps_num == 0 || fps_den == 0 || count == 0 {
            return Err("splash pack has an empty header field".into());
        }
        let mut frames = Vec::with_capacity(count as usize);
        let mut cursor = HEADER;
        for i in 0..count {
            if cursor + 4 > bytes.len() {
                return Err(format!("splash pack is truncated at frame {i}"));
            }
            let len = u32::from_le_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
            cursor += 4;
            let end = cursor.checked_add(len).ok_or("splash pack frame length overflow")?;
            if end > bytes.len() {
                return Err(format!("splash pack is truncated inside frame {i}"));
            }
            frames.push(bytes[cursor..end].to_vec());
            cursor = end;
        }
        if cursor != bytes.len() {
            return Err("splash pack has trailing bytes".into());
        }
        Ok(Self {
            width,
            height,
            fps_num,
            fps_den,
            frames,
        })
    }

    /// The committed `splash.ofsp` pack, compiled into the GUI binary.
    pub fn embedded() -> Result<Self, String> {
        const PACK: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/startup/splash.ofsp"));
        Self::parse(PACK)
    }

    /// Frame count in the pack.
    pub fn len(&self) -> u32 {
        self.frames.len() as u32
    }

    /// Whether the pack contains no frames (the parser also rejects this).
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Packed image size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Frames per second as a reduced-or-not fraction.
    pub fn fps(&self) -> (u32, u32) {
        (self.fps_num, self.fps_den)
    }

    fn decode(&self, index: u32) -> Result<ColorImage, String> {
        let bytes = self
            .frames
            .get(index as usize)
            .ok_or_else(|| format!("splash frame {index} is out of range"))?;
        let image = image::load_from_memory(bytes).map_err(|e| format!("splash frame {index} did not decode: {e}"))?;
        let rgba = image.to_rgba8();
        let width = rgba.width() as usize;
        let height = rgba.height() as usize;
        if width != self.width as usize || height != self.height as usize {
            return Err(format!(
                "splash frame {index} is {width}x{height}, expected {}x{}",
                self.width, self.height
            ));
        }
        Ok(ColorImage::from_rgba_unmultiplied([width, height], rgba.as_raw()))
    }
}

/// Live playback of the embedded splash over the desktop window.
pub struct SplashView {
    frames: SplashFrames,
    state: SplashState,
    start: Instant,
    fade_started: Option<Instant>,
    texture: Option<TextureHandle>,
    last_index: Option<u32>,
    painted: bool,
}

impl SplashView {
    /// Load the embedded pack and start it in the Playing phase.
    pub fn load() -> Result<Self, String> {
        let frames = SplashFrames::embedded()?;
        let state = SplashState::new(frames.len(), frames.fps().0, frames.fps().1, FADE_DURATION);
        Ok(Self {
            frames,
            state,
            start: Instant::now(),
            fade_started: None,
            texture: None,
            last_index: None,
            painted: false,
        })
    }

    /// At least one frame has been submitted to egui.
    pub fn has_painted(&self) -> bool {
        self.painted
    }

    /// Mark that a splash frame was painted (so init can start on the next frame).
    pub fn mark_painted(&mut self) {
        self.painted = true;
    }

    /// Whether the splash is still on screen (playing, holding, or fading).
    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }

    /// Whether the main UI should be hidden underneath an opaque splash.
    pub fn is_covering(&self) -> bool {
        self.state.is_covering()
    }

    /// Whether the main UI should be drawn under a fading splash.
    pub fn is_fading(&self) -> bool {
        self.state.is_fading()
    }

    /// Apply skip input, advance clocks, and paint the current frame.
    pub fn show(&mut self, ctx: &Context, init_ready: bool) {
        if !self.state.is_active() {
            return;
        }
        if wants_skip(ctx) {
            self.state.skip(init_ready);
        }
        self.state.advance_to(self.start.elapsed(), init_ready);
        if self.state.is_fading() {
            let fade_started = *self.fade_started.get_or_insert_with(Instant::now);
            self.state.set_fade_elapsed(fade_started.elapsed());
        }
        if !self.state.is_active() {
            return;
        }
        self.paint(ctx);
        ctx.request_repaint();
        self.painted = true;
    }
}

impl SplashState {
    /// Set fade elapsed to an absolute time from the fade start instant.
    fn set_fade_elapsed(&mut self, elapsed: Duration) {
        if self.phase != SplashPhase::Fading {
            return;
        }
        self.fade_elapsed = elapsed;
        if self.fade_elapsed >= self.fade_duration {
            self.fade_elapsed = self.fade_duration;
            self.phase = SplashPhase::Done;
        }
    }

    fn abort(&mut self) {
        self.elapsed = self.play_duration;
        self.fade_elapsed = self.fade_duration;
        self.phase = SplashPhase::Done;
    }
}

impl SplashView {
    fn paint(&mut self, ctx: &Context) {
        let index = self.state.frame_index();
        if self.last_index != Some(index) {
            match self.frames.decode(index) {
                Ok(image) => {
                    match self.texture.as_mut() {
                        Some(texture) => texture.set(image, TextureOptions::LINEAR),
                        None => {
                            self.texture = Some(ctx.load_texture("ohmforge-splash", image, TextureOptions::LINEAR));
                        }
                    }
                    self.last_index = Some(index);
                }
                Err(err) => {
                    // A decode failure should not trap the user on a black window.
                    eprintln!("Ohmforge splash: {err}");
                    self.state.abort();
                    return;
                }
            }
        }
        let Some(texture) = self.texture.as_ref() else {
            return;
        };
        let alpha = self.state.fade_alpha();
        if alpha <= 0.0 {
            return;
        }
        let screen = ctx.content_rect();
        let (width, height) = self.frames.size();
        let tint = Color32::from_white_alpha((alpha * 255.0).round() as u8);
        let bar = Color32::from_black_alpha((alpha * 255.0).round() as u8);
        let dest = fitted(screen, Vec2::new(width as f32, height as f32));
        let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("ohmforge-splash")));
        painter.rect_filled(screen, 0.0, bar);
        painter.image(
            texture.id(),
            dest,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            tint,
        );
        // A full-screen area captures clicks (already used for skip) so they
        // do not fall through to the schematic during the fade.
        egui::Area::new(Id::unique("ohmforge-splash-input"))
            .fixed_pos(screen.min)
            .order(Order::Foreground)
            .interactable(true)
            .show(ctx, |ui| {
                let _ = ui.allocate_rect(screen, Sense::click());
            });
    }
}

fn fitted(area: Rect, size: Vec2) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 || area.width() <= 0.0 || area.height() <= 0.0 {
        return area;
    }
    let scale = (area.width() / size.x).min(area.height() / size.y);
    Rect::from_center_size(area.center(), size * scale)
}

fn wants_skip(ctx: &Context) -> bool {
    ctx.input(|i| {
        i.pointer.any_click()
            || i.events
                .iter()
                .any(|event| matches!(event, Event::Key { pressed: true, .. } | Event::Text(_)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED_PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00,
        0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xde, 0x00, 0x00, 0x00,
        0x0c, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0xc9,
        0xfe, 0x92, 0xef, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    fn machine(frames: u32, fps: u32) -> SplashState {
        SplashState::new(frames, fps, 1, FADE_DURATION)
    }

    fn pack(frames: &[&[u8]], width: u32, height: u32, fps_num: u32, fps_den: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"OFSP");
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&fps_num.to_le_bytes());
        bytes.extend_from_slice(&fps_den.to_le_bytes());
        bytes.extend_from_slice(&(frames.len() as u32).to_le_bytes());
        for frame in frames {
            bytes.extend_from_slice(&(frame.len() as u32).to_le_bytes());
            bytes.extend_from_slice(frame);
        }
        bytes
    }

    #[test]
    fn starts_on_the_first_frame() {
        let state = machine(145, 24);
        assert_eq!(state.phase(), SplashPhase::Playing);
        assert_eq!(state.frame_index(), 0);
        assert_eq!(state.fade_alpha(), 1.0);
        assert!(state.is_covering());
    }

    #[test]
    fn frame_index_tracks_realtime_fps() {
        let mut state = machine(145, 24);
        state.advance_to(Duration::from_millis(1000), true);
        assert_eq!(state.frame_index(), 24);
        assert_eq!(state.phase(), SplashPhase::Playing);
        state.advance_to(Duration::from_nanos(6_000_000_000), true);
        assert_eq!(state.frame_index(), 144);
        assert_eq!(state.phase(), SplashPhase::Playing);
    }

    #[test]
    fn ends_in_a_fade_when_init_is_ready() {
        let mut state = machine(145, 24);
        state.advance_to(play_duration(145, 24, 1), true);
        assert_eq!(state.phase(), SplashPhase::Fading);
        assert_eq!(state.frame_index(), 144);
        assert_eq!(state.fade_alpha(), 1.0);
        state.advance_fade(Duration::from_millis(150));
        let mid = state.fade_alpha();
        assert!((mid - 0.5).abs() < 0.02, "{mid}");
        state.advance_fade(Duration::from_millis(150));
        assert_eq!(state.phase(), SplashPhase::Done);
        assert_eq!(state.fade_alpha(), 0.0);
    }

    #[test]
    fn holds_the_last_frame_until_init_is_ready() {
        let mut state = machine(24, 24);
        state.advance_to(Duration::from_secs(2), false);
        assert_eq!(state.phase(), SplashPhase::Holding);
        assert_eq!(state.frame_index(), 23);
        assert_eq!(state.fade_alpha(), 1.0);
        state.advance_to(Duration::from_secs(3), false);
        assert_eq!(state.phase(), SplashPhase::Holding);
        state.advance_to(Duration::from_secs(3), true);
        assert_eq!(state.phase(), SplashPhase::Fading);
        assert_eq!(state.frame_index(), 23);
    }

    #[test]
    fn skip_jumps_to_the_last_frame() {
        let mut state = machine(145, 24);
        state.advance_to(Duration::from_millis(500), false);
        state.skip(false);
        assert_eq!(state.phase(), SplashPhase::Holding);
        assert_eq!(state.frame_index(), 144);
        state.skip(true);
        assert_eq!(state.phase(), SplashPhase::Fading);
        state.skip(true);
        assert_eq!(state.phase(), SplashPhase::Done);
    }

    #[test]
    fn skip_with_init_ready_starts_the_fade() {
        let mut state = machine(10, 24);
        state.skip(true);
        assert_eq!(state.phase(), SplashPhase::Fading);
        assert_eq!(state.frame_index(), 9);
    }

    #[test]
    fn parse_rejects_a_truncated_pack() {
        assert!(SplashFrames::parse(b"OFSP").is_err());
        let mut bytes = pack(&[RED_PNG], 1, 1, 24, 1);
        bytes.pop();
        assert!(SplashFrames::parse(&bytes).is_err());
    }

    #[test]
    fn parse_and_decode_a_png_frame() {
        let frames = SplashFrames::parse(&pack(&[RED_PNG], 1, 1, 24, 1)).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames.size(), (1, 1));
        let image = frames.decode(0).unwrap();
        assert_eq!(image.width(), 1);
        assert_eq!(image.height(), 1);
        assert_eq!(image.pixels[0], Color32::from_rgb(255, 0, 0));
    }

    #[test]
    fn embedded_pack_matches_the_source_clip() {
        let frames = SplashFrames::embedded().unwrap();
        assert_eq!(frames.size(), (736, 400));
        assert_eq!(frames.fps(), (24, 1));
        assert_eq!(frames.len(), 145);
        let first = frames.decode(0).unwrap();
        assert_eq!(first.width(), 736);
        assert_eq!(first.height(), 400);
        let last = frames.decode(144).unwrap();
        assert_eq!(last.width(), 736);
        assert_eq!(last.height(), 400);
    }
}
