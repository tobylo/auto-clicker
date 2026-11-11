//! Compiles the window's .slint files in Slint's fluent style, with the
//! translations under `translations/` bundled into the binary.

use slint_build::{CompilerConfiguration, DefaultTranslationContext};

fn main() {
    let config = CompilerConfiguration::new()
        .with_style("fluent".to_owned())
        .with_bundled_translations("translations")
        .with_default_translation_context(DefaultTranslationContext::None);
    if let Err(err) = slint_build::compile_with_config("ui/window.slint", config) {
        // A build script reports failure by panicking.
        #[allow(clippy::panic)]
        {
            panic!("ui/window.slint: {err}");
        }
    }
}
