//! macOS: portapapeles de archivos (`NSPasteboard`), abrir con el programa asociado
//! (`NSWorkspace`), "Obtener información" del Finder y un menú contextual propio (macOS no
//! ofrece el menú del Finder a otras apps).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use objc2::ClassType;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, ProtocolObject};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString, NSPasteboardURLReadingFileURLsOnlyKey, NSPasteboardWriting, NSWorkspace};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, NSURL};

use super::{Entry, Item};
use crate::fs::Location;
use crate::platform::{Handle, MenuAt, popup_menu};

/// Error de una operación del sistema (mismo uso que `windows::core::Error`).
#[derive(Debug, Clone)]
pub struct ShellError(pub String);

impl ShellError {
    pub fn message(&self) -> String {
        self.0.clone()
    }
}

impl std::fmt::Display for ShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub type Result<T> = std::result::Result<T, ShellError>;

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

/// Un valor booleano de recurso de un volumen (`None` si no se sabe).
fn volume_flag(url: &NSURL, key: &objc2_foundation::NSURLResourceKey) -> Option<bool> {
    let mut value: Option<Retained<AnyObject>> = None;
    // SAFETY: `value` recibe un objeto (o nada) del tipo que corresponde a la clave.
    unsafe { url.getResourceValue_forKey_error(&mut value, key) }.ok()?;
    value?.downcast::<NSNumber>().ok().map(|n| n.boolValue())
}

/// De estas raíces, las que se expulsan como en el Finder: expulsables, extraíbles, no internas
/// (imágenes de disco, discos externos) o de red. El disco de arranque nunca.
pub fn ejectable(roots: &[PathBuf]) -> Vec<PathBuf> {
    use objc2_foundation::{NSURLVolumeIsEjectableKey, NSURLVolumeIsInternalKey, NSURLVolumeIsLocalKey, NSURLVolumeIsRemovableKey};
    roots
        .iter()
        .filter(|r| r.as_path() != Path::new("/"))
        .filter(|r| {
            let url = file_url(r);
            // SAFETY: claves estáticas de Foundation.
            let (ej, rem, int, local) = unsafe {
                (
                    volume_flag(&url, NSURLVolumeIsEjectableKey),
                    volume_flag(&url, NSURLVolumeIsRemovableKey),
                    volume_flag(&url, NSURLVolumeIsInternalKey),
                    volume_flag(&url, NSURLVolumeIsLocalKey),
                )
            };
            ej == Some(true) || rem == Some(true) || int == Some(false) || local == Some(false)
        })
        .cloned()
        .collect()
}

