fn main() {
    // gtk4-layer-shell is the installed C library (no Rust crate is used);
    // declare the link order explicitly: layer-shell before wayland-client.
    // The Windows port has no native libraries to link.
    if std::env::var("CARGO_CFG_UNIX").is_ok() {
        println!("cargo:rustc-link-lib=gtk4-layer-shell");
        println!("cargo:rustc-link-lib=wayland-client");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
