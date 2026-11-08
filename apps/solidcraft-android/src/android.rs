//! The Android host: `android_main`, the app's folders, the window and graphics device, the app's
//! services, and each frame.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use solidcraft_engine::Session;
use solidcraft_ui_egui::{Services, SolveApp};
use winit::platform::android::activity::AndroidApp;
use winit::platform::android::activity::ndk::configuration::UiModeNight;

use crate::files::Picker;
use crate::storage;
use crate::touch::{self, TouchRouter};

/// The viewport scene's depth and stencil bits and its default anti-aliasing, as on the desktop.
const DEPTH_BITS: u8 = 24;
const STENCIL_BITS: u8 = 8;
const MSAA: u16 = 4;
/// The interface scale of a first start: a tablet-sized screen at a phone's density leaves the
/// desktop layout little room at 1. Preferences › Appearance changes it.
const FIRST_UI_SCALE: f32 = 0.9;

/// Android's entry point: NativeActivity runs it on a thread of its own once the library is loaded.
#[allow(unsafe_code)] // `no_mangle`: android-activity finds the function by its name.
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("SolidCraft"));
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("SolidCraft panicked: {info}");
        previous(info);
    }));
    let dirs = Dirs::new(&app);
    export_env(&dirs);
    if let Err(e) = run(app, &dirs) {
        log::error!("SolidCraft stopped: {e}");
    }
    // winit opens its event loop once per process, so the next start must be a new process: this
    // one ends with its activity.
    std::process::exit(0);
}

/// The app's private folders.
struct Dirs {
    /// Android's files folder for the app; the app's home.
    home: PathBuf,
    /// eframe's settings (`app.ron`) and the graphics settings read before the window opens.
    settings: PathBuf,
    /// Designs saved without access to the phone's storage.
    own: PathBuf,
}

impl Dirs {
    fn new(app: &AndroidApp) -> Self {
        let home = app.internal_data_path().unwrap_or_else(|| PathBuf::from(format!("/data/data/{}/files", storage::PACKAGE)));
        Self { settings: home.join("settings"), own: home.join("designs"), home }
    }
}

/// The engine finds its folders (recovery, caches, fonts, settings) as on Linux, through `HOME`
/// and the XDG variables, and the temporary folder through `TMPDIR`: none is set for an Android
/// app, and Android's default temporary folder is not writable by apps.
#[allow(unsafe_code)]
fn export_env(dirs: &Dirs) {
    let home = &dirs.home;
    let vars = [
        ("HOME", home.clone()),
        ("TMPDIR", home.join("tmp")),
        ("XDG_CONFIG_HOME", home.join(".config")),
        ("XDG_DATA_HOME", home.join(".local").join("share")),
        ("XDG_CACHE_HOME", home.join(".cache")),
    ];
    for (name, dir) in vars {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("folder {}: {e}", dir.display());
        }
        // SAFETY: the first thing `android_main` does, before this library starts any thread;
        // Android's own threads in the process don't read these variables.
        unsafe { std::env::set_var(name, &dir) };
    }
    // Android's own fonts: sketch text can use them, and the interface finds its CJK fallback
    // (NotoSansCJK) there.
    if std::env::var_os("SOLIDCRAFT_FONT_DIRS").is_none() {
        // SAFETY: as above.
        unsafe { std::env::set_var("SOLIDCRAFT_FONT_DIRS", "/system/fonts") };
    }
    if let Err(e) = std::fs::create_dir_all(&dirs.settings) {
        log::warn!("folder {}: {e}", dirs.settings.display());
    }
}

fn run(app: AndroidApp, dirs: &Dirs) -> eframe::Result {
    let graphics_path = dirs.settings.join("graphics.json");
    let saved_samples = saved_msaa(&graphics_path);
    let options = eframe::NativeOptions {
        android_app: Some(app.clone()),
        viewport: egui::ViewportBuilder::default().with_title("SolidCraft"),
        // The window's pass (egui's) has no depth and one sample: the viewport draws its scene
        // offscreen with its own depth and MSAA, as on the desktop.
        depth_buffer: 0,
        stencil_buffer: 0,
        multisampling: 1,
        persistence_path: Some(dirs.settings.join("app.ron")),
        persist_window: false,
        wgpu_options: wgpu_options(),
        ..Default::default()
    };
    let own = dirs.own.clone();
    eframe::run_native("SolidCraft", options, Box::new(move |cc| Ok(Box::new(Host::new(cc, app, own, &graphics_path, saved_samples)))))
}

