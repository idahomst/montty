//! Modal widgets in the window: the ≡ menu, a pane's profile menu, the info
//! panels (keyboard shortcuts, about), the text prompts (tab rename, session name) and
//! the close confirmation. At most one is open; while it is, it takes every key and
//! pointer press. The widgets' state and geometry are pure
//! (`ui::{menu,panel,textfield,confirm}`); this module opens and routes them
//! (`content`: what they hold; `paint`: how they look).

use std::time::Instant;

use winit::event::{KeyEvent, MouseButton};
use winit::window::CursorIcon;

use super::Window;
use crate::clipboard::ClipKind;
use crate::input::Mods;
use crate::input::bindings::Action;
use crate::layout::{PaneId, Rect};
use crate::term::pane::Pane;
use crate::ui::confirm::{CloseTarget, Confirm, ConfirmOutcome};
use crate::ui::menu::{Menu, MenuAction, MenuOutcome, profile_menu};
use crate::ui::panel::InfoPanel;
use crate::ui::textfield::{FieldOutcome, TextField};
use crate::ui::topbar::{Target, tab_label};

mod content;
mod paint;

pub(super) use content::*;
pub(super) use paint::*;

/// The save prompt's title.
pub(super) const SAVE_PROMPT_TITLE: &str = "Save session";
/// The new profile prompt's title.
pub(super) const NEW_PROFILE_TITLE: &str = "New profile";
/// The titled prompts' width in columns.
const TITLED_PROMPT_COLS: i32 = 40;

/// The open modal widget, if any. Opening one replaces the current one.
pub(super) enum Modal {
    None,
    /// The ≡ menu; with `profiles` a pane's profile menu; with `sessions` the session
    /// list (the names it opened with, so a change on disk can't shift its rows); with
    /// `pane` the menu a right-click in that pane opened.
    Menu {
        menu: Menu,
        profiles: Option<ProfileMenu>,
        sessions: Option<Vec<String>>,
        pane: Option<PaneId>,
    },
    Panel(InfoPanel),
    /// A single-line text field for `purpose`.
    Prompt {
        purpose: PromptPurpose,
        field: TextField,
    },
    /// Asks before closing a pane, tab or window with running processes.
    Confirm(Confirm),
}

impl Modal {
    /// An open modal takes every key and press: nothing reaches bindings or the PTY.
    pub(super) fn is_open(&self) -> bool {
        !matches!(self, Modal::None)
    }
}

/// What a text prompt is for; Enter commits `field`'s text to it.
pub(super) enum PromptPurpose {
    /// Renames the tab holding `pane` (its focused pane when opened), so the
    /// rename survives tab index shifts. Edited in place over the tab's label.
    /// `original` is the tab's user title when opened and `seed` the label
    /// the field started with.
    RenameTab {
        pane: PaneId,
        original: Option<String>,
        seed: String,
    },
    /// The name to save the session under: a titled panel over the window.
    SaveSession,
    /// The name of a new profile copied from `pane`'s, which then switches to it: a
    /// titled panel like `SaveSession`.
    NewProfile { pane: PaneId },
}

impl PromptPurpose {
    /// The title of a prompt shown as a titled panel; None for the in-place rename.
    pub(super) fn title(&self) -> Option<&'static str> {
        match self {
            PromptPurpose::SaveSession => Some(SAVE_PROMPT_TITLE),
            PromptPurpose::NewProfile { .. } => Some(NEW_PROFILE_TITLE),
            PromptPurpose::RenameTab { .. } => None,
        }
    }
}

impl Window {
    /// Whether a modal widget is open: it gets every key and press. (The pane
    /// overlay is not drawn meanwhile.)
    pub(super) fn modal_open(&self) -> bool {
        self.modal.is_open()
    }

    /// Replace the open modal (or close it with `Modal::None`) and redraw.
    /// Opening one ends a divider drag (it takes the pointer from now on).
    pub(super) fn set_modal(&mut self, m: Modal) {
        if m.is_open() {
            self.mouse.cancel_drag();
            self.size_labels.clear();
        }
        self.modal = m;
        self.schedule();
    }

