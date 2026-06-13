/// How often a phase executes relative to the display frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TickRate {
    /// Execute exactly once per display frame. Systems receive the actual frame delta.
    Render,
    /// Execute at a fixed frequency, independent of display framerate.
    /// `hz` is the target steps per second (e.g. 60.0, 30.0, 20.0).
    /// Systems always receive `1.0 / hz` as their delta — it never varies.
    Fixed(f32),
}