/// Vulkan, or OpenGL ES where a phone has no usable Vulkan driver (`WGPU_BACKEND` overrides).
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    let mut options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_setup {
        setup.instance_descriptor.backends =
            eframe::wgpu::Backends::from_env().unwrap_or(eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL);
        // Everything the GPU offers: a phone's limits differ from WebGPU's defaults either way,
        // and asking for more than it has would keep the device from opening.
        setup.device_descriptor = std::sync::Arc::new(|adapter: &eframe::wgpu::Adapter| eframe::wgpu::DeviceDescriptor {
            label: Some("SolidCraft device"),
            required_features: adapter.features() & eframe::wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
            required_limits: adapter.limits(),
            ..Default::default()
        });
    }
    options
}

/// The anti-aliasing chosen in Preferences › Graphics (`graphics.json`'s `msaa`), if any.
fn saved_msaa(path: &Path) -> Option<u16> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("msaa").and_then(serde_json::Value::as_u64).and_then(|m| u16::try_from(m).ok()).filter(|m| [1, 2, 4, 8].contains(m))
}

struct Host {
    solve: SolveApp,
    touch: TouchRouter,
    files: Picker,
    prefs_writer: PrefsWriter,
}

impl Host {
    fn new(cc: &eframe::CreationContext<'_>, android: AndroidApp, own: PathBuf, graphics_path: &Path, saved_samples: Option<u16>) -> Self {
        solidcraft_ui_egui::fonts::setup_bundled_fonts(&cc.egui_ctx);
        solidcraft_ui_egui::fonts::load_system_fonts_in_background(&cc.egui_ctx);
        // A fingertip wanders further than a mouse between press and release.
        cc.egui_ctx.options_mut(|o| o.input_options.max_click_dist = 12.0);
        let files = Picker::new(android.clone(), own);
        let mut app = SolveApp::new(Session::default(), services(&files, &android, graphics_path));
        app.apply_off_thread = Some(solidcraft_ui_egui::dialogs::APPLY_AT_ONCE);
        let stored = cc.storage.and_then(|s| solidcraft_ui_egui::prefs::stored(|k| s.get_string(k)));
        match stored.as_deref() {
            Some(prefs) => app.load_prefs(prefs),
            None => app.preferences.ui_scale = FIRST_UI_SCALE,
        }
        match &cc.wgpu_render_state {
            Some(rs) => set_up_gpu(&mut app, rs, saved_samples),
            None => solidcraft_ui_egui::gpu::install_cpu_picture_renderer(),
        }
        // Lock files next to open designs: a design open on another device says so.
        solidcraft_ui_egui::open_elsewhere::start(&mut app);
        // The designs open when the app last closed, when the preference asks; one that stopped
        // the last session while it was reopening is skipped.
        let recovery_dir = solidcraft_engine::recovery::default_dir();
        app.reopen_guard_dir = recovery_dir.clone();
        let (reopen, skipped) = solidcraft_ui_egui::documents::reopen_plan(&app, recovery_dir.as_deref());
        for path in &reopen {
            app.open_path_async(path);
        }
        if reopen.is_empty() {
            solidcraft_ui_egui::prefs::apply_new_design(&mut app);
            let rev = app.session.revision;
            app.home.show(rev);
        }
        // Autosave, and offer what an ended session left behind.
        if let Some(dir) = recovery_dir.as_ref() {
            let every = if app.autosave_minutes > 0.0 { app.autosave_minutes * 60.0 } else { f64::MAX / 4.0 };
            solidcraft_ui_egui::recovery_ui::start(&mut app, dir, Duration::from_secs_f64(every.min(1e9)));
        }
        if !skipped.is_empty() {
            let names: Vec<String> =
                skipped.iter().map(|p| Path::new(p).file_name().map_or_else(|| p.clone(), |n| n.to_string_lossy().into_owned())).collect();
            app.set_status(
                format!("The last session stopped while reopening {}. It was not reopened this time; open it yourself if you need it.", names.join(", ")),
                true,
            );
        }
        Self { prefs_writer: PrefsWriter::read(app.prefs()), solve: app, touch: TouchRouter::default(), files }
    }

