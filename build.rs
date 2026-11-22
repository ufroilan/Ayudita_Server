//! En Windows, el icono y el nombre del `.exe` (los que se ven en el Explorador y en la barra de
//! tareas). En los demás sistemas no hace nada: el icono de macOS va en el `.app`
//! (`scripts/package-macos.sh`) y la ventana dibuja el suyo (`src/icon.rs`).

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/icon.ico");
        res.set("ProductName", "Ayudita Comandas");
        res.set("FileDescription", "Ayudita Comandas");
        res.compile().expect("no se pudo incrustar el icono en el .exe");
    }
}
