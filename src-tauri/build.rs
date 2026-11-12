//! Build glue: Tauri's own codegen, plus the link line for the Lean parser.
//!
//! `lean4-ast/build.py` writes `lean4-ast/build/link.json` once it has built the
//! static library that wraps Lean's parser. That file holds everything the linker
//! needs — the archive, Lean's own archives in the order `leanc` uses, the
//! toolchain directories — so nothing here has to know about Lean versions or
//! paths. `cargo:rustc-link-arg` is per-target, which is why this lives in the
//! application's build script rather than in the `klean-ast` crate's.

use std::path::PathBuf;

fn main() {
    link_lean_parser();
    tauri_build::build()
}

fn link_lean_parser() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let link_file = std::env::var_os("KLEAN_AST_LINK")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("../../lean4-ast/build/link.json"));
    println!("cargo:rerun-if-env-changed=KLEAN_AST_LINK");
    println!("cargo:rerun-if-changed={}", link_file.display());

    let Ok(text) = std::fs::read_to_string(&link_file) else {
        // A checkout without a built parser still has to configure, so the app can
        // be built far enough to report the problem at run time.
        println!(
            "cargo:warning=lean4-ast is not built ({}); run `python3 lean4-ast/build.py`",
            link_file.display()
        );
        return;
    };
    let link: serde_json::Value =
        serde_json::from_str(&text).expect("lean4-ast: link.json is not valid JSON");
    let static_lib = link
        .get("static_lib")
        .and_then(|value| value.as_str())
        .expect("lean4-ast: link.json has no `static_lib`");
    if !std::path::Path::new(static_lib).is_file() {
        println!(
            "cargo:warning=lean4-ast: {static_lib} is missing; run `python3 lean4-ast/build.py`"
        );
        return;
    }

    // Lean's own flags, in Lean's own order: the archives have to be repeated
    // inside the same groups `leanc` uses, or the static link fails.
    //
    // The flags reach every executable target of this package — the app binary,
    // its unit tests and the examples. They must never reach a *shared* library:
    // Lean's runtime is built with initial-exec TLS, which the linker refuses to
    // place in a `-shared` object, so this package does not build a `cdylib`
    // (see `crate-type` in Cargo.toml).
    let mut flags = vec![static_lib.to_string()];
    // The interpreter resolves compiled symbols through the dynamic table, which
    // is what `supportInterpreter := true` sets up for a Lake executable.
    #[cfg(unix)]
    flags.push("-rdynamic".to_string());
    if let Some(ldflags) = link.get("ldflags").and_then(|value| value.as_array()) {
        for flag in ldflags {
            if let Some(flag) = flag.as_str() {
                flags.push(flag.to_string());
            }
        }
    }
    for flag in &flags {
        println!("cargo:rustc-link-arg={flag}");
    }
}