    /// ≡ clicked: the menu hangs below the bar, its right edge on the button's.
    pub(super) fn open_menu(&mut self) {
        let bar = self.bar();
        let Some(button) = bar.items.iter().find(|i| i.target == Target::Menu) else {
            return;
        };
        let right = button.rect.x + button.rect.w;
        let menu = Menu::new(menu_items(&self.store.global().bindings), right, bar.height);
        self.set_modal(Modal::Menu {
            menu,
            profiles: None,
            sessions: None,
            pane: None,
        });
    }

    /// The Profile button (`button`) of pane `pane`'s overlay clicked: a row per
    /// profile, the one the pane resolves to marked, then editing it and creating a new
    /// one, hanging from the button (see `profile_menu_origin`). No profiles and no
    /// config folder: nothing.
    pub(super) fn open_profile_menu(&mut self, pane: PaneId, button: Rect) {
        let stems = self.store.stems();
        let files = self.user_config_dir().is_some();
        if stems.is_empty() && !files {
            return;
        }
        let current = self.resolved_stem(pane);
        let bm = self.bar_metrics();
        let (w, h) = self.size_px();
        let (max_rows, max_cols) = profile_menu_limits(button, w, h, &bm);
        let items = profile_menu(&stems, current, files, max_rows, max_cols);
        let probe = Menu::new(items, 0, 0);
        let r = probe.rect(&bm);
        let (x, y) = profile_menu_origin(button, r.w, r.h, w, h);
        let menu = Menu::new(probe.items, x + r.w, y);
        let profiles = Some(ProfileMenu { pane, stems });
        self.set_modal(Modal::Menu {
            menu,
            profiles,
            sessions: None,
            pane: None,
        });
    }

    /// Right-click in pane `pane` at (`x`, `y`): its menu opens on the pointer. The
    /// pane takes the focus first, so a split happens next to it.
    pub(super) fn open_pane_menu(&mut self, pane: PaneId, x: i32, y: i32) {
        if pane != self.tabs.active().focused {
            self.set_focus(pane);
        }
        let has_selection = self.selection_text(pane).is_some_and(|t| !t.is_empty());
        let items = pane_menu_items(&self.store.global().bindings, has_selection);
        let bm = self.bar_metrics();
        let (w, h) = self.size_px();
        let probe = Menu::new(items, 0, 0);
        let r = probe.rect(&bm);
        // On the pointer, moved to stay inside the window like a profile menu.
        let (x, y) = profile_menu_origin(Rect { x, y, w: 0, h: 0 }, r.w, r.h, w, h);
        self.set_modal(Modal::Menu {
            menu: Menu::new(probe.items, x + r.w, y),
            profiles: None,
            sessions: None,
            pane: Some(pane),
        });
    }

    /// Tab `i` double-clicked: edit its label in place, starting from the
    /// label as shown.
    pub(super) fn open_rename(&mut self, i: usize) {
        let Some(tab) = self.tabs.get(i) else {
            return;
        };
        let title = self
            .panes
            .get(&tab.focused)
            .and_then(|p| p.title.as_deref());
        let seed = tab_label(tab.title.as_deref(), title);
        let modal = Modal::Prompt {
            purpose: PromptPurpose::RenameTab {
                pane: tab.focused,
                original: tab.title.clone(),
                seed: seed.clone(),
            },
            field: TextField::new(&seed),
        };
        self.set_modal(modal);
    }

    /// Before pane `closed` is removed: when it anchors the open rename, another
    /// pane of its tab (to find the tab again afterwards).
    pub(super) fn rename_sibling(&self, closed: PaneId) -> Option<PaneId> {
        let Modal::Prompt {
            purpose: PromptPurpose::RenameTab { pane, .. },
            ..
        } = &self.modal
        else {
            return None;
        };
        let i = self.tabs.tab_of(*pane).filter(|_| *pane == closed)?;
        self.tabs
            .get(i)?
            .tree
            .panes()
            .into_iter()
            .find(|&p| p != closed)
    }

