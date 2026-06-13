// WinitEventProcessor — type-erased bridge so app_loop.rs can forward winit events
// to subsystems (e.g. egui) without engine-app depending on rendering crates.
//
// Usage:
//   // In engine-renderer::init(), register the real impl:
//   resources.insert(WinitEventProcessor::new(move |w, e| state.on_event(w, e)));
//
//   // app_loop.rs calls it each event before game input:
//   let consumed = resources.get_mut::<WinitEventProcessor>()
//       .map(|p| p.process(window, event))
//       .unwrap_or(false);

use winit::event::WindowEvent;
use winit::window::Window;

pub struct WinitEventProcessor(Box<dyn FnMut(&Window, &WindowEvent) -> bool>);

impl WinitEventProcessor {
    pub fn new(f: impl FnMut(&Window, &WindowEvent) -> bool + 'static) -> Self {
        Self(Box::new(f))
    }

    pub fn process(&mut self, window: &Window, event: &WindowEvent) -> bool {
        (self.0)(window, event)
    }
}
