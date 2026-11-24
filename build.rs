fn main() {
    #[cfg(target_os = "macos")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=src/macos_chrome.m");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=objc");
        cc::Build::new()
            .file("src/macos_chrome.m")
            .flag("-fobjc-arc")
            .compile("trainer_macos_chrome");
    }
    println!("cargo:rerun-if-changed=assets/trainer-icon.ico");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        winresource::WindowsResource::new()
            .set_icon("assets/trainer-icon.ico")
            .compile()
            .expect("compile trainer icon");
    }
}