    /// After a pane close: a rename anchored on it moves to its tab's focused
    /// pane when the tab survived (`sibling` from `rename_sibling`).
    pub(super) fn reanchor_rename(&mut self, sibling: Option<PaneId>) {
        if let Some(new) = reanchor(&self.tabs, sibling)
            && let Modal::Prompt {
                purpose: PromptPurpose::RenameTab { pane, .. },
                ..
            } = &mut self.modal
        {
            *pane = new;
        }
    }

    /// Ask before closing `target` when it has running processes (see
    /// `running_in`); with none it closes right away. The prompt replaces any
    /// other modal; a repeat request for the open prompt's own target does
    /// nothing.
    pub(super) fn request_close(&mut self, target: CloseTarget) {
        if matches!(&self.modal, Modal::Confirm(c) if c.target == target) {
            return;
        }
        let running = running_in(&target, &self.tabs, &self.panes, |p: &Pane| p.is_running());
        let confirm_window = self.store.global().confirm_close_window;
        let Some(msg) = close_prompt(&target, &self.tabs, running, confirm_window) else {
            self.close_target(target);
            return;
        };
        let mut c = Confirm::new(target, msg);
        let (w, h) = self.size_px();
        let (x, y) = self.mouse.pos();
        c.hover(x, y, w, h, &self.bar_metrics());
        self.set_modal(Modal::Confirm(c));
    }

    /// The prompt's Close: it closes, then its target does unless that is
    /// already gone.
    fn confirm_yes(&mut self) {
        let Modal::Confirm(c) = std::mem::replace(&mut self.modal, Modal::None) else {
            return;
        };
        self.schedule();
        if target_alive(&c.target, &self.tabs, &self.panes) {
            self.close_target(c.target);
        }
    }

    fn confirm_outcome(&mut self, out: ConfirmOutcome) {
        match out {
            ConfirmOutcome::None => {}
            ConfirmOutcome::Yes => self.confirm_yes(),
            ConfirmOutcome::No => self.set_modal(Modal::None),
        }
    }

    /// A menu hangs off the ≡ button or a pane's overlay, which move with the
    /// size and scale: it closes then.
    pub(super) fn close_menu(&mut self) {
        if matches!(self.modal, Modal::Menu { .. }) {
            self.set_modal(Modal::None);
        }
    }

    /// Body rows `panel` shows in the current window.
    fn panel_visible(&self, panel: &InfoPanel) -> usize {
        let (w, h) = self.size_px();
        let bm = self.bar_metrics();
        panel.visible_lines(panel.rect(w, h, &bm), &bm)
    }

    /// Whether the pointer is over the open info panel.
    pub(super) fn pointer_over_panel(&self) -> bool {
        let Modal::Panel(panel) = &self.modal else {
            return false;
        };
        let (w, h) = self.size_px();
        let (x, y) = self.mouse.pos();
        panel.rect(w, h, &self.bar_metrics()).contains(x, y)
    }

    /// Scroll the open info panel by `delta` lines; a redraw only on a change.
    pub(super) fn scroll_panel(&mut self, delta: isize) {
        let Modal::Panel(panel) = &self.modal else {
            return;
        };
        let visible = self.panel_visible(panel);
        if let Modal::Panel(panel) = &mut self.modal
            && panel.scroll_by(delta, visible)
        {
            self.schedule();
        }
    }

