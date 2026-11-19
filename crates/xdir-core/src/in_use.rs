//! Cuando algo falla porque otro programa tiene abierto el archivo (`ErrKind::InUse`): qué se
//! intentaba hacer y quién tiene cada archivo. Una operación suelta lo muestra en la ventana de
//! «archivo en uso»; los trabajos de copia, mover y borrar lo dicen en la fila del error de la
//! ventana de copia (`held_by` y `InUseDialog::holder_line`).

use std::path::PathBuf;

use crate::ops::Op;
use crate::ops;
use crate::sys::Holder;

/// Un archivo que no se pudo tocar y los programas que lo tienen.
#[derive(Clone, Debug, PartialEq)]
pub struct InUseItem {
    pub path: PathBuf,
    pub holders: Vec<Holder>,
}

/// Una operación suelta que falló por archivos en uso.
#[derive(Clone, Debug, PartialEq)]
pub struct InUseFailure {
    /// La operación que falló (para reintentarla).
    pub op: Op,
    pub items: Vec<InUseItem>,
}

/// Cuántos programas se listan en la ventana; el resto va en «y N más».
pub const MAX_GROUPS: usize = 4;
/// Cuántos archivos se listan por programa; el resto va en «y N más».
pub const MAX_FILES: usize = 5;

/// Qué se reintenta con el botón «Reintentar».
#[derive(Clone, Debug, PartialEq)]
pub enum InUseRetry {
    /// Una operación suelta: se vuelve a lanzar.
    Op(Op),
}

/// Un programa que tiene archivos y cuáles (los primeros `MAX_FILES`).
#[derive(Clone, Debug, PartialEq)]
pub struct HolderGroup {
    pub holder: Holder,
    /// Nombres de los archivos que tiene (a lo sumo `MAX_FILES`).
    pub files: Vec<String>,
    /// Cuántos archivos más tiene que no entran en `files`.
    pub more_files: usize,
}

/// Lo que muestra la ventana de «archivo en uso».
#[derive(Clone, Debug, PartialEq)]
pub struct InUseDialog {
    pub title: String,
    /// Los programas, en el orden en que aparecen (a lo sumo `MAX_GROUPS`).
    pub groups: Vec<HolderGroup>,
    /// Cuántos programas más hay que no entran en `groups`.
    pub more_groups: usize,
    pub retry: InUseRetry,
}

impl InUseDialog {
    /// La ventana de una operación suelta que falló; `f.items` no está vacío y tiene holders.
    pub fn from_op(f: &InUseFailure) -> InUseDialog {
        let path = f.items.first().map(|i| i.path.clone()).unwrap_or_default();
        let (groups, more_groups) = split_groups(group_all(f.items.iter().map(|i| (i.path.clone(), crate::sys::shown_name(&i.path), i.holders.as_slice()))));
        InUseDialog { title: format!("No se pudo {}", ops::doing(&f.op, &path)), groups, more_groups, retry: InUseRetry::Op(f.op.clone()) }
    }

    /// Hay más de un programa (los listados y los que no entran): decide el singular o el plural.
    fn several(&self) -> bool {
        self.groups.len() + self.more_groups > 1
    }

    /// La frase que antecede a la lista: singular con un solo programa, plural con varios.
    pub fn intro(&self) -> &'static str {
        if self.several() { "Lo están usando otros programas:" } else { "Lo está usando otro programa:" }
    }

    /// La frase de cierre, con el mismo criterio de singular y plural que `intro`.
    pub fn outro(&self) -> &'static str {
        if self.several() { "Cerralos y reintentá." } else { "Cerralo y reintentá." }
    }

    /// «Microsoft Excel (EXCEL.EXE)»; solo el nombre si no hay ejecutable o es el mismo.
    /// Sin nombre, lo de `holder_name` (el ejecutable o «Proceso <pid>»).
    pub fn holder_label(h: &Holder) -> String {
        match exe_of(h) {
            Some(exe) if !h.name.trim().is_empty() && exe != h.name => format!("{} ({exe})", h.name),
            _ => holder_name(h),
        }
    }

    /// «PID 4812 · usuario PC\Leandro» («usuario desconocido» si no se pudo leer).
    pub fn holder_line(h: &Holder) -> String {
        format!("PID {} · usuario {}", h.pid, h.user.as_deref().unwrap_or("desconocido"))
    }
}

/// El ejecutable del programa, si se sabe (vacío cuenta como que no).
fn exe_of(h: &Holder) -> Option<&str> {
    h.exe.as_deref().filter(|e| !e.trim().is_empty())
}

/// El nombre del programa para mostrar: el amigable («PowerShell 7»); sin él, el ejecutable, y sin
/// ninguno de los dos, «Proceso <pid>».
pub fn holder_name(h: &Holder) -> String {
    if !h.name.trim().is_empty() {
        return h.name.clone();
    }
    match exe_of(h) {
        Some(exe) => exe.to_string(),
        None => format!("Proceso {}", h.pid),
    }
}

/// Lo que dice la fila de un error de la ventana de copia cuando se sabe quién tiene el archivo:
/// «En uso por PowerShell 7», con « y N más» si lo tienen otros procesos (distintos por `pid`).
/// `None` si no se sabe quién lo tiene.
pub fn held_by(holders: &[Holder]) -> Option<String> {
    let first = holders.first()?;
    let mut pids: Vec<u32> = holders.iter().map(|h| h.pid).collect();
    pids.sort_unstable();
    pids.dedup();
    let name = holder_name(first);
    Some(match pids.len() - 1 {
        0 => format!("En uso por {name}"),
        more => format!("En uso por {name} y {more} más"),
    })
}

/// Todos los programas con todos sus archivos, por `pid` y en orden de aparición. Cada archivo es
/// `(clave, nombre a mostrar, holders)`: la clave (su ruta) los distingue y el nombre solo se muestra.
fn group_all<'a>(items: impl Iterator<Item = (PathBuf, String, &'a [Holder])>) -> Vec<(Holder, Vec<String>)> {
    // El programa, las claves de sus archivos y los nombres que se muestran.
    let mut all: Vec<(Holder, Vec<PathBuf>, Vec<String>)> = Vec::new();
    for (key, name, holders) in items {
        for h in holders {
            match all.iter_mut().find(|(g, _, _)| g.pid == h.pid) {
                Some((_, keys, files)) => {
                    if !keys.contains(&key) {
                        keys.push(key.clone());
                        files.push(name.clone());
                    }
                }
                None => all.push((h.clone(), vec![key.clone()], vec![name.clone()])),
            }
        }
    }
    all.into_iter().map(|(h, _, files)| (h, files)).collect()
}

/// Aplica los límites: `MAX_GROUPS` programas con `MAX_FILES` archivos cada uno; devuelve los que sobran.
fn split_groups(all: Vec<(Holder, Vec<String>)>) -> (Vec<HolderGroup>, usize) {
    let more_groups = all.len().saturating_sub(MAX_GROUPS);
    let groups = all
        .into_iter()
        .take(MAX_GROUPS)
        .map(|(holder, mut files)| {
            let more_files = files.len().saturating_sub(MAX_FILES);
            files.truncate(MAX_FILES);
            HolderGroup { holder, files, more_files }
        })
        .collect();
    (groups, more_groups)
}

