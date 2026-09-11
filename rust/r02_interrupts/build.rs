// Assembles every .s/.S file in src/ and links the result into
// `libboot_asm.a`.

fn main() {
    println!("cargo:rerun-if-changed=src");
    let mut build = cc::Build::new();
    let mut any = false;

    for entry in std::fs::read_dir("src").expect("failed to read src/") {
        let path = entry.expect("failed to read dir entry").path();
        if matches!(path.extension().and_then(|e| e.to_str()), Some("s" | "S")) {
            println!("cargo:rerun-if-changed={}", path.display());
            build.file(&path);
            any = true;
        }
    }

    if any {
        build.compile("boot_asm");
    }
}
