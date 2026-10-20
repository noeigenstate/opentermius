fn main() {
    #[cfg(feature = "live-preview")]
    // SAFETY: this build-script process is still single-threaded; set the mode
    // before slint-build starts any compiler work. It does not affect the app
    // build script, test macros, or the calling shell's environment.
    unsafe {
        std::env::set_var("SLINT_LIVE_PREVIEW", "1");
    }

    slint_build::compile_with_config(
        "../ui/main.slint",
        slint_build::CompilerConfiguration::new()
            .with_style("fluent-light".into())
            .with_default_translation_context(slint_build::DefaultTranslationContext::None)
            .with_bundled_translations("../translations"),
    )
    .expect("Cannot compile the OpenTermius UI");
}
