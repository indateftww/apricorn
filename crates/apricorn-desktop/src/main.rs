//! Desktop shell for apricorn. Real windowing (winit + wgpu) arrives in
//! Phase 3; this stub exists so the crate graph and CI builds verify now.

fn main() {
    println!(
        "apricorn {} — desktop shell stub (engine state format v{})",
        env!("CARGO_PKG_VERSION"),
        apricorn_core::STATE_FORMAT_VERSION
    );
    println!("Renderer and windowing arrive in PLAN.md Phase 3.");
}