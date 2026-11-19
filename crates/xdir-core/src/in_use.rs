//! Cuando algo falla porque otro programa tiene abierto el archivo (`ErrKind::InUse`): qué se
//! intentaba hacer y quién tiene cada archivo, para mostrarlo en la ventana de «archivo en uso».

use std::path::PathBuf;

use crate::ops::Op;
use crate::ops;
use crate::sys::Holder;
use crate::transfer::{JobError, JobId};

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
    /// Un trabajo de copia/mover/borrar: se reintentan sus errores.
    Job(JobId),
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
        let (groups, more_groups) = split_groups(group_all(&f.items));
        InUseDialog { title: format!("No se pudo {}", ops::doing(&f.op, &path)), groups, more_groups, retry: InUseRetry::Op(f.op.clone()) }
    }

    /// La ventana de un trabajo cuyos errores nombran quién tiene los archivos; `None` si ningún
    /// error trae holders (no hay nada que mostrar).
    pub fn from_job(id: JobId, errors: &[JobError]) -> Option<InUseDialog> {
        let with: Vec<&JobError> = errors.iter().filter(|e| !e.holders.is_empty()).collect();
        let first = with.first()?;
        let title = if with.len() == 1 { format!("«{}» está en uso", first.shown_name()) } else { format!("{} elementos están en uso", with.len()) };
        let items: Vec<InUseItem> = with.iter().map(|e| InUseItem { path: PathBuf::from(e.shown_name()), holders: e.holders.clone() }).collect();
        let (groups, more_groups) = split_groups(group_all(&items));
        Some(InUseDialog { title, groups, more_groups, retry: InUseRetry::Job(id) })
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
    pub fn holder_label(h: &Holder) -> String {
        match &h.exe {
            Some(exe) if *exe != h.name => format!("{} ({exe})", h.name),
            _ => h.name.clone(),
        }
    }

    /// «PID 4812 · usuario PC\Leandro» («usuario desconocido» si no se pudo leer).
    pub fn holder_line(h: &Holder) -> String {
        format!("PID {} · usuario {}", h.pid, h.user.as_deref().unwrap_or("desconocido"))
    }
}

/// Todos los programas con todos sus archivos, por `pid` y en orden de aparición.
fn group_all(items: &[InUseItem]) -> Vec<(Holder, Vec<String>)> {
    let mut all: Vec<(Holder, Vec<String>)> = Vec::new();
    for it in items {
        let name = crate::sys::shown_name(&it.path);
        for h in &it.holders {
            match all.iter_mut().find(|(g, _)| g.pid == h.pid) {
                Some((_, files)) => {
                    if !files.contains(&name) {
                        files.push(name.clone());
                    }
                }
                None => all.push((h.clone(), vec![name.clone()])),
            }
        }
    }
    all
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
/// sobran no se devuelven (ver `from_op` y `from_job`, que cuentan `more_groups`).
#[cfg(test)]
fn group(items: &[InUseItem]) -> Vec<HolderGroup> {
    split_groups(group_all(items)).0
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

    fn job_error(name: &str, holders: Vec<Holder>) -> JobError {
        JobError { src: PathBuf::from(format!("/u/{name}")), dst: PathBuf::new(), message: String::new(), retry: true, entry: None, holders }
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
        let d = InUseDialog { title: String::new(), groups: group(&items), more_groups: 0, retry: InUseRetry::Job(1) };
        assert_eq!(d.groups.len(), MAX_GROUPS);
        assert_eq!(d.groups[0].files.len(), MAX_FILES);
        assert_eq!(d.groups[0].more_files, 4);
    }

    #[test]
    fn from_job_counts_hidden_programs() {
        let errs: Vec<JobError> = (1..=7).map(|p| job_error(&format!("f{p}"), vec![holder(p)])).collect();
        let d = InUseDialog::from_job(3, &errs).unwrap();
        assert_eq!(d.groups.len(), 4);
        assert_eq!(d.more_groups, 3);
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
    fn from_job_none_without_holders() {
        assert!(InUseDialog::from_job(1, &[job_error("a", vec![])]).is_none());
        assert!(InUseDialog::from_job(1, &[]).is_none());
    }

    #[test]
    fn from_job_titles_one_and_many() {
        let one = InUseDialog::from_job(7, &[job_error("a.xlsx", vec![holder(1)]), job_error("sin", vec![])]).unwrap();
        assert_eq!(one.title, "«a.xlsx» está en uso");
        assert_eq!(one.retry, InUseRetry::Job(7));
        let many = InUseDialog::from_job(7, &[job_error("a", vec![holder(1)]), job_error("b", vec![holder(1)])]).unwrap();
        assert_eq!(many.title, "2 elementos están en uso");
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
