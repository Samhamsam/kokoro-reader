fn main() {
    // espeak-rs-sys builds espeak-ng statically but doesn't link its runtime deps.
    // We must provide them. Order matters for the linker.
    println!("cargo:rustc-link-search=native=/usr/lib");
    println!("cargo:rustc-link-lib=dylib=pcaudio");
    println!("cargo:rustc-link-lib=dylib=sonic");
    // Force re-run if this file changes
    println!("cargo:rerun-if-changed=build.rs");
}
