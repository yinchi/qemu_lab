//! Rebuilds the programs when their linker script changes (cargo does not watch it on its own).
fn main() {
    println!("cargo:rerun-if-changed=link.ld");
}
