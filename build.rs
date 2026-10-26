//! Compila assets/xdir.rc (el ícono) con rc.exe del Windows SDK y lo enlaza al exe.
//! Sin crates: busca rc.exe en el PATH (entorno de vcvars) o en la instalación del SDK.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/xdir.rc");
    println!("cargo:rerun-if-changed=assets/xdir.ico");
    println!("cargo:rerun-if-changed=assets/xdir.manifest");

    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("xdir.res");
    let status = Command::new(rc_exe())
        .args(["/nologo", "/fo"])
        .arg(&out)
        .arg("assets/xdir.rc")
        .status()
        .expect("no se pudo ejecutar rc.exe");
    assert!(status.success(), "rc.exe falló al compilar assets/xdir.rc");
    // link.exe acepta el .res compilado como una entrada más.
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

/// rc.exe del PATH si está; si no, el de la versión más nueva del SDK en Windows Kits\10\bin.
fn rc_exe() -> PathBuf {
    let in_path = Command::new("rc.exe").arg("/?").output().is_ok_and(|o| o.status.success());
    if in_path {
        return PathBuf::from("rc.exe");
    }
    let base = std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"))
        .join(r"Windows Kits\10\bin");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&base)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join(r"x64\rc.exe"))
        .filter(|p| p.exists())
        .collect();
    // Las carpetas se llaman 10.0.<build>.0: el orden por versión numérica.
    versions.sort_by_key(|p| {
        let name = p.parent().and_then(|x| x.parent()).and_then(|x| x.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        name.split('.').map(|n| n.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>()
    });
    versions.pop().unwrap_or_else(|| panic!("no se encontró rc.exe (Windows SDK) en {}", base.display()))
}