/// Expulsa el volumen de `root` (`NSWorkspace`). Si falla, `lsof` dice qué programas lo usan.
pub fn eject(root: &Path) -> std::result::Result<(), xdir_core::shell::EjectError> {
    let ws = NSWorkspace::sharedWorkspace();
    let Err(e) = ws.unmountAndEjectDeviceAtURL_error(&file_url(root)) else { return Ok(()) };
    let message = e.localizedDescription().to_string();
    let holders = std::process::Command::new("/usr/sbin/lsof")
        // Con el punto de montaje, `lsof` lista lo abierto en todo ese sistema de archivos.
        .args(["-F", "pc"])
        .arg(root)
        .stderr(std::process::Stdio::null())
        .output()
        .map(|o| parse_lsof(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    // Lo que tenga abierto xdir ya se soltó: si aparece, no es «otro programa».
    let own = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let others: Vec<String> = holders.into_iter().filter(|h| Some(h) != own.as_ref()).collect();
    Err(xdir_core::shell::EjectError { busy_by: others.first().cloned(), busy: !others.is_empty(), message })
}

/// Nombres de programa (líneas `c…`) de la salida de `lsof -F pc`, sin repetir y en orden.
pub fn parse_lsof(out: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for name in out.lines().filter_map(|l| l.strip_prefix('c')) {
        if !name.is_empty() && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    names
}

/// Crea en `alias` un alias del Finder que apunta a `target` (bookmark de NSURL).
pub fn create_alias(target: &Path, alias: &Path) -> std::result::Result<(), String> {
    use objc2_foundation::{NSURLBookmarkCreationOptions, NSURLBookmarkFileCreationOptions};
    let data = file_url(target)
        .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
            NSURLBookmarkCreationOptions::SuitableForBookmarkFile,
            None,
            None,
        )
        .map_err(|e| format!("{}: {}", target.display(), e.localizedDescription()))?;
    NSURL::writeBookmarkData_toURL_options_error(&data, &file_url(alias), 0 as NSURLBookmarkFileCreationOptions)
        .map_err(|e| format!("{}: {}", alias.display(), e.localizedDescription()))
}

/// A dónde apunta un enlace simbólico o un alias del Finder (sin interfaz ni montar discos).
pub fn link_target(path: &Path) -> Option<PathBuf> {
    use objc2_foundation::NSURLBookmarkResolutionOptions;
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        let to = std::fs::read_link(path).ok()?;
        return Some(if to.is_absolute() { to } else { path.parent()?.join(to) });
    }
    if !crate::fs::is_alias(path) {
        return None;
    }
    let options = NSURLBookmarkResolutionOptions::WithoutUI | NSURLBookmarkResolutionOptions::WithoutMounting;
    let url = NSURL::URLByResolvingAliasFileAtURL_options_error(&file_url(path), options).ok()?;
    let resolved = PathBuf::from(url.path()?.to_string());
    (resolved != path).then_some(resolved)
}

// ---------- portapapeles ----------

/// macOS no tiene "cortar" archivos: se copia y se recuerda que fue un corte (para mover al pegar,
/// como ⌘⌥V en el Finder) mientras el portapapeles no cambie.
static CUT: Mutex<Option<isize>> = Mutex::new(None);

pub fn copy_files(items: &[Item], cut: bool) -> Result<()> {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    let urls: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
        items.iter().map(|i| ProtocolObject::from_retained(file_url(&i.path))).collect();
    if !pb.writeObjects(&NSArray::from_retained_slice(&urls)) {
        return Err(ShellError("el portapapeles no aceptó los archivos".into()));
    }
    *CUT.lock().unwrap() = cut.then(|| pb.changeCount());
    Ok(())
}

/// Archivos del portapapeles y si se cortaron.
pub fn clipboard_files() -> Option<(Vec<PathBuf>, bool)> {
    let pb = NSPasteboard::generalPasteboard();
    let classes = NSArray::from_slice(&[NSURL::class() as &AnyClass]);
    let key: &NSString = unsafe { NSPasteboardURLReadingFileURLsOnlyKey };
    let yes = NSNumber::new_bool(true);
    let options = NSDictionary::<NSString, AnyObject>::from_slices(&[key], &[&*yes as &AnyObject]);
    let objects = unsafe { pb.readObjectsForClasses_options(&classes, Some(&options)) }?;
    let paths: Vec<PathBuf> = objects
        .iter()
        .filter_map(|o| o.downcast::<NSURL>().ok())
        .filter_map(|u| u.path().map(|p| PathBuf::from(p.to_string())))
        .collect();
    if paths.is_empty() {
        return None;
    }
    let cut = CUT.lock().unwrap().is_some_and(|c| c == pb.changeCount());
    Some((paths, cut))
}

/// Pegar sin rutas de disco: en macOS no hay nada que hacer.
pub fn paste_virtual(_owner: Handle, _dest: &Location) -> Result<bool> {
    Ok(false)
}

/// Lo cortado se pega una sola vez.
pub fn clear_clipboard() {
    *CUT.lock().unwrap() = None;
    NSPasteboard::generalPasteboard().clearContents();
}

pub fn copy_text(_owner: Handle, text: &str) -> Result<()> {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    if pb.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString }) {
        Ok(())
    } else {
        Err(ShellError("no se pudo copiar el texto".into()))
    }
}

