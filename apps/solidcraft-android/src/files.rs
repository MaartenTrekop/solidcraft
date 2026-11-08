//! Open and Save dialogs over the phone's storage.
//!
//! Android's own file chooser answers with content URIs, which the app's commands cannot use: they
//! read and write paths, as on the desktop. So the host draws a dialog of its own. With "All files
//! access" ([`storage`]) it reaches the whole shared storage (what the Files app calls Internal
//! storage); without it, the app's own folder, with an offer to allow access.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use solidcraft_ui_egui::FilePickerFuture;
use winit::platform::android::activity::AndroidApp;

use crate::storage;

/// The phone's shared storage.
const SHARED: &str = "/storage/emulated/0";
/// The most entries one folder shows.
const MAX_ENTRIES: usize = 4000;
/// How often (seconds) an open dialog asks Android again whether access was allowed, so it
/// notices the user coming back from the settings page.
const ACCESS_RECHECK: f64 = 1.0;

/// The dialog, shared by the app's services (which ask for a file) and the host (which draws it).
#[derive(Clone)]
pub struct Picker(Rc<RefCell<State>>);

struct State {
    android: AndroidApp,
    /// The app's own folder, writable without any permission.
    own: PathBuf,
    /// Where the next dialog opens: the folder of the file chosen last.
    last_dir: Option<PathBuf>,
    /// Whether all files access is allowed (`None`: unknown), and when that was last asked.
    access: Option<bool>,
    access_at: Option<f64>,
    request: Option<Request>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Open,
    Save,
}

/// One open dialog.
struct Request {
    kind: Kind,
    /// The file types asked for: lower case, without the dot; empty for any.
    exts: Vec<String>,
    /// Open: list files of every type, not only those asked for.
    all_types: bool,
    dir: PathBuf,
    entries: Result<Vec<Entry>, String>,
    /// Save: the file name typed or picked.
    name: String,
    /// Save: the existing file the next Save replaces, once asked about.
    replace: Option<PathBuf>,
    /// Why the last action did nothing.
    message: Option<String>,
    answer: Rc<RefCell<Slot>>,
}

struct Entry {
    name: String,
    path: PathBuf,
    dir: bool,
    size: u64,
}

/// Where a dialog's answer waits for the app's picker future.
#[derive(Default)]
struct Slot {
    answer: Option<Option<String>>,
    waker: Option<Waker>,
}

/// The app's side of a dialog: ready once the dialog has an answer.
struct Answer(Rc<RefCell<Slot>>);

