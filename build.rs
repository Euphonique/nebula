include!("src/icon.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=assets/nebula_logo.png");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("nebula.ico");
    std::fs::write(&ico, encode_ico(&[16, 24, 32, 48, 64, 256])).unwrap();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        linux_icons(&out);
    }
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

/// The app icon in the hicolor sizes, for the Linux package (cargo-deb picks
/// them up from `target/<profile>/icons/<size>/nebula.png`).
fn linux_icons(out_dir: &std::path::Path) {
    // OUT_DIR lies in target/<profile>/build/… (how deep depends on Cargo).
    let Some(profile_dir) = out_dir.ancestors().find(|p| p.file_name().is_some_and(|n| n == "build")).and_then(|p| p.parent()) else { return };
    for size in [16, 24, 32, 48, 64, 128, 256, 512] {
        let dir = profile_dir.join("icons").join(format!("{size}x{size}"));
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let file = std::fs::File::create(dir.join("nebula.png")).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size, size);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&render_icon(size)).unwrap();
    }
}
