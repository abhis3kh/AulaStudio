use crate::keyboard_scene::{self, BoardView, SceneKey};
#[cfg(test)]
use crate::keyboard_scene::{key_path, polygon_triangles};
use crate::layout;
use crate::profiles::{self, Profile};
use crate::scene::Camera;
use crate::{
    live_rgb::LiveRgb,
    music::{self, MusicSession},
    music_dsp::{self, Mode, Palette, Settings as MusicSettings},
};
use aula_studio::{
    device::{DeviceStatus, Keyboard},
    macros::{self, MacroBinding, MacroEvent},
    protocol::{Lighting, Transport},
};
use eframe::egui::{self, Color32, FontId, RichText, Stroke, Vec2};
use serde::Deserialize;
use std::{
    sync::mpsc::{self, Receiver, Sender},
    time::Duration,
};

const ACCENT: Color32 = Color32::from_rgb(59, 224, 126);
const BACKGROUND: Color32 = Color32::from_rgb(7, 11, 9);
const PANEL: Color32 = Color32::from_rgb(13, 20, 16);
const MUTED: Color32 = Color32::from_rgb(133, 151, 140);
const EFFECTS: [(u8, &str); 20] = [
    (0, "Off"),
    (1, "Fixed on"),
    (2, "Respire"),
    (3, "Rainbow"),
    (4, "Flash away"),
    (5, "Raindrops"),
    (6, "Rainbow wheel"),
    (7, "Ripples shining"),
    (8, "Stars twinkle"),
    (9, "Shadow disappear"),
    (10, "Retro snake"),
    (11, "Neon stream"),
    (12, "Reaction"),
    (13, "Sine wave"),
    (14, "Retinue scanning"),
    (15, "Rotating windmill"),
    (16, "Colorful waterfall"),
    (17, "Blossoming"),
    (18, "Rotating storm"),
    (21, "Per-key"),
];
const COLORS: [(&str, [u8; 3]); 8] = [
    ("Red", [255, 0, 0]),
    ("Green", [0, 255, 0]),
    ("Blue", [0, 0, 255]),
    ("Yellow", [255, 255, 0]),
    ("Magenta", [255, 0, 255]),
    ("Cyan", [0, 255, 255]),
    ("White", [255, 255, 255]),
    ("Random", [172, 139, 255]),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Lighting,
    Music,
    Keys,
    Macros,
    Profiles,
    Settings,
}
impl Tab {
    fn name(self) -> &'static str {
        match self {
            Self::Lighting => "Lighting",
            Self::Music => "Music sync",
            Self::Keys => "Key assignments",
            Self::Macros => "Macros",
            Self::Profiles => "Profiles",
            Self::Settings => "Device settings",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Lighting => "Make the board yours. Hardware effects and per-key color.",
            Self::Music => "Let your keyboard follow the music playing on this computer.",
            Self::Keys => "Select a key, choose an action, and apply it to your board.",
            Self::Macros => "Build a sequence and store it on the keyboard.",
            Self::Profiles => "Keep your setups together, locally on this computer.",
            Self::Settings => "Connection details, power settings, and application information.",
        }
    }
}
#[derive(Deserialize)]
struct Key {
    label: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    matrix: usize,
}
enum Request {
    MusicStart(music::Shared),
    MusicStop,
    Refresh,
    Snapshot,
    Lighting(Lighting),
    Sleep(u16),
    Paint(Box<[[u8; 3]; 126]>),
    Keymap(u8),
    Remap(u8, usize, [u8; 4]),
    Macros(Vec<MacroBinding>),
    ClearMacroAssignments,
    Profile(Profile),
}
enum Reply {
    MusicStarted,
    MusicStopped(Result<Option<DeviceStatus>, String>),
    Status(DeviceStatus),
    Colors(Box<[[u8; 3]; 126]>),
    Keymap(u8, Vec<u8>),
    MacroBank(Vec<MacroBinding>),
    Done(String),
    CommandFinished,
    Error(String),
}

fn worker(context: egui::Context) -> (Sender<Request>, Receiver<Reply>) {
    let (tx, commands) = mpsc::channel();
    let (results, rx) = mpsc::channel();
    std::thread::Builder::new().name("f75-hid".into()).spawn(move || {
        let mut keyboard: Option<Keyboard> = None;
        let mut live: Option<LiveRgb> = None;
        loop {
            let request=if live.is_some() {
                match commands.recv_timeout(Duration::from_millis(10)) {
                    Ok(request)=>request,
                    Err(mpsc::RecvTimeoutError::Timeout)=>{
                        let update=match (live.as_mut(),keyboard.as_ref()) {
                            (Some(stream),Some(board))=>stream.tick(board),
                            _=>Err("The USB music connection was lost".into()),
                        };
                        if matches!(update,Ok(true)){continue;}
                        let restored=finish_music(&mut live,&mut keyboard);
                        let result=match (update,restored) {
                            (Err(error),Ok(_))=>Err(format!("Music sync stopped: {error}. Prior lighting was restored.")),
                            (_,result)=>result,
                        };
                        if results.send(Reply::MusicStopped(result)).is_err(){return;}
                        context.request_repaint();
                        continue;
                    },
                    Err(mpsc::RecvTimeoutError::Disconnected)=>{let _=finish_music(&mut live,&mut keyboard);return;}
                }
            }else{match commands.recv(){Ok(request)=>request,Err(_)=>return}};
            if matches!(request,Request::MusicStop) {
                // A cancelled audio worker may already have triggered restoration.
                // Do not emit a second stop reply that could cancel a new session.
                if live.is_none(){continue;}
                let result=finish_music(&mut live,&mut keyboard);
                if results.send(Reply::MusicStopped(result)).is_err(){return;}
                context.request_repaint();
                continue;
            }
            if live.is_some() {
                if results.send(Reply::Error("Stop music sync before changing device settings.".into())).is_err(){let _=finish_music(&mut live,&mut keyboard);return;}
                context.request_repaint();continue;
            }
            let starting_music=matches!(&request,Request::MusicStart(_));
            let result: Result<Vec<Reply>, String> = (|| {
                if matches!(&request, Request::Refresh) { keyboard = None; }
                if keyboard.is_none() { keyboard = Some(Keyboard::connect()?); }
                let board = keyboard.as_mut().ok_or("Keyboard unavailable")?;
                Ok(match request {
                    Request::MusicStart(shared)=>{live=Some(LiveRgb::start(board,shared)?);vec![Reply::MusicStarted]},
                    Request::MusicStop=>return Err("Music stop was not dispatched".into()),
                    Request::Refresh => vec![Reply::Status(board.status()?),Reply::Colors(Box::new(board.read_colors()?))],
                    Request::Snapshot => vec![Reply::Keymap(0,board.keymap(0)?),Reply::Keymap(1,board.keymap(1)?),Reply::Done("Both keymap layers read from the keyboard. Save a preset to keep them.".into())],
                    Request::Lighting(lighting) => vec![Reply::Status(board.apply_lighting(lighting)?), Reply::Done("Lighting applied and verified by readback.".into())],
                    Request::Sleep(seconds) => vec![Reply::Status(board.apply_sleep(seconds)?), Reply::Done("Sleep timer applied and verified by readback.".into())],
                    Request::Paint(colors) => {
                        let status=board.paint(&colors)?;
                        let actual=board.read_colors()?;
                        if actual!=*colors{return Err("Per-key color readback mismatch; refresh before retrying".into());}
                        vec![Reply::Status(status),Reply::Colors(Box::new(actual)),Reply::Done("Per-key colors applied and verified by readback.".into())]
                    },
                    Request::Keymap(layer) => vec![Reply::Keymap(layer,board.keymap(layer)?)],
                    Request::Remap(layer,matrix,code) => vec![Reply::Keymap(layer,board.remap(layer,matrix,code)?), Reply::Done("Key assignment applied and verified by readback.".into())],
                    Request::Macros(bindings) => {
                        let [base,function]=board.apply_macros(&bindings)?;
                        vec![Reply::Keymap(0,base),Reply::Keymap(1,function),Reply::Done("Macro bank applied and both keymap layers verified by readback.".into())]
                    },
                    Request::ClearMacroAssignments => {
                        let [base,function]=board.clear_macro_assignments()?;
                        vec![Reply::Keymap(0,base),Reply::Keymap(1,function),Reply::Done("Hardware macro assignments removed and verified. Former macro keys are disabled; assign normal actions in Key assignments. Inactive macro-bank data remains on the keyboard.".into())]
                    },
                    Request::Profile(profile) => {
                        let maps = profile.keymaps_for_apply()?;
                        board.apply_lighting(profile.lighting())?;
                        board.apply_sleep(profile.sleep_seconds)?;
                        if profile.lighting[0] == 21 {
                            let colors: [[u8;3];126] = profile.colors.as_slice().try_into().map_err(|_|"Invalid profile color count")?;
                            board.paint(&colors)?;
                            if board.read_colors()?!=colors{return Err("Profile RGB readback mismatch; some settings may have applied".into());}
                        }
                        let mut replies = Vec::new();
                        for (layer,map) in maps.iter().enumerate() {
                            board.keymap(layer as u8)?;
                            board.write(if board.transport==Transport::Usb {3}else{1},layer as u8,map)?;
                            let readback = board.keymap(layer as u8)?;
                            if readback != *map { return Err("Profile keymap readback mismatch; some profile settings may have applied".into()); }
                            replies.push(Reply::Keymap(layer as u8,readback));
                        }
                        if !profile.macros.is_empty() {
                            let [base,function]=board.apply_macros(&profile.macros)?;
                            replies.push(Reply::Keymap(0,base));
                            replies.push(Reply::Keymap(1,function));
                            replies.push(Reply::MacroBank(profile.macros.clone()));
                        }
                        if profile.lighting[0]==21 {replies.push(Reply::Colors(Box::new(board.read_colors()?)));}
                        replies.push(Reply::Status(board.status()?));
                        replies.push(Reply::Done(format!("Profile '{}' applied and verified.",profile.name)));
                        replies
                    }
                })
            })();
            match result { Ok(replies)=> {
                for reply in replies {
                    if results.send(reply).is_err() {let _=finish_music(&mut live,&mut keyboard);return;}
                }
                // Status/keymap replies can arrive on separate UI frames. Keep
                // the operation owned until every result has been delivered.
                if !starting_music && results.send(Reply::CommandFinished).is_err(){return;}
            }, Err(error)=> { keyboard=None; let reply=if starting_music {Reply::MusicStopped(Err(error))} else {Reply::Error(error)}; if results.send(reply).is_err() { return; } } }
            context.request_repaint();
        }
    }).expect("Cannot start HID worker");
    (tx, rx)
}