/// Diálogo del sistema para elegir una carpeta (`NSOpenPanel` solo con carpetas).
pub fn pick_folder(_owner: Handle, title: &str) -> Option<PathBuf> {
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    let mtm = objc2_foundation::MainThreadMarker::new()?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setAllowsMultipleSelection(false);
    panel.setMessage(Some(&NSString::from_str(title)));
    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    panel.URLs().firstObject()?.path().map(|p| PathBuf::from(p.to_string()))
}

// ---------- abrir y propiedades ----------

/// Abre un elemento con su programa (las apps se ejecutan).
pub fn open_item(owner: Handle, item: &Item) {
    open_file(owner, &item.path, None);
}

pub fn open_file(_owner: Handle, file: &Path, _dir: Option<&Path>) {
    NSWorkspace::sharedWorkspace().openURL(&file_url(file));
}

/// Abre el Finder con los elementos seleccionados.
pub fn reveal(items: &[Item]) {
    let urls: Vec<Retained<NSURL>> = items.iter().map(|i| file_url(&i.path)).collect();
    NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&urls));
}

/// "Obtener información" del Finder para cada elemento.
pub fn properties(_owner: Handle, items: &[Item]) {
    let paths: Vec<String> = items.iter().map(|i| i.path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"")).collect();
    std::thread::spawn(move || {
        let mut script = String::from("tell application \"Finder\"\nactivate\n");
        for p in paths {
            script.push_str(&format!("open information window of (POSIX file \"{p}\" as alias)\n"));
        }
        script.push_str("end tell");
        let _ = std::process::Command::new("osascript").arg("-e").arg(script).status();
    });
}

/// Disco de una ruta (para saber si mover o copiar al soltar).
pub fn volume_of(p: &Path) -> Option<OsString> {
    use std::os::unix::fs::MetadataExt;
    let meta = match std::fs::metadata(p) {
        Ok(m) => m,
        Err(_) => std::fs::metadata(p.parent()?).ok()?,
    };
    Some(OsString::from(meta.dev().to_string()))
}

/// Menús nativos al modo claro u oscuro: en macOS siguen a la ventana.
pub fn set_menu_mode(_dark: Option<bool>) {}

// ---------- verbos y menú contextual ----------

/// Ejecuta un verbo sobre los elementos. En macOS solo "delete" (a la Papelera).
pub fn invoke_verb(_owner: Handle, _folder: &Location, items: &[Item], verb: &str) -> Result<()> {
    match verb {
        "delete" => {
            let fm = objc2_foundation::NSFileManager::defaultManager();
            for i in items {
                fm.trashItemAtURL_resultingItemURL_error(&file_url(&i.path), None).map_err(|e| ShellError(e.localizedDescription().to_string()))?;
            }
            Ok(())
        }
        _ => Err(ShellError(format!("«{verb}» no está disponible en macOS"))),
    }
}

/// Acceso rápido de Windows: no existe en macOS.
pub fn pin_quick_access(_owner: Handle, _path: &Path) -> Result<()> {
    Err(ShellError("el Acceso rápido es de Windows".into()))
}

pub fn unpin_quick_access(_owner: Handle, _path: &Path) -> Result<()> {
    Err(ShellError("el Acceso rápido es de Windows".into()))
}

/// Identificadores propios: por encima de los del menú de archivos.
const OWN_BASE: u32 = 0x8000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    Open = 1,
    Reveal,
    Cut,
    Copy,
    Paste,
    Rename,
    Trash,
    CopyPath,
    Info,
    Terminal,
    Compress,
    OpenWith,
}