    fn prefs_for_storage(&self, storage: &dyn eframe::Storage) -> Result<String, String> {
        self.solve.try_prefs().or_else(|reason| {
            log::warn!("Panel layout changes were not saved: {reason}");
            let previous = solidcraft_ui_egui::prefs::stored(|key| storage.get_string(key));
            self.solve.prefs_with_stored_panel_layout(previous.as_deref())
        })
    }
}

/// The viewport on the GPU: the scene's depth, stencil and anti-aliasing, as the desktop app sets
/// them up, and the GPU for pictures of the model (start page cards, file previews).
fn set_up_gpu(app: &mut SolveApp, rs: &eframe::egui_wgpu::RenderState, saved_samples: Option<u16>) {
    let info = rs.adapter.get_info();
    let samples = saved_samples.unwrap_or(MSAA);
    let depth = eframe::egui_wgpu::depth_format_from_bits(DEPTH_BITS, STENCIL_BITS);
    let supported = eframe::egui_wgpu::supported_msaa_samples(&rs.adapter, &rs.device, rs.target_format, depth);
    let scene_samples = scene_samples(info.backend, u32::from(samples), &supported);
    log::info!("graphics: {:?} {} ({}), MSAA {scene_samples}x (requested {samples}x)", info.backend, info.name, info.driver);
    app.services.graphics_adapters = rs
        .available_adapters
        .iter()
        .map(|adapter| {
            let info = adapter.get_info();
            (info.name.clone(), format!("{} ({:?}, {:?})", info.name, info.device_type, info.backend))
        })
        .collect();
    let _ = app.set_wgpu_split(rs, DEPTH_BITS, STENCIL_BITS, scene_samples);
    app.msaa_supported = supported;
    app.preferences.msaa = u32::from(samples);
    solidcraft_ui_egui::gpu::install_picture_renderer(rs);
    solidcraft_ui_egui::prefs::check_msaa(app, u32::from(samples));
}

/// The scene's sample count: the app's anti-aliasing, clamped by the backend (OpenGL renders 1x);
/// a count the device can't render with falls back to the largest supported default.
fn scene_samples(backend: eframe::wgpu::Backend, samples: u32, supported: &[u32]) -> u32 {
    let requested = eframe::egui_wgpu::effective_msaa_samples(backend, samples.max(1));
    if requested <= 1 || supported.is_empty() || supported.contains(&requested) {
        requested
    } else {
        supported.iter().copied().filter(|&n| n <= solidcraft_ui_egui::prefs::Prefs::default().msaa).max().unwrap_or(1)
    }
}

fn services(files: &Picker, android: &AndroidApp, graphics_path: &Path) -> Services {
    let (open, save) = (files.clone(), files.clone());
    let config = android.config();
    Services {
        pick_open: Some(Box::new(move |exts: &[&str]| open.open(exts))),
        pick_save: Some(Box::new(move |name: &str, exts: &[&str]| save.save(name, exts))),
        graphics_path: Some(graphics_path.to_string_lossy().into_owned()),
        // Theme › System follows the phone's dark theme.
        desktop_dark: Some(Box::new(move || match config.ui_mode_night() {
            UiModeNight::Yes => Some(true),
            UiModeNight::No => Some(false),
            _ => None,
        })),
        ..Services::default()
    }
}

