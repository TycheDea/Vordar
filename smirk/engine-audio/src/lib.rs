// engine-audio — kira audio wrapper
//
// Owns:
//   - AudioManager setup
//   - Asset loading and caching (sounds, music)
//   - SFX playback: hit, death, skill use
//   - Background music loop
//
// Registers with App via:
//   app.insert_resource(AudioResources::init())
//      .add_system(AudioSystem, Phase::PostUpdate, SystemOrder::Default)
//
// Listens to engine events:
//   EntityDespawned { entity } → play death SFX if entity had AudioTag
//   CollisionStarted { a, b }  → play hit SFX

// pub mod manager;  // AudioManager wrapper
// pub mod assets;   // SoundHandle loading + caching
