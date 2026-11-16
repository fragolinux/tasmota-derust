//! Windows executables get a version resource (product name, description, version), like
//! any regular Windows program, and the app icon. Other targets: nothing to do.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../packaging/windows/icon.ico");
    res.set("ProductName", "Tasmota Derust")
        .set("FileDescription", "Tasmota Derust")
        .set("LegalCopyright", "MIT OR Apache-2.0")
        .set("CompanyName", "Tasmota Derust project");
    if let Err(e) = res.compile() {
        panic!("Windows resource: {e}");
    }
}
