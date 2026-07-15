pub fn workspace_root() {
    // Prefabs load from content/ relative to cwd — run as if from workspace root.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::set_current_dir(root).unwrap();
}