/// Menú contextual de archivos (o del fondo de la carpeta, sin `items`). Devuelve lo que la
/// aplicación tiene que hacer: un verbo de `OWN_VERBS`, `own:<id>` (una de las entradas `own`) o
/// `"shell"` si ya se resolvió acá.
#[allow(clippy::too_many_arguments)]
pub fn context_menu(
    owner: Handle,
    folder: &Location,
    items: &[Item],
    at: (f32, f32),
    _extended: bool,
    _intercept: bool,
    own: &[Entry],
    _pinned: bool,
) -> Option<String> {
    // Apps que pueden abrir el elemento ("Abrir con"), solo con uno.
    let apps: Vec<PathBuf> = match items {
        [one] => open_with_apps(&one.path),
        _ => Vec::new(),
    };
    let entries = menu_entries(items, own, &apps);
    let chosen = popup_menu(&entries, MenuAt::Point(at.0, at.1), None)?;
    if chosen >= OWN_BASE {
        return Some(format!("own:{}", chosen - OWN_BASE));
    }
    if chosen >= Act::OpenWith as u32 * 1000 {
        let app = apps.get((chosen - Act::OpenWith as u32 * 1000) as usize)?;
        let mut cmd = std::process::Command::new("open");
        cmd.arg("-a").arg(app);
        for i in items {
            cmd.arg(&i.path);
        }
        let _ = cmd.spawn();
        return Some("shell".into());
    }
    let target: Vec<Item> = if items.is_empty() { folder.target().map(Item::from).into_iter().collect() } else { items.to_vec() };
    let verb = match chosen {
        c if c == Act::Open as u32 => {
            for i in items {
                open_item(owner, i);
            }
            "shell"
        }
        c if c == Act::Reveal as u32 => {
            reveal(&target);
            "shell"
        }
        c if c == Act::Cut as u32 => "cut",
        c if c == Act::Copy as u32 => "copy",
        c if c == Act::Paste as u32 => "paste",
        c if c == Act::Rename as u32 => "rename",
        c if c == Act::Trash as u32 => "delete",
        c if c == Act::CopyPath as u32 => {
            let text: Vec<String> = target.iter().map(|i| i.path.to_string_lossy().into_owned()).collect();
            let _ = copy_text(owner, &text.join("\n"));
            "shell"
        }
        c if c == Act::Info as u32 => {
            properties(owner, &target);
            "shell"
        }
        c if c == Act::Terminal as u32 => {
            if let Some(dir) = folder.target() {
                let _ = std::process::Command::new("open").args(["-a", "Terminal"]).arg(dir).spawn();
            }
            "shell"
        }
        c if c == Act::Compress as u32 => {
            compress(items);
            "shell"
        }
        _ => return None,
    };
    Some(verb.to_string())
}

/// Entradas del menú contextual propio: las de la Mac y las de la aplicación (`own`, con
/// identificadores corridos), arriba en el menú del fondo (sin `items`) y después de «Copiar la
/// ruta» en el de los elementos. `apps` son las de "Abrir con".
fn menu_entries(items: &[Item], own: &[Entry], apps: &[PathBuf]) -> Vec<Entry> {
    let item = |a: Act, label: &str| Entry::Item { id: a as u32, label: label.to_string(), enabled: true, checked: false };
    let mut entries = Vec::new();
    if items.is_empty() {
        entries.extend(offset_own(own));
        entries.push(Entry::Sep);
        entries.push(item(Act::Terminal, "Abrir en Terminal"));
        entries.push(item(Act::Reveal, "Mostrar en el Finder"));
        entries.push(item(Act::Info, "Obtener información"));
        return entries;
    }
    entries.push(item(Act::Open, "Abrir"));
    if !apps.is_empty() {
        let subs = apps
            .iter()
            .enumerate()
            .map(|(i, app)| Entry::Item {
                id: Act::OpenWith as u32 * 1000 + i as u32,
                label: app.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                enabled: true,
                checked: false,
            })
            .collect();
        entries.push(Entry::Sub("Abrir con".into(), subs));
    }
    entries.push(item(Act::Reveal, "Mostrar en el Finder"));
    entries.push(Entry::Sep);
    entries.push(item(Act::Cut, "Cortar"));
    entries.push(item(Act::Copy, "Copiar"));
    entries.push(item(Act::CopyPath, "Copiar la ruta"));
    // Entradas propias de la aplicación («Copiar como ruta de acceso»...), junto a «Copiar la ruta».
    entries.extend(offset_own(own));
    entries.push(item(Act::Paste, "Pegar"));
    entries.push(Entry::Sep);
    entries.push(item(Act::Rename, "Renombrar"));
    entries.push(item(Act::Compress, "Comprimir"));
    entries.push(item(Act::Trash, "Trasladar a la Papelera"));
    entries.push(Entry::Sep);
    entries.push(item(Act::Info, "Obtener información"));
    entries
}

