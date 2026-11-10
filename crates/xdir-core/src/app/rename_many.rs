//! Renombrar varios en la `App`: el panel de la derecha (en el lugar de la vista previa, con su
//! ancho y su borde), sus cajas de texto, la vista previa en la lista y aplicar o deshacer en un
//! hilo (`rename_many::rename_all`). Los elementos y su orden son los de la lista visible al
//! abrirlo; si la carpeta se recarga, los que desaparecieron salen.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use super::preview::RightPanel;
use super::tasks::TaskDone;
use super::*;
use crate::format::{DAYS_1601_TO_1970, civil_from_days};
use crate::rename_many::{Clash, FormatStyle, Mode, Source, SysRenames, clashes, compute, rename_all};
use crate::rename_many_draw::{self as panel, Field, ModeKind, PanelHit, PanelState};

/// Lo que se calculó para un elemento: posición en `entries`, nombre viejo, nuevo y choque.
type Row = (usize, String, String, Option<Clash>);

/// Renombrar varios abierto.
pub(super) struct RenameMany {
    /// Pestaña (y carpeta) cuyos elementos se renombran.
    pub pane: usize,
    pub tab_id: usize,
    pub dir: PathBuf,
    /// Los nombres, en el orden de la lista al abrirlo.
    pub items: Vec<OsString>,
    pub mode: ModeKind,
    pub texts: HashMap<Field, String>,
    pub match_case: bool,
    pub add_before: bool,
    pub style: FormatStyle,
    pub format_before: bool,
    pub case_insensitive: bool,
    /// Cajas de texto abiertas y el lugar del panel con que se crearon.
    pub boxes: Vec<(Field, TextBox)>,
    pub boxes_at: Option<(geom::Rect, ModeKind, FormatStyle)>,
    /// Lo último que se calculó.
    pub rows: Vec<Row>,
}

impl RenameMany {
    fn text(&self, f: Field) -> &str {
        self.texts.get(&f).map_or("", String::as_str)
    }

    fn mode(&self) -> Mode {
        match self.mode {
            ModeKind::Replace => Mode::Replace { find: self.text(Field::Find).to_string(), with: self.text(Field::With).to_string(), match_case: self.match_case },
            ModeKind::Add => Mode::Add { text: self.text(Field::Text).to_string(), before: self.add_before },
            ModeKind::Format => Mode::Format {
                style: self.style,
                base: self.text(Field::Base).trim().to_string(),
                before: self.format_before,
                start: self.text(Field::Start).trim().parse().unwrap_or(1),
            },
        }
    }
}

/// Día local (año, mes, día) de un FILETIME con la diferencia `offset` de la zona horaria.
fn local_day(filetime: u64, offset: i64) -> (i32, u8, u8) {
    let secs = (filetime / 10_000_000) as i64 + offset;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400) - DAYS_1601_TO_1970);
    (y as i32, m as u8, d as u8)
}

impl App {
    /// ¿Se puede abrir Renombrar varios en ese panel? (una carpeta del disco, sin búsqueda.)
    pub(super) fn can_rename_many(&self, pane: usize) -> bool {
        let tab = self.panes[pane].tab();
        matches!(tab.location, Location::Path(_)) && tab.search.is_none() && !tab.list.selected_positions().is_empty()
    }

    /// F2: con más de un seleccionado, Renombrar varios; si no, el renombrado en línea.
    pub(super) fn rename_command(&mut self, pane: usize) {
        // Cuentan los seleccionados a la vista (los que un filtro oculta no se renombran).
        if self.panes[pane].tab().list.selected_positions().len() > 1 && self.can_rename_many(pane) {
            self.open_rename_many(pane);
        } else {
            self.begin_rename(pane);
        }
    }

    /// Abre el panel con los seleccionados del panel, en el orden de la lista.
    pub(super) fn open_rename_many(&mut self, pane: usize) {
        if !self.can_rename_many(pane) {
            self.host.beep();
            return;
        }
        // Hay un solo panel a la vez: si estaba el de información, se cierra.
        self.close_rename_many();
        self.close_info();
        self.end_rename(false);
        self.end_edit(false);
        self.end_filter(true);
        self.close_quick_look();
        let tab = self.panes[pane].tab();
        let Location::Path(dir) = tab.location.clone() else { return };
        let list = &tab.list;
        let items: Vec<OsString> =
            list.selected_positions().into_iter().filter_map(|p| list.entry_of_pos(p)).map(|i| list.entries[i].name.clone()).collect();
        self.rename_many = Some(RenameMany {
            pane,
            tab_id: tab.id,
            dir: dir.clone(),
            items,
            mode: ModeKind::Replace,
            texts: HashMap::from([(Field::Start, "1".to_string())]),
            match_case: true,
            add_before: true,
            style: FormatStyle::Number,
            format_before: false,
            // Hasta saberlo, como un disco que no distingue: se marcan más choques, nunca menos.
            case_insensitive: true,
            boxes: Vec::new(),
            boxes_at: None,
            rows: Vec::new(),
        });
        self.preview.owner = RightPanel::RenameMany;
        self.set_active(pane);
        self.spawn_task(move || TaskDone::CaseInfo { case_insensitive: sys::file_system().case_insensitive(&dir), dir });
        self.refresh_rename_many();
        self.sync_rename_boxes();
        if let Some((_, b)) = self.rename_many.as_ref().and_then(|r| r.boxes.first().copied()) {
            self.host.text_focus(b);
        }
        self.redraw();
    }

