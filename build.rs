//! - Incrusta el manifiesto de Windows en la interfaz gráfica: controles comunes v6
//!   (sin él, el exe no arranca) y soporte de pantallas con escalado (DPI).
//! - Incrusta los iconos (res/recursos.rc) en los dos ejecutables.
fn main() {
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let manifest = root.join("res/medidor-gui.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    for f in ["recursos.rc", "icono.ico", "verde.ico", "naranja.ico", "rojo.ico", "gris.ico"] {
        println!("cargo:rerun-if-changed=res/{f}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("res/recursos.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bin=medidor-gui=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bin=medidor-gui=/MANIFESTINPUT:{}", manifest.display());
    }
}
