// DevStats — resource backing the F3 debug overlay.
//
// engine-app owns frame timing (recorded in App::tick); any system or plugin
// can publish its own counters with `stats.set("key", value)` — the overlay
// renders whatever is in here, so modules add debug lines without touching
// the renderer.
//
// Cost when closed: one f32 accumulation per frame; writers should check
// `stats.open` before formatting values.

/// Seconds over which fps / frame-time stats are averaged.
const WINDOW: f32 = 0.5;

pub struct DevStats {
    /// Overlay visibility — toggled with F3 (DevOverlaySystem in engine-renderer).
    pub open: bool,
    // current accumulation window
    accum:  f32,
    frames: u32,
    worst:  f32,
    // last completed window
    fps:      f32,
    avg_ms:   f32,
    worst_ms: f32,
    /// Custom lines published by systems — insertion order is display order.
    custom: Vec<(String, String)>,
}

impl Default for DevStats {
    fn default() -> Self {
        Self::new()
    }
}

impl DevStats {
    pub fn new() -> Self {
        Self {
            open:     false,
            accum:    0.0,
            frames:   0,
            worst:    0.0,
            fps:      0.0,
            avg_ms:   0.0,
            worst_ms: 0.0,
            custom:   Vec::new(),
        }
    }

    /// Called once per rendered frame by App::tick.
    pub(crate) fn record_frame(&mut self, dt: f32) {
        self.accum  += dt;
        self.frames += 1;
        self.worst   = self.worst.max(dt);
        if self.accum >= WINDOW {
            self.fps      = self.frames as f32 / self.accum;
            self.avg_ms   = self.accum / self.frames as f32 * 1000.0;
            self.worst_ms = self.worst * 1000.0;
            self.accum  = 0.0;
            self.frames = 0;
            self.worst  = 0.0;
        }
    }

    /// Publish (or update) a custom overlay line. Keys keep their first
    /// insertion position so the display doesn't reorder between frames.
    pub fn set(&mut self, key: &str, value: impl std::fmt::Display) {
        let value = value.to_string();
        match self.custom.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = value,
            None        => self.custom.push((key.to_owned(), value)),
        }
    }

    /// Everything the overlay shows, built-ins first.
    pub fn display_lines(&self) -> Vec<(String, String)> {
        let mut lines = vec![
            ("fps".to_owned(),         format!("{:.0}", self.fps)),
            ("frame avg".to_owned(),   format!("{:.2} ms", self.avg_ms)),
            ("frame worst".to_owned(), format!("{:.2} ms", self.worst_ms)),
        ];
        lines.extend(self.custom.iter().cloned());
        lines
    }
}
