fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=src/menu_bar.m");
        cc::Build::new()
            .file("src/menu_bar.m")
            .flag("-fobjc-arc")
            .compile("hostlane_menu_bar");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rustc-link-lib=framework=Security");
    }
}
