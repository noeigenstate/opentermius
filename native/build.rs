fn main() {
    println!("cargo:rerun-if-changed=../assets/icon.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("../assets/icon.ico")
            .set("ProductName", "OpenTermius")
            .set("FileDescription", "OpenTermius SSH & SFTP Client")
            .set("InternalName", "opentermius")
            .set("OriginalFilename", "OpenTermius.exe")
            .compile()
            .expect("Cannot compile Windows application icon and version resources");
    }
    slint_build::compile_with_config(
        "ui/main.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent-light".into()),
    )
    .unwrap();
}
