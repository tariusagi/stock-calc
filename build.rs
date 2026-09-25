//! Embeds the app icon into the Windows executables (Explorer, shortcuts, taskbar).

fn main() {
    println!("cargo:rerun-if-changed=stock_calc.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("stock_calc.ico");
        res.compile().expect("failed to embed the Windows icon resource");
    }
}