/// Las entradas propias de la aplicación, con identificadores corridos para no chocar.
fn offset_own(own: &[Entry]) -> Vec<Entry> {
    own.iter()
        .map(|e| match e {
            Entry::Item { id, label, enabled, checked } => Entry::Item { id: id + OWN_BASE, label: label.clone(), enabled: *enabled, checked: *checked },
            Entry::Sub(label, children) => Entry::Sub(label.clone(), offset_own(children)),
            Entry::Sep => Entry::Sep,
        })
        .collect()
}

/// Apps que pueden abrir un archivo, la predeterminada primero.
fn open_with_apps(path: &Path) -> Vec<PathBuf> {
    let ws = NSWorkspace::sharedWorkspace();
    let url = file_url(path);
    let default = ws.URLForApplicationToOpenURL(&url).and_then(|u| u.path()).map(|p| PathBuf::from(p.to_string()));
    let mut out: Vec<PathBuf> = default.iter().cloned().collect();
    for u in ws.URLsForApplicationsToOpenURL(&url).iter() {
        if let Some(p) = u.path().map(|p| PathBuf::from(p.to_string())) {
            if !out.contains(&p) {
                out.push(p);
            }
        }
        if out.len() >= 15 {
            break;
        }
    }
    out
}

/// Comprime en un .zip junto a los originales, como "Comprimir" del Finder.
fn compress(items: &[Item]) {
    let Some(parent) = items.first().and_then(|i| i.path.parent()).map(Path::to_path_buf) else { return };
    let base = match items {
        [one] => one.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archivo".into()),
        _ => "Archivo".into(),
    };
    let mut zip = parent.join(format!("{base}.zip"));
    let mut n = 2;
    while zip.exists() {
        zip = parent.join(format!("{base} {n}.zip"));
        n += 1;
    }
    let names: Vec<OsString> = items.iter().filter_map(|i| i.path.file_name().map(|n| n.to_os_string())).collect();
    std::thread::spawn(move || {
        let _ = std::process::Command::new("zip").arg("-qry").arg(&zip).args(&names).current_dir(&parent).status();
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_lsof_names() {
        let out = "p123\ncPreview\np456\ncWord\np789\ncPreview\n";
        assert_eq!(super::parse_lsof(out), ["Preview", "Word"]);
        assert!(super::parse_lsof("").is_empty());
        assert!(super::parse_lsof("p1\nc\n").is_empty());
    }

    /// El disco de arranque no se expulsa.
    #[test]
    fn startup_disk_not_ejectable() {
        assert!(super::ejectable(&[std::path::PathBuf::from("/")]).is_empty());
    }

    use super::*;

    fn labels(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Item { label, .. } | Entry::Sub(label, _) => label.clone(),
                Entry::Sep => "-".into(),
            })
            .collect()
    }

    /// Las entradas propias de la aplicación aparecen también en el menú de los elementos, justo
    /// después del «Copiar la ruta» de la Mac; «Abrir» sigue primero.
    #[test]
    fn item_menu_includes_own_entries() {
        let own = [Entry::Item { id: 0, label: "Copiar como ruta de acceso".into(), enabled: true, checked: false }];
        let items = [Item::from(PathBuf::from("/tmp/a.txt")), Item::from(PathBuf::from("/tmp/b.txt"))];
        let entries = menu_entries(&items, &own, &[]);
        let l = labels(&entries);
        assert_eq!(l[0], "Abrir");
        let at = l.iter().position(|s| s == "Copiar la ruta").expect("falta «Copiar la ruta»");
        assert_eq!(l[at + 1], "Copiar como ruta de acceso");
        let Entry::Item { id, .. } = &entries[at + 1] else { panic!() };
        assert_eq!(*id, OWN_BASE);
        // Sin entradas propias, el menú de siempre.
        let plain = labels(&menu_entries(&items, &[], &[]));
        assert_eq!(plain[0], "Abrir");
        assert_eq!(plain.len(), l.len() - 1);
    }
}