impl eframe::App for Host {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.solve.logic(ctx);
        if self.solve.quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        touch::back_is_escape(raw);
        // Back closes an open file dialog first.
        self.files.take_escape(raw);
        if let Some(wait) = self.touch.route(raw) {
            ctx.request_repaint_after(Duration::from_secs_f64(wait));
        }
        self.solve.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        content_ui(&mut self.solve, ui);
        self.files.show(ui.ctx(), &self.solve.preferences.language);
        // After the frame's UI, so a setting changed in it is written now: Android may end the
        // app without warning once it is in the background.
        let now = Instant::now();
        if self.prefs_writer.due(ui.input(|i| i.events.iter().any(changes_settings)), now)
            && let Some(storage) = frame.storage_mut()
            && let Ok(prefs) = self.prefs_for_storage(storage)
            && let Some(wait) = self.prefs_writer.store(storage, prefs, now)
        {
            ui.ctx().request_repaint_after(wait);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        match self.prefs_for_storage(storage) {
            Ok(prefs) => {
                storage.set_string(solidcraft_ui_egui::prefs::STORAGE_KEY, prefs.clone());
                self.prefs_writer.written = Some(prefs);
            }
            Err(reason) => log::warn!("Settings were not written: {reason}"),
        }
        // eframe asks for a save as the app goes to the background, after which Android may end
        // the process at any time: unsaved designs are kept for recovery now, not at the next
        // autosave.
        self.solve.big_operation = true;
        solidcraft_ui_egui::recovery_ui::tick(&mut self.solve);
    }

    fn on_exit(&mut self) {
        // A save still being written ends before the app does.
        solidcraft_ui_egui::save_job::tick(&mut self.solve, true);
        // Pictures still being drawn off the window thread end before the GPU is torn down.
        if !solidcraft_engine::capture::close(Duration::from_secs(5)) {
            log::warn!("a picture was still being drawn as the app closed");
        }
        solidcraft_ui_egui::recovery_ui::close(&mut self.solve);
        solidcraft_ui_egui::render_workspace::close(&mut self.solve);
        solidcraft_ui_egui::open_elsewhere::close(&mut self.solve);
    }
}

/// egui's root Ui spans the whole window; the app is laid out in the part nothing covers.
fn content_ui(app: &mut SolveApp, ui: &mut egui::Ui) {
    let content = ui.ctx().content_rect();
    let above = egui::Rect::from_min_max(ui.max_rect().min, egui::pos2(ui.max_rect().right(), content.top()));
    ui.painter().rect_filled(above, 0.0, solidcraft_ui_egui::theme::Tokens::get().app_bar);
    ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| app.ui(ui));
}

/// Settings written to the settings file soon after they change, as the desktop app does: eframe
/// saves them only every 30 s and when the app goes to the background.
#[derive(Default)]
struct PrefsWriter {
    /// The settings written (or read at start) last.
    written: Option<String>,
    /// The previous frame had input that can change settings.
    input_before: bool,
    last_write: Option<Instant>,
    /// A change waits until then to be written.
    pending_until: Option<Instant>,
}

impl PrefsWriter {
    /// The least time between two writes of the settings file.
    const INTERVAL: Duration = Duration::from_secs(1);

    fn read(prefs: String) -> Self {
        Self { written: Some(prefs), ..Self::default() }
    }

    fn due(&mut self, input: bool, now: Instant) -> bool {
        let due = input || self.input_before || self.pending_until.is_some_and(|at| now >= at);
        self.input_before = input;
        due
    }

    /// Write `prefs` if they changed and the last write is old enough; else how long a change
    /// waits (the caller asks for a frame then).
    fn store(&mut self, storage: &mut dyn eframe::Storage, prefs: String, now: Instant) -> Option<Duration> {
        if self.written.as_deref() == Some(prefs.as_str()) {
            self.pending_until = None;
            return None;
        }
        if let Some(wait) = self.last_write.map(|at| Self::INTERVAL.saturating_sub(now.saturating_duration_since(at))).filter(|w| !w.is_zero()) {
            self.pending_until = Some(now + wait);
            return Some(wait);
        }
        storage.set_string(solidcraft_ui_egui::prefs::STORAGE_KEY, prefs.clone());
        storage.flush();
        self.written = Some(prefs);
        self.last_write = Some(now);
        self.pending_until = None;
        None
    }
}

/// Input that can change a setting: a tap or click, a key, typed text, a pinch.
fn changes_settings(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::PointerButton { .. }
            | egui::Event::Key { .. }
            | egui::Event::Text(_)
            | egui::Event::Paste(_)
            | egui::Event::Ime(_)
            | egui::Event::MouseWheel { .. }
            | egui::Event::Zoom(_)
    )
}