/// Agrupa por programa (mismo `pid`) con los límites `MAX_GROUPS` y `MAX_FILES`; los programas que
/// sobran no se devuelven (ver `from_op`, que cuenta `more_groups`).
#[cfg(test)]
fn group(items: &[InUseItem]) -> Vec<HolderGroup> {
    split_groups(group_all(items.iter().map(|i| (i.path.clone(), crate::sys::shown_name(&i.path), i.holders.as_slice())))).0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holder(pid: u32) -> Holder {
        Holder { name: format!("Prog{pid}"), exe: None, pid, user: None }
    }

    fn item(name: &str, pids: &[u32]) -> InUseItem {
        InUseItem { path: PathBuf::from(format!("/u/{name}")), holders: pids.iter().map(|p| holder(*p)).collect() }
    }

    #[test]
    fn group_merges_files_of_the_same_process() {
        let g = group(&[item("a.txt", &[1]), item("b.txt", &[1, 2]), item("c.txt", &[2])]);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].holder.pid, 1);
        assert_eq!(g[0].files, vec!["a.txt", "b.txt"]);
        assert_eq!(g[1].holder.pid, 2);
        assert_eq!(g[1].files, vec!["b.txt", "c.txt"]);
    }

    #[test]
    fn group_caps_programs_and_files() {
        let mut items: Vec<InUseItem> = (0..9).map(|i| item(&format!("f{i}"), &[1])).collect();
        items.extend((2..=7).map(|p| item(&format!("g{p}"), &[p])));
        let d = InUseDialog { title: String::new(), groups: group(&items), more_groups: 0, retry: InUseRetry::Op(Op::NewFolder { parent: PathBuf::from("/u") }) };
        assert_eq!(d.groups.len(), MAX_GROUPS);
        assert_eq!(d.groups[0].files.len(), MAX_FILES);
        assert_eq!(d.groups[0].more_files, 4);
    }

    #[test]
    fn group_handles_200_files_from_3_processes() {
        let items: Vec<InUseItem> = (0..200).map(|i| item(&format!("f{i}"), &[(i % 3) as u32 + 1])).collect();
        let g = group(&items);
        assert_eq!(g.len(), 3);
        // 200 = 67 + 67 + 66
        assert_eq!(g.iter().map(|x| x.files.len()).collect::<Vec<_>>(), vec![5, 5, 5]);
        assert_eq!(g.iter().map(|x| x.more_files).collect::<Vec<_>>(), vec![62, 62, 61]);
    }

    #[test]
    fn label_shows_exe_only_when_it_adds_something() {
        let mut h = Holder { name: "Microsoft Excel".into(), exe: Some("EXCEL.EXE".into()), pid: 1, user: None };
        assert_eq!(InUseDialog::holder_label(&h), "Microsoft Excel (EXCEL.EXE)");
        h.exe = Some("Microsoft Excel".into());
        assert_eq!(InUseDialog::holder_label(&h), "Microsoft Excel");
        h.exe = None;
        assert_eq!(InUseDialog::holder_label(&h), "Microsoft Excel");
    }

    #[test]
    fn label_without_name_uses_exe_or_pid() {
        let mut h = Holder { name: "  ".into(), exe: Some("EXCEL.EXE".into()), pid: 77, user: None };
        assert_eq!(InUseDialog::holder_label(&h), "EXCEL.EXE");
        h.exe = None;
        assert_eq!(InUseDialog::holder_label(&h), "Proceso 77");
        h.name = String::new();
        h.exe = Some(String::new());
        assert_eq!(InUseDialog::holder_label(&h), "Proceso 77");
    }

    #[test]
    fn line_says_unknown_user() {
        let mut h = Holder { name: "x".into(), exe: None, pid: 4812, user: Some(r"PC\Leandro".into()) };
        assert_eq!(InUseDialog::holder_line(&h), r"PID 4812 · usuario PC\Leandro");
        h.user = None;
        assert_eq!(InUseDialog::holder_line(&h), "PID 4812 · usuario desconocido");
    }

    #[test]
    fn from_op_title_names_the_action() {
        let f = InUseFailure {
            op: Op::Delete { paths: vec![PathBuf::from("/u/informe.xlsx")], recycle: false },
            items: vec![item("informe.xlsx", &[1])],
        };
        let d = InUseDialog::from_op(&f);
        assert_eq!(d.title, "No se pudo eliminar «informe.xlsx»");
        assert_eq!(d.retry, InUseRetry::Op(f.op.clone()));
        assert_eq!(d.groups.len(), 1);
    }

    #[test]
    fn from_op_counts_hidden_programs_and_files() {
        let op = Op::NewFolder { parent: PathBuf::from("/u") };
        // 7 programas, uno por archivo: se listan 4 y quedan 3 en «y N programas más».
        let items: Vec<InUseItem> = (1..=7).map(|p| item(&format!("f{p}"), &[p])).collect();
        let d = InUseDialog::from_op(&InUseFailure { op: op.clone(), items });
        assert_eq!(d.groups.len(), MAX_GROUPS);
        assert_eq!(d.more_groups, 3);
        // 8 archivos del mismo programa: se listan 5 y quedan 3 en «y N más».
        let items: Vec<InUseItem> = (0..8).map(|i| item(&format!("g{i}"), &[1])).collect();
        let d = InUseDialog::from_op(&InUseFailure { op, items });
        assert_eq!((d.groups.len(), d.more_groups), (1, 0));
        assert_eq!(d.groups[0].files.len(), MAX_FILES);
        assert_eq!(d.groups[0].more_files, 3);
    }

    #[test]
    fn from_op_keeps_same_leaf_files_apart() {
        let at = |dir: &str| InUseItem { path: PathBuf::from(format!("/u/{dir}/config.json")), holders: vec![holder(1)] };
        let op = Op::Delete { paths: vec![PathBuf::from("/u/a/config.json"), PathBuf::from("/u/b/config.json")], recycle: false };
        let d = InUseDialog::from_op(&InUseFailure { op: op.clone(), items: vec![at("a"), at("b")] });
        assert_eq!(d.groups.len(), 1);
        assert_eq!(d.groups[0].files, vec!["config.json", "config.json"]);
        // El mismo archivo dos veces sigue contando uno.
        let d = InUseDialog::from_op(&InUseFailure { op, items: vec![at("a"), at("a")] });
        assert_eq!(d.groups[0].files.len(), 1);
    }

    #[test]
    fn name_and_held_by() {
        let mut h = Holder { name: "PowerShell 7".into(), exe: Some("pwsh.exe".into()), pid: 24012, user: None };
        assert_eq!(holder_name(&h), "PowerShell 7");
        assert_eq!(held_by(std::slice::from_ref(&h)).as_deref(), Some("En uso por PowerShell 7"));
        assert_eq!(held_by(&[h.clone(), h.clone(), holder(9)]).as_deref(), Some("En uso por PowerShell 7 y 1 más"));
        assert_eq!(held_by(&[]), None);
        h.name = " ".into();
        assert_eq!(holder_name(&h), "pwsh.exe");
        h.exe = None;
        assert_eq!(holder_name(&h), "Proceso 24012");
    }

    #[test]
    fn intro_outro_singular_and_plural() {
        let one = InUseDialog::from_op(&InUseFailure { op: Op::NewFolder { parent: PathBuf::from("/u") }, items: vec![item("a", &[1])] });
        assert_eq!(one.intro(), "Lo está usando otro programa:");
        assert_eq!(one.outro(), "Cerralo y reintentá.");
        let two = InUseDialog::from_op(&InUseFailure { op: Op::NewFolder { parent: PathBuf::from("/u") }, items: vec![item("a", &[1, 2])] });
        assert_eq!(two.intro(), "Lo están usando otros programas:");
        assert_eq!(two.outro(), "Cerralos y reintentá.");
    }
}