    /// Cierra el panel (Esc, Cancelar, ✕ o después de aplicar): la lista vuelve a la normal y, si
    /// estaba prendida, vuelve la vista previa.
    pub(super) fn close_rename_many(&mut self) {
        let Some(st) = self.rename_many.take() else { return };
        for (_, b) in st.boxes {
            self.host.text_destroy(b);
        }
        for pane in &mut self.panes {
            for tab in &mut pane.tabs {
                if tab.id == st.tab_id {
                    tab.list.set_rename_preview(None);
                }
            }
        }
        if self.preview.owner == RightPanel::RenameMany {
            self.preview.owner = RightPanel::Preview;
        }
        self.host.focus_main();
        self.sync_preview();
        self.redraw();
    }

    /// Recalcula los nombres nuevos y los choques, y se los pasa a la lista. Si la pestaña ya no
    /// muestra la carpeta (o no quedó ninguno de los elementos), cierra el panel.
    pub(super) fn refresh_rename_many(&mut self) {
        let Some(st) = &self.rename_many else { return };
        let tab = self.panes[st.pane].tab();
        let shown = st.pane < self.layout.panes();
        if !shown || tab.id != st.tab_id || !tab.location.is_path(&st.dir) || tab.search.is_some() {
            self.close_rename_many();
            return;
        }
        if tab.loading && tab.list.entries.is_empty() {
            return;
        }
        let list = &tab.list;
        let offset = sys::env().local_now().offset;
        let index: HashMap<&OsString, usize> = list.entries.iter().enumerate().map(|(i, e)| (&e.name, i)).collect();
        let found: Vec<(OsString, usize)> = st.items.iter().filter_map(|n| index.get(n).map(|&i| (n.clone(), i))).collect();
        if found.is_empty() && !tab.loading {
            self.close_rename_many();
            return;
        }
        let sources: Vec<Source> = found
            .iter()
            .map(|(_, i)| {
                let e = &list.entries[*i];
                Source { name: e.name.to_string_lossy().into_owned(), is_dir: e.is_dir, modified_day: local_day(e.modified, offset) }
            })
            .collect();
        let old: Vec<String> = sources.iter().map(|s| s.name.clone()).collect();
        let mut new = compute(&sources, &st.mode());
        // Los nombres que no son texto válido no se tocan.
        for (k, (n, _)) in found.iter().enumerate() {
            if n.to_str().is_none() {
                new[k] = old[k].clone();
            }
        }
        let taken: std::collections::HashSet<usize> = found.iter().map(|(_, i)| *i).collect();
        let others: Vec<String> =
            list.entries.iter().enumerate().filter(|(i, _)| !taken.contains(i)).map(|(_, e)| e.name.to_string_lossy().into_owned()).collect();
        let marks = clashes(&old, &new, &others, st.case_insensitive, &sys::file_system().name_rules());
        let rows: Vec<Row> = found.iter().zip(old).zip(new).zip(marks).map(|((((_, i), o), n), c)| (*i, o, n, c)).collect();
        let preview = rows.iter().map(|(i, _, n, c)| (*i, n.clone(), *c)).collect();
        let (pane, items) = (st.pane, found.into_iter().map(|(n, _)| n).collect());
        self.panes[pane].tab_mut().list.set_rename_preview(Some(preview));
        if let Some(st) = &mut self.rename_many {
            st.rows = rows;
            st.items = items;
        }
    }

