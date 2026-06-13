// Motion — shared movement mechanics: velocity integration and the solid-
// overlap separation response. Entity-agnostic by design.

pub mod movement;
pub mod separation;

pub use movement::MovementSystem;
pub use separation::SeparationSystem;
