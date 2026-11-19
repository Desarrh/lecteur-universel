// Icône et nom de l'application dans l'exe : Explorateur, barre des tâches, menu Démarrer.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("FileDescription", "Lecteur universel")
            .set("ProductName", "Lecteur universel")
            .set("InternalName", "lecteur-universel")
            .set("OriginalFilename", "lecteur-universel.exe")
            .set("LegalCopyright", "");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icône non intégrée : {e}");
        }
    }
}
