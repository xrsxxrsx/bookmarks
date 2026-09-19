fn main() {
    // Tauri embeds `frontendDist` at compile time, but Cargo does not know the frontend
    // exists. Without these hints, rebuilding the interface without touching Rust would
    // leave the previous assets baked into the executable — a stale UI that is very
    // confusing to debug.
    println!("cargo:rerun-if-changed=../../dist");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=capabilities");
    println!("cargo:rerun-if-changed=icons");

    tauri_build::build()
}