    /// A rename whose tab has closed, or a close prompt or profile menu whose
    /// target has, is dismissed; true when that happened. Called after panes or
    /// tabs are removed, and before each key, press and draw.
    pub(super) fn dismiss_stale_modal(&mut self) -> bool {
        let stale = match &self.modal {
            Modal::Prompt {
                purpose: PromptPurpose::RenameTab { pane, .. },
                ..
            } => self.tabs.tab_of(*pane).is_none(),
            Modal::Confirm(c) => !target_alive(&c.target, &self.tabs, &self.panes),
            Modal::Menu {
                profiles: Some(ProfileMenu { pane, .. }),
                ..
            }
            | Modal::Menu {
                pane: Some(pane), ..
            }
            | Modal::Prompt {
                purpose: PromptPurpose::NewProfile { pane },
                ..
            } => !target_alive(&CloseTarget::Pane(*pane), &self.tabs, &self.panes),
            _ => false,
        };
        if stale {
            self.set_modal(Modal::None);
        }
        stale
    }

    /// Commit the open rename with `text` (see `renamed_title`) and close it.
    fn commit_rename(&mut self, text: &str) {
        let Modal::Prompt {
            purpose:
                PromptPurpose::RenameTab {
                    pane,
                    original,
                    seed,
                },
            ..
        } = &self.modal
        else {
            return;
        };
        let title = renamed_title(original.as_deref(), seed, text);
        if let Some(i) = self.tabs.tab_of(*pane) {
            if let Some(tab) = self.tabs.get_mut(i) {
                tab.title = title;
            }
            self.layout_changed();
            if i == self.tabs.active_index() {
                self.update_title();
            }
        }
        self.set_modal(Modal::None);
    }

    /// Enter in the open prompt: its text goes to what the prompt is for.
    fn commit_prompt(&mut self, text: &str) {
        match &self.modal {
            Modal::Prompt {
                purpose: PromptPurpose::RenameTab { .. },
                ..
            } => self.commit_rename(text),
            Modal::Prompt {
                purpose: PromptPurpose::SaveSession,
                ..
            } => self.commit_save(text),
            Modal::Prompt {
                purpose: PromptPurpose::NewProfile { pane },
                ..
            } => self.commit_new_profile(*pane, text),
            _ => {}
        }
    }

    /// A menu row was chosen: the menu closes and its action runs.
    fn choose(&mut self, a: MenuAction) {
        match a {
            MenuAction::NewTab => {
                self.set_modal(Modal::None);
                self.perform(Action::NewTab);
            }
            MenuAction::NewWindow => {
                self.set_modal(Modal::None);
                self.perform(Action::NewWindow);
            }
            MenuAction::SaveSession => self.open_save_prompt(),
            MenuAction::OpenSessionMenu => self.open_session_menu(),
            MenuAction::EditConfig => {
                self.set_modal(Modal::None);
                self.edit_config();
            }
            MenuAction::OpenConfigFolder => {
                self.set_modal(Modal::None);
                self.open_config_folder();
            }
            MenuAction::ReloadConfig => {
                self.set_modal(Modal::None);
                self.reload_config(Instant::now());
            }
            MenuAction::Shortcuts => {
                self.set_modal(Modal::Panel(shortcuts_panel(&self.store.global().bindings)))
            }
            MenuAction::About => self.set_modal(Modal::Panel(about_panel())),
            MenuAction::SetProfile(i) => self.choose_profile(i),
            MenuAction::OpenSession(i) => {
                let name = match &self.modal {
                    Modal::Menu {
                        sessions: Some(names),
                        ..
                    } => names.get(i).cloned(),
                    _ => None,
                };
                self.set_modal(Modal::None);
                if let Some(name) = name {
                    self.open_session(&name);
                }
            }
            MenuAction::SplitRight
            | MenuAction::SplitDown
            | MenuAction::SplitLeft
            | MenuAction::SplitUp => {
                self.set_modal(Modal::None);
                self.perform(match a {
                    MenuAction::SplitRight => Action::SplitRight,
                    MenuAction::SplitDown => Action::SplitDown,
                    MenuAction::SplitLeft => Action::SplitLeft,
                    _ => Action::SplitUp,
                });
            }
            MenuAction::Copy | MenuAction::Paste => {
                let pane = match &self.modal {
                    Modal::Menu { pane, .. } => *pane,
                    _ => None,
                };
                self.set_modal(Modal::None);
                // The pane clicked, even with the find bar open (its paste goes to the field).
                match (a, pane) {
                    (MenuAction::Copy, _) => self.copy(),
                    (_, Some(pane)) => self.paste_into(pane, ClipKind::Clipboard),
                    _ => {}
                }
            }
            MenuAction::EditProfile | MenuAction::NewProfile => {
                let pane = match &self.modal {
                    Modal::Menu {
                        profiles: Some(p), ..
                    } => p.pane,
                    _ => return,
                };
                match a {
                    MenuAction::EditProfile => {
                        self.set_modal(Modal::None);
                        self.edit_profile(pane);
                    }
                    _ => self.set_modal(Modal::Prompt {
                        purpose: PromptPurpose::NewProfile { pane },
                        field: TextField::new(""),
                    }),
                }
            }
            MenuAction::Disabled | MenuAction::Separator => {}
        }
    }

