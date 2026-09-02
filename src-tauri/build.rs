use std::path::Path;

// The bundled sidecar tree (src-tauri/sidecar) is produced once by scripts/package-sidecar.ps1
// and is gitignored, so its worker.mjs went stale the moment sidecar/*.mjs changed — the app
// then ran months-old scraper code while the tests passed against the current source. Node
// modules and node.exe still come from the packaging script; only the sources are re-synced.
fn main() {
    let source = Path::new("../sidecar");
    let dest = Path::new("sidecar");
    println!("cargo:rerun-if-changed=../sidecar");
    // The window and taskbar icon is a resource compiled into the executable, so replacing the
    // file is only half the job: without this, Cargo has no reason to rebuild and the old icon
    // keeps running.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    if dest.is_dir() {
        for entry in std::fs::read_dir(source).expect("sidecar sources unreadable") {
            let path = entry.expect("sidecar entry unreadable").path();
            if path.extension().is_some_and(|e| e == "mjs") {
                let name = path.file_name().expect("sidecar file has no name");
                std::fs::copy(&path, dest.join(name)).expect("could not refresh bundled sidecar");
            }
        }
    }
    tauri_build::build()
}
