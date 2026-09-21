//! Rebuilds the programs when the linker script changes (cargo does not watch it on its own). Every tier
//! links with the base tier's `link.ld`, so all programs share one address-space layout.
fn main() {
    println!("cargo:rerun-if-changed=../progs/link.ld");
}