    /// Lo que dibuja el panel.
    pub(super) fn rename_panel_state(&self) -> Option<PanelState> {
        let st = self.rename_many.as_ref()?;
        let changing = st.rows.iter().filter(|(_, o, n, _)| o != n).count();
        let bad: Vec<&Row> = st.rows.iter().filter(|r| r.3.is_some()).collect();
        let mut clash = bad.first().map(|(_, _, n, c)| {
            let s = if bad.len() == 1 { "" } else { "s" };
            format!("{} choque{s}: {}", bad.len(), c.unwrap().short(n))
        });
        // «Número inicial» que no es un número: se avisa y no se aplica.
        let bad_start = st.mode == ModeKind::Format && st.style != FormatStyle::Date && st.text(Field::Start).trim().parse::<u32>().is_err();
        if bad_start && clash.is_none() {
            clash = Some("«Número inicial» tiene que ser un número".to_string());
        }
        Some(PanelState {
            count: st.rows.len(),
            mode: st.mode,
            match_case: st.match_case,
            add_before: st.add_before,
            style: st.style,
            format_before: st.format_before,
            changing,
            can_apply: changing > 0 && bad.is_empty() && !bad_start,
            clash,
        })
    }

    /// Rectángulo del panel, si se ve.
    pub(super) fn rename_panel_rect(&self) -> Option<geom::Rect> {
        self.rename_many.as_ref()?;
        self.preview_column()
    }

    /// Qué hay del panel en `(x, y)`.
    pub(super) fn rename_panel_hit(&self, x: f32, y: f32) -> Option<PanelHit> {
        panel::hit(self.rename_panel_rect()?, &self.rename_panel_state()?, x, y)
    }

    /// Las cajas de texto siguen al panel: se crean (o se vuelven a crear donde va) si cambió el
    /// lugar del panel, el modo o el estilo; sin panel a la vista, se cierran.
    pub(super) fn sync_rename_boxes(&mut self) {
        let (Some(r), Some(ps)) = (self.rename_panel_rect(), self.rename_panel_state()) else {
            if let Some(st) = &mut self.rename_many {
                for (_, b) in std::mem::take(&mut st.boxes) {
                    self.host.text_destroy(b);
                }
                st.boxes_at = None;
            }
            return;
        };
        let Some(st) = &mut self.rename_many else { return };
        let want = (r, st.mode, st.style);
        if st.boxes_at == Some(want) {
            return;
        }
        // Lo escrito se guarda y la caja con el foco lo vuelve a tener.
        let mut focused = None;
        for (f, b) in std::mem::take(&mut st.boxes) {
            st.texts.insert(f, self.host.text_get(b));
            if self.host.text_has_focus(b) {
                focused = Some(f);
            }
            self.host.text_destroy(b);
        }
        for (f, fr) in panel::fields(r, &ps) {
            let inner = geom::rect(fr.left + 4.0, fr.top + 4.0, fr.right - 4.0, fr.bottom - 4.0);
            let text = st.texts.get(&f).cloned().unwrap_or_default();
            if let Some(b) = self.host.text_create(TextKind::Field, inner, &text, &self.theme) {
                st.boxes.push((f, b));
            }
        }
        st.boxes_at = Some(want);
        if let Some(b) = focused.and_then(|f| st.boxes.iter().find(|(x, _)| *x == f)).map(|(_, b)| *b) {
            self.host.text_focus(b);
        }
    }

    /// Cambió el texto de una caja; `true` si era del panel.
    pub(super) fn rename_text_changed(&mut self, id: isize) -> bool {
        let Some(st) = &mut self.rename_many else { return false };
        let Some(&(f, b)) = st.boxes.iter().find(|(_, b)| b.0 as isize == id) else { return false };
        st.texts.insert(f, self.host.text_get(b));
        self.refresh_rename_many();
        self.redraw();
        true
    }

    /// Enter (aplicar) o Escape (cerrar) en una caja del panel.
    pub(super) fn on_field_done(&mut self, commit: bool) {
        if self.connect_field_done(commit) {
            return;
        }
        if commit {
            self.apply_rename_many();
        } else {
            self.close_rename_many();
        }
    }

    /// Tab (o Shift+Tab con `back`) en una caja del panel: el foco pasa a la siguiente (o a la
    /// anterior), dando la vuelta.
    pub(super) fn on_field_tab(&mut self, back: bool) {
        let Some(st) = &self.rename_many else { return };
        let n = st.boxes.len();
        if n == 0 {
            return;
        }
        let at = st.boxes.iter().position(|(_, b)| self.host.text_has_focus(*b));
        let next = match (at, back) {
            (Some(i), false) => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
            (None, false) => 0,
            (None, true) => n - 1,
        };
        self.host.text_focus(st.boxes[next].1);
    }

