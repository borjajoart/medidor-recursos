//! Incrusta el manifiesto de Windows en la interfaz gráfica: controles comunes v6
//! (sin él, el exe no arranca) y soporte de pantallas con escalado (DPI).
fn main() {
    let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("res/medidor-gui.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bin=medidor-gui=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bin=medidor-gui=/MANIFESTINPUT:{}", manifest.display());
    }
}