    /// Row `i` of the profile menu was chosen: the menu closes, and its pane, when
    /// still there, switches to that profile and takes the focus.
    fn choose_profile(&mut self, i: usize) {
        let Modal::Menu {
            profiles: Some(p), ..
        } = std::mem::replace(&mut self.modal, Modal::None)
        else {
            return;
        };
        self.schedule();
        // A profile deleted while the menu was open: nothing changes.
        let Some(stem) = profile_choice(&p.stems, i, |s| self.store.contains(s)) else {
            return;
        };
        if !target_alive(&CloseTarget::Pane(p.pane), &self.tabs, &self.panes) {
            return;
        }
        self.set_pane_profile(p.pane, stem);
        if self.tabs.tab_of(p.pane) == Some(self.tabs.active_index()) {
            self.set_focus(p.pane);
        }
    }

    /// A key press while a modal is open: it goes to the modal only. Esc closes
    /// any of them (cancelling a rename).
    pub(super) fn modal_key(&mut self, event: &KeyEvent) {
        if self.dismiss_stale_modal() {
            return;
        }
        let key = &event.logical_key;
        let mods = Mods::from(self.modifiers);
        let visible = match &self.modal {
            Modal::Panel(panel) => self.panel_visible(panel),
            _ => 0,
        };
        match &mut self.modal {
            Modal::None => {}
            Modal::Menu { menu, .. } => {
                let before = menu.selected();
                let out = menu.key(key);
                let moved = menu.selected() != before;
                match out {
                    MenuOutcome::None if moved => self.schedule(),
                    MenuOutcome::None => {}
                    MenuOutcome::Chosen(a) => self.choose(a),
                    MenuOutcome::Close => self.set_modal(Modal::None),
                }
            }
            Modal::Panel(panel) => {
                if panel_closes(key) {
                    self.set_modal(Modal::None);
                } else if let Some(d) = panel_scroll_delta(key, visible, panel.lines.len()) {
                    self.scroll_panel(d);
                }
            }
            Modal::Prompt { field, .. } => match field.key(key, event.text.as_deref(), mods) {
                FieldOutcome::Edited | FieldOutcome::Moved => self.schedule(),
                FieldOutcome::Unchanged => {}
                FieldOutcome::Commit(text) => self.commit_prompt(&text),
                FieldOutcome::Cancel => self.set_modal(Modal::None),
            },
            Modal::Confirm(c) => {
                let out = c.key(key);
                self.confirm_outcome(out);
            }
        }
    }

    /// Pointer moved while a modal is open: the menu row or prompt button
    /// under it is highlighted (a redraw only when that changes). Nothing else
    /// reacts.
    pub(super) fn modal_pointer_moved(&mut self) {
        self.set_cursor_icon(CursorIcon::Default);
        let (x, y) = self.mouse.pos();
        let (w, h) = self.size_px();
        let bm = self.bar_metrics();
        let changed = match &mut self.modal {
            Modal::Menu { menu, .. } => {
                let before = menu.selected();
                menu.hover(x - menu_shift(menu, &bm), y, &bm);
                menu.selected() != before
            }
            Modal::Confirm(c) => c.hover(x, y, w, h, &bm),
            _ => false,
        };
        if changed {
            self.schedule();
        }
    }