    /// Clic en el panel.
    pub(super) fn rename_panel_click(&mut self, h: PanelHit) {
        let Some(st) = &mut self.rename_many else { return };
        match h {
            PanelHit::Close | PanelHit::Cancel => return self.close_rename_many(),
            PanelHit::Apply => return self.apply_rename_many(),
            PanelHit::Mode(m) => st.mode = m,
            PanelHit::Style(s) => st.style = s,
            PanelHit::Before(b) if st.mode == ModeKind::Add => st.add_before = b,
            PanelHit::Before(b) => st.format_before = b,
            PanelHit::MatchCase => st.match_case = !st.match_case,
            PanelHit::Panel => return,
        }
        self.sync_rename_boxes();
        if matches!(h, PanelHit::Mode(_))
            && let Some((_, b)) = self.rename_many.as_ref().and_then(|r| r.boxes.first().copied())
        {
            self.host.text_focus(b);
        }
        self.refresh_rename_many();
        self.redraw();
    }

    /// «Renombrar»: corre `rename_all` en un hilo (si no hay choques y algo cambia) y cierra.
    pub(super) fn apply_rename_many(&mut self) {
        if self.refuse_while_renaming() {
            return;
        }
        // Lo último escrito, por si el aviso de la caja todavía no llegó.
        if let Some(st) = &mut self.rename_many {
            for &(f, b) in &st.boxes {
                st.texts.insert(f, self.host.text_get(b));
            }
        }
        self.refresh_rename_many();
        if !self.rename_panel_state().is_some_and(|s| s.can_apply) {
            self.host.beep();
            return;
        }
        let Some(st) = &self.rename_many else { return };
        let (dir, pane) = (st.dir.clone(), st.pane);
        let pairs: Vec<(PathBuf, String)> = st.rows.iter().filter(|(_, o, n, _)| o != n).map(|(_, o, n, _)| (dir.join(o), n.clone())).collect();
        let first = pairs.first().map(|(_, n)| n.clone());
        self.close_rename_many();
        if let Some(name) = first {
            let tab = self.panes[pane].tab_mut();
            tab.focus_after_load(OsString::from(name));
        }
        self.renaming = true;
        self.spawn_task(move || TaskDone::Renamed { result: rename_all(&SysRenames, &pairs), dir, undoing: None });
    }

    /// Con un renombrado corriendo no se larga otro (dos a la vez en la misma carpeta podrían
    /// cruzarse): pita y avisa. `true` si se rechazó.
    fn refuse_while_renaming(&mut self) -> bool {
        if self.renaming {
            self.host.beep();
            self.notice = Some("Todavía se está renombrando; esperá a que termine".to_string());
            self.redraw();
        }
        self.renaming
    }

    /// Ctrl+Z de un Renombrar varios: el mismo cruce seguro, en un hilo (no por `FileOps`).
    pub(super) fn undo_renames(&mut self, renames: Vec<(PathBuf, String)>) {
        if self.refuse_while_renaming() {
            // Queda para después.
            self.undo.push(Undo::Renames(renames));
            return;
        }
        let dir = renames.first().and_then(|(p, _)| p.parent()).map(Path::to_path_buf).unwrap_or_default();
        self.renaming = true;
        self.spawn_task(move || {
            let result = rename_all(&SysRenames, &renames);
            TaskDone::Renamed { result, dir, undoing: Some(renames) }
        });
    }

    /// Terminó un renombrado (o su deshacer): se apila el deshacer, se avisa el error y se recargan
    /// las pestañas de esa carpeta.
    pub(super) fn on_renamed(&mut self, dir: PathBuf, result: Result<Undo, String>, undoing: Option<Vec<(PathBuf, String)>>) {
        self.renaming = false;
        match (result, undoing) {
            (Ok(undo), None) => {
                self.notice = None;
                self.undo.push(undo);
                if self.undo.len() > UNDO_MAX {
                    self.undo.remove(0);
                }
            }
            (Ok(_), Some(_)) => self.notice = None,
            (Err(e), None) => self.notice = Some(format!("No se pudo renombrar: {e}")),
            // El deshacer que falló vuelve a la pila (`rename_all` dejó todo como estaba o dice qué no).
            (Err(e), Some(renames)) => {
                self.undo.push(Undo::Renames(renames));
                self.notice = Some(format!("No se pudo deshacer el renombrado: {e}"));
            }
        }
        let (hwnd, tx) = (self.hwnd, self.tx.clone());
        for tab in self.panes.iter_mut().flat_map(|p| p.tabs.iter_mut()) {
            if tab.location.is_path(&dir) && tab.is_loaded() {
                tab.reload(hwnd, &tx, true);
            }
        }
        self.redraw();
    }

    /// Llegó si el disco distingue mayúsculas: se recalculan los choques.
    pub(super) fn on_case_info(&mut self, dir: &Path, case_insensitive: bool) {
        if let Some(st) = self.rename_many.as_mut().filter(|s| s.dir == dir) {
            st.case_insensitive = case_insensitive;
            self.refresh_rename_many();
            self.redraw();
        }
    }
}
