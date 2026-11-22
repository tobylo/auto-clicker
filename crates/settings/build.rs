//! Compiles the window's .slint files in Slint's fluent style, with the
//! translations under `translations/` bundled into the binary, and the
//! recordings list alone in a window of its own, which only the tests
//! include.

use std::path::{Path, PathBuf};

use slint_build::{CompileError, CompilerConfiguration, DefaultTranslationContext};

fn main() {
    let fluent = || CompilerConfiguration::new().with_style("fluent".to_owned());
    if let Err(err) = compile_for_tests("ui/tests/recordings.slint", fluent()) {
        fail("ui/tests/recordings.slint", &err);
    }
    // Last, since the generated code `slint::include_modules!` takes is
    // the last compiled with `compile_with_config`.
    let config = fluent()
        .with_bundled_translations("translations")
        .with_default_translation_context(DefaultTranslationContext::None);
    if let Err(err) = slint_build::compile_with_config("ui/window.slint", config) {
        fail("ui/window.slint", &err);
    }
}

/// Compiles `file` to `<its stem>.rs` in `OUT_DIR`, for the tests to
/// include by name.
fn compile_for_tests(file: &str, config: CompilerConfiguration) -> Result<(), CompileError> {
    let dir = |name| std::env::var_os(name).map(PathBuf::from);
    let (Some(manifest), Some(out)) = (dir("CARGO_MANIFEST_DIR"), dir("OUT_DIR")) else {
        return Err(CompileError::NotRunViaCargo);
    };
    let source = manifest.join(file);
    let stem = Path::new(file).file_stem().unwrap_or_default();
    let output = out.join(Path::new(stem).with_extension("rs"));
    for dependency in slint_build::compile_with_output_path(source, output, config)? {
        println!("cargo:rerun-if-changed={}", dependency.display());
    }
    Ok(())
}

fn fail(file: &str, err: &CompileError) {
    // A build script reports failure by panicking.
    #[allow(clippy::panic)]
    {
        panic!("{file}: {err}");
    }
}
