// Assembles every .s/.S file under src/ (they live in src/arch/) and links the result into
// `libboot_asm.a`.

use std::path::Path;

/// Adds every `.s`/`.S` file under `dir`, recursively, to `build`; returns whether it found any.
fn add_asm(dir: &Path, build: &mut cc::Build) -> bool {
    let mut any = false;
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("failed to read {dir:?}: {e}")) {
        let path = entry.expect("failed to read dir entry").path();
        if path.is_dir() {
            any |= add_asm(&path, build);
        } else if matches!(path.extension().and_then(|e| e.to_str()), Some("s" | "S")) {
            println!("cargo:rerun-if-changed={}", path.display());
            build.file(&path);
            any = true;
        }
    }
    any
}

fn main() {
    println!("cargo:rerun-if-changed=src");
    let mut build = cc::Build::new();
    if add_asm(Path::new("src"), &mut build) {
        build.compile("boot_asm");
    }
}
