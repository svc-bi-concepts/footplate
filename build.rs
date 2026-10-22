//! Windows gives the main thread 1 MB of stack, where macOS and Linux give
//! it 8. gpui draws by recursion over the tree of a window's elements, whose
//! frames are large in a debug build, so the linker is asked for 8 MB there.
//! Nothing else is built here.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target = |name: &str| std::env::var(name).unwrap_or_default();
    if target("CARGO_CFG_TARGET_OS") == "windows" && target("CARGO_CFG_TARGET_ENV") == "msvc" {
        println!("cargo:rustc-link-arg=/STACK:8388608");
    }
}
