fn main() {
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    // Checks the target, not the machine doing the build.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("../../assets/icon.ico");
        resource.set("ProductName", "FluxDM");
        resource.set("FileDescription", "FluxDM download manager");
        if let Err(e) = resource.compile() {
            panic!("could not embed the Windows icon: {e}");
        }
    }
}