    /// Where tab `i`'s label is drawn; None while the tab is clipped out of the bar.
    fn tab_text_rect(&self, i: usize) -> Option<Rect> {
        self.bar()
            .tabs
            .into_iter()
            .find(|t| t.index == i)
            .map(|t| t.text_rect)
    }

    /// A button press while a modal is open, always consumed. Outside a menu or panel it
    /// closes it, outside the rename field it commits as Enter does; a prompt stays until
    /// a button answers it.
    pub(super) fn modal_press(&mut self, button: MouseButton) {
        self.mouse.clicks.clear();
        if self.dismiss_stale_modal() {
            return;
        }
        let (x, y) = self.mouse.pos();
        let bm = self.bar_metrics();
        match &self.modal {
            Modal::None => {}
            Modal::Menu { menu, .. } => match menu.click(x - menu_shift(menu, &bm), y, &bm) {
                MenuOutcome::Chosen(a) if button == MouseButton::Left => self.choose(a),
                MenuOutcome::Chosen(_) | MenuOutcome::None => {}
                MenuOutcome::Close => self.set_modal(Modal::None),
            },
            Modal::Panel(panel) => {
                let (w, h) = self.size_px();
                if !panel.rect(w, h, &bm).contains(x, y) {
                    self.set_modal(Modal::None);
                }
            }
            Modal::Prompt {
                purpose: PromptPurpose::RenameTab { pane, .. },
                field,
            } => {
                let text = field.text().to_string();
                let inside = self
                    .tabs
                    .tab_of(*pane)
                    .and_then(|i| self.tab_text_rect(i))
                    .is_some_and(|r| field_rect(r, &bm, self.chrome_stroke()).contains(x, y));
                if !inside {
                    self.commit_rename(&text);
                }
            }
            Modal::Prompt {
                purpose: PromptPurpose::SaveSession | PromptPurpose::NewProfile { .. },
                ..
            } => {
                let (w, h) = self.size_px();
                let (panel, ..) = titled_prompt_layout(w, h, &bm, self.chrome_stroke());
                if !panel.contains(x, y) {
                    self.set_modal(Modal::None);
                }
            }
            Modal::Confirm(c) if button == MouseButton::Left => {
                let (w, h) = self.size_px();
                let out = c.click(x, y, w, h, &bm);
                self.confirm_outcome(out);
            }
            Modal::Confirm(_) => {}
        }
    }

    /// The open modal, over everything else. A rename whose tab is clipped
    /// out of the bar (the window shrank) is not drawn but stays open.
    pub(super) fn draw_modal(&mut self) {
        if self.dismiss_stale_modal() {
            return;
        }
        let bm = self.bar_metrics();
        let stroke = self.chrome_stroke();
        let (w, h) = self.size_px();
        let paint = match &self.modal {
            Modal::None => return,
            Modal::Menu { menu, .. } => paint_menu(menu, &bm, stroke),
            Modal::Panel(panel) => paint_panel(panel, w, h, &bm, stroke),
            Modal::Prompt {
                purpose: PromptPurpose::RenameTab { pane, .. },
                field,
            } => {
                let Some(text) = self.tabs.tab_of(*pane).and_then(|i| self.tab_text_rect(i)) else {
                    return;
                };
                paint_rename(field, text, &bm, stroke)
            }
            Modal::Prompt {
                purpose: purpose @ (PromptPurpose::SaveSession | PromptPurpose::NewProfile { .. }),
                field,
            } => paint_titled_prompt(
                purpose.title().unwrap_or_default(),
                field,
                w,
                h,
                &bm,
                stroke,
            ),
            Modal::Confirm(c) => paint_confirm(c, w, h, &bm, stroke),
        };
        self.draw_paint(&paint);
    }
}
