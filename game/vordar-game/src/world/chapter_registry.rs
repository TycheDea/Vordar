// Chapter registration and installation — the registry that links chapter
// crates into a binary and orchestrates their dependency graph at startup.
// Each chapter crate registers itself as a ChapterModule; the registry topologically
// sorts dependencies and installs both content and full simulation plugins.

use engine_app::app::App;

/// One chapter crate's self-description. `requires` is the dependency chain
/// ("chapter02 requires chapter01"): installing a chapter first installs the
/// CONTENT of everything it requires (prefabs/components must exist for
/// carried-over entities), then its own full plugin.
pub struct ChapterModule {
    pub name: &'static str,
    pub requires: &'static [&'static str],
    /// Full simulation plugin (server zones, sandbox).
    pub install: fn(&mut App),
    /// Registration-only content subset (networked display clients, deps).
    pub install_content: fn(&mut App),
}

pub struct ChapterRegistry {
    modules: Vec<ChapterModule>,
}

impl ChapterRegistry {
    pub fn new(modules: Vec<ChapterModule>) -> Self {
        Self { modules }
    }

    fn find(&self, name: &str) -> Result<&ChapterModule, String> {
        self.modules
            .iter()
            .find(|m| m.name == name)
            .ok_or_else(|| format!("unknown chapter '{name}' (not linked into this binary)"))
    }

    /// Names of `name`'s transitive dependencies, dependencies first.
    /// Depth-first, cycle-checked; chapter chains are tiny.
    fn deps_of(&self, name: &str) -> Result<Vec<&'static str>, String> {
        fn visit<'a>(
            reg: &'a ChapterRegistry,
            name: &str,
            ordered: &mut Vec<&'static str>,
            visiting: &mut Vec<&'a str>,
        ) -> Result<(), String> {
            if visiting.contains(&name) {
                return Err(format!("chapter dependency cycle through '{name}'"));
            }
            let module = reg.find(name)?;
            visiting.push(module.name);
            for dep in module.requires {
                if !ordered.contains(dep) {
                    visit(reg, dep, ordered, visiting)?;
                    ordered.push(dep);
                }
            }
            visiting.pop();
            Ok(())
        }
        let mut ordered = Vec::new();
        let mut visiting = Vec::new();
        visit(self, name, &mut ordered, &mut visiting)?;
        Ok(ordered)
    }

    /// Install chapter `name` into a simulation App: content of its
    /// transitive dependencies first, then its own full plugin.
    pub fn install(&self, name: &str, app: &mut App) -> Result<(), String> {
        for dep in self.deps_of(name)? {
            (self.find(dep)?.install_content)(app);
        }
        (self.find(name)?.install)(app);
        Ok(())
    }

    /// Install every linked chapter's CONTENT (a display client must be able
    /// to show replicated entities from any zone it can be redirected to).
    pub fn install_all_content(&self, app: &mut App) {
        for module in &self.modules {
            (module.install_content)(app);
        }
    }
}