fn finish_music(
    live: &mut Option<LiveRgb>,
    keyboard: &mut Option<Keyboard>,
) -> Result<Option<DeviceStatus>, String> {
    if let Some(stream) = live.take() {
        let path = stream.checkpoint().display().to_string();
        let board = keyboard
            .as_mut()
            .ok_or_else(|| format!("USB recovery required. Retained snapshot: {path}"))?;
        stream.stop(board).map(Some)
    } else {
        Ok(None)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MusicUsb {
    Off,
    Starting,
    Active,
    Stopping,
}

pub struct Studio {
    music: Option<MusicSession>,
    music_settings: MusicSettings,
    music_to_keyboard: bool,
    music_usb: MusicUsb,
    music_error: Option<String>,
    music_frame: music_dsp::AudioFrame,
    music_colors: [[u8; 3]; 126],
    music_frames: u64,
    music_usb_frames: u64,
    music_sample_rate: u32,
    close_after_operation: bool,
    tab: Tab,
    keys: Vec<Key>,
    selected: usize,
    lighting: Lighting,
    sleep: u16,
    colors: [[u8; 3]; 126],
    colors_loaded: bool,
    paint: [u8; 3],
    status: Option<DeviceStatus>,
    tx: Sender<Request>,
    rx: Receiver<Reply>,
    busy: bool,
    notice: String,
    error: bool,
    layer: u8,
    keymaps: [Option<Vec<u8>>; 2],
    keymap_from_device: [bool; 2],
    action: usize,
    usage: u8,
    modifiers: u8,
    assignment_cursor: Option<(u8, usize)>,
    profiles: Vec<Profile>,
    profile_name: String,
    selected_profile: Option<usize>,
    profile_error: Option<String>,
    macro_name: String,
    macro_text: String,
    macro_playback: String,
    macro_delay: u32,
    macros: Vec<MacroBinding>,
    macro_confirm: bool,
    clear_macros_confirm: bool,
    macro_mode: u8,
    macro_events: Vec<MacroEvent>,
    macro_event_usage: u8,
    macro_event_pressed: bool,
    profile_confirm: bool,
    view_3d: bool,
    camera: Camera,
}
impl Studio {
    fn configure_style(context: &egui::Context) {
        context.set_theme(egui::Theme::Dark);
        let mut style = (*context.style_of(egui::Theme::Dark)).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = BACKGROUND;
        style.visuals.window_fill = PANEL;
        style.visuals.extreme_bg_color = Color32::from_rgb(7, 12, 9);
        style.visuals.selection.bg_fill = Color32::from_rgb(19, 66, 39);
        style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(22, 32, 26);
        style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(17, 26, 21);
        style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(38, 54, 44));
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(28, 58, 40);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(25, 85, 49);
        style.spacing.item_spacing = Vec2::new(10.0, 12.0);
        style.spacing.button_padding = Vec2::new(14.0, 10.0);
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(15.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(14.0));
        context.set_style_of(egui::Theme::Dark, style);
    }
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let context = &cc.egui_ctx;
        Self::configure_style(context);
        let (tx, rx) = worker(context.clone());
        let profile_result = profiles::load(&profiles::path());
        let (profiles, profile_error) = match profile_result {
            Ok(p) => (p, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let mut app = Self::with_channels(tx, rx, profiles, profile_error);
        app.send(Request::Refresh);
        #[cfg(feature = "capture")]
        {
            app.view_3d = std::env::var("AULA_CAPTURE_VIEW").unwrap_or_default() != "2d";
            if let Ok(orbit) = std::env::var("AULA_CAPTURE_ORBIT") {
                app.camera = Camera {
                    yaw: if orbit == "left" { -0.42 } else { 0.42 },
                    pitch: 0.95,
                };
            }
            app.tab = match std::env::var("AULA_CAPTURE_TAB")
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str()
            {
                "keys" => Tab::Keys,
                "music" => Tab::Music,
                "macros" => Tab::Macros,
                "profiles" => Tab::Profiles,
                "settings" => Tab::Settings,
                _ => Tab::Lighting,
            };
            if std::env::var_os("AULA_CAPTURE_READY").is_some() {
                let deadline = std::time::Instant::now() + Duration::from_secs(15);
                while std::time::Instant::now() < deadline {
                    app.poll();
                    if app.error || (!app.busy && app.colors_loaded) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                if app.tab == Tab::Keys && app.status.is_some() && !app.busy {
                    app.send(Request::Keymap(app.layer));
                    while std::time::Instant::now() < deadline {
                        app.poll();
                        if !app.busy {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        }
        #[cfg(feature = "capture")]
        // Eframe's screenshot helper exits the process directly. Restrict
        // auto-start captures to audio preview so it cannot interrupt USB recovery.
        if app.tab == Tab::Music && std::env::var("AULA_CAPTURE_AUDIO").as_deref() == Ok("preview")
        {
            app.music_to_keyboard = false;
            app.start_music(context);
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while std::time::Instant::now() < deadline {
                app.poll_music();
                if app.music_frames >= 15 || app.music_error.is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        app
    }
    fn with_channels(
        tx: Sender<Request>,
        rx: Receiver<Reply>,
        profiles: Vec<Profile>,
        profile_error: Option<String>,
    ) -> Self {
        Self {
            music: None,
            music_settings: MusicSettings::default(),
            music_to_keyboard: true,
            music_usb: MusicUsb::Off,
            music_error: None,
            music_frame: music_dsp::AudioFrame::default(),
            music_colors: [[0; 3]; 126],
            music_frames: 0,
            music_usb_frames: 0,
            music_sample_rate: 0,
            close_after_operation: false,
            tab: Tab::Lighting,
            keys: serde_json::from_str(include_str!("../assets/keyboard-layout.json"))
                .expect("Bundled keyboard geometry is invalid"),
            selected: 0,
            lighting: Lighting {
                effect: 3,
                brightness: 6,
                speed: 2,
                color: 0,
            },
            sleep: 300,
            colors: [[172, 139, 255]; 126],
            colors_loaded: false,
            paint: [172, 139, 255],
            status: None,
            tx,
            rx,
            busy: false,
            notice: "Discovering F75 interfaces. No settings are changed on launch.".into(),
            error: false,
            layer: 0,
            keymaps: [None, None],
            keymap_from_device: [false, false],
            action: 0,
            usage: 4,
            modifiers: 0,
            assignment_cursor: None,
            profiles,
            profile_name: "My setup".into(),
            selected_profile: None,
            profile_error,
            macro_name: "My macro".into(),
            macro_text: String::new(),
            macro_playback: String::new(),
            macro_delay: 40,
            macros: Vec::new(),
            macro_confirm: false,
            clear_macros_confirm: false,
            profile_confirm: false,
            macro_mode: 0,
            macro_events: Vec::new(),
            macro_event_usage: 4,
            macro_event_pressed: true,
            view_3d: true,
            camera: Camera::default(),
        }
    }
    fn send(&mut self, request: Request) {
        if self.music_usb != MusicUsb::Off
            && !matches!(request, Request::MusicStart(_) | Request::MusicStop)
        {
            self.notice = "Stop music sync before other keyboard operations.".into();
            self.error = true;
            return;
        }
        if self.busy && !matches!(request, Request::MusicStop) {
            return;
        }
        self.error = false;
        let notice = match &request {
            Request::MusicStart(_) => "Preparing USB music sync...",
            Request::MusicStop => "Stopping music and restoring lighting...",
            Request::Refresh | Request::Snapshot | Request::Keymap(_) => "Reading keyboard...",
            _ => "Applying changes...",
        };
        match self.tx.send(request) {
            Ok(()) => {
                self.busy = true;
                self.notice = notice.into();
            }
            Err(_) => {
                if self.music_usb != MusicUsb::Off {
                    self.music_worker_lost();
                } else {
                    self.busy = false;
                    self.status = None;
                    self.error = true;
                    self.notice = "The device worker stopped. Restart the app.".into();
                }
            }
        }
    }
    fn music_worker_lost(&mut self) {
        if let Some(mut session) = self.music.take() {
            session.stop();
        }
        self.music_usb = MusicUsb::Off;
        self.music_frame = music_dsp::AudioFrame::default();
        self.music_colors = [[0; 3]; 126];
        self.busy = false;
        self.status = None;
        self.error = true;
        self.notice = "The USB worker stopped. Lighting restoration could not be verified. Restart the app; retained music checkpoints are in %LOCALAPPDATA%\\AulaStudio\\recovery.".into();
        self.music_error = Some(self.notice.clone());
    }
    fn poll(&mut self) {
        loop {
            let reply = match self.rx.try_recv() {
                Ok(reply) => reply,
                Err(mpsc::TryRecvError::Disconnected) if self.music_usb != MusicUsb::Off => {
                    self.music_worker_lost();
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) if self.busy => {
                    self.busy = false;
                    self.status = None;
                    self.error = true;
                    self.notice = "The device worker stopped during an operation. Some changes may have applied. Restart the app and refresh the keyboard before retrying.".into();
                    break;
                }
                Err(_) => break,
            };
            match reply {
                Reply::MusicStarted => {
                    self.busy = false;
                    if self.music_usb != MusicUsb::Stopping {
                        self.music_usb = MusicUsb::Active;
                    }
                    self.notice =
                        "Music sync is streaming to USB. Stop restores the prior lighting.".into();
                }
                Reply::MusicStopped(result) => {
                    self.busy = false;
                    self.music_usb = MusicUsb::Off;
                    if let Some(mut session) = self.music.take() {
                        if let Ok(state) = session.snapshot() {
                            self.music_usb_frames = state.usb_frames;
                            if let Some(error) = state.error {
                                self.music_error = Some(error);
                            }
                        }
                        session.stop();
                    }
                    match result {
                        Ok(status) => {
                            if let Some(status) = status {
                                self.status = Some(status);
                            }
                            if let Some(error) = &self.music_error {
                                self.notice =
                                    format!("{error}. Prior lighting restored and verified.");
                                self.error = true;
                            } else {
                                self.notice =
                                    "Music stopped. Prior lighting restored and verified.".into();
                                self.error = false;
                            }
                        }
                        Err(error) => {
                            self.music_error = Some(error.clone());
                            self.notice = error;
                            self.error = true;
                            self.status = None;
                        }
                    }
                    self.music_frame = music_dsp::AudioFrame::default();
                    self.music_colors = [[0; 3]; 126];
                }
                Reply::Status(status) => {
                    self.lighting = status.lighting;
                    self.sleep = status.sleep_seconds.unwrap_or(self.sleep);
                    self.status = Some(status);
                    self.notice = "Connected. Settings read from your keyboard.".into();
                    self.error = false;
                }
                Reply::Colors(colors) => {
                    self.colors = *colors;
                    self.colors_loaded = true;
                }
                Reply::Keymap(layer, map) => {
                    self.keymaps[layer as usize] = Some(map);
                    self.keymap_from_device[layer as usize] = true;
                    self.assignment_cursor = None;
                    self.notice = "Keymap read from your keyboard.".into();
                    self.error = false;
                }
                Reply::MacroBank(bindings) => {
                    self.macros = bindings;
                    self.macro_confirm = false;
                }
                Reply::Done(message) => {
                    self.notice = message;
                    self.error = false;
                }
                Reply::CommandFinished => self.busy = false,
                Reply::Error(error) => {
                    self.busy = false;
                    self.notice = error;
                    self.error = true;
                    self.status = None;
                }
            }
        }
    }
    fn info(ui: &mut egui::Ui, text: impl Into<String>) {
        ui.label(RichText::new(text.into()).color(MUTED).size(13.0));
    }
    fn title(ui: &mut egui::Ui, text: &str) {
        ui.label(RichText::new(text).strong().size(18.0));
    }
    fn selected_label(&self) -> &str {
        self.keys
            .iter()
            .find(|key| key.matrix == self.selected)
            .map_or("Key", |key| key.label.as_str())
    }
    fn connection_name(&self) -> String {
        self.status
            .as_ref()
            .map(|s| {
                format!(
                    "{}  /  {}",
                    s.product,
                    match s.transport {
                        Transport::Usb => "USB",
                        Transport::Receiver => "2.4 GHz (unsupported in the USB edition)",
                        Transport::Bluetooth => "Bluetooth",
                    }
                )
            })
            .unwrap_or_else(|| "F75 disconnected".into())
    }
    fn can_apply(&self) -> bool {
        !self.busy
            && self.music_usb == MusicUsb::Off
            && self
                .status
                .as_ref()
                .is_some_and(|status| status.transport == Transport::Usb)
    }
    fn keyboard(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let height = if self.tab == Tab::Music {
            (width * 0.5)
                .clamp(240.0, 440.0)
                .min((ui.ctx().content_rect().height() - 512.0).max(144.0))
        } else if self.tab == Tab::Macros {
            (width * 0.46).clamp(240.0, 340.0)
        } else {
            (width * 0.68)
                .clamp(300.0, 610.0)
                .min((ui.ctx().content_rect().height() - 345.0).max(220.0))
        };
        let (outer, surface) =
            ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click_and_drag());
        if self.view_3d && surface.dragged() {
            self.camera.orbit(ui.input(|input| input.pointer.delta()));
        }
        let blend = ui
            .ctx()
            .animate_bool_with_time(ui.id().with("view_blend"), self.view_3d, 0.28);
        let view = BoardView::new(outer, self.camera.blended(blend));
        let keys = self
            .keys
            .iter()
            .filter_map(|key| {
                layout::from_oem(key.x, key.y, key.width, key.height, key.matrix).map(|geometry| {
                    let rgb = if self.tab == Tab::Music {
                        self.music_colors[key.matrix]
                    } else if self.lighting.effect == 21 {
                        self.colors[key.matrix]
                    } else if matches!(self.lighting.effect, 3 | 6 | 11 | 16 | 17 | 18) {
                        preview_spectrum(key.x / 650.0)
                    } else if self.lighting.color == 7 {
                        COLORS[key.matrix % 7].1
                    } else {
                        COLORS[self.lighting.color as usize].1
                    };
                    SceneKey {
                        geometry,
                        matrix: key.matrix,
                        label: &key.label,
                        rgb,
                    }
                })
            })
            .collect::<Vec<_>>();
        let hovered = ui
            .input(|input| input.pointer.hover_pos())
            .filter(|point| outer.contains(*point))
            .and_then(|point| keyboard_scene::hit_key(view, &keys, point));
        if surface.clicked()
            && let Some(matrix) = hovered
        {
            self.selected = matrix;
            if self.tab == Tab::Lighting
                && self.lighting.effect == 21
                && self.colors_loaded
                && !self.busy
            {
                self.colors[matrix] = self.paint;
            }
        }
        if surface.double_clicked() && hovered.is_none() {
            self.camera = Camera::default();
        }
        keyboard_scene::paint(
            ui.painter(),
            view,
            &keys,
            self.selected,
            hovered.filter(|_| surface.hovered()),
            surface.is_pointer_button_down_on(),
            if self.tab == Tab::Music {
                self.music.is_some()
            } else {
                self.lighting.effect != 0 && self.lighting.brightness > 0
            },
        );
        if self.view_3d {
            surface.clone().on_hover_cursor(if surface.dragged() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            });
        }
        if let Some(matrix) = hovered
            && let Some(key) = keys.iter().find(|key| key.matrix == matrix)
        {
            surface.on_hover_text(format!("{} / click to select", key.label));
        }
        ui.horizontal(|ui| {
            Self::info(ui, "F75 / 75% layout");
            ui.separator();
            Self::info(ui, format!("Selected: {}", self.selected_label()));
            if self.view_3d {
                Self::info(
                    ui,
                    if width < 620.0 {
                        "Drag to orbit"
                    } else {
                        "Drag to orbit / double-click background to reset"
                    },
                );
            }
        });
    }
    fn start_music(&mut self, context: &egui::Context) {
        if self.music.is_some() || self.busy {
            return;
        }
        self.music_error = None;
        self.music_frames = 0;
        self.music_usb_frames = 0;
        self.music_sample_rate = 0;
        let positions = self
            .keys
            .iter()
            .filter_map(|key| {
                layout::from_oem(key.x, key.y, key.width, key.height, key.matrix).map(|geometry| {
                    let center = geometry.legend_center();
                    (
                        key.matrix,
                        ((center.x - 20.0) / 640.0).clamp(0.0, 1.0),
                        ((center.y - 44.0) / 216.0).clamp(0.0, 1.0),
                    )
                })
            })
            .collect();
        match MusicSession::start(context.clone(), self.music_settings, positions) {
            Ok(session) => {
                self.music = Some(session);
                self.notice = "Opening Windows playback audio...".into();
                self.error = false;
            }
            Err(error) => {
                self.music_error = Some(error.clone());
                self.notice = error;
                self.error = true;
            }
        }
    }
    fn stop_music(&mut self) {
        if let Some(mut session) = self.music.take() {
            if let Ok(state) = session.snapshot() {
                self.music_usb_frames = state.usb_frames;
                if let Some(error) = state.error {
                    self.music_error = Some(error);
                }
            }
            session.stop();
        }
        self.music_frame = music_dsp::AudioFrame::default();
        self.music_colors = [[0; 3]; 126];
        if self.music_usb != MusicUsb::Off {
            self.music_usb = MusicUsb::Stopping;
            self.send(Request::MusicStop);
        } else if let Some(error) = &self.music_error {
            self.notice = error.clone();
            self.error = true;
        } else {
            self.notice = "Music listening stopped.".into();
        }
    }
    fn poll_music(&mut self) {
        let Some((snapshot, shared)) = self
            .music
            .as_ref()
            .map(|session| (session.snapshot(), session.shared.clone()))
        else {
            return;
        };
        match snapshot {
            Ok(state) => {
                if self.music_frames == 0 && state.frames > 0 && !self.music_to_keyboard {
                    self.notice = "Listening to Windows playback. Audio preview is live.".into();
                    self.error = false;
                }
                self.music_frame = state.frame;
                self.music_colors = state.colors;
                self.music_frames = state.frames;
                self.music_usb_frames = state.usb_frames;
                self.music_sample_rate = state.sample_rate;
                if !state.active {
                    if let Some(error) = state.error {
                        self.music_error = Some(error);
                    }
                    self.stop_music();
                } else if state.ready
                    && self.music_to_keyboard
                    && self.music_usb == MusicUsb::Off
                    && self.can_apply()
                {
                    self.music_usb = MusicUsb::Starting;
                    self.send(Request::MusicStart(shared));
                }
            }
            Err(error) => {
                self.music_error = Some(error);
                self.stop_music();
            }
        }
    }
    fn music_controls(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Audio sync").size(28.0).strong());
        Self::info(
            ui,
            "Reacts to music playing through your current speakers or headphones.",
        );
        ui.add_space(10.0);
        let listening = self.music.is_some();
        let state = match self.music_usb {
            MusicUsb::Starting => "Preparing USB...",
            MusicUsb::Active => "Listening / USB sync",
            MusicUsb::Stopping => "Restoring lighting...",
            MusicUsb::Off if listening => {
                if self.music_frames > 0 {
                    "Listening / preview"
                } else {
                    "Opening audio..."
                }
            }
            MusicUsb::Off => "Ready to listen",
        };
        ui.colored_label(if listening { ACCENT } else { MUTED }, state);
        ui.add_enabled_ui(!listening && self.music_usb == MusicUsb::Off, |ui| {
            ui.checkbox(&mut self.music_to_keyboard, "Sync the USB keyboard");
        });
        if listening || self.music_usb != MusicUsb::Off {
            if ui
                .add_sized(
                    [ui.available_width(), 38.0],
                    egui::Button::new("Stop music sync"),
                )
                .clicked()
            {
                self.stop_music();
            }
        } else if ui
            .add_enabled(
                !self.busy && (!self.music_to_keyboard || self.can_apply()),
                egui::Button::new("Start music sync")
                    .fill(Color32::from_rgb(23, 79, 45))
                    .min_size(Vec2::new(ui.available_width(), 38.0)),
            )
            .clicked()
        {
            self.start_music(ui.ctx());
        }
        if self.music_to_keyboard {
            Self::info(
                ui,
                "Stop restores your previous lighting. Keep the app open while syncing.",
            );
        } else {
            Self::info(ui, "Preview only. The keyboard will not be changed.");
        }
        ui.add_space(8.0);
        ui.separator();
        Self::title(ui, "Effect");
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.music_settings.mode, Mode::Spectrum, "Spectrum");
            ui.selectable_value(&mut self.music_settings.mode, Mode::Pulse, "Beat pulse");
            ui.selectable_value(&mut self.music_settings.mode, Mode::Wave, "Rhythm wave");
        });
        ui.add_space(8.0);
        Self::title(ui, "Palette");
        music_palette_picker(ui, &mut self.music_settings.palette);
        ui.add_space(8.0);
        music_slider(ui, "Gain", &mut self.music_settings.gain, 0.25..=8.0);
        music_slider(
            ui,
            "Smoothing",
            &mut self.music_settings.smoothing,
            0.0..=0.95,
        );
        music_slider(
            ui,
            "Brightness",
            &mut self.music_settings.brightness,
            0.0..=1.0,
        );
        if let Some(session) = &self.music {
            session.update(self.music_settings);
        }
        ui.add_space(6.0);
        ui.separator();
        Self::info(
            ui,
            "Audio is processed on this computer and stays in memory.",
        );
        ui.collapsing("Connection details", |ui| {
            Self::info(ui, "Playback audio / Windows WASAPI");
            if self.music_sample_rate > 0 {
                Self::info(
                    ui,
                    format!(
                        "{} Hz / {} audio frames",
                        self.music_sample_rate, self.music_frames
                    ),
                );
            }
            if self.music_usb_frames > 0 {
                Self::info(ui, format!("{} USB frames accepted", self.music_usb_frames));
            }
        });
        if let Some(error) = &self.music_error {
            ui.colored_label(Color32::from_rgb(255, 132, 143), error);
        }
        if !self
            .status
            .as_ref()
            .is_some_and(|status| status.transport == Transport::Usb)
            && self.music_to_keyboard
        {
            Self::info(
                ui,
                "Connect wired USB, or turn off keyboard sync to preview audio.",
            );
        }
    }
    fn music_meter(&self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(PANEL)
            .corner_radius(10)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("LIVE SPECTRUM")
                            .size(10.0)
                            .strong()
                            .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(if self.music_frame.beat > 0.4 {
                                "BEAT"
                            } else {
                                ""
                            })
                            .size(11.0)
                            .color(ACCENT),
                        );
                    });
                });
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 68.0),
                    egui::Sense::hover(),
                );
                let step = rect.width() / music_dsp::BANDS as f32;
                for (i, level) in self.music_frame.bands.iter().enumerate() {
                    let background = egui::Rect::from_min_max(
                        egui::pos2(rect.left() + step * i as f32 + 2.0, rect.top()),
                        egui::pos2(rect.left() + step * (i + 1) as f32 - 2.0, rect.bottom()),
                    );
                    ui.painter()
                        .rect_filled(background, 3.0, Color32::from_rgb(20, 32, 26));
                    let rgb = preview_spectrum(i as f32 / (music_dsp::BANDS - 1) as f32);
                    let mut bar = background;
                    bar.min.y = bar.max.y - background.height() * level.clamp(0.0, 1.0);
                    ui.painter()
                        .rect_filled(bar, 3.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Bass").size(10.0).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("Treble").size(10.0).color(MUTED));
                    });
                });
            });
    }
    fn lighting_controls(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 6.0;
        let effect = EFFECTS
            .iter()
            .find(|(id, _)| *id == self.lighting.effect)
            .map_or("Hardware effect", |(_, name)| *name);
        ui.label(RichText::new(effect).size(28.0).strong());
        Self::info(ui, "Built-in effect / runs on the keyboard");
        ui.add_space(12.0);
        let quick_effects = [
            (1, "Fixed on"),
            (2, "Respire"),
            (3, "Rainbow"),
            (7, "Ripples"),
            (11, "Neon stream"),
            (21, "Per-key"),
        ];
        let tile_width = (ui.available_width() - ui.spacing().item_spacing.x) * 0.5;
        for row in quick_effects.chunks(2) {
            ui.horizontal(|ui| {
                for &(id, label) in row {
                    if ui
                        .add_sized(
                            [tile_width, 32.0],
                            egui::Button::new(label).selected(self.lighting.effect == id),
                        )
                        .clicked()
                    {
                        self.lighting.effect = id;
                    }
                }
            });
        }
        ui.add_space(4.0);
        egui::ComboBox::from_id_salt("effect")
            .selected_text(
                EFFECTS
                    .iter()
                    .find(|(id, _)| *id == self.lighting.effect)
                    .map_or("Unknown", |(_, label)| *label),
            )
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for (id, label) in EFFECTS {
                    ui.selectable_value(&mut self.lighting.effect, id, label);
                }
            });
        ui.add_space(12.0);
        lighting_slider(ui, "Brightness", &mut self.lighting.brightness, 9);
        lighting_slider(ui, "Speed", &mut self.lighting.speed, 4);
        Self::info(ui, "These sliders edit a draft. Apply writes to hardware.");
        if ui
            .add_enabled(
                self.can_apply(),
                egui::Button::new("Apply lighting")
                    .fill(Color32::from_rgb(23, 79, 45))
                    .min_size(Vec2::new(ui.available_width(), 38.0)),
            )
            .clicked()
        {
            self.send(Request::Lighting(self.lighting));
        }
        ui.separator();
        if self.lighting.effect == 21 {
            Self::title(ui, "Per-key color");
            ui.horizontal(|ui| {
                ui.color_edit_button_srgb(&mut self.paint);
                ui.label("Brush color");
            });
            Self::info(
                ui,
                "Click a key to paint it. Apply sends the whole color map.",
            );
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.colors_loaded && !self.busy,
                        egui::Button::new("Fill all keys"),
                    )
                    .clicked()
                {
                    self.colors.fill(self.paint);
                }
                if ui
                    .add_enabled(
                        self.colors_loaded && self.can_apply(),
                        egui::Button::new("Apply colors"),
                    )
                    .clicked()
                {
                    self.send(Request::Paint(Box::new(self.colors)));
                }
            });
            Self::info(
                ui,
                if self.colors_loaded {
                    "Color map is ready. Apply verifies hardware readback."
                } else {
                    "Refresh the connection to read existing colors before painting."
                },
            );
        } else {
            Self::title(ui, "Color palette");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.spacing_mut().button_padding = Vec2::splat(4.0);
                for (index, (name, rgb)) in COLORS.iter().enumerate() {
                    let button = egui::Button::new(
                        RichText::new(if self.lighting.color as usize == index {
                            "+"
                        } else {
                            " "
                        })
                        .color(if matches!(index, 1 | 3 | 5 | 6) {
                            Color32::BLACK
                        } else {
                            Color32::WHITE
                        }),
                    )
                    .fill(Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
                    .min_size(Vec2::splat(28.0));
                    if ui.add(button).on_hover_text(*name).clicked() {
                        self.lighting.color = index as u8;
                    }
                }
            });
            ui.label(COLORS[self.lighting.color as usize].0);
            Self::info(
                ui,
                "Effect support for color depends on the keyboard firmware.",
            );
        }
    }
    fn keymap_layer_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let old = self.layer;
            ui.selectable_value(&mut self.layer, 0, "Base layer");
            ui.selectable_value(&mut self.layer, 1, "Fn layer");
            if self.layer != old && self.keymaps[self.layer as usize].is_none() && !self.busy {
                self.send(Request::Keymap(self.layer));
            }
            if ui
                .add_enabled(
                    !self.busy && self.status.is_some(),
                    egui::Button::new("Read keymap"),
                )
                .clicked()
            {
                self.send(Request::Keymap(self.layer));
            }
        });
    }
    fn assignment_controls(&mut self, ui: &mut egui::Ui) {
        let cursor = (self.layer, self.selected);
        if self.assignment_cursor != Some(cursor)
            && let Some(map) = &self.keymaps[self.layer as usize]
        {
            let code = &map[self.selected * 4..self.selected * 4 + 4];
            self.action = match code {
                [0, 0, 0, 0] => 2,
                [0, modifier, 0, usage] if *modifier <= 15 && *usage > 0 => 0,
                [2, 0, 0, _] => 1,
                _ => 3,
            };
            self.modifiers = if self.action == 0 { code[1] } else { 0 };
            self.usage = code[3];
            self.assignment_cursor = Some(cursor);
        }
        ui.spacing_mut().item_spacing.y = 8.0;
        Self::title(ui, &format!("Assign {}", self.selected_label()));
        if let Some(map) = &self.keymaps[self.layer as usize] {
            ui.label(format!(
                "{}: {}",
                if self.keymap_from_device[self.layer as usize] {
                    "Hardware"
                } else {
                    "Loaded preset"
                },
                assignment_name(&map[self.selected * 4..self.selected * 4 + 4])
            ));
        } else {
            Self::info(ui, "Read the keymap before applying a key assignment.");
        }
        let previous_action = self.action;
        egui::ComboBox::from_id_salt("action")
            .selected_text(
                [
                    "Keyboard key / shortcut",
                    "Media action",
                    "Disable key",
                    "Keep current firmware action",
                ][self.action],
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.action, 0, "Keyboard key / shortcut");
                ui.selectable_value(&mut self.action, 1, "Media action");
                ui.selectable_value(&mut self.action, 2, "Disable key");
            });
        if self.action != previous_action {
            self.usage = if self.action == 1 { 0xe9 } else { 4 };
            self.modifiers = 0;
        }
        if self.action == 0 {
            egui::ComboBox::from_id_salt("usage")
                .selected_text(usage_name(self.usage))
                .width(180.0)
                .show_ui(ui, |ui| {
                    for (usage, name) in usages() {
                        ui.selectable_value(&mut self.usage, usage, name);
                    }
                });
            ui.horizontal(|ui| {
                for (name, mask) in [("Ctrl", 1), ("Shift", 2), ("Alt", 4), ("Win", 8)] {
                    let mut active = self.modifiers & mask != 0;
                    if ui.checkbox(&mut active, name).changed() {
                        if active {
                            self.modifiers |= mask;
                        } else {
                            self.modifiers &= !mask;
                        }
                    }
                }
            });
        } else if self.action == 1 {
            egui::ComboBox::from_id_salt("media")
                .selected_text(media_name(self.usage))
                .show_ui(ui, |ui| {
                    for (code, name) in [
                        (0xe9, "Volume up"),
                        (0xea, "Volume down"),
                        (0xe2, "Mute"),
                        (0xcd, "Play / pause"),
                        (0xb5, "Next track"),
                        (0xb6, "Previous track"),
                        (0xb7, "Stop"),
                    ] {
                        ui.selectable_value(&mut self.usage, code, name);
                    }
                });
        }
        if self.action == 3 {
            Self::info(ui, "Choose a new action above to replace this assignment.");
        }
        let ready =
            self.can_apply() && self.keymaps[self.layer as usize].is_some() && self.action < 3;
        if ui
            .add_enabled(
                ready,
                egui::Button::new("Apply key assignment").fill(Color32::from_rgb(23, 79, 45)),
            )
            .clicked()
        {
            let code = action_code(self.action, self.modifiers, self.usage);
            self.send(Request::Remap(self.layer, self.selected, code));
        }
        Self::info(
            ui,
            "Fn layer changes alter hardware shortcuts. Preserve a profile before editing important keys.",
        );
    }
    fn macro_playback_field(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.horizontal(|ui| {
            Self::title(ui, "Test playback");
            if ui.button("Clear test output").clicked() {
                self.macro_playback.clear();
            }
        });
        Self::info(ui, "Click here and press the programmed key. Session only.");
        ui.add(
            egui::TextEdit::multiline(&mut self.macro_playback)
                .id_salt("macro_playback")
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .hint_text("Macro output appears here"),
        );
    }
    fn macro_sequence(&mut self, ui: &mut egui::Ui) {
        Self::title(ui, "Macro sequence");
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut self.macro_name);
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.macro_mode, 0, "Text");
            ui.selectable_value(&mut self.macro_mode, 1, "Key sequence");
        });
        if self.macro_mode == 0 {
            ui.add(
                egui::TextEdit::multiline(&mut self.macro_text)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text("Text to type with the selected key"),
            );
        } else {
            egui::ComboBox::from_id_salt("macro_usage")
                .selected_text(usage_name(self.macro_event_usage))
                .show_ui(ui, |ui| {
                    for (usage, name) in usages() {
                        ui.selectable_value(&mut self.macro_event_usage, usage, name);
                    }
                });
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.macro_event_pressed, true, "Key down");
                ui.selectable_value(&mut self.macro_event_pressed, false, "Key up");
            });
            ui.horizontal(|ui| {
                if ui.button("Append event").clicked() {
                    self.macro_events.push(MacroEvent {
                        usage: self.macro_event_usage,
                        pressed: self.macro_event_pressed,
                        delay_ms: self.macro_delay,
                    });
                }
                if ui.button("Clear sequence").clicked() {
                    self.macro_events.clear();
                }
            });
            let mut remove = None;
            egui::ScrollArea::vertical()
                .id_salt("macro_events")
                .max_height(100.0)
                .show(ui, |ui| {
                    for (index, event) in self.macro_events.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!(
                                "{} {} / {} ms",
                                usage_name(event.usage),
                                if event.pressed { "down" } else { "up" },
                                event.delay_ms
                            ));
                            if ui.small_button("x").clicked() {
                                remove = Some(index);
                            }
                        });
                    }
                });
            if let Some(index) = remove {
                self.macro_events.remove(index);
            }
            Self::info(
                ui,
                "Every key down needs a matching key up. Use modifier keys to build shortcuts.",
            );
        }
        ui.add(egui::Slider::new(&mut self.macro_delay, 0..=1000).text("Delay (ms)"));
        Self::info(
            ui,
            format!(
                "Assign to {}. Text macros use a US keyboard layout.",
                self.selected_label()
            ),
        );
        if ui.button("Add / replace macro for selected key").clicked() {
            let events = if self.macro_mode == 0 {
                macros::text_events(&self.macro_text, self.macro_delay)
            } else {
                Ok(self.macro_events.clone())
            };
            match events {
                Ok(events) => {
                    let binding = MacroBinding {
                        name: self.macro_name.trim().into(),
                        matrix: self.selected,
                        events,
                    };
                    let mut bank = self.macros.clone();
                    bank.retain(|m| m.matrix != self.selected);
                    bank.push(binding);
                    match macros::encode(&bank) {
                        Ok(_) => {
                            self.macros = bank;
                            self.macro_confirm = false;
                            self.notice =
                                "Macro added to the local draft. Apply bank to write to hardware."
                                    .into();
                            self.error = false;
                        }
                        Err(e) => {
                            self.notice = e;
                            self.error = true;
                        }
                    }
                }
                Err(e) => {
                    self.notice = e;
                    self.error = true;
                }
            }
        }
    }
    fn macro_bank_list(&mut self, ui: &mut egui::Ui) {
        Self::title(ui, "Macro bank");
        if self.macros.is_empty() {
            Self::info(
                ui,
                "No macros in the local draft. Create a sequence to add one.",
            );
        }
        let mut remove = None;
        let mut edit = None;
        egui::ScrollArea::vertical()
            .max_height(100.0)
            .show(ui, |ui| {
                for (index, binding) in self.macros.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "{} / key {} / {} events",
                            binding.name,
                            binding.matrix,
                            binding.events.len()
                        ));
                        if ui.small_button("Edit").clicked() {
                            edit = Some(index);
                        }
                        if ui.small_button("Remove").clicked() {
                            remove = Some(index);
                        }
                    });
                }
            });
        if let Some(index) = edit {
            let binding = &self.macros[index];
            self.macro_name = binding.name.clone();
            self.selected = binding.matrix;
            self.macro_events = binding.events.clone();
            self.macro_mode = 1;
        }
        if let Some(index) = remove {
            self.macros.remove(index);
            self.macro_confirm = false;
        }
    }
    fn macro_apply(&mut self, ui: &mut egui::Ui) {
        Self::info(
            ui,
            "Applying replaces the entire hardware macro bank, removes previous macro assignments, and assigns these keys on the base layer. USB only.",
        );
        ui.checkbox(&mut self.macro_confirm, "Replace existing hardware macros");
        let usb = self
            .status
            .as_ref()
            .is_some_and(|s| s.transport == Transport::Usb);
        if ui
            .add_enabled(
                usb && !self.busy && !self.macros.is_empty() && self.macro_confirm,
                egui::Button::new("Apply macro bank"),
            )
            .clicked()
        {
            self.macro_confirm = false;
            self.send(Request::Macros(self.macros.clone()));
        }
        if let Ok(bytes) = macros::encode(&self.macros) {
            Self::info(ui, format!("{} / 4096 bytes used", bytes.len()));
        }
    }
    fn macro_removal(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Remove hardware macro assignments",|ui|{
            Self::info(ui,"Removing hardware macro assignments disables every macro key on both layers. Remap those keys afterward. Inactive bank data remains stored.");
            ui.checkbox(&mut self.clear_macros_confirm,"Disable the existing hardware macro keys");
            if ui.add_enabled(self.can_apply()&&self.clear_macros_confirm,egui::Button::new("Remove hardware macro assignments")).clicked(){self.clear_macros_confirm=false;self.send(Request::ClearMacroAssignments);}
        });
    }
    fn save_profiles(&mut self, updated: Vec<Profile>) -> bool {
        if self.profile_error.is_some() {
            self.notice="Existing profile file could not be read. Repair it before saving to avoid overwriting your data.".into();
            self.error = true;
            return false;
        }
        match profiles::save(&profiles::path(), &updated) {
            Ok(()) => {
                self.profiles = updated;
                self.notice = "Profiles saved locally.".into();
                self.error = false;
                true
            }
            Err(e) => {
                self.notice = e;
                self.error = true;
                false
            }
        }
    }
    fn load_profile_draft(&mut self, profile: &Profile) -> Result<(), String> {
        profile.validate()?;
        let colors: [[u8; 3]; 126] = profile
            .colors
            .as_slice()
            .try_into()
            .map_err(|_| "Invalid profile color count")?;
        self.lighting = profile.lighting();
        self.sleep = profile.sleep_seconds;
        self.colors = colors;
        self.colors_loaded = true;
        self.macros = profile.macros.clone();
        // Omitted layers belong to this preset too: do not inherit an old draft.
        self.keymaps = [
            profile.keymaps.first().cloned(),
            profile.keymaps.get(1).cloned(),
        ];
        self.keymap_from_device = [false; 2];
        self.assignment_cursor = None;
        self.macro_confirm = false;
        self.clear_macros_confirm = false;
        self.profile_confirm = false;
        self.notice = format!(
            "Loaded '{}' locally. Device settings are unchanged.",
            profile.name
        );
        self.error = false;
        Ok(())
    }
    fn profile_from_draft(&self) -> Result<Profile, String> {
        if self.keymaps[0].is_none() && self.keymaps[1].is_some() {
            return Err("Read the base keymap before saving a preset with an Fn layer.".into());
        }
        let mut profile = Profile::new(
            self.profile_name.trim().into(),
            self.lighting,
            self.sleep,
            &self.colors,
        );
        profile.keymaps = self.keymaps.iter().flatten().cloned().collect();
        profile.macros = self.macros.clone();
        profile.validate()?;
        Ok(profile)
    }
    fn profile_editor(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = self.profile_error.clone() {
            ui.colored_label(Color32::from_rgb(255, 132, 143), error);
            if ui.button("Retry loading profiles").clicked() {
                match profiles::load(&profiles::path()) {
                    Ok(p) => {
                        self.profiles = p;
                        self.profile_error = None;
                    }
                    Err(e) => self.profile_error = Some(e),
                }
            }
        }
        Self::title(ui, "Your presets");
        Self::info(
            ui,
            "A preset stores lighting, sleep, custom colors, loaded keymaps, and macros authored in this app. Existing hardware macro-bank data is not captured.",
        );
        if ui
            .add_enabled(
                self.status.is_some() && !self.busy,
                egui::Button::new("Read both keymap layers to include in preset"),
            )
            .clicked()
        {
            self.send(Request::Snapshot);
        }
        ui.add_space(12.0);
        for index in 0..self.profiles.len() {
            let profile = &self.profiles[index];
            let subtitle = format!(
                "{} keymap layers / {} macros",
                profile.keymaps.len(),
                profile.macros.len()
            );
            if ui
                .selectable_label(
                    self.selected_profile == Some(index),
                    RichText::new(format!("{}\n{}", profile.name, subtitle)).size(16.0),
                )
                .clicked()
            {
                self.selected_profile = Some(index);
                self.profile_confirm = false;
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Preset name");
            ui.text_edit_singleline(&mut self.profile_name);
        });
        if ui.button("Save current draft as preset").clicked() {
            match self.profile_from_draft() {
                Ok(profile) => {
                    let mut updated = self.profiles.clone();
                    if let Some(index) = updated.iter().position(|p| p.name == profile.name) {
                        updated[index] = profile;
                    } else {
                        updated.push(profile);
                    }
                    self.save_profiles(updated);
                }
                Err(error) => {
                    self.notice = error;
                    self.error = true;
                }
            }
        }
        if let Some(index) = self.selected_profile.filter(|i| *i < self.profiles.len()) {
            let profile = self.profiles[index].clone();
            ui.horizontal(|ui| {
                if ui.button("Load into draft").clicked()
                    && let Err(error) = self.load_profile_draft(&profile)
                {
                    self.notice = error;
                    self.error = true;
                }
                if ui.button("Delete preset").clicked() {
                    let mut updated = self.profiles.clone();
                    updated.remove(index);
                    if self.save_profiles(updated) {
                        self.selected_profile = None;
                    }
                }
            });
            ui.checkbox(
                &mut self.profile_confirm,
                "Apply this preset to hardware, replacing included keymaps and macros",
            );
            let compatible = profile.macros.is_empty()
                || self
                    .status
                    .as_ref()
                    .is_some_and(|s| s.transport == Transport::Usb);
            if ui
                .add_enabled(
                    self.can_apply() && self.profile_confirm && compatible,
                    egui::Button::new("Apply selected preset").fill(Color32::from_rgb(23, 79, 45)),
                )
                .clicked()
            {
                self.profile_confirm = false;
                self.send(Request::Profile(profile));
            }
            Self::info(
                ui,
                "A failed operation can leave part of a preset applied. Refresh before retrying.",
            );
        }
        ui.add_space(20.0);
        Self::info(ui, format!("Stored at {}", profiles::path().display()));
    }
    fn settings(&mut self, ui: &mut egui::Ui) {
        Self::title(ui, "Power management");
        egui::ComboBox::from_id_salt("sleep")
            .selected_text(if self.sleep == 0 {
                "Never sleep".into()
            } else {
                format!("{} seconds", self.sleep)
            })
            .show_ui(ui, |ui| {
                for (seconds, _) in aula_studio::protocol::SLEEP {
                    ui.selectable_value(
                        &mut self.sleep,
                        seconds,
                        if seconds == 0 {
                            "Never sleep".into()
                        } else {
                            format!("{} seconds", seconds)
                        },
                    );
                }
            });
        if ui
            .add_enabled(self.can_apply(), egui::Button::new("Apply sleep timer"))
            .clicked()
        {
            self.send(Request::Sleep(self.sleep));
        }
        ui.add_space(20.0);
        ui.separator();
        Self::title(ui, "Connection");
        ui.label(self.connection_name());
        Self::info(
            ui,
            "This version uses wired USB for lighting, key assignments, macros, and power settings. Connect the USB cable and switch the keyboard to wired mode.",
        );
        Self::info(
            ui,
            "Close the original AULA driver if it is holding the device interface. Wake the keyboard before retrying.",
        );
        ui.add_space(20.0);
        ui.separator();
        Self::title(ui, "Aula Studio");
        Self::info(
            ui,
            format!(
                "Version {} / native Rust / no web runtime",
                env!("CARGO_PKG_VERSION")
            ),
        );
        Self::info(
            ui,
            "Device access runs on a dedicated worker. Launch and Refresh are read-only. Apply saves changes. Start music sync temporarily controls lighting until Stop or app exit.",
        );
        Self::info(
            ui,
            "No cloud account, telemetry, or internet connection is used by this app.",
        );
    }
}
impl Studio {
    fn render(&mut self, root: &mut egui::Ui) {
        let context = root.ctx().clone();
        let close_requested = context.input(|input| input.viewport().close_requested());
        if close_requested {
            if self.busy || self.music_usb != MusicUsb::Off {
                // Keep the process alive through persistent multi-report writes
                // and music restoration. Failures remain visible before exit.
                context.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.close_after_operation = true;
            }
            // Audio may become ready on this exact frame. Cancel pending capture
            // before it can dispatch a new USB session while the window exits.
            if self.music.is_some()
                || matches!(self.music_usb, MusicUsb::Starting | MusicUsb::Active)
            {
                self.stop_music();
            }
        }
        self.poll();
        if !close_requested && !self.close_after_operation {
            self.poll_music();
        }
        if self.close_after_operation && !self.busy && self.music_usb == MusicUsb::Off {
            self.close_after_operation = false;
            if self.error {
                if self.music_error.is_some() {
                    self.tab = Tab::Music;
                }
            } else {
                context.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if self.busy {
            context.request_repaint_after(Duration::from_millis(100));
        }
        egui::Panel::top("studio_header")
            .exact_size(68.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(14.0))
            .show(root, |ui| {
                ui.horizontal_centered(|ui| {
                    let (logo, _) = ui.allocate_exact_size(Vec2::splat(36.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(logo, 8.0, Color32::from_rgb(19, 55, 32));
                    for row in 0..3 {
                        for column in 0..3 {
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(
                                    logo.min
                                        + Vec2::new(
                                            8.0 + column as f32 * 7.0,
                                            8.0 + row as f32 * 7.0,
                                        ),
                                    Vec2::splat(4.0),
                                ),
                                1.0,
                                ACCENT,
                            );
                        }
                    }
                    ui.label(RichText::new("F75 STUDIO").size(21.0).strong());
                    ui.label(RichText::new("AULA").size(19.0).strong().color(ACCENT));
                    if ui.available_width() > 740.0 {
                        ui.add_space(18.0);
                        let name = self
                            .selected_profile
                            .and_then(|i| self.profiles.get(i))
                            .map_or("Profiles", |profile| profile.name.as_str());
                        egui::ComboBox::from_id_salt("header_profile")
                            .width(150.0)
                            .selected_text(name)
                            .show_ui(ui, |ui| {
                                if self.profiles.is_empty() {
                                    ui.label("Save a preset in Profiles");
                                }
                                for index in 0..self.profiles.len() {
                                    if ui
                                        .selectable_label(
                                            self.selected_profile == Some(index),
                                            &self.profiles[index].name,
                                        )
                                        .clicked()
                                    {
                                        self.selected_profile = Some(index);
                                        self.profile_confirm = false;
                                        self.tab = Tab::Profiles;
                                    }
                                }
                                if ui.button("Manage profiles").clicked() {
                                    self.tab = Tab::Profiles;
                                }
                            });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !self.busy && self.music_usb == MusicUsb::Off,
                                egui::Button::new("Refresh connection"),
                            )
                            .clicked()
                        {
                            self.keymaps = [None, None];
                            self.colors_loaded = false;
                            self.send(Request::Refresh);
                        }
                        egui::Frame::new()
                            .fill(Color32::from_rgb(16, 29, 21))
                            .stroke(Stroke::new(1.0, Color32::from_rgb(35, 58, 43)))
                            .corner_radius(10)
                            .inner_margin(8.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.colored_label(
                                        if self.status.is_some() { ACCENT } else { MUTED },
                                        if self.status.is_some() {
                                            "ONLINE"
                                        } else {
                                            "OFFLINE"
                                        },
                                    );
                                    ui.label(
                                        RichText::new(self.status.as_ref().map_or(
                                            "AULA F75",
                                            |s| match s.transport {
                                                Transport::Usb => "F75 / USB",
                                                Transport::Receiver => "F75 / 2.4G read only",
                                                Transport::Bluetooth => "F75 / Bluetooth",
                                            },
                                        ))
                                        .size(12.0),
                                    );
                                });
                            });
                    });
                });
            });
        egui::Panel::left("navigation")
            .exact_size(96.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(8.0))
            .show(root, |ui| {
                ui.add_space(8.0);
                for tab in [
                    Tab::Lighting,
                    Tab::Music,
                    Tab::Keys,
                    Tab::Macros,
                    Tab::Profiles,
                    Tab::Settings,
                ] {
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::new(79.0, 72.0), egui::Sense::hover());
                    let response =
                        ui.interact(rect, ui.id().with(tab as usize), egui::Sense::click());
                    let selected = self.tab == tab;
                    let active =
                        ui.ctx()
                            .animate_bool_with_time(response.id.with("active"), selected, 0.16);
                    let hover = ui.ctx().animate_bool_with_time(
                        response.id.with("hover"),
                        response.hovered(),
                        0.12,
                    );
                    if active > 0.0 || hover > 0.0 {
                        ui.painter().rect_filled(
                            rect,
                            9.0,
                            mix_color(
                                mix_color(PANEL, Color32::from_rgb(21, 32, 25), hover),
                                Color32::from_rgb(15, 48, 29),
                                active,
                            ),
                        );
                    }
                    if selected {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                rect.min + Vec2::new(0.0, 14.0),
                                Vec2::new(3.0, 44.0),
                            ),
                            2.0,
                            ACCENT,
                        );
                    }
                    let color = mix_color(MUTED, ACCENT, active);
                    paint_navigation_icon(
                        ui.painter(),
                        rect.center_top() + Vec2::new(0.0, 25.0),
                        tab,
                        color,
                    );
                    let label = match tab {
                        Tab::Lighting => "Lighting",
                        Tab::Music => "Music",
                        Tab::Keys => "Keys",
                        Tab::Macros => "Macros",
                        Tab::Profiles => "Profiles",
                        Tab::Settings => "Power",
                    };
                    ui.painter().text(
                        rect.center_bottom() - Vec2::new(0.0, 16.0),
                        egui::Align2::CENTER_CENTER,
                        label,
                        FontId::proportional(11.0),
                        color,
                    );
                    if response.clicked() {
                        self.tab = tab;
                        if tab == Tab::Keys
                            && self.keymaps[self.layer as usize].is_none()
                            && self.status.is_some()
                            && !self.busy
                        {
                            self.send(Request::Keymap(self.layer));
                        }
                    }
                    response.on_hover_text(tab.name());
                    ui.add_space(4.0);
                }
            });
        let has_inspector = matches!(
            self.tab,
            Tab::Lighting | Tab::Music | Tab::Keys | Tab::Macros
        );
        if has_inspector {
            egui::Panel::right("inspector")
                .exact_size(310.0)
                .resizable(false)
                .frame(egui::Frame::new().fill(PANEL).inner_margin(18.0))
                .show(root, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("inspector_scroll")
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(self.tab.name().to_uppercase())
                                    .size(10.0)
                                    .strong()
                                    .color(ACCENT),
                            );
                            ui.add_space(8.0);
                            match self.tab {
                                Tab::Lighting => self.lighting_controls(ui),
                                Tab::Music => self.music_controls(ui),
                                Tab::Keys => self.assignment_controls(ui),
                                Tab::Macros => {
                                    self.macro_sequence(ui);
                                    ui.separator();
                                    self.macro_apply(ui);
                                }
                                _ => {}
                            }
                        });
                });
        }
        egui::Panel::bottom("status")
            .frame(egui::Frame::new().fill(BACKGROUND).inner_margin(10.0))
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    if self.busy {
                        ui.spinner();
                    } else {
                        ui.colored_label(
                            if self.error {
                                Color32::from_rgb(255, 132, 143)
                            } else {
                                ACCENT
                            },
                            if self.error { "!" } else { "OK" },
                        );
                    }
                    ui.label(RichText::new(&self.notice).size(12.0));
                });
            });
        if self.tab != Tab::Macros {
            egui::Panel::bottom("device_summary")
                .exact_size(98.0)
                .frame(egui::Frame::new().fill(BACKGROUND).inner_margin(16.0))
                .show(root, |ui| {
                    let effect = EFFECTS
                        .iter()
                        .find(|(id, _)| *id == self.lighting.effect)
                        .map_or("Unknown", |(_, name)| *name);
                    let sleep = if self.sleep == 0 {
                        "Never".to_string()
                    } else {
                        format!("{} min", self.sleep / 60)
                    };
                    let transport = self
                        .status
                        .as_ref()
                        .map_or("Offline", |s| match s.transport {
                            Transport::Usb => "USB",
                            Transport::Receiver => "2.4G / read only",
                            Transport::Bluetooth => "Bluetooth",
                        });
                    let music = self.tab == Tab::Music;
                    let values = [
                        (
                            if music {
                                "MUSIC EFFECT"
                            } else {
                                "DRAFT EFFECT"
                            },
                            if music {
                                match self.music_settings.mode {
                                    Mode::Spectrum => "Spectrum",
                                    Mode::Pulse => "Beat pulse",
                                    Mode::Wave => "Rhythm wave",
                                }
                                .to_string()
                            } else {
                                effect.to_string()
                            },
                        ),
                        (
                            "BRIGHTNESS",
                            if music {
                                format!(
                                    "{}%",
                                    (self.music_settings.brightness * 100.0).round() as u32
                                )
                            } else {
                                format!("{} / 9", self.lighting.brightness)
                            },
                        ),
                        ("SLEEP", sleep),
                        ("CONNECTION", transport.to_string()),
                    ];
                    ui.columns(4, |columns| {
                        for (column, (label, value)) in columns.iter_mut().zip(values) {
                            egui::Frame::new()
                                .fill(PANEL)
                                .stroke(Stroke::new(1.0, Color32::from_rgb(29, 44, 35)))
                                .corner_radius(10)
                                .inner_margin(12.0)
                                .show(column, |ui| {
                                    ui.set_min_width((ui.available_width() - 2.0).max(0.0));
                                    ui.label(RichText::new(label).size(9.0).strong().color(MUTED));
                                    ui.label(RichText::new(value).size(15.0).strong());
                                });
                        }
                    });
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BACKGROUND).inner_margin(20.0))
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(self.tab.name().to_uppercase())
                            .color(ACCENT)
                            .size(12.0)
                            .strong(),
                    )
                    .on_hover_text(self.tab.description());
                    if has_inspector {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.selectable_value(&mut self.view_3d, true, "3D");
                            ui.selectable_value(&mut self.view_3d, false, "2D");
                            if ui.add_enabled(self.view_3d, egui::Button::new("Reset view")).on_hover_text("Restore the default camera angle").clicked() {
                                self.camera = Camera::default();
                            }
                            ui.label(RichText::new(if self.tab==Tab::Music {"Live audio preview"}else{"Draft preview"}).size(11.0).color(MUTED));
                        });
                    }
                });
                ui.add_space(12.0);
                egui::ScrollArea::vertical()
                    .id_salt("stage_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.tab {
                        Tab::Lighting => {
                            self.keyboard(ui);
                            Self::info(
                                ui,
                            "Illustrative effect preview. Apply sends the draft settings to the keyboard.",
                            );
                        }
                        Tab::Music => {
                            self.keyboard(ui);
                            ui.add_space(6.0);
                            self.music_meter(ui);
                        }
                        Tab::Keys => {
                            self.keymap_layer_controls(ui);
                            self.keyboard(ui);
                        }
                        Tab::Macros => {
                            self.keyboard(ui);
                            self.macro_playback_field(ui);
                            self.macro_bank_list(ui);
                            self.macro_removal(ui);
                        }
                        Tab::Profiles => self.profile_editor(ui),
                        Tab::Settings => self.settings(ui),
                    });
            });
    }
}
impl eframe::App for Studio {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.render(root);
    }
}

