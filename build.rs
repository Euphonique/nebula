include!("src/icon.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("nebula.ico");
    std::fs::write(&ico, encode_ico(&[16, 24, 32, 48, 64, 256])).unwrap();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico.to_str().unwrap());
        res.set("FileDescription", "Nebula Editor");
        res.set("ProductName", "Nebula");
        if let Err(e) = res.compile() {
            // A missing resource compiler only costs the .exe icon.
            println!("cargo:warning=App-Icon nicht eingebettet: {e}");
        }
    }
}
