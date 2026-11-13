use std::path::{Path, PathBuf};

fn dist_dir() -> PathBuf {
    std::env::var("GOLDEN_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dist"))
}

#[test]
fn corpus() {
    let dist = dist_dir();
    if !dist.exists() {
        eprintln!(
            "skip golden corpus: {} does not exist (run scripts/build-wasm.sh)",
            dist.display()
        );
        return;
    }
    if let Err(e) = golden::run_corpus(&dist) {
        panic!("golden corpus failed:\n{e}");
    }
}