fn palette_swatch(painter: &egui::Painter, rect: egui::Rect, palette: Palette) {
    let mut mesh = egui::Mesh::default();
    let radius = 6.0_f32.min(rect.height() * 0.5);
    // Sample the same gradient used by the USB frames. Rounded strip edges avoid textures.
    let edge = |x: f32| {
        let distance = x.min(rect.width() - x);
        if distance < radius {
            radius
                - (radius * radius - (radius - distance).powi(2))
                    .max(0.0)
                    .sqrt()
        } else {
            0.0
        }
    };
    for step in 0..=64 {
        let fraction = step as f32 / 64.0;
        let x = rect.width() * fraction;
        let [r, g, b] = palette.color(fraction);
        let color = Color32::from_rgb(r as u8, g as u8, b as u8);
        mesh.colored_vertex(egui::pos2(rect.left() + x, rect.top() + edge(x)), color);
        mesh.colored_vertex(egui::pos2(rect.left() + x, rect.bottom() - edge(x)), color);
        if step < 64 {
            let index = step * 2;
            mesh.add_triangle(index, index + 1, index + 2);
            mesh.add_triangle(index + 1, index + 3, index + 2);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn music_palette_picker(ui: &mut egui::Ui, selected: &mut Palette) {
    let button_id = ui.make_persistent_id("music_palette");
    let popup_id = button_id.with("palette_menu");
    let is_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    // Consume Enter before the focused trigger can toggle the open menu closed.
    let keyboard_commit = is_open
        && ui.memory(|memory| memory.has_focus(button_id))
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    let (_, rect) = ui.allocate_space(Vec2::new(ui.available_width(), 68.0));
    let response = ui.interact(rect, button_id, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            format!("Music palette: {}", selected.name()),
        )
    });
    let state_id = popup_id.with("navigation");
    let selected_index = Palette::ALL
        .iter()
        .position(|palette| palette == selected)
        .unwrap_or(0);
    let mut navigation = ui.data_mut(|data| {
        data.get_temp::<(&'static str, usize)>(state_id)
            .unwrap_or((selected.family(), selected_index))
    });
    if response.clicked() {
        navigation = (selected.family(), selected_index);
        response.request_focus();
    }
    let highlight = is_open || response.hovered() || response.has_focus();
    ui.painter().rect(
        rect,
        10.0,
        if highlight {
            Color32::from_rgb(23, 37, 30)
        } else {
            PANEL
        },
        Stroke::new(
            1.0,
            if highlight {
                ACCENT
            } else {
                Color32::from_rgb(48, 67, 56)
            },
        ),
        egui::StrokeKind::Inside,
    );
    palette_swatch(
        ui.painter(),
        egui::Rect::from_min_size(rect.min + Vec2::new(11.0, 13.0), Vec2::new(52.0, 42.0)),
        *selected,
    );
    ui.painter().text(
        rect.min + Vec2::new(75.0, 15.0),
        egui::Align2::LEFT_TOP,
        selected.name(),
        FontId::proportional(15.0),
        Color32::from_rgb(229, 238, 232),
    );
    ui.painter().text(
        rect.min + Vec2::new(75.0, 38.0),
        egui::Align2::LEFT_TOP,
        format!("{} collection", selected.family()),
        FontId::proportional(11.0),
        MUTED,
    );
    let center = egui::pos2(rect.right() - 19.0, rect.center().y);
    let direction = if is_open { -1.0 } else { 1.0 };
    ui.painter().add(egui::Shape::line(
        vec![
            center + Vec2::new(-4.0, -2.0 * direction),
            center + Vec2::new(0.0, 2.0 * direction),
            center + Vec2::new(4.0, -2.0 * direction),
        ],
        Stroke::new(1.5, MUTED),
    ));
    egui::Popup::menu(&response)
        .id(popup_id)
        .width(rect.width().max(300.0))
        .gap(6.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .frame(
            egui::Frame::popup(ui.style())
                .fill(Color32::from_rgb(14, 22, 18))
                .stroke(Stroke::new(1.0, Color32::from_rgb(49, 71, 58)))
                .corner_radius(12)
                .inner_margin(10),
        )
        .show(|ui| {
            let mut family_changed = false;
            ui.spacing_mut().item_spacing = Vec2::new(5.0, 6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Color collection").size(14.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} palettes", Palette::ALL.len()))
                            .size(11.0)
                            .color(MUTED),
                    );
                });
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().button_padding = Vec2::new(12.0, 7.0);
                for family in ["Ocean", "Sunset", "Studio"] {
                    if ui
                        .selectable_label(navigation.0 == family, family)
                        .clicked()
                    {
                        family_changed = true;
                        response.request_focus();
                        navigation = (
                            family,
                            Palette::ALL
                                .iter()
                                .position(|palette| palette.family() == family)
                                .unwrap_or(0),
                        );
                    }
                }
            });
            ui.separator();
            let indices: Vec<_> = Palette::ALL
                .iter()
                .enumerate()
                .filter_map(|(index, palette)| (palette.family() == navigation.0).then_some(index))
                .collect();
            let mut cursor = indices
                .iter()
                .position(|index| *index == navigation.1)
                .unwrap_or(0);
            let mut moved = family_changed;
            ui.input_mut(|input| {
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    cursor = (cursor + 1) % indices.len();
                    moved = true;
                }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    cursor = (cursor + indices.len() - 1) % indices.len();
                    moved = true;
                }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::Home) {
                    cursor = 0;
                    moved = true;
                }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::End) {
                    cursor = indices.len() - 1;
                    moved = true;
                }
            });
            if moved {
                ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
                response.request_focus();
            }
            navigation.1 = indices[cursor];
            if keyboard_commit {
                *selected = Palette::ALL[navigation.1];
                ui.close();
            }
            egui::ScrollArea::vertical()
                .id_salt("palette_choices")
                .max_height(310.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for index in indices {
                        let palette = Palette::ALL[index];
                        let active = *selected == palette;
                        let (row, choice) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 53.0),
                            egui::Sense::click(),
                        );
                        choice.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::WidgetType::SelectableLabel,
                                ui.is_enabled(),
                                active,
                                palette.name(),
                            )
                        });
                        if choice.clicked() {
                            *selected = palette;
                            ui.close();
                        }
                        if (moved || response.clicked()) && index == navigation.1 {
                            choice.scroll_to_me(Some(egui::Align::Center));
                        }
                        if !ui.is_rect_visible(row) {
                            continue;
                        }
                        let focused =
                            choice.hovered() || choice.has_focus() || index == navigation.1;
                        ui.painter().rect(
                            row,
                            8.0,
                            if active {
                                Color32::from_rgb(22, 48, 34)
                            } else if focused {
                                Color32::from_rgb(29, 40, 34)
                            } else {
                                Color32::TRANSPARENT
                            },
                            Stroke::new(
                                1.0,
                                if active {
                                    Color32::from_rgb(59, 117, 80)
                                } else {
                                    Color32::TRANSPARENT
                                },
                            ),
                            egui::StrokeKind::Inside,
                        );
                        palette_swatch(
                            ui.painter(),
                            egui::Rect::from_min_size(
                                row.min + Vec2::new(8.0, 9.0),
                                Vec2::new(57.0, 35.0),
                            ),
                            palette,
                        );
                        ui.painter().text(
                            row.min + Vec2::new(76.0, 9.0),
                            egui::Align2::LEFT_TOP,
                            palette.name(),
                            FontId::proportional(14.0),
                            if active {
                                ACCENT
                            } else {
                                Color32::from_rgb(229, 238, 232)
                            },
                        );
                        ui.painter().text(
                            row.min + Vec2::new(76.0, 30.0),
                            egui::Align2::LEFT_TOP,
                            palette.description(),
                            FontId::proportional(10.0),
                            MUTED,
                        );
                        if active {
                            let mark = egui::pos2(row.right() - 13.0, row.top() + 16.0);
                            ui.painter().add(egui::Shape::line(
                                vec![
                                    mark + Vec2::new(-4.0, 0.0),
                                    mark + Vec2::new(-1.0, 3.0),
                                    mark + Vec2::new(5.0, -4.0),
                                ],
                                Stroke::new(1.5, ACCENT),
                            ));
                        }
                    }
                });
            ui.separator();
            ui.label(
                RichText::new("Up / Down to browse  ·  Enter to select")
                    .size(10.0)
                    .color(MUTED),
            );
        });
    ui.data_mut(|data| data.insert_temp(state_id, navigation));
}

