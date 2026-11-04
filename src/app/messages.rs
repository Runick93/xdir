//! Despacho de mensajes de la ventana (`WM_*`) hacia los métodos de `App`.

use super::*;

impl App {
    pub(super) fn handle(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        unsafe {
            match msg {
                WM_PAINT => {
                    // BeginPaint valida antes de pintar (si `paint` pide otro frame, no se
                    // pierde) y dispara el borrado de fondo cuando hace falta.
                    let mut ps = PAINTSTRUCT::default();
                    BeginPaint(self.hwnd, &mut ps);
                    let _ = EndPaint(self.hwnd, &ps);
                    self.paint();
                    LRESULT(0)
                }
                WM_ERASEBKGND => {
                    // Con Mica, la superficie GDI de la ventana va en negro: con el marco
                    // extendido, el negro es transparente y deja ver la composición y el fondo.
                    // En modo opaco todo el fondo lo pinta Direct2D.
                    if self.transparent {
                        let mut rc = RECT::default();
                        let _ = GetClientRect(self.hwnd, &mut rc);
                        FillRect(HDC(wparam.0 as *mut _), &rc, HBRUSH(GetStockObject(BLACK_BRUSH).0));
                    }
                    LRESULT(1)
                }
                WM_SIZE => {
                    let width = (lparam.0 & 0xFFFF) as u32;
                    let height = ((lparam.0 >> 16) & 0xFFFF) as u32;
                    self.renderer.resize(width, height, self.dpi);
                    self.end_edit(false);
                    self.end_filter(true);
                    self.close_palette();
                    LRESULT(0)
                }
                WM_DPICHANGED => {
                    self.dpi = (wparam.0 & 0xFFFF) as u32;
                    self.end_edit(false);
                    self.end_filter(true);
                    self.close_palette();
                    self.renderer.set_dpi(self.dpi);
                    self.update_edit_resources();
                    let r = &*(lparam.0 as *const RECT);
                    let _ = SetWindowPos(
                        self.hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    LRESULT(0)
                }
                WM_GETMINMAXINFO => {
                    let info = &mut *(lparam.0 as *mut MINMAXINFO);
                    info.ptMinTrackSize.x = scale(560, self.dpi);
                    info.ptMinTrackSize.y = scale(360, self.dpi);
                    LRESULT(0)
                }
                WM_SETTINGCHANGE => {
                    // "Mostrar animaciones en Windows" (Accesibilidad → Efectos visuales).
                    self.apply_animation_setting();
                    // Windows avisa así cuando el usuario cambia entre modo claro y oscuro.
                    let area = PCWSTR(lparam.0 as *const u16);
                    if !area.is_null() && area.to_string().is_ok_and(|s| s == "ImmersiveColorSet") {
                        self.theme = Theme::from_system(self.accent, self.dark_mode);
                        let mica = self.theme.apply_to_window(self.hwnd);
                        self.renderer.set_transparent(mica);
                        self.transparent = mica;
                        self.update_edit_resources();
                        self.redraw();
                    }
                    LRESULT(0)
                }
                WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
                    let (x, y) = self.cursor_pos();
                    let splitter = match self.drag {
                        Drag::Splitter(s) => Some(s),
                        _ => match self.hit(x, y) {
                            Hit::Splitter(s) => Some(s),
                            _ => None,
                        },
                    };
                    // Borde de un encabezado de columna: cambiar el ancho.
                    let column_edge = match (self.drag, self.hit(x, y)) {
                        (Drag::List(i), _) | (_, Hit::Pane(i, PaneHit::List)) => self.panes[i].tab().list.wants_resize_cursor(x, y),
                        _ => false,
                    };
                    #[cfg(feature = "arbol")]
                    let tree_edge = matches!(self.drag, Drag::TreeEdge) || (matches!(self.drag, Drag::None) && self.tree.edge_at(x, y));
                    #[cfg(not(feature = "arbol"))]
                    let tree_edge = false;
                    let cursor = match splitter {
                        Some(Splitter::Vertical) => IDC_SIZEWE,
                        Some(Splitter::Horizontal) => IDC_SIZENS,
                        None if column_edge || tree_edge => IDC_SIZEWE,
                        None => IDC_ARROW,
                    };
                    if let Ok(c) = LoadCursorW(None, cursor) {
                        SetCursor(Some(c));
                    }
                    LRESULT(1)
                }
                // Se enchufó (0x8000) o se quitó (0x8004) una unidad: se actualiza como el Explorador.
                WM_DEVICECHANGE => {
                    if matches!(wparam.0, 0x8000 | 0x8004) {
                        self.drives_changed();
                    }
                    LRESULT(1)
                }
                WM_APP_LOAD => {
                    let t0 = Instant::now();
                    let mut changed = false;
                    let (hwnd, tx) = (self.hwnd, self.tx.clone());
                    while let Ok(event) = self.rx.try_recv() {
                        if let Some(tab) = self.tab_by_id(event.panel) {
                            changed |= tab.on_load(event);
                            tab.retry_with_credentials(hwnd, &tx);
                            if let Some(path) = tab.visited.take() {
                                self.history.record(&path, jump::now());
                            }
                        }
                    }
                    self.ui_load_ms += ms_since(t0);
                    if changed {
                        self.try_pending_rename();
                        self.try_new_watch();
                        self.redraw();
                    }
                    LRESULT(0)
                }
                icons::WM_APP_ICONS => {
                    self.redraw();
                    LRESULT(0)
                }
                WM_APP_CHANGED => {
                    let (hwnd, tx) = (self.hwnd, self.tx.clone());
                    if let Some(tab) = self.tab_by_id(wparam.0) {
                        tab.on_changed(hwnd, &tx, lparam.0 as u64);
                    }
                    LRESULT(0)
                }
                WM_APP_EDIT_DONE => {
                    self.end_edit(wparam.0 == 1);
                    LRESULT(0)
                }
                WM_CTLCOLOREDIT => {
                    let hdc = HDC(wparam.0 as *mut _);
                    SetTextColor(hdc, colorref(self.theme.text));
                    SetBkColor(hdc, colorref(self.theme.surface_alt));
                    LRESULT(self.edit_brush.0 as isize)
                }
                WM_APP_FAVEDIT_DONE => {
                    self.end_fav_edit(wparam.0 == 1);
                    LRESULT(0)
                }
                WM_APP_FILTER_DONE => {
                    self.end_filter(wparam.0 == 1);
                    LRESULT(0)
                }
                // El filtro y la paleta se actualizan con cada tecla.
                WM_COMMAND if (wparam.0 >> 16) & 0xFFFF == EN_CHANGE_CODE => {
                    if let Some((edit, pane)) = self.filter_edit {
                        if lparam.0 == edit.0 as isize {
                            let text = window_text(edit);
                            self.panes[pane].tab_mut().list.set_filter(&text);
                            self.redraw();
                        }
                    }
                    if self.palette.as_ref().is_some_and(|(_, e)| lparam.0 == e.0 as isize) {
                        self.refresh_palette();
                    }
                    if let Some((edit, _)) = self.edit {
                        if lparam.0 == edit.0 as isize {
                            let text = window_text(edit);
                            if let Some(sg) = self.suggest.as_mut().filter(|sg| !sg.quiet) {
                                sg.set_query(&text);
                                self.redraw();
                            }
                        }
                    }
                    LRESULT(0)
                }
                #[cfg(feature = "arbol")]
                tree::WM_APP_TREE => {
                    self.tree.on_loaded();
                    self.redraw();
                    LRESULT(0)
                }
                // Terminó de leerse el Acceso rápido de Windows (wparam 1), o el vigilante avisó que
                // cambió (wparam 0): se relee.
                #[cfg(feature = "arbol")]
                tree::WM_APP_QUICK => {
                    if wparam.0 == 1 {
                        if self.tree.on_quick_loaded() {
                            self.tree.request_quick(self.hwnd);
                        }
                    } else {
                        self.tree.request_quick(self.hwnd);
                    }
                    self.redraw();
                    LRESULT(0)
                }
                // Solo con `--features pruebas`: otro proceso maneja la app por texto (capturas y
                // pruebas automáticas sin mouse ni teclado reales).
                #[cfg(feature = "pruebas")]
                WM_COPYDATA => {
                    let cds = &*(lparam.0 as *const windows::Win32::System::DataExchange::COPYDATASTRUCT);
                    let wide = std::slice::from_raw_parts(cds.lpData as *const u16, cds.cbData as usize / 2);
                    let text = String::from_utf16_lossy(wide);
                    self.test_hook(text.trim_end_matches('\0'));
                    LRESULT(1)
                }
                WM_APP_SUGGEST_KEY => {
                    self.on_suggest_key(VIRTUAL_KEY(wparam.0 as u16));
                    LRESULT(0)
                }
                WM_APP_PALETTE_KEY => {
                    self.on_palette_key(VIRTUAL_KEY(wparam.0 as u16), lparam.0 == 1);
                    LRESULT(0)
                }
                WM_MENURBUTTONUP if self.fav_menu.is_some() => {
                    self.on_fav_menu_right_click(lparam.0, wparam.0 as u32);
                    LRESULT(0)
                }
                // Al volver a la ventana se releen los favoritos (por si se editó el archivo).
                WM_ACTIVATEAPP if wparam.0 != 0 => {
                    // Al volver a la ventana, el lector tiene que leer de nuevo el elemento con foco.
                    self.uia_focus = None;
                    #[cfg(feature = "arbol")]
                    self.tree.request_quick(self.hwnd);
                    let fresh = Favorites::load();
                    if fresh != self.favorites {
                        self.favorites = fresh;
                        self.redraw();
                    }
                    if self.probe.is_none() {
                        self.reload_config();
                    }
                    LRESULT(0)
                }
                WM_APP_RENAME_DONE => {
                    self.end_rename(wparam.0 == 1);
                    LRESULT(0)
                }
                WM_APP_OP_DONE => {
                    while let Ok(result) = self.ops_rx.try_recv() {
                        self.on_op_done(result);
                    }
                    self.redraw();
                    LRESULT(0)
                }
                WM_APP_DRAG => {
                    let ev = &mut *(lparam.0 as *mut DragEvent);
                    self.on_drag(ev);
                    LRESULT(0)
                }
                WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR => {
                    match shell::handle_menu_message(msg, wparam, lparam) {
                        Some(r) => r,
                        None => DefWindowProcW(self.hwnd, msg, wparam, lparam),
                    }
                }
                WM_RBUTTONDOWN => {
                    let (x, y) = self.mouse_pos(lparam);
                    let _ = SetFocus(Some(self.hwnd));
                    self.fav_ctx = None;
                    self.header_ctx = None;
                    self.term_ctx = None;
                    #[cfg(feature = "arbol")]
                    {
                        self.tree_ctx = self.tree.contains(x, y).then(|| self.tree.path_at(x, y));
                    }
                    match self.hit(x, y) {
                        Hit::Pane(i, PaneHit::List) if self.panes[i].tab().list.in_header(x, y) => {
                            self.set_active(i);
                            self.header_ctx = Some(i);
                        }
                        Hit::Pane(i, PaneHit::List) => {
                            self.set_active(i);
                            self.panes[i].tab_mut().list.on_right_down(x, y);
                            self.redraw();
                        }
                        Hit::TermMenu(Some(c)) => {
                            if let Some(pane) = self.term_menu {
                                self.term_ctx = Some((pane, c));
                            }
                        }
                        Hit::Pane(i, PaneHit::Fav(j)) => self.fav_ctx = Some((i, Some(vec![j]))),
                        Hit::Pane(i, PaneHit::FavBar | PaneHit::FavOverflow) => self.fav_ctx = Some((i, None)),
                        _ => {}
                    }
                    LRESULT(0)
                }
                WM_CONTEXTMENU => {
                    if let Some(pane) = self.header_ctx.take().filter(|_| lparam.0 != -1) {
                        let pt = POINT { x: (lparam.0 & 0xFFFF) as u16 as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32 };
                        self.show_columns_menu(pane, pt);
                        return LRESULT(0);
                    }
                    if let Some((pane, console)) = self.term_ctx.take().filter(|_| lparam.0 != -1) {
                        self.console_context_menu(pane, console);
                        return LRESULT(0);
                    }
                    #[cfg(feature = "arbol")]
                    if let Some(target) = self.tree_ctx.take().filter(|_| lparam.0 != -1) {
                        if let Some(path) = target {
                            self.tree_context_menu(path, lparam);
                        }
                        return LRESULT(0);
                    }
                    match self.fav_ctx.take() {
                        Some((pane, path)) if lparam.0 != -1 => {
                            if let Some(act) = self.fav_context_menu(path.as_ref(), false) {
                                self.fav_action(pane, path, act);
                            }
                        }
                        _ => self.on_context_menu(lparam),
                    }
                    LRESULT(0)
                }
                WM_KEYDOWN => {
                    if self.on_key(VIRTUAL_KEY(wparam.0 as u16)) {
                        LRESULT(0)
                    } else {
                        DefWindowProcW(self.hwnd, msg, wparam, lparam)
                    }
                }
                WM_SYSKEYDOWN => {
                    // Alt+flechas como en el Explorador; el resto (Alt+F4, Alt+Espacio) sigue de largo.
                    match self.config.keymap.lookup(VIRTUAL_KEY(wparam.0 as u16), key_down(VK_CONTROL), key_down(VK_SHIFT), true) {
                        Some(c) => self.run_command(c),
                        None => return DefWindowProcW(self.hwnd, msg, wparam, lparam),
                    }
                    LRESULT(0)
                }
                WM_CHAR => {
                    if let Some(c) = char::from_u32(wparam.0 as u32) {
                        if !c.is_control() && !key_down(VK_CONTROL) {
                            let action = self.tab_mut().list.on_char(c);
                            self.apply(self.active, action);
                        }
                    }
                    LRESULT(0)
                }
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                    let (x, y) = self.mouse_pos(lparam);
                    // El desplegable de la barra de dirección no le quita el foco a la caja.
                    if !matches!(self.hit(x, y), Hit::Suggest(_) | Hit::Pane(_, PaneHit::AddressClear)) {
                        let _ = SetFocus(Some(self.hwnd));
                    }
                    self.on_left_down(x, y, msg == WM_LBUTTONDBLCLK);
                    LRESULT(0)
                }
                WM_LBUTTONUP => {
                    let (x, y) = self.mouse_pos(lparam);
                    self.on_left_up(x, y);
                    LRESULT(0)
                }
                WM_MBUTTONUP => {
                    let (x, y) = self.mouse_pos(lparam);
                    self.on_middle_click(x, y);
                    LRESULT(0)
                }
                WM_MOUSEMOVE => {
                    let (x, y) = self.mouse_pos(lparam);
                    self.on_mouse_move(x, y);
                    LRESULT(0)
                }
                WM_MOUSELEAVE => {
                    self.tracking_mouse = false;
                    self.hover = Hit::None;
                    #[cfg(feature = "arbol")]
                    if matches!(self.drag, Drag::None) {
                        self.tree.on_mouse_leave();
                    }
                    for p in &mut self.panes {
                        p.tab_mut().list.on_mouse_leave();
                    }
                    self.redraw();
                    LRESULT(0)
                }
                WM_MOUSEWHEEL => {
                    // La rueda va al panel bajo el mouse, aunque no sea el activo.
                    let (x, y) = self.cursor_pos();
                    let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / WHEEL_DELTA as f32;
                    let pane = match self.hit(x, y) {
                        Hit::Palette(_) => {
                            if let Some((pal, _)) = &mut self.palette {
                                pal.on_wheel(delta.round() as i32 * 3);
                                self.redraw();
                            }
                            return LRESULT(0);
                        }
                        Hit::Suggest(_) => {
                            if let Some(sg) = &mut self.suggest {
                                sg.on_wheel(delta.round() as i32 * 3);
                                self.redraw();
                            }
                            return LRESULT(0);
                        }
                        Hit::Pane(i, _) => i,
                        _ => self.active,
                    };
                    #[cfg(feature = "arbol")]
                    if self.tree.contains(x, y) {
                        if key_down(VK_SHIFT) {
                            self.tree.on_wheel_h(delta * wheel_lines());
                        } else {
                            self.tree.on_wheel(delta * wheel_lines());
                        }
                        self.redraw();
                        return LRESULT(0);
                    }
                    // Ctrl+rueda: tamaño de la vista, como en el Explorador.
                    if key_down(VK_CONTROL) {
                        self.cycle_view(pane, delta > 0.0);
                        return LRESULT(0);
                    }
                    let action = self.panes[pane].tab_mut().list.on_wheel(delta * wheel_lines());
                    self.apply(pane, action);
                    LRESULT(0)
                }
                // Rueda horizontal (touchpad, mouse con inclinación): el árbol se mueve a los costados.
                0x020E => {
                    let (x, y) = self.cursor_pos();
                    #[cfg(feature = "arbol")]
                    if self.tree.contains(x, y) {
                        let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32 / WHEEL_DELTA as f32;
                        self.tree.on_wheel_h(-delta * wheel_lines());
                        self.redraw();
                    }
                    let _ = (x, y);
                    LRESULT(0)
                }
                WM_XBUTTONUP => {
                    // Botones laterales del mouse: atrás y adelante en el panel bajo el mouse.
                    let (x, y) = self.mouse_pos(lparam);
                    if let Hit::Pane(i, _) = self.hit(x, y) {
                        self.set_active(i);
                    }
                    let (hwnd, tx) = (self.hwnd, self.tx.clone());
                    match ((wparam.0 >> 16) & 0xFFFF) as u16 {
                        1 => self.tab_mut().go_back(hwnd, &tx),
                        2 => self.tab_mut().go_forward(hwnd, &tx),
                        _ => {}
                    }
                    self.redraw();
                    LRESULT(1)
                }
                WM_CLOSE => {
                    if self.probe.is_none() {
                        self.autosave_group();
                        let _ = session::save(&self.session());
                        let _ = self.history.save();
                    }
                    let _ = DestroyWindow(self.hwnd);
                    LRESULT(0)
                }
                // Un lector de pantalla pregunta por la raíz de UI Automation (ver uia.rs).
                WM_GETOBJECT if lparam.0 as i32 == windows::Win32::UI::Accessibility::UiaRootObjectId => {
                    let provider = uia::root_provider(self);
                    windows::Win32::UI::Accessibility::UiaReturnRawElementProvider(self.hwnd, wparam, lparam, &provider)
                }
                WM_DESTROY => {
                    // Se desconectan los lectores: los proveedores que les quedan dejan de responder.
                    uia::shutdown();
                    let _ = windows::Win32::UI::Accessibility::UiaReturnRawElementProvider(
                        self.hwnd,
                        WPARAM(0),
                        LPARAM(0),
                        None::<&windows::Win32::UI::Accessibility::IRawElementProviderSimple>,
                    );
                    let _ = RevokeDragDrop(self.hwnd);
                    let _ = DeleteObject(self.edit_font.into());
                    let _ = DeleteObject(self.edit_brush.into());
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                _ => DefWindowProcW(self.hwnd, msg, wparam, lparam),
            }
        }
    }
}