impl Future for Answer {
    type Output = Option<String>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<String>> {
        let mut slot = self.0.borrow_mut();
        match slot.answer.take() {
            Some(answer) => Poll::Ready(answer),
            None => {
                slot.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// What a frame of the dialog asked for, done once it is drawn.
enum Action {
    None,
    Go(PathBuf),
    Refresh,
    Choose(PathBuf),
    Name(String),
    Save,
    Cancel,
    AskAccess,
}

impl Picker {
    pub fn new(android: AndroidApp, own: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&own);
        Self(Rc::new(RefCell::new(State { android, own, last_dir: None, access: None, access_at: None, request: None })))
    }

    /// Ask for a file to open; `exts` are the types the command reads (none: any).
    pub fn open(&self, exts: &[&str]) -> FilePickerFuture {
        self.begin(Kind::Open, "", exts)
    }

    /// Ask where to save `name`, as one of `exts`.
    pub fn save(&self, name: &str, exts: &[&str]) -> FilePickerFuture {
        self.begin(Kind::Save, name, exts)
    }

    fn begin(&self, kind: Kind, name: &str, exts: &[&str]) -> FilePickerFuture {
        let slot = Rc::new(RefCell::new(Slot::default()));
        let mut state = self.0.borrow_mut();
        // A dialog still open (the app asked again) is cancelled.
        if let Some(old) = state.request.take() {
            answer(&old.answer, None);
        }
        state.access = storage::has_all_files_access(&state.android);
        state.access_at = None;
        let exts: Vec<String> =
            exts.iter().map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase()).filter(|e| !e.is_empty() && e != "*").collect();
        let dir = start_dir(&state);
        let entries = list(&dir, &exts, false);
        state.request = Some(Request {
            kind,
            exts,
            all_types: false,
            dir,
            entries,
            name: name.to_string(),
            replace: None,
            message: None,
            answer: slot.clone(),
        });
        Box::pin(Answer(slot))
    }

    pub fn is_open(&self) -> bool {
        self.0.borrow().request.is_some()
    }

    /// Back (Escape) closes an open dialog, before the app sees the key.
    pub fn take_escape(&self, raw: &mut egui::RawInput) {
        if !self.is_open() {
            return;
        }
        let before = raw.events.len();
        raw.events.retain(|e| !matches!(e, egui::Event::Key { key: egui::Key::Escape, .. }));
        if raw.events.len() != before {
            self.finish(None);
        }
    }

    fn finish(&self, chosen: Option<PathBuf>) {
        let mut state = self.0.borrow_mut();
        let Some(request) = state.request.take() else { return };
        if let Some(dir) = chosen.as_deref().and_then(Path::parent) {
            state.last_dir = Some(dir.to_path_buf());
        }
        answer(&request.answer, chosen.map(|p| p.to_string_lossy().into_owned()));
    }

    /// Draw the dialog, if one is open.
    pub fn show(&self, ctx: &egui::Context) {
        let action = {
            let mut guard = self.0.borrow_mut();
            let state = &mut *guard;
            let Some(request) = state.request.as_mut() else { return };
            let now = ctx.input(|i| i.time);
            if state.access != Some(true) && state.access_at.is_none_or(|at| now - at >= ACCESS_RECHECK) {
                state.access_at = Some(now);
                let access = storage::has_all_files_access(&state.android);
                if access == Some(true) && state.access != Some(true) {
                    // Allowed on the settings page just now: the folder may show more.
                    request.entries = list(&request.dir, &request.exts, request.all_types);
                }
                state.access = access;
            }
            if state.access != Some(true) {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(ACCESS_RECHECK));
            }
            draw(ctx, request, &state.own, state.access)
        };
        self.act(action);
    }

    fn act(&self, action: Action) {
        match action {
            Action::None => {}
            Action::Cancel => self.finish(None),
            Action::Choose(path) => self.finish(Some(path)),
            Action::AskAccess => {
                let mut state = self.0.borrow_mut();
                let asked = storage::ask_for_all_files_access(&state.android);
                state.access_at = None;
                if let (Err(e), Some(request)) = (asked, state.request.as_mut()) {
                    request.message = Some(format!("The settings page did not open: {e}"));
                }
            }
            Action::Go(dir) => self.with_request(|request| {
                if dir.starts_with(SHARED) && !dir.exists() {
                    // Documents/SolidCraft: made the first time it is chosen.
                    let _ = std::fs::create_dir_all(&dir);
                }
                request.entries = list(&dir, &request.exts, request.all_types);
                request.dir = dir;
                request.replace = None;
                request.message = None;
            }),
            Action::Refresh => self.with_request(|request| request.entries = list(&request.dir, &request.exts, request.all_types)),
            Action::Name(name) => self.with_request(|request| {
                request.name = name;
                request.replace = None;
            }),
            Action::Save => {
                let target = {
                    let mut state = self.0.borrow_mut();
                    let Some(request) = state.request.as_mut() else { return };
                    save_target(request)
                };
                if let Some(path) = target {
                    self.finish(Some(path));
                }
            }
        }
    }

    fn with_request(&self, f: impl FnOnce(&mut Request)) {
        if let Some(request) = self.0.borrow_mut().request.as_mut() {
            f(request);
        }
    }
}

fn answer(slot: &Rc<RefCell<Slot>>, chosen: Option<String>) {
    let waker = {
        let mut slot = slot.borrow_mut();
        slot.answer = Some(chosen);
        slot.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// Where a dialog opens: the last folder a file was chosen in, else SolidCraft's folder in
/// Documents (with access), else the app's own folder.
fn start_dir(state: &State) -> PathBuf {
    if let Some(dir) = state.last_dir.as_ref().filter(|d| d.is_dir()) {
        return dir.clone();
    }
    if state.access == Some(true) {
        let documents = Path::new(SHARED).join("Documents").join("SolidCraft");
        if std::fs::create_dir_all(&documents).is_ok() {
            return documents;
        }
    }
    state.own.clone()
}

/// Save: the path to answer with, or `None` with the reason (or the replace question) shown.
fn save_target(request: &mut Request) -> Option<PathBuf> {
    let typed = request.name.trim();
    if typed.is_empty() || typed.contains('/') || typed == "." || typed == ".." {
        request.message = Some("Type a file name (without a folder).".into());
        return None;
    }
    let name = with_extension(typed, &request.exts);
    let path = request.dir.join(&name);
    request.name = name;
    if path.exists() && request.replace.as_ref() != Some(&path) {
        request.replace = Some(path);
        request.message = None;
        return None;
    }
    Some(path)
}

/// `name` with the first of `exts` added when it has none of them.
fn with_extension(name: &str, exts: &[String]) -> String {
    match exts.first() {
        Some(first) if !has_extension(name, exts) => format!("{name}.{first}"),
        _ => name.to_string(),
    }
}

fn has_extension(name: &str, exts: &[String]) -> bool {
    Path::new(name).extension().and_then(|e| e.to_str()).is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// A folder's entries: folders first, then the files of the types asked for, by name. Hidden
/// entries are left out.
fn list(dir: &Path, exts: &[String], all_types: bool) -> Result<Vec<Entry>, String> {
    let read = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    let mut entries: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let path = e.path();
            let meta = std::fs::metadata(&path).ok()?;
            let dir = meta.is_dir();
            if !dir && !all_types && !exts.is_empty() && !has_extension(&name, exts) {
                return None;
            }
            Some(Entry { name, path, dir, size: meta.len() })
        })
        .take(MAX_ENTRIES)
        .collect();
    entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(entries)
}

/// The shortcuts along the top: the phone's usual folders and the app's own.
fn places(own: &Path) -> Vec<(&'static str, PathBuf)> {
    let shared = Path::new(SHARED);
    vec![
        ("SolidCraft", shared.join("Documents").join("SolidCraft")),
        ("Documents", shared.join("Documents")),
        ("Download", shared.join("Download")),
        ("Phone", shared.to_path_buf()),
        ("App folder", own.to_path_buf()),
    ]
}

/// A folder as the user knows it: "Phone/Documents" rather than the mount point.
fn shown_path(dir: &Path, own: &Path) -> String {
    if let Ok(rest) = dir.strip_prefix(SHARED) {
        return Path::new("Phone").join(rest).to_string_lossy().into_owned();
    }
    if let Ok(rest) = dir.strip_prefix(own) {
        return Path::new("App folder").join(rest).to_string_lossy().into_owned();
    }
    dir.to_string_lossy().into_owned()
}

fn size_text(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * 1024.0 {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    } else if b >= 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// One frame of the dialog; the action it asked for is done after.
fn draw(ctx: &egui::Context, request: &mut Request, own: &Path, access: Option<bool>) -> Action {
    let mut action = Action::None;
    let screen = ctx.content_rect();
    let width = (screen.width() - 32.0).clamp(260.0, 760.0);
    let height = (screen.height() - 48.0).clamp(260.0, 980.0);
    let _ = egui::Modal::new(egui::Id::new("solidcraft_android_files")).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().button_padding = egui::vec2(12.0, 8.0);
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.heading(match request.kind {
            Kind::Open => "Open",
            Kind::Save => "Save as",
        });
        ui.horizontal_wrapped(|ui| {
            for (label, path) in places(own) {
                if ui.selectable_label(request.dir == path, label).clicked() {
                    action = Action::Go(path);
                }
            }
        });
        ui.horizontal(|ui| {
            let parent = request.dir.parent().map(Path::to_path_buf);
            if ui.add_enabled(parent.is_some(), egui::Button::new("Up")).clicked()
                && let Some(parent) = parent
            {
                action = Action::Go(parent);
            }
            ui.label(egui::RichText::new(shown_path(&request.dir, own)).weak());
        });
        if access != Some(true) {
            ui.horizontal_wrapped(|ui| {
                ui.label("To open and save files anywhere on the phone, allow SolidCraft to manage all files.");
                if ui.button("Allow").clicked() {
                    action = Action::AskAccess;
                }
            });
        }
        ui.separator();
        // What stays below the list: the name row or the type switch, a message, the buttons.
        let below = 150.0;
        let list_height = (height - ui.min_rect().height() - below).max(120.0);
        egui::ScrollArea::vertical().max_height(list_height).auto_shrink([false, false]).show(ui, |ui| {
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| match &request.entries {
                Err(e) => {
                    ui.label(format!("This folder can't be read: {e}"));
                }
                Ok(entries) if entries.is_empty() => {
                    ui.label(egui::RichText::new("Nothing to show here.").weak());
                }
                Ok(entries) => {
                    for entry in entries {
                        let text = if entry.dir {
                            egui::RichText::new(format!("{}/", entry.name)).strong()
                        } else {
                            egui::RichText::new(format!("{}    {}", entry.name, size_text(entry.size)))
                        };
                        let picked = request.kind == Kind::Save && !entry.dir && request.name == entry.name;
                        if ui.selectable_label(picked, text).clicked() {
                            action = match (entry.dir, request.kind) {
                                (true, _) => Action::Go(entry.path.clone()),
                                (false, Kind::Open) => Action::Choose(entry.path.clone()),
                                (false, Kind::Save) => Action::Name(entry.name.clone()),
                            };
                        }
                    }
                }
            });
        });
        ui.separator();
        match request.kind {
            Kind::Save => {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    let edit = ui.add(egui::TextEdit::singleline(&mut request.name).desired_width(f32::INFINITY));
                    if edit.changed() {
                        request.replace = None;
                        request.message = None;
                    }
                });
            }
            Kind::Open if !request.exts.is_empty() => {
                if ui.checkbox(&mut request.all_types, "Show every kind of file").changed() {
                    action = Action::Refresh;
                }
            }
            Kind::Open => {}
        }
        if let Some(path) = &request.replace {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            ui.colored_label(ui.visuals().warn_fg_color, format!("{name} already exists. Save again to replace it."));
        }
        if let Some(message) = &request.message {
            ui.colored_label(ui.visuals().error_fg_color, message);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if request.kind == Kind::Save {
                let label = if request.replace.is_some() { "Replace" } else { "Save" };
                if ui.button(label).clicked() {
                    action = Action::Save;
                }
            }
            if ui.button("Cancel").clicked() {
                action = Action::Cancel;
            }
        });
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_extension_is_added() {
        let exts = vec!["sc".to_string(), "json".to_string()];
        assert_eq!(with_extension("bracket", &exts), "bracket.sc");
        assert_eq!(with_extension("bracket.JSON", &exts), "bracket.JSON");
        assert_eq!(with_extension("bracket", &[]), "bracket");
    }

    #[test]
    fn folders_show_as_the_user_knows_them() {
        let own = Path::new("/data/user/0/app/files/designs");
        assert_eq!(shown_path(Path::new("/storage/emulated/0/Documents"), own), "Phone/Documents");
        assert_eq!(shown_path(own, own), "App folder");
    }
}