impl Drop for Studio {
    fn drop(&mut self) {
        let waiting = self.busy || self.music_usb != MusicUsb::Off;
        self.stop_music();
        if waiting {
            let deadline = std::time::Instant::now() + Duration::from_secs(4);
            while std::time::Instant::now() < deadline {
                match self.rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(Reply::MusicStopped(_) | Reply::CommandFinished | Reply::Error(_)) => break,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    _ => {}
                }
            }
        }
    }
}
fn preview_spectrum(position: f32) -> [u8; 3] {
    const PALETTE: [[u8; 3]; 7] = [
        [48, 184, 244],
        [68, 77, 242],
        [158, 56, 236],
        [229, 37, 145],
        [246, 76, 44],
        [237, 180, 36],
        [51, 211, 122],
    ];
    let position = position.clamp(0.0, 1.0) * 6.0;
    let index = (position as usize).min(5);
    let fraction = position - index as f32;
    std::array::from_fn(|channel| {
        (PALETTE[index][channel] as f32 * (1.0 - fraction)
            + PALETTE[index + 1][channel] as f32 * fraction)
            .round() as u8
    })
}
fn mix_color(from: Color32, to: Color32, amount: f32) -> Color32 {
    let amount = amount.clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    Color32::from_rgb(
        channel(from.r(), to.r()),
        channel(from.g(), to.g()),
        channel(from.b(), to.b()),
    )
}
fn lighting_slider(ui: &mut egui::Ui, label: &str, value: &mut u8, maximum: u8) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.0).color(MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{} / {}", value, maximum))
                    .size(12.0)
                    .strong(),
            );
        });
    });
    ui.spacing_mut().slider_width = ui.available_width();
    ui.add(egui::Slider::new(value, 0..=maximum).show_value(false));
    ui.add_space(5.0);
}
fn music_slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.0).color(MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("{value:.2}")).size(12.0));
        });
    });
    ui.spacing_mut().slider_width = ui.available_width();
    ui.add(egui::Slider::new(value, range).show_value(false));
    ui.add_space(5.0);
}
fn paint_navigation_icon(painter: &egui::Painter, center: egui::Pos2, tab: Tab, color: Color32) {
    let stroke = Stroke::new(1.6, color);
    match tab {
        Tab::Music => {
            for (i, height) in [7.0, 15.0, 22.0, 12.0, 5.0].into_iter().enumerate() {
                let x = center.x - 10.0 + i as f32 * 5.0;
                painter.line_segment(
                    [
                        egui::pos2(x, center.y - height * 0.5),
                        egui::pos2(x, center.y + height * 0.5),
                    ],
                    stroke,
                );
            }
        }
        Tab::Lighting => {
            painter.circle_stroke(center - Vec2::new(0.0, 3.0), 7.0, stroke);
            painter.line_segment(
                [center + Vec2::new(-3.0, 6.0), center + Vec2::new(3.0, 6.0)],
                stroke,
            );
            painter.line_segment(
                [
                    center + Vec2::new(-2.0, 10.0),
                    center + Vec2::new(2.0, 10.0),
                ],
                stroke,
            );
        }
        Tab::Keys => {
            painter.rect_stroke(
                egui::Rect::from_center_size(center, Vec2::new(22.0, 14.0)),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            for row in 0..2 {
                for column in 0..4 {
                    painter.circle_filled(
                        center + Vec2::new(-7.0 + column as f32 * 4.5, -3.0 + row as f32 * 5.0),
                        1.0,
                        color,
                    );
                }
            }
        }
        Tab::Macros => {
            painter.add(egui::Shape::line(
                vec![
                    center + Vec2::new(3.0, -11.0),
                    center + Vec2::new(-7.0, 1.0),
                    center + Vec2::new(1.0, 1.0),
                    center + Vec2::new(-3.0, 11.0),
                    center + Vec2::new(8.0, -2.0),
                    center + Vec2::new(1.0, -2.0),
                ],
                stroke,
            ));
        }
        Tab::Profiles => {
            for offset in [5.0, 0.0, -5.0] {
                painter.add(egui::Shape::line(
                    vec![
                        center + Vec2::new(-10.0, offset),
                        center + Vec2::new(0.0, offset + 5.0),
                        center + Vec2::new(10.0, offset),
                    ],
                    stroke,
                ));
            }
        }
        Tab::Settings => {
            let points = (0..=18)
                .map(|i| {
                    let angle = 0.25 + i as f32 * 5.1 / 18.0;
                    center + Vec2::new(angle.cos(), angle.sin()) * 10.0
                })
                .collect();
            painter.add(egui::Shape::line(points, stroke));
            painter.line_segment(
                [center - Vec2::new(0.0, 12.0), center - Vec2::new(0.0, 1.0)],
                stroke,
            );
        }
    }
}
fn usages() -> Vec<(u8, String)> {
    let mut values = (4..=29)
        .map(|code| (code, ((b'A' + code - 4) as char).to_string()))
        .collect::<Vec<_>>();
    values.extend((0x1e..=0x26).map(|code| (code, ((b'1' + code - 0x1e) as char).to_string())));
    values.extend(
        [
            (0x27, "0"),
            (0x28, "Enter"),
            (0x29, "Escape"),
            (0x2a, "Backspace"),
            (0x2b, "Tab"),
            (0x2c, "Space"),
            (0x2d, "-"),
            (0x2e, "="),
            (0x2f, "["),
            (0x30, "]"),
            (0x31, "\\"),
            (0x33, ";"),
            (0x34, "'"),
            (0x35, "`"),
            (0x36, ","),
            (0x37, "."),
            (0x38, "/"),
            (0x39, "Caps Lock"),
            (0x49, "Insert"),
            (0x4a, "Home"),
            (0x4b, "Page Up"),
            (0x4c, "Delete"),
            (0x4d, "End"),
            (0x4e, "Page Down"),
            (0x4f, "Right"),
            (0x50, "Left"),
            (0x51, "Down"),
            (0x52, "Up"),
            (0xe0, "Left Ctrl"),
            (0xe1, "Left Shift"),
            (0xe2, "Left Alt"),
            (0xe3, "Left Win"),
            (0xe4, "Right Ctrl"),
            (0xe5, "Right Shift"),
            (0xe6, "Right Alt"),
            (0xe7, "Right Win"),
        ]
        .map(|(code, name)| (code, name.into())),
    );
    values.extend((0x3a..=0x45).map(|code| (code, format!("F{}", code - 0x39))));
    values
}
fn usage_name(usage: u8) -> String {
    usages()
        .into_iter()
        .find(|(code, _)| *code == usage)
        .map_or_else(|| format!("HID usage 0x{usage:02X}"), |(_, name)| name)
}
fn media_name(usage: u8) -> String {
    match usage {
        0xe9 => "Volume up",
        0xea => "Volume down",
        0xe2 => "Mute",
        0xcd => "Play / pause",
        0xb5 => "Next track",
        0xb6 => "Previous track",
        0xb7 => "Stop",
        _ => "Other media action",
    }
    .into()
}
fn assignment_name(code: &[u8]) -> String {
    match code {
        [0, 0, 0, 0] => "Disabled".into(),
        [0, modifier, 0, usage] if *modifier <= 15 => {
            let mut parts = [(1, "Ctrl"), (2, "Shift"), (4, "Alt"), (8, "Win")]
                .into_iter()
                .filter(|(mask, _)| modifier & mask != 0)
                .map(|(_, name)| name.to_string())
                .collect::<Vec<_>>();
            parts.push(usage_name(*usage));
            parts.join(" + ")
        }
        [2, 0, 0, usage] => media_name(*usage),
        [3, _, _, index] => format!("On-board macro {}", *index as usize + 1),
        _ => "Firmware action".into(),
    }
}
fn action_code(action: usize, modifiers: u8, usage: u8) -> [u8; 4] {
    match action {
        0 => [0, modifiers, 0, usage],
        1 => [2, 0, 0, usage],
        _ => [0; 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Opt-in real playback and USB music check; play a test rhythm and close the app first"]
    fn usb_music_sync_restores_the_current_hardware_state() {
        use std::time::{Instant, SystemTime, UNIX_EPOCH};
        let mut board = Keyboard::connect().expect("USB keyboard required; close the app first");
        let settings = board.settings().unwrap();
        let colors = board.read_colors().unwrap();
        let maps = [board.keymap(0).unwrap(), board.keymap(1).unwrap()];
        let bank = board.read(5, 0, 4096).unwrap();
        let checkpoint = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("verification")
            .join(format!(
                "recovery-music-{}.json",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let snapshot = serde_json::json!({"version":2,"settings":settings,"colors":colors.to_vec(),"keymaps":maps,"macro_bank":bank});
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&checkpoint)
            .unwrap();
        file.write_all(&serde_json::to_vec(&snapshot).unwrap())
            .unwrap();
        file.sync_all().unwrap();
        drop(file);
        drop(board);
        println!("Full recovery checkpoint: {}", checkpoint.display());
        let keys: Vec<Key> =
            serde_json::from_str(include_str!("../assets/keyboard-layout.json")).unwrap();
        let positions = keys
            .iter()
            .filter_map(|key| {
                layout::from_oem(key.x, key.y, key.width, key.height, key.matrix).map(|geometry| {
                    let center = geometry.legend_center();
                    (
                        key.matrix,
                        ((center.x - 20.0) / 640.0).clamp(0.0, 1.0),
                        ((center.y - 44.0) / 216.0).clamp(0.0, 1.0),
                    )
                })
            })
            .collect();
        let mut session = MusicSession::start(
            egui::Context::default(),
            MusicSettings::default(),
            positions,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let state = session.snapshot().unwrap();
            assert!(
                state.active,
                "Audio failed before USB writes: {:?}",
                state.error
            );
            if state.ready && state.frames > 3 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Audio did not become ready before USB writes"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let (tx, replies) = worker(egui::Context::default());
        tx.send(Request::MusicStart(session.shared.clone()))
            .unwrap();
        let mut failure = None;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match replies.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Reply::MusicStarted) => break,
                Ok(Reply::MusicStopped(result)) => {
                    failure = Some(format!("Music start failed: {result:?}"));
                    break;
                }
                Ok(Reply::Error(error)) => {
                    failure = Some(error);
                    break;
                }
                Ok(_) => {}
                Err(error) => {
                    failure = Some(format!("Music worker did not start: {error}"));
                    break;
                }
            }
        }
        let mut peak_level = 0.0_f32;
        let mut peak_beat = 0.0_f32;
        let mut lit_frames = 0_u64;
        let started = Instant::now();
        while failure.is_none() && started.elapsed() < Duration::from_secs(4) {
            match session.snapshot() {
                Ok(state) => {
                    peak_level = peak_level.max(state.frame.level);
                    peak_beat = peak_beat.max(state.frame.beat);
                    if state.colors.iter().any(|rgb| *rgb != [0; 3]) {
                        lit_frames += 1;
                    }
                    if !state.active {
                        failure = Some(format!("Capture ended: {:?}", state.error));
                    }
                }
                Err(error) => failure = Some(error),
            }
            let mode = if started.elapsed() < Duration::from_millis(1300) {
                Mode::Spectrum
            } else if started.elapsed() < Duration::from_millis(2600) {
                Mode::Pulse
            } else {
                Mode::Wave
            };
            session.update(MusicSettings {
                mode,
                ..MusicSettings::default()
            });
            std::thread::sleep(Duration::from_millis(20));
        }
        let final_state = session.snapshot().unwrap();
        session.stop();
        let _ = tx.send(Request::MusicStop);
        let deadline = Instant::now() + Duration::from_secs(15);
        let restored = loop {
            match replies.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Reply::MusicStopped(result)) => break result.map(|_| ()),
                Ok(Reply::Error(error)) => break Err(error),
                Ok(_) => {}
                Err(error) => break Err(format!("Music restoration did not finish: {error}")),
            }
        };
        drop(tx);
        drop(replies);
        drop(session);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut board = loop {
            match Keyboard::connect() {
                Ok(board) => break board,
                Err(error) if Instant::now() >= deadline => panic!(
                    "Cannot verify recovery: {error}. Checkpoint: {}",
                    checkpoint.display()
                ),
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        let exact = board.settings().unwrap() == settings
            && board.read_colors().unwrap() == colors
            && board.keymap(0).unwrap() == maps[0]
            && board.keymap(1).unwrap() == maps[1]
            && board.read(5, 0, 4096).unwrap() == bank;
        let report = serde_json::json!({"sample_rate":final_state.sample_rate,"audio_frames":final_state.frames,"usb_frames":final_state.usb_frames,"peak_level":peak_level,"peak_beat":peak_beat,"lit_observations":lit_frames,"exact_restoration":exact,"restore_result":restored,"failure":failure,"checkpoint":checkpoint});
        std::fs::write(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("verification/music-usb-check.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        println!("Music USB check: {report}");
        assert!(
            exact,
            "Original data did not match. Retain checkpoint: {}",
            checkpoint.display()
        );
        assert!(restored.is_ok(), "Music restoration failed: {restored:?}");
        assert!(failure.is_none(), "Music sync failed: {failure:?}");
        assert!(
            final_state.usb_frames >= 10,
            "Too few accepted USB music frames"
        );
        assert!(
            peak_level > 0.005 && lit_frames > 10,
            "No audible test rhythm was detected"
        );
        assert!(peak_beat > 0.1, "No test-rhythm onset detected");
    }
    #[test]
    #[ignore = "Explicit USB hardware check; closes no other app and saves a recovery snapshot before writes"]
    fn usb_profile_apply_preserves_the_current_hardware_state() {
        let mut board = Keyboard::connect().expect("USB keyboard required; close the app first");
        let status = board.status().unwrap();
        let settings = board.settings().unwrap();
        let colors = board.read_colors().unwrap();
        let maps = [board.keymap(0).unwrap(), board.keymap(1).unwrap()];
        let bank = board.read(5, 0, 4096).unwrap();
        let checkpoint = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("verification")
            .join(format!(
                "recovery-profile-{}-{}.json",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        std::fs::create_dir_all(checkpoint.parent().unwrap()).unwrap();
        let snapshot = serde_json::json!({"version":2,"settings":settings,"colors":colors.to_vec(),"keymaps":maps,"macro_bank":bank});
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&checkpoint)
            .unwrap();
        file.write_all(&serde_json::to_vec(&snapshot).unwrap())
            .unwrap();
        file.sync_all().unwrap();
        drop(file);
        println!("Recovery checkpoint: {}", checkpoint.display());
        let mut profile = Profile::new(
            "USB verification".into(),
            status.lighting,
            status.sleep_seconds.expect("Supported sleep timer"),
            &colors,
        );
        profile.keymaps = maps.to_vec();
        drop(board);
        let (tx, replies) = worker(egui::Context::default());
        tx.send(Request::Profile(profile)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let applied = loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            match replies.recv_timeout(remaining) {
                Ok(Reply::Done(_)) => break Ok(()),
                Ok(Reply::Error(error)) => break Err(error),
                Ok(_) => {}
                Err(error) => break Err(format!("Profile worker did not finish: {error}")),
            }
        };
        drop(tx);
        drop(replies);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut board = loop {
            match Keyboard::connect() {
                Ok(board) => break board,
                Err(error) if std::time::Instant::now() >= deadline => panic!(
                    "Cannot verify/recover: {error}. Retained snapshot: {}",
                    checkpoint.display()
                ),
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        let matches = |board: &mut Keyboard| -> Result<bool, String> {
            Ok(board.settings()? == settings
                && board.read_colors()? == colors
                && board.keymap(0)? == maps[0]
                && board.keymap(1)? == maps[1]
                && board.read(5, 0, 4096)? == bank)
        };
        if applied.is_err() || !matches(&mut board).unwrap_or(false) {
            board
                .write(5, 0, &bank)
                .expect("Restore macro bank; retain checkpoint if this fails");
            board
                .write(6, 0, &aula_studio::protocol::color_planes(&colors))
                .unwrap();
            for (layer, map) in maps.iter().enumerate() {
                board.write(3, layer as u8, map).unwrap();
            }
            board.write(4, 0, &settings).unwrap();
            assert!(
                matches(&mut board).unwrap(),
                "Restoration mismatch. Snapshot: {}",
                checkpoint.display()
            );
            panic!(
                "Profile verification failed; original state restored. Result: {applied:?}. Snapshot: {}",
                checkpoint.display()
            );
        }
        println!(
            "USB profile application passed through the real worker. Settings, RGB, both keymaps, and the complete macro bank match the original snapshot."
        );
    }
    #[test]
    fn orbit_drag_and_reset_preserve_selection_and_do_not_request_hardware() {
        let (mut app, commands, _) = mock();
        let context = egui::Context::default();
        Studio::configure_style(&context);
        let start = egui::pos2(20.0, 20.0);
        let end = start + Vec2::new(60.0, 30.0);
        let scene_frame = |app: &mut Studio, events| {
            let mut output = context.run_ui(input(events), |ui| app.keyboard(ui));
            output.textures_delta.clear();
        };
        scene_frame(&mut app, vec![]);
        scene_frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        scene_frame(&mut app, vec![egui::Event::PointerMoved(end)]);
        scene_frame(
            &mut app,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.camera.yaw > Camera::default().yaw);
        assert!(app.camera.pitch > Camera::default().pitch);
        assert_eq!(app.selected, 0);
        let output = frame(&context, &mut app, vec![]);
        click(
            &context,
            &mut app,
            text_position(&output, "Reset view", 1200.0),
        );
        assert_eq!(app.camera.yaw, Camera::default().yaw);
        assert_eq!(app.camera.pitch, Camera::default().pitch);
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn tilted_camera_extremes_keep_physical_keys_selectable() {
        for yaw in [-0.42, 0.42] {
            let (mut app, commands, _) = mock();
            app.tab = Tab::Keys;
            app.camera = Camera { yaw, pitch: 0.95 };
            let context = egui::Context::default();
            Studio::configure_style(&context);
            frame(&context, &mut app, vec![]);
            let output = frame(&context, &mut app, vec![]);
            click(&context, &mut app, text_position(&output, "F12", 1200.0));
            assert_eq!(app.selected, 78);
            assert!(commands.try_recv().is_err());
        }
    }
    #[test]
    fn key_edge_clicks_at_steep_angles_are_not_captured_by_neighbour_bounds() {
        for (yaw, side) in [(-0.42, -1.0), (0.42, 1.0)] {
            let (mut app, commands, _) = mock();
            app.tab = Tab::Keys;
            app.camera = Camera { yaw, pitch: 0.95 };
            let context = egui::Context::default();
            Studio::configure_style(&context);
            frame(&context, &mut app, vec![]);
            let output = frame(&context, &mut app, vec![]);
            let one = text_position(&output, "1", 1200.0);
            let step_x = text_position(&output, "2", 1200.0) - one;
            let step_y = text_position(&output, "Q", 1200.0) - one - step_x * 0.5;
            click(
                &context,
                &mut app,
                one + step_x * (0.375 * side) - step_y * 0.25,
            );
            assert_eq!(app.selected, 7, "Edge click failed at yaw {yaw}");
            assert!(commands.try_recv().is_err());
        }
    }
    #[test]
    fn opening_music_does_not_start_audio_or_send_usb_commands() {
        let (mut app, commands, _) = mock();
        let context = egui::Context::default();
        Studio::configure_style(&context);
        frame(&context, &mut app, vec![]);
        let output = frame(&context, &mut app, vec![]);
        click(&context, &mut app, text_position(&output, "Music", 1200.0));
        assert!(app.tab == Tab::Music);
        assert!(app.music.is_none());
        assert!(app.music_usb == MusicUsb::Off);
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn music_usb_ownership_blocks_writes_and_stop_bypasses_busy() {
        let (mut app, commands, results) = mock();
        app.music_usb = MusicUsb::Active;
        app.send(Request::Lighting(app.lighting));
        assert!(commands.try_recv().is_err());
        app.busy = true;
        app.stop_music();
        assert!(matches!(commands.try_recv(), Ok(Request::MusicStop)));
        assert!(app.music_usb == MusicUsb::Stopping);
        results.send(Reply::MusicStopped(Ok(None))).unwrap();
        app.poll();
        assert!(app.music_usb == MusicUsb::Off);
        assert!(!app.busy);
    }
    #[test]
    fn capture_failure_remains_visible_after_successful_usb_restoration() {
        let (mut app, _, results) = mock();
        let mut state = music::MusicState::new(MusicSettings::default());
        state.active = false;
        state.error = Some("Playback device disconnected".into());
        app.music = Some(MusicSession::from_state(state));
        app.music_usb = MusicUsb::Active;
        results.send(Reply::MusicStopped(Ok(None))).unwrap();
        app.poll();
        app.poll_music();
        assert!(app.music.is_none());
        assert!(app.error);
        assert!(app.notice.contains("Playback device disconnected"));
        assert!(app.notice.contains("restored and verified"));
    }
    #[test]
    fn lost_usb_worker_ends_listening_and_does_not_trap_window_close() {
        for stopping in [false, true] {
            let (mut app, commands, results) = mock();
            app.music_usb = MusicUsb::Active;
            app.music = Some(MusicSession::from_state(music::MusicState::new(
                MusicSettings::default(),
            )));
            drop(commands);
            drop(results);
            if stopping {
                app.stop_music();
            } else {
                app.poll();
            }
            assert!(app.music.is_none());
            assert!(app.music_usb == MusicUsb::Off);
            assert!(app.error);
            assert!(app.notice.contains("restoration could not be verified"));
            let context = egui::Context::default();
            Studio::configure_style(&context);
            let mut raw = input(vec![]);
            raw.viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .events
                .push(egui::ViewportEvent::Close);
            let mut output = context.run_ui(raw, |ui| app.render(ui));
            output.textures_delta.clear();
            assert!(
                !output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&egui::ViewportCommand::CancelClose)
            );
        }
    }
    #[test]
    fn window_close_cancels_pending_audio_without_starting_usb() {
        for ready in [false, true] {
            for sync_keyboard in [false, true] {
                let (mut app, commands, _results) = mock();
                app.tab = Tab::Music;
                app.status = Some(DeviceStatus {
                    transport: Transport::Usb,
                    product: "Synthetic F75".into(),
                    lighting: app.lighting,
                    sleep_seconds: Some(300),
                });
                app.music_to_keyboard = sync_keyboard;
                let mut state = music::MusicState::new(MusicSettings::default());
                state.ready = ready;
                let session = MusicSession::from_state(state);
                let shared = session.shared.clone();
                app.music = Some(session);
                let context = egui::Context::default();
                Studio::configure_style(&context);
                let mut raw = input(vec![]);
                raw.viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .events
                    .push(egui::ViewportEvent::Close);
                let mut output = context.run_ui(raw, |ui| app.render(ui));
                output.textures_delta.clear();

                assert!(commands.try_recv().is_err());
                assert!(app.music.is_none());
                assert!(app.music_usb == MusicUsb::Off);
                assert!(!app.busy && !app.close_after_operation);
                assert!(!shared.lock().unwrap().active);
                assert!(
                    !output.viewport_output[&egui::ViewportId::ROOT]
                        .commands
                        .contains(&egui::ViewportCommand::CancelClose)
                );
            }
        }
    }
    #[test]
    fn window_close_waits_for_music_restoration_and_keeps_errors_visible() {
        for failure in [false, true] {
            let (mut app, commands, results) = mock();
            app.music_usb = MusicUsb::Active;
            let context = egui::Context::default();
            Studio::configure_style(&context);
            let mut raw = input(vec![]);
            raw.viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .events
                .push(egui::ViewportEvent::Close);
            let mut output = context.run_ui(raw, |ui| app.render(ui));
            output.textures_delta.clear();
            assert!(
                output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&egui::ViewportCommand::CancelClose)
            );
            assert!(matches!(commands.try_recv(), Ok(Request::MusicStop)));
            assert!(app.close_after_operation);
            results
                .send(Reply::MusicStopped(if failure {
                    Err("USB recovery required".into())
                } else {
                    Ok(None)
                }))
                .unwrap();
            let output = frame(&context, &mut app, vec![]);
            assert_eq!(
                output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&egui::ViewportCommand::Close),
                !failure
            );
            assert!(!app.close_after_operation);
            if failure {
                assert!(app.error);
                assert!(app.tab == Tab::Music);
            }
        }
    }
    #[test]
    fn window_close_waits_for_entire_persistent_operation_and_keeps_failure_visible() {
        for failure in [false, true] {
            let (mut app, commands, results) = mock();
            app.tab = Tab::Macros;
            app.send(Request::Macros(vec![MacroBinding {
                name: "Synthetic macro".into(),
                matrix: 78,
                events: macros::text_events("aula", 40).unwrap(),
            }]));
            assert!(matches!(commands.try_recv(), Ok(Request::Macros(_))));
            let context = egui::Context::default();
            Studio::configure_style(&context);
            let mut raw = input(vec![]);
            raw.viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .events
                .push(egui::ViewportEvent::Close);
            let mut output = context.run_ui(raw, |ui| app.render(ui));
            output.textures_delta.clear();
            assert!(
                output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&egui::ViewportCommand::CancelClose)
            );
            assert!(app.close_after_operation && app.busy);
            assert!(commands.try_recv().is_err());

            let mut map = vec![0; 512];
            map[508..].copy_from_slice(&[0, 0, 0x5a, 0xa5]);
            results.send(Reply::Keymap(0, map)).unwrap();
            let output = frame(&context, &mut app, vec![]);
            assert!(app.busy);
            assert!(
                !output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&egui::ViewportCommand::Close)
            );
            if failure {
                results
                    .send(Reply::Error("Macro assignment readback failed".into()))
                    .unwrap();
            } else {
                results
                    .send(Reply::Done("Macro bank verified.".into()))
                    .unwrap();
                let output = frame(&context, &mut app, vec![]);
                assert!(app.busy);
                assert!(
                    !output.viewport_output[&egui::ViewportId::ROOT]
                        .commands
                        .contains(&egui::ViewportCommand::Close)
                );
                results.send(Reply::CommandFinished).unwrap();
            }
            let output = frame(&context, &mut app, vec![]);
            assert!(!app.busy && !app.close_after_operation);
            assert_eq!(
                output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .contains(&egui::ViewportCommand::Close),
                !failure
            );
            if failure {
                assert!(app.error && app.tab == Tab::Macros);
                assert!(app.notice.contains("readback failed"));
            }
        }
    }
    #[test]
    fn lost_worker_during_persistent_operation_releases_close_wait_with_error() {
        let (mut app, commands, results) = mock();
        app.send(Request::Sleep(300));
        assert!(matches!(commands.try_recv(), Ok(Request::Sleep(300))));
        app.close_after_operation = true;
        drop(commands);
        drop(results);
        let context = egui::Context::default();
        Studio::configure_style(&context);
        let output = frame(&context, &mut app, vec![]);
        assert!(!app.busy && !app.close_after_operation);
        assert!(app.error && app.status.is_none());
        assert!(app.notice.contains("Some changes may have applied"));
        assert!(
            !output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&egui::ViewportCommand::Close)
        );
    }
    #[test]
    fn iso_enter_mesh_preserves_its_notch_at_all_camera_angles() {
        let geometry = layout::from_oem(566.0, 127.0, 46.0, 28.0, 81).unwrap();
        for yaw in [-0.42, 0.0, 0.42] {
            for pitch in [0.0, 0.63, 0.95] {
                for inset in [-2.0, 0.0, 1.0] {
                    let camera = Camera { yaw, pitch };
                    let points = key_path(geometry, inset, |point| {
                        camera.project(point.to_vec2()).to_pos2()
                    });
                    let indices = polygon_triangles(&points);
                    assert_eq!(indices.len(), (points.len() - 2) * 3);
                    let cross = |a: egui::Pos2, b: egui::Pos2| {
                        a.x as f64 * b.y as f64 - a.y as f64 * b.x as f64
                    };
                    let polygon_area = (0..points.len())
                        .map(|i| cross(points[i], points[(i + 1) % points.len()]))
                        .sum::<f64>()
                        .abs()
                        * 0.5;
                    let triangle_area = indices
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .map(|triangle| {
                            let [a, b, c] = [
                                points[triangle[0] as usize],
                                points[triangle[1] as usize],
                                points[triangle[2] as usize],
                            ];
                            (cross(b, c) - cross(b, a) - cross(a, c)).abs() * 0.5
                        })
                        .sum::<f64>();
                    assert!(
                        (polygon_area - triangle_area).abs() < 0.00001,
                        "Mesh fills outside Enter: {polygon_area} versus {triangle_area}"
                    );
                }
            }
        }
    }
    #[test]
    fn iso_enter_stem_and_adjacent_key_select_distinct_hardware_positions() {
        let (mut app, commands, _) = mock();
        app.tab = Tab::Keys;
        let context = egui::Context::default();
        Studio::configure_style(&context);
        for (label, matrix) in [("Enter", 81), ("\\", 75), ("Enter", 81)] {
            frame(&context, &mut app, vec![]);
            let output = frame(&context, &mut app, vec![]);
            click(&context, &mut app, text_position(&output, label, 1200.0));
            assert_eq!(app.selected, matrix);
            assert!(commands.try_recv().is_err());
        }
    }
    #[test]
    fn effect_tiles_and_full_width_slider_only_change_the_local_draft() {
        let (mut app, commands, _) = mock();
        let context = egui::Context::default();
        Studio::configure_style(&context);
        frame(&context, &mut app, vec![]);
        let output = frame(&context, &mut app, vec![]);
        click(
            &context,
            &mut app,
            text_position(&output, "Neon stream", 1200.0),
        );
        assert_eq!(app.lighting.effect, 11);
        let output = frame(&context, &mut app, vec![]);
        let brightness = text_position(&output, "Brightness", 1200.0);
        let rail = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(shape)
                    if shape.rect.width() > 200.0
                        && shape.rect.height() <= 12.0
                        && shape.rect.top() > brightness.y =>
                {
                    Some(shape.rect)
                }
                _ => None,
            })
            .min_by(|a, b| a.top().total_cmp(&b.top()))
            .expect("Brightness slider rail is visible");
        click(
            &context,
            &mut app,
            rail.right_center() - Vec2::new(4.0, 0.0),
        );
        assert!(app.lighting.brightness > 6);
        let output = frame(&context, &mut app, vec![]);
        click(
            &context,
            &mut app,
            text_position(&output, "Per-key", 1200.0),
        );
        assert_eq!(app.lighting.effect, 21);
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn preview_motion_settles_without_an_idle_repaint_loop() {
        let (mut app, commands, _) = mock();
        let context = egui::Context::default();
        Studio::configure_style(&context);
        frame(&context, &mut app, vec![]);
        app.view_3d = false;
        for tick in 0..120 {
            let mut raw = input(if tick == 0 {
                vec![egui::Event::PointerGone]
            } else {
                vec![]
            });
            raw.time = Some(1.0 + tick as f64 / 60.0);
            let mut output = context.run_ui(raw, |ui| app.render(ui));
            output.textures_delta.clear();
            if tick == 119 {
                assert!(
                    output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
                        > Duration::from_secs(1),
                    "Idle repaint: {:?}; causes: {:?}",
                    output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
                    context.repaint_causes()
                );
            }
        }
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn both_keyboard_views_select_the_correct_physical_key_without_writes() {
        let (mut app, commands, _) = mock();
        app.tab = Tab::Keys;
        let context = egui::Context::default();
        Studio::configure_style(&context);
        for (tilted, label, expected) in [(true, "F12", 78), (false, "Esc", 0)] {
            app.view_3d = tilted;
            frame(&context, &mut app, vec![]);
            let output = frame(&context, &mut app, vec![]);
            click(&context, &mut app, text_position(&output, label, 1200.0));
            assert_eq!(app.selected, expected);
            assert!(commands.try_recv().is_err());
        }
    }
    #[test]
    fn view_toggle_changes_preview_without_device_requests() {
        let (mut app, commands, _) = mock();
        let context = egui::Context::default();
        Studio::configure_style(&context);
        frame(&context, &mut app, vec![]);
        let output = frame(&context, &mut app, vec![]);
        click(&context, &mut app, text_position(&output, "2D", 1200.0));
        assert!(!app.view_3d);
        let output = frame(&context, &mut app, vec![]);
        click(&context, &mut app, text_position(&output, "3D", 1200.0));
        assert!(app.view_3d);
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn selecting_keys_loads_current_assignments_and_preserves_unknown_fn_actions() {
        let (mut app, commands, results) = mock();
        app.tab = Tab::Keys;
        let mut base = vec![0; 512];
        base[508..].copy_from_slice(&[0, 0, 0x5a, 0xa5]);
        base[..4].copy_from_slice(&[0, 0, 0, 0x29]);
        base[78 * 4..78 * 4 + 4].copy_from_slice(&[0, 0, 0, 0x45]);
        let mut function = base.clone();
        function[78 * 4..78 * 4 + 4].copy_from_slice(&[3, 1, 1, 0]);
        results
            .send(Reply::Status(DeviceStatus {
                transport: Transport::Usb,
                product: "Synthetic F75".into(),
                lighting: app.lighting,
                sleep_seconds: Some(300),
            }))
            .unwrap();
        results.send(Reply::Keymap(0, base)).unwrap();
        results.send(Reply::Keymap(1, function)).unwrap();
        let context = egui::Context::default();
        Studio::configure_style(&context);
        frame(&context, &mut app, vec![]);
        assert_eq!(app.usage, 0x29);
        app.usage = 4;
        let output = frame(&context, &mut app, vec![]);
        assert_eq!(app.usage, 4);
        click(&context, &mut app, text_position(&output, "F12", 1200.0));
        let output = frame(&context, &mut app, vec![]);
        assert_eq!(app.usage, 0x45);
        assert!(commands.try_recv().is_err());
        click(
            &context,
            &mut app,
            text_position(&output, "Apply key assignment", 1200.0),
        );
        assert!(matches!(
            commands.try_recv(),
            Ok(Request::Remap(0, 78, [0, 0, 0, 0x45]))
        ));
        results
            .send(Reply::Done("Synthetic verification".into()))
            .unwrap();
        results.send(Reply::CommandFinished).unwrap();
        let output = frame(&context, &mut app, vec![]);
        click(
            &context,
            &mut app,
            text_position(&output, "Fn layer", 1200.0),
        );
        let output = frame(&context, &mut app, vec![]);
        assert_eq!(app.action, 3);
        click(
            &context,
            &mut app,
            text_position(&output, "Apply key assignment", 1200.0),
        );
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn loading_lighting_only_profile_clears_previous_keymaps_and_confirmations() {
        let (mut app, commands, _) = mock();
        app.tab = Tab::Profiles;
        let profile = Profile::new(
            "Lighting only".into(),
            Lighting {
                effect: 1,
                brightness: 8,
                speed: 3,
                color: 4,
            },
            300,
            &[[10, 20, 30]; 126],
        );
        app.profiles.push(profile);
        app.selected_profile = Some(0);
        let mut old_map = vec![0; 512];
        old_map[508..].copy_from_slice(&[0, 0, 0x5a, 0xa5]);
        app.keymaps = [Some(old_map.clone()), Some(old_map)];
        app.keymap_from_device = [true, true];
        app.macro_confirm = true;
        app.clear_macros_confirm = true;
        app.profile_confirm = true;
        let context = egui::Context::default();
        Studio::configure_style(&context);
        frame(&context, &mut app, vec![]);
        let output = frame(&context, &mut app, vec![]);
        click(
            &context,
            &mut app,
            text_position(&output, "Load into draft", 1200.0),
        );
        assert!(app.keymaps.iter().all(Option::is_none));
        assert_eq!(app.keymap_from_device, [false; 2]);
        assert!(!app.macro_confirm && !app.clear_macros_confirm && !app.profile_confirm);
        assert_eq!(app.lighting.brightness, 8);
        assert_eq!(app.colors, [[10, 20, 30]; 126]);
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn partial_profile_replaces_only_included_draft_layers_and_invalid_load_is_atomic() {
        let (mut app, commands, _) = mock();
        let mut old_map = vec![0; 512];
        old_map[508..].copy_from_slice(&[0, 0, 0x5a, 0xa5]);
        app.keymaps = [Some(old_map.clone()), Some(old_map.clone())];
        let mut profile = Profile::new(
            "Base only".into(),
            Lighting {
                effect: 1,
                brightness: 8,
                speed: 3,
                color: 4,
            },
            300,
            &[[10, 20, 30]; 126],
        );
        let mut new_map = old_map;
        new_map[..4].copy_from_slice(&[0, 0, 0, 0x29]);
        profile.keymaps.push(new_map.clone());
        app.load_profile_draft(&profile).unwrap();
        assert_eq!(app.keymaps[0].as_ref(), Some(&new_map));
        assert!(app.keymaps[1].is_none());
        let original_lighting = app.lighting;
        let original_colors = app.colors;
        let original_maps = app.keymaps.clone();
        profile.colors.pop();
        profile.lighting[1] = 0;
        assert!(app.load_profile_draft(&profile).is_err());
        assert_eq!(app.lighting, original_lighting);
        assert_eq!(app.colors, original_colors);
        assert_eq!(app.keymaps, original_maps);
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn saving_base_only_profile_retains_its_map_and_fn_only_requires_base() {
        let (mut app, commands, _) = mock();
        let mut map = vec![0; 512];
        map[508..].copy_from_slice(&[0, 0, 0x5a, 0xa5]);
        map[..4].copy_from_slice(&[0, 0, 0, 0x29]);
        app.keymaps = [Some(map.clone()), None];
        let profile = app.profile_from_draft().unwrap();
        assert_eq!(profile.keymaps, vec![map.clone()]);
        app.load_profile_draft(&profile).unwrap();
        assert_eq!(app.keymaps, [Some(map.clone()), None]);

        app.keymaps = [None, Some(map.clone())];
        let error = app.profile_from_draft().err().unwrap();
        assert!(error.contains("Read the base keymap"));
        assert_eq!(app.keymaps, [None, Some(map)]);
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn shortcut_modifiers_occupy_the_second_byte() {
        assert_eq!(action_code(0, 3, 4), [0, 3, 0, 4]);
        assert_eq!(action_code(1, 3, 0xe9), [2, 0, 0, 0xe9]);
        assert_eq!(action_code(2, 3, 4), [0; 4]);
    }
    #[test]
    fn hardware_palette_matches_documented_color_nibbles() {
        assert_eq!(
            COLORS.map(|(name, _)| name),
            [
                "Red", "Green", "Blue", "Yellow", "Magenta", "Cyan", "White", "Random"
            ]
        );
        assert_eq!(COLORS[0].1, [255, 0, 0]);
        assert_eq!(COLORS[1].1, [0, 255, 0]);
        assert_eq!(COLORS[2].1, [0, 0, 255]);
        assert_eq!(COLORS[6].1, [255, 255, 255]);
    }
    #[test]
    fn bundled_keys_have_valid_unique_matrix_positions() {
        let keys: Vec<Key> =
            serde_json::from_str(include_str!("../assets/keyboard-layout.json")).unwrap();
        let mut positions = std::collections::HashSet::new();
        for key in keys {
            assert!(key.matrix < 126);
            assert!(positions.insert(key.matrix));
            assert!(key.width >= 0.0 && key.height >= 0.0);
        }
    }
    fn mock() -> (Studio, Receiver<Request>, Sender<Reply>) {
        let (tx, commands) = mpsc::channel();
        let (results, rx) = mpsc::channel();
        (
            Studio::with_channels(tx, rx, Vec::new(), None),
            commands,
            results,
        )
    }
    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1200.0, 780.0),
            )),
            events,
            ..Default::default()
        }
    }
    fn frame(
        context: &egui::Context,
        app: &mut Studio,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut output = context.run_ui(input(events), |ui| app.render(ui));
        // This harness inspects shapes without a GPU, so consume the texture updates explicitly.
        output.textures_delta.clear();
        output
    }
    fn text_position(output: &egui::FullOutput, text: &str, max_x: f32) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(shape)
                    if shape.galley.job.text == text && shape.pos.x < max_x =>
                {
                    Some(
                        shape.pos
                            + egui::emath::Rot2::from_angle(shape.angle)
                                * shape.galley.rect.center().to_vec2(),
                    )
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("Text {text:?} was not rendered"))
    }
    fn click(context: &egui::Context, app: &mut Studio, pos: egui::Pos2) {
        frame(
            context,
            app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(
            context,
            app,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    #[test]
    fn sidebar_click_changes_tab_without_device_access() {
        let (mut app, commands, _) = mock();
        let context = egui::Context::default();
        Studio::configure_style(&context);
        frame(&context, &mut app, vec![]);
        let output = frame(&context, &mut app, vec![]);
        let position = text_position(&output, "Macros", 188.0);
        click(&context, &mut app, position);
        assert!(app.tab == Tab::Macros);
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn apply_lighting_click_queues_draft_and_worker_error_marks_offline() {
        let (mut app, commands, results) = mock();
        let context = egui::Context::default();
        Studio::configure_style(&context);
        results
            .send(Reply::Status(DeviceStatus {
                transport: Transport::Usb,
                product: "Synthetic F75".into(),
                lighting: app.lighting,
                sleep_seconds: Some(300),
            }))
            .unwrap();
        frame(&context, &mut app, vec![]);
        app.lighting.brightness = 8;
        let output = frame(&context, &mut app, vec![]);
        let position = text_position(&output, "Apply lighting", 1200.0);
        click(&context, &mut app, position);
        match commands.try_recv().expect("Apply must enqueue one request") {
            Request::Lighting(lighting) => assert_eq!(lighting.brightness, 8),
            _ => panic!("Wrong hardware request"),
        }
        assert!(app.busy);
        results
            .send(Reply::Error("Synthetic disconnected device".into()))
            .unwrap();
        let output = frame(&context, &mut app, vec![]);
        assert!(app.status.is_none());
        assert!(app.error);
        assert!(!app.busy);
        text_position(&output, "OFFLINE", 1200.0);
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn receiver_apply_is_disabled_while_refresh_remains_available() {
        let (mut app, commands, results) = mock();
        let context = egui::Context::default();
        Studio::configure_style(&context);
        results
            .send(Reply::Status(DeviceStatus {
                transport: Transport::Receiver,
                product: "Synthetic receiver".into(),
                lighting: app.lighting,
                sleep_seconds: Some(300),
            }))
            .unwrap();
        frame(&context, &mut app, vec![]);
        let output = frame(&context, &mut app, vec![]);
        click(
            &context,
            &mut app,
            text_position(&output, "Apply lighting", 1200.0),
        );
        assert!(commands.try_recv().is_err());
        let output = frame(&context, &mut app, vec![]);
        click(
            &context,
            &mut app,
            text_position(&output, "Refresh connection", 1200.0),
        );
        assert!(matches!(commands.try_recv(), Ok(Request::Refresh)));
    }
    #[test]
    fn playback_field_accepts_text_without_device_requests() {
        let (mut app, commands, _) = mock();
        app.tab = Tab::Macros;
        let context = egui::Context::default();
        Studio::configure_style(&context);
        frame(&context, &mut app, vec![]);
        let output = frame(&context, &mut app, vec![]);
        click(
            &context,
            &mut app,
            text_position(&output, "Macro output appears here", 1200.0),
        );
        frame(&context, &mut app, vec![egui::Event::Text("aula".into())]);
        assert_eq!(app.macro_playback, "aula");
        assert!(commands.try_recv().is_err());
        let output = frame(&context, &mut app, vec![]);
        click(
            &context,
            &mut app,
            text_position(&output, "Clear test output", 1200.0),
        );
        assert!(app.macro_playback.is_empty());
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn primary_apply_buttons_fit_default_native_viewport_without_scroll() {
        for (tab, effect, label) in [
            (Tab::Lighting, 3, "Apply lighting"),
            (Tab::Music, 3, "Start music sync"),
            (Tab::Lighting, 3, "Per-key"),
            (Tab::Lighting, 21, "Apply colors"),
            (Tab::Keys, 3, "Apply key assignment"),
            (Tab::Macros, 3, "Add / replace macro for selected key"),
            (Tab::Macros, 3, "Apply macro bank"),
        ] {
            let (mut app, _, results) = mock();
            let context = egui::Context::default();
            Studio::configure_style(&context);
            app.tab = tab;
            app.lighting.effect = effect;
            app.colors_loaded = true;
            if tab == Tab::Macros {
                app.macros.push(MacroBinding {
                    name: "Synthetic macro".into(),
                    matrix: 0,
                    events: macros::text_events("Hi", 40).unwrap(),
                });
                app.macro_confirm = true;
            }
            results
                .send(Reply::Status(DeviceStatus {
                    transport: Transport::Usb,
                    product: "Synthetic F75".into(),
                    lighting: app.lighting,
                    sleep_seconds: Some(300),
                }))
                .unwrap();
            frame(&context, &mut app, vec![]);
            frame(&context, &mut app, vec![]);
            let output = frame(&context, &mut app, vec![]);
            let (bounds, clip) = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.job.text == label => Some((
                        egui::Rect::from_min_size(text.pos, text.galley.size())
                            .expand2(Vec2::new(14.0, 10.0)),
                        shape.clip_rect,
                    )),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("Missing {label} button"));
            assert!(
                bounds.bottom() <= 733.0,
                "{label} extends below default content area: {bounds:?}"
            );
            assert!(
                clip.contains_rect(bounds),
                "{label} is clipped at default native size: button {bounds:?}, clip {clip:?}"
            );
        }
    }
    #[test]
    fn music_palette_menu_selects_each_collection_without_device_writes() {
        let (mut app, commands, _) = mock();
        app.tab = Tab::Music;
        let context = egui::Context::default();
        Studio::configure_style(&context);
        for (family, target) in [
            ("Ocean", Palette::Lagoon),
            ("Sunset", Palette::RoseDusk),
            ("Studio", Palette::Rainbow),
        ] {
            frame(&context, &mut app, vec![]);
            let output = frame(&context, &mut app, vec![]);
            let trigger = text_position(&output, app.music_settings.palette.name(), 1200.0);
            click(&context, &mut app, trigger);
            frame(&context, &mut app, vec![]);
            let output = frame(&context, &mut app, vec![]);
            assert!(egui::Popup::is_any_open(&context));
            click(&context, &mut app, text_position(&output, family, 1200.0));
            frame(&context, &mut app, vec![]);
            let output = frame(&context, &mut app, vec![]);
            click(
                &context,
                &mut app,
                text_position(&output, target.name(), 1200.0),
            );
            assert_eq!(app.music_settings.palette, target);
            assert!(!egui::Popup::is_any_open(&context));
        }
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn music_palette_keyboard_scrolls_selects_and_escape_cancels() {
        let (mut app, commands, _) = mock();
        app.tab = Tab::Music;
        app.music_settings.palette = Palette::Ocean;
        let context = egui::Context::default();
        Studio::configure_style(&context);
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        for (navigation, expected, commit) in [
            (egui::Key::ArrowDown, Palette::Lagoon, true),
            (egui::Key::End, Palette::MidnightTide, true),
            (egui::Key::Home, Palette::MidnightTide, false),
        ] {
            frame(&context, &mut app, vec![]);
            let output = frame(&context, &mut app, vec![]);
            let trigger = text_position(&output, app.music_settings.palette.name(), 1200.0);
            click(&context, &mut app, trigger);
            frame(&context, &mut app, vec![]);
            frame(&context, &mut app, vec![key(navigation)]);
            frame(&context, &mut app, vec![]);
            frame(
                &context,
                &mut app,
                vec![key(if commit {
                    egui::Key::Enter
                } else {
                    egui::Key::Escape
                })],
            );
            assert_eq!(app.music_settings.palette, expected);
            assert!(!egui::Popup::is_any_open(&context));
        }
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn switching_palette_collections_reveals_the_first_choices() {
        let (mut app, commands, _) = mock();
        app.tab = Tab::Music;
        app.music_settings.palette = Palette::Ocean;
        let context = egui::Context::default();
        Studio::configure_style(&context);
        frame(&context, &mut app, vec![]);
        let output = frame(&context, &mut app, vec![]);
        click(&context, &mut app, text_position(&output, "Ocean", 1200.0));
        frame(&context, &mut app, vec![]);
        frame(
            &context,
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::End,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(&context, &mut app, vec![]);
        let output = frame(&context, &mut app, vec![]);
        click(&context, &mut app, text_position(&output, "Sunset", 1200.0));
        frame(&context, &mut app, vec![]);
        let output = frame(&context, &mut app, vec![]);
        let choice = text_position(&output, "Golden hour", 1200.0);
        click(&context, &mut app, choice);
        assert_eq!(app.music_settings.palette, Palette::GoldenHour);
        assert!(commands.try_recv().is_err());
    }
    #[test]
    fn music_spectrum_and_start_fit_supported_window_sizes() {
        for size in [
            Vec2::new(1440.0, 900.0),
            Vec2::new(1200.0, 780.0),
            Vec2::new(920.0, 650.0),
        ] {
            let (mut app, commands, _) = mock();
            app.tab = Tab::Music;
            let context = egui::Context::default();
            Studio::configure_style(&context);
            let render = |app: &mut Studio| {
                let mut raw = input(vec![]);
                raw.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size));
                let mut output = context.run_ui(raw, |ui| app.render(ui));
                output.textures_delta.clear();
                output
            };
            render(&mut app);
            render(&mut app);
            let output = render(&mut app);
            for label in ["Start music sync", "Bass", "Treble"] {
                let (bounds, clip) = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) if text.galley.job.text == label => Some((
                            egui::Rect::from_min_size(text.pos, text.galley.size()),
                            shape.clip_rect,
                        )),
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("Missing {label} at {size:?}"));
                assert!(
                    clip.contains_rect(bounds),
                    "{label} clipped at {size:?}: {bounds:?}, clip {clip:?}"
                );
            }
            assert!(commands.try_recv().is_err());
        }
    }
    #[test]
    fn text_macro_requires_explicit_bank_apply_before_queueing_hardware() {
        let (mut app, commands, results) = mock();
        let context = egui::Context::default();
        Studio::configure_style(&context);
        app.tab = Tab::Macros;
        app.macro_name = "Synthetic text macro".into();
        app.macro_text = "Hi".into();
        results
            .send(Reply::Status(DeviceStatus {
                transport: Transport::Usb,
                product: "Synthetic F75".into(),
                lighting: app.lighting,
                sleep_seconds: Some(300),
            }))
            .unwrap();
        frame(&context, &mut app, vec![]);
        let output = frame(&context, &mut app, vec![]);
        click(
            &context,
            &mut app,
            text_position(&output, "Add / replace macro for selected key", 1200.0),
        );
        assert_eq!(app.macros.len(), 1);
        assert_eq!(app.macros[0].name, "Synthetic text macro");
        assert!(!app.macros[0].events.is_empty());
        assert!(commands.try_recv().is_err());
        let output = frame(&context, &mut app, vec![]);
        click(
            &context,
            &mut app,
            text_position(&output, "Replace existing hardware macros", 1200.0),
        );
        assert!(app.macro_confirm);
        let output = frame(&context, &mut app, vec![]);
        click(
            &context,
            &mut app,
            text_position(&output, "Apply macro bank", 1200.0),
        );
        match commands
            .try_recv()
            .expect("Explicit bank Apply must enqueue one hardware request")
        {
            Request::Macros(bindings) => {
                assert_eq!(bindings.len(), 1);
                assert_eq!(bindings[0].name, "Synthetic text macro");
                assert!(!bindings[0].events.is_empty());
            }
            _ => panic!("Wrong hardware request"),
        }
        assert!(app.busy);
        assert!(!app.macro_confirm);
        assert!(commands.try_recv().is_err());
    }
}
