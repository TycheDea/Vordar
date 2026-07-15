// Chapter 1's enemy archetypes — one module per enemy. Each owns everything
// that makes its enemy ITS enemy: the doc of how it plays, and a pointer to its
// stats/model RON. Shared mechanics (engagement, projectiles, contact damage,
// death) stay in vordar-game.

pub mod cinder_imp;
pub mod grunt;
pub mod mossback;
pub mod sentinel;
