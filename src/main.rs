#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod auth;
mod config;
mod launch;
mod modrinth;
mod mojang;
mod net;
mod skin;

use anyhow::{anyhow, Result};
use config::{data_dir, mods_dir, now, Account, Config};
use eframe::egui::{self, Color32, RichText};
use net::Progress;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{channel, Receiver, Sender},
        Arc,
    },
    time::Duration,
};

const ACCENT: Color32 = Color32::from_rgb(76, 175, 80);

// ───────────────────────────── Tin nhắn từ thread nền → UI ─────────────────────────────

enum Msg {
    Log(String),
    Versions(Vec<String>),
    DeviceCode(auth::DeviceCode),
    LoggedIn(Account),
    TokenRefreshed(Account),
    LoginFailed(String),
    Hits(Vec<modrinth::Hit>),
    Installed,
    Skin(egui::ColorImage, bool),
    SkinUploaded(std::result::Result<(), String>),
    GameStarted,
    GameExited,
}

#[derive(Clone)]
struct Tx(Sender<Msg>, egui::Context);

impl Tx {
    fn send(&self, m: Msg) {
        let _ = self.0.send(m);
        self.1.request_repaint();
    }
    fn log(&self, s: impl Into<String>) {
        self.send(Msg::Log(s.into()));
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Play,
    Mods,
    Skin,
    Settings,
}

struct App {
    cfg: Config,
    tab: Tab,
    tx: Tx,
    rx: Receiver<Msg>,
    log: Vec<String>,
    versions: Vec<String>,
    progress: Arc<Progress>,
    launching: bool,
    running: bool,

    device: Option<auth::DeviceCode>,
    logging_in: bool,
    login_err: Option<String>,
    cancel_login: Arc<AtomicBool>,
    offline_name: String,

    query: String,
    hits: Vec<modrinth::Hit>,
    searching: bool,
    installing: bool,
    installed: Vec<String>,

    skin_tex: Option<egui::TextureHandle>,
    local_tex: Option<egui::TextureHandle>,
    skin_path: Option<PathBuf>,
    new_slim: bool,
    skin_busy: bool,
    skin_status: Option<(bool, String)>,
}

fn to_color(img: &image::RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied(
        [img.width() as usize, img.height() as usize],
        img.as_raw(),
    )
}

fn fmt_num(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}K", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// Trả về tài khoản có token còn hạn (tự làm mới nếu cần).
fn valid_account(cfg: &Config, tx: &Tx) -> Result<Account> {
    let acc = cfg.account.clone().ok_or_else(|| anyhow!("Hãy đăng nhập Microsoft trước"))?;
    if acc.offline || acc.expires_at > now() {
        return Ok(acc);
    }
    let fresh = auth::refresh(&cfg.client_id, &acc.refresh_token)?;
    tx.send(Msg::TokenRefreshed(fresh.clone()));
    Ok(fresh)
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (s, rx) = channel();
        let tx = Tx(s, cc.egui_ctx.clone());
        let mut app = App {
            cfg: Config::load(),
            tab: Tab::Play,
            tx,
            rx,
            log: vec!["Chào mừng đến với Nexia Launcher.".into()],
            versions: vec![],
            progress: Progress::new(),
            launching: false,
            running: false,
            device: None,
            logging_in: false,
            login_err: None,
            cancel_login: Arc::new(AtomicBool::new(false)),
            offline_name: String::new(),
            query: String::new(),
            hits: vec![],
            searching: false,
            installing: false,
            installed: modrinth::list_installed(),
            skin_tex: None,
            local_tex: None,
            skin_path: None,
            new_slim: false,
            skin_busy: false,
            skin_status: None,
        };
        app.fetch_versions();
        if app.cfg.account.is_some() {
            app.load_skin();
        }
        app
    }

    fn bg(&self, f: impl FnOnce(Tx) + Send + 'static) {
        let tx = self.tx.clone();
        std::thread::spawn(move || f(tx));
    }

    // ───────────── Hành động chạy nền ─────────────

    fn fetch_versions(&self) {
        self.bg(|tx| match mojang::list_versions() {
            Ok(v) => tx.send(Msg::Versions(v)),
            Err(e) => tx.log(format!("Không tải được danh sách phiên bản: {e:#}")),
        });
    }

    fn login(&mut self) {
        self.logging_in = true;
        self.login_err = None;
        self.cancel_login = Arc::new(AtomicBool::new(false));
        let cid = self.cfg.client_id.clone();
        let cancel = self.cancel_login.clone();
        self.bg(move |tx| {
            let r = (|| -> Result<Account> {
                let dc = auth::start_device_flow(&cid)?;
                tx.send(Msg::DeviceCode(dc.clone()));
                auth::poll_device_flow(&cid, &dc, &cancel)
            })();
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            match r {
                Ok(a) => tx.send(Msg::LoggedIn(a)),
                Err(e) => tx.send(Msg::LoginFailed(format!("{e:#}"))),
            }
        });
    }

    fn cancel_login(&mut self) {
        self.cancel_login.store(true, Ordering::Relaxed);
        self.logging_in = false;
        self.device = None;
    }

    fn login_offline(&mut self) {
        let name = self.offline_name.trim().to_string();
        let ok = (3..=16).contains(&name.len())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !ok {
            self.login_err = Some("Tên phải dài 3–16 ký tự, chỉ gồm chữ, số và dấu _".into());
            return;
        }
        self.login_err = None;
        self.cfg.account = Some(auth::offline_account(&name));
        self.cfg.save();
        self.log.push(format!("Đang dùng tài khoản offline: {name}"));
    }

    fn logout(&mut self) {
        self.cfg.account = None;
        self.cfg.save();
        self.skin_tex = None;
    }

    fn launch(&mut self) {
        self.launching = true;
        self.progress.set("Chuẩn bị…");
        self.cfg.save();
        let cfg = self.cfg.clone();
        let prog = self.progress.clone();
        self.bg(move |tx| {
            let run = || -> Result<()> {
                let acc = valid_account(&cfg, &tx)?;
                let prep = mojang::prepare(&cfg.mc_version, &cfg.loader, &prog)?;
                prog.set("Đang khởi động Minecraft…");
                let logger: launch::Logger = {
                    let tx = tx.clone();
                    Arc::new(move |s| tx.log(s))
                };
                let mut child = launch::launch(&cfg, &acc, &prep, logger)?;
                tx.send(Msg::GameStarted);
                let status = child.wait()?;
                tx.log(format!("Game đã thoát ({status})"));
                Ok(())
            };
            if let Err(e) = run() {
                tx.log(format!("Lỗi: {e:#}"));
            }
            tx.send(Msg::GameExited);
        });
    }

    fn search(&mut self) {
        self.searching = true;
        let q = self.query.clone();
        let (mc, loader) = (self.cfg.mc_version.clone(), self.cfg.loader.clone());
        self.bg(move |tx| match modrinth::search(&q, &mc, &loader) {
            Ok(h) => tx.send(Msg::Hits(h)),
            Err(e) => {
                tx.log(format!("Lỗi tìm mod: {e:#}"));
                tx.send(Msg::Hits(vec![]));
            }
        });
    }

    fn install_mod(&mut self, id: String) {
        self.installing = true;
        let (mc, loader) = (self.cfg.mc_version.clone(), self.cfg.loader.clone());
        self.bg(move |tx| {
            let log = |s: String| tx.log(s);
            let mut seen = HashSet::new();
            match modrinth::install(&id, &mc, &loader, &mut seen, &log) {
                Ok(()) => tx.log("Cài mod xong."),
                Err(e) => tx.log(format!("Lỗi cài mod: {e:#}")),
            }
            tx.send(Msg::Installed);
        });
    }

    fn load_skin(&self) {
        if self.cfg.account.as_ref().map_or(true, |a| a.offline) {
            return;
        }
        let cfg = self.cfg.clone();
        self.bg(move |tx| {
            let r = (|| -> Result<_> {
                let acc = valid_account(&cfg, &tx)?;
                let (img, slim) = skin::current(&acc.mc_token)?;
                Ok((skin::front_preview(&img, slim), slim))
            })();
            match r {
                Ok((pv, slim)) => tx.send(Msg::Skin(to_color(&pv), slim)),
                Err(e) => tx.log(format!("Không tải được skin: {e:#}")),
            }
        });
    }

    fn upload_skin(&mut self) {
        let Some(path) = self.skin_path.clone() else { return };
        self.skin_busy = true;
        self.skin_status = None;
        let cfg = self.cfg.clone();
        let slim = self.new_slim;
        self.bg(move |tx| {
            let r = (|| -> Result<()> {
                let acc = valid_account(&cfg, &tx)?;
                skin::upload(&acc.mc_token, &path, slim)
            })();
            tx.send(Msg::SkinUploaded(r.map_err(|e| format!("{e:#}"))));
        });
    }

    fn refresh_local(&mut self, ctx: &egui::Context) {
        let Some(p) = self.skin_path.clone() else { return };
        match skin::load_local(&p) {
            Ok(img) => {
                let pv = skin::front_preview(&img, self.new_slim);
                self.local_tex =
                    Some(ctx.load_texture("local-skin", to_color(&pv), egui::TextureOptions::NEAREST));
                self.skin_status = None;
            }
            Err(e) => {
                self.skin_status = Some((false, format!("{e:#}")));
                self.local_tex = None;
                self.skin_path = None;
            }
        }
    }

    // ───────────── Xử lý tin nhắn ─────────────

    fn handle(&mut self, ctx: &egui::Context, m: Msg) {
        match m {
            Msg::Log(s) => {
                self.log.push(s);
                if self.log.len() > 1500 {
                    self.log.drain(..300);
                }
            }
            Msg::Versions(v) => {
                self.versions = v;
            }
            Msg::DeviceCode(d) => self.device = Some(d),
            Msg::LoggedIn(a) => {
                self.logging_in = false;
                self.device = None;
                self.log.push(format!("Đăng nhập thành công: {}", a.username));
                self.cfg.account = Some(a);
                self.cfg.save();
                self.load_skin();
            }
            Msg::TokenRefreshed(a) => {
                self.cfg.account = Some(a);
                self.cfg.save();
            }
            Msg::LoginFailed(e) => {
                self.logging_in = false;
                self.device = None;
                self.log.push(format!("Đăng nhập lỗi: {e}"));
                self.login_err = Some(e);
            }
            Msg::Hits(h) => {
                self.hits = h;
                self.searching = false;
            }
            Msg::Installed => {
                self.installing = false;
                self.installed = modrinth::list_installed();
            }
            Msg::Skin(img, slim) => {
                self.skin_tex = Some(ctx.load_texture("skin", img, egui::TextureOptions::NEAREST));
                self.new_slim = slim;
            }
            Msg::SkinUploaded(r) => {
                self.skin_busy = false;
                match r {
                    Ok(()) => {
                        self.skin_status = Some((true, "Đã đổi skin thành công!".into()));
                        self.load_skin();
                    }
                    Err(e) => self.skin_status = Some((false, e)),
                }
            }
            Msg::GameStarted => {
                self.launching = false;
                self.running = true;
            }
            Msg::GameExited => {
                self.launching = false;
                self.running = false;
            }
        }
    }

    // ───────────── Giao diện ─────────────

    fn ui_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.label(RichText::new("Nexia").size(26.0).strong().color(ACCENT));
        ui.label(RichText::new("Minecraft Java Launcher").size(12.0).weak());
        ui.add_space(16.0);

        for (t, label) in [
            (Tab::Play, "Chơi"),
            (Tab::Mods, "Mod (Modrinth)"),
            (Tab::Skin, "Skin"),
            (Tab::Settings, "Cài đặt"),
        ] {
            let r = ui.add_sized(
                [ui.available_width(), 38.0],
                egui::SelectableLabel::new(self.tab == t, RichText::new(label).size(16.0)),
            );
            if r.clicked() {
                self.tab = t;
            }
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(10.0);
            if let Some(acc) = self.cfg.account.clone() {
                if ui.button("Đăng xuất").clicked() {
                    self.logout();
                }
                ui.label(RichText::new(&acc.username).strong().size(16.0));
                ui.label(
                    RichText::new(if acc.offline { "Tài khoản offline" } else { "Tài khoản Microsoft" })
                        .small()
                        .weak(),
                );
            } else if self.logging_in {
                if ui.button("Huỷ").clicked() {
                    self.cancel_login();
                }
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Đang chờ đăng nhập…");
                });
            } else {
                // Layout bottom-up: mục thêm trước nằm dưới cùng.
                if ui.button("Chơi offline").clicked() {
                    self.login_offline();
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.offline_name)
                        .hint_text("Tên người chơi")
                        .desired_width(ui.available_width()),
                );
                ui.label(RichText::new("Hoặc chơi offline").small().weak());
                if let Some(e) = &self.login_err {
                    ui.colored_label(Color32::LIGHT_RED, e);
                }
                if ui
                    .add(
                        egui::Button::new(RichText::new("Đăng nhập Microsoft").strong())
                            .fill(ACCENT)
                            .min_size(egui::vec2(ui.available_width(), 36.0)),
                    )
                    .clicked()
                {
                    self.login();
                }
            }
        });
    }

    fn ui_device_window(&mut self, ctx: &egui::Context) {
        let Some(dc) = self.device.clone() else { return };
        egui::Window::new("Đăng nhập Microsoft")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("1. Mở trang đăng nhập:");
                ui.hyperlink(&dc.verification_uri);
                ui.add_space(6.0);
                ui.label("2. Nhập mã này:");
                ui.label(RichText::new(&dc.user_code).monospace().size(34.0).strong().color(ACCENT));
                ui.horizontal(|ui| {
                    if ui.button("Mở trang đăng nhập").clicked() {
                        let _ = open::that(&dc.verification_uri);
                    }
                    if ui.button("Sao chép mã").clicked() {
                        ctx.copy_text(dc.user_code.clone());
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Đang chờ bạn hoàn tất…");
                });
                if ui.button("Huỷ").clicked() {
                    self.cancel_login();
                }
            });
    }

    fn ui_play(&mut self, ui: &mut egui::Ui) {
        ui.heading("Chơi Minecraft");
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Phiên bản:");
            egui::ComboBox::from_id_salt("ver")
                .selected_text(self.cfg.mc_version.clone())
                .width(140.0)
                .show_ui(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for v in &self.versions {
                            ui.selectable_value(&mut self.cfg.mc_version, v.clone(), v.as_str());
                        }
                    });
                });
            ui.add_space(20.0);
            ui.label("Loader:");
            ui.radio_value(&mut self.cfg.loader, "vanilla".to_string(), "Vanilla");
            ui.radio_value(&mut self.cfg.loader, "fabric".to_string(), "Fabric");
        });
        ui.add_space(8.0);

        let ready = self.cfg.account.is_some() && !self.launching && !self.running;
        let label = if self.running {
            "ĐANG CHẠY"
        } else if self.launching {
            "ĐANG TẢI…"
        } else {
            "CHƠI"
        };
        let btn = egui::Button::new(RichText::new(label).size(22.0).strong())
            .fill(ACCENT)
            .min_size(egui::vec2(240.0, 52.0));
        if ui.add_enabled(ready, btn).clicked() {
            self.launch();
        }
        if self.cfg.account.is_none() {
            ui.colored_label(
                Color32::YELLOW,
                "Hãy đăng nhập tài khoản Microsoft ở thanh bên trái để chơi.",
            );
        }

        ui.add_space(10.0);
        ui.separator();
        ui.label(RichText::new("Nhật ký").strong());
        egui::ScrollArea::vertical()
            .id_salt("log")
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for l in &self.log {
                    ui.label(RichText::new(l).monospace().size(12.0));
                }
            });
    }

    fn ui_mods(&mut self, ui: &mut egui::Ui) {
        ui.heading("Mod từ Modrinth");
        let vanilla = self.cfg.loader == "vanilla";
        if vanilla {
            ui.colored_label(
                Color32::YELLOW,
                "Loader đang là Vanilla — hãy chọn Fabric ở tab Chơi để dùng mod.",
            );
        }
        ui.label(
            RichText::new(format!(
                "Lọc theo Minecraft {} · {}",
                self.cfg.mc_version, self.cfg.loader
            ))
            .weak(),
        );

        let mut do_search = false;
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Tìm mod (vd: sodium, lithium, iris)…")
                    .desired_width(360.0),
            );
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                do_search = true;
            }
            if ui.add_enabled(!self.searching, egui::Button::new("Tìm")).clicked() {
                do_search = true;
            }
            if ui.button("Mở thư mục mods").clicked() {
                let _ = open::that(mods_dir());
            }
            if self.searching || self.installing {
                ui.spinner();
            }
        });
        if do_search {
            self.search();
        }

        let mut install: Option<String> = None;
        let mut remove: Option<String> = None;
        egui::ScrollArea::vertical()
            .id_salt("mods")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.hits.is_empty() && !self.searching {
                    ui.label(RichText::new("Nhấn “Tìm” để xem các mod phổ biến.").weak());
                }
                for h in &self.hits {
                    ui.group(|ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(&h.title).strong().size(16.0));
                        ui.label(
                            RichText::new(format!("{} · {} lượt tải", h.author, fmt_num(h.downloads)))
                                .small()
                                .weak(),
                        );
                        ui.label(h.desc.as_str());
                        if ui
                            .add_enabled(!self.installing && !vanilla, egui::Button::new("Cài"))
                            .clicked()
                        {
                            install = Some(h.id.clone());
                        }
                    });
                }
                ui.add_space(8.0);
                ui.separator();
                ui.label(RichText::new(format!("Đã cài ({})", self.installed.len())).strong());
                for f in &self.installed {
                    ui.horizontal(|ui| {
                        ui.label(f.as_str());
                        if ui.small_button("Xoá").clicked() {
                            remove = Some(f.clone());
                        }
                    });
                }
            });

        if let Some(id) = install {
            self.install_mod(id);
        }
        if let Some(f) = remove {
            modrinth::remove(&f);
            self.installed = modrinth::list_installed();
        }
    }

    fn ui_skin(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Skin");
        if self.cfg.account.is_none() {
            ui.label("Hãy đăng nhập Microsoft để xem và đổi skin.");
            return;
        }
        if self.cfg.account.as_ref().map_or(false, |a| a.offline) {
            ui.label("Tài khoản offline không hỗ trợ đổi skin. Hãy đăng xuất rồi đăng nhập Microsoft.");
            return;
        }
        let size = egui::vec2(128.0, 256.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.label("Hiện tại");
                match &self.skin_tex {
                    Some(t) => {
                        ui.add(egui::Image::new(egui::load::SizedTexture::new(t.id(), size)));
                    }
                    None => {
                        ui.allocate_space(size);
                    }
                }
            });
            ui.add_space(24.0);
            ui.vertical(|ui| {
                ui.label("Skin mới");
                match &self.local_tex {
                    Some(t) => {
                        ui.add(egui::Image::new(egui::load::SizedTexture::new(t.id(), size)));
                    }
                    None => {
                        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                        ui.painter().rect_stroke(
                            rect,
                            6.0,
                            egui::Stroke::new(1.0, Color32::DARK_GRAY),
                        );
                    }
                }
            });
            ui.add_space(24.0);
            ui.vertical(|ui| {
                if ui.button("Chọn file skin (.png)…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("PNG", &["png"]).pick_file() {
                        self.skin_path = Some(p);
                        self.refresh_local(ctx);
                    }
                }
                if let Some(p) = &self.skin_path {
                    ui.label(
                        RichText::new(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                            .small()
                            .weak(),
                    );
                }
                ui.add_space(6.0);
                ui.label("Kiểu tay:");
                let before = self.new_slim;
                ui.radio_value(&mut self.new_slim, false, "Classic (tay 4px)");
                ui.radio_value(&mut self.new_slim, true, "Slim (tay 3px)");
                if before != self.new_slim {
                    self.refresh_local(ctx);
                }
                ui.add_space(10.0);
                let can = self.skin_path.is_some() && !self.skin_busy;
                if ui
                    .add_enabled(
                        can,
                        egui::Button::new(RichText::new("Đổi skin").strong())
                            .fill(ACCENT)
                            .min_size(egui::vec2(160.0, 36.0)),
                    )
                    .clicked()
                {
                    self.upload_skin();
                }
                if ui.button("Tải lại skin hiện tại").clicked() {
                    self.load_skin();
                }
                if self.skin_busy {
                    ui.spinner();
                }
                if let Some((ok, msg)) = &self.skin_status {
                    let c = if *ok { ACCENT } else { Color32::LIGHT_RED };
                    ui.colored_label(c, msg.as_str());
                }
            });
        });
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Cài đặt");
        ui.add_space(6.0);
        egui::Grid::new("settings").num_columns(2).spacing([16.0, 12.0]).show(ui, |ui| {
            ui.label("Azure Client ID:");
            ui.add(
                egui::TextEdit::singleline(&mut self.cfg.client_id)
                    .hint_text("xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx")
                    .desired_width(380.0),
            );
            ui.end_row();

            ui.label("RAM tối đa:");
            ui.add(
                egui::Slider::new(&mut self.cfg.ram_mb, 1024..=16384)
                    .step_by(256.0)
                    .suffix(" MB"),
            );
            ui.end_row();

            ui.label("Đường dẫn Java:");
            ui.add(egui::TextEdit::singleline(&mut self.cfg.java_path).desired_width(380.0));
            ui.end_row();
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Lưu cài đặt").clicked() {
                self.cfg.save();
                self.log.push("Đã lưu cài đặt.".into());
            }
            if ui.button("Mở thư mục dữ liệu").clicked() {
                let _ = open::that(data_dir());
            }
        });
        ui.add_space(14.0);
        ui.separator();
        ui.label(
            RichText::new(
                "Azure Client ID: đăng ký app tại portal.azure.com (loại tài khoản có “personal Microsoft \
                 accounts”, bật “Allow public client flows”). Chi tiết trong README.",
            )
            .small()
            .weak(),
        );
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(m) = self.rx.try_recv() {
            self.handle(ctx, m);
        }
        if self.launching {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::SidePanel::left("nav")
            .exact_width(210.0)
            .show(ctx, |ui| self.ui_sidebar(ui));

        if self.launching {
            egui::TopBottomPanel::bottom("progress").show(ctx, |ui| {
                let (status, done, total) = self.progress.snapshot();
                let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                ui.add_space(6.0);
                ui.add(
                    egui::ProgressBar::new(frac)
                        .text(if total > 0 {
                            format!("{status} ({done}/{total})")
                        } else {
                            status
                        })
                        .animate(true),
                );
                ui.add_space(6.0);
            });
        }

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Play => self.ui_play(ui),
            Tab::Mods => self.ui_mods(ui),
            Tab::Skin => self.ui_skin(ui, ctx),
            Tab::Settings => self.ui_settings(ui),
        });

        self.ui_device_window(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cfg.save();
    }
}

fn setup_style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::from_rgb(20, 22, 27);
    v.window_fill = Color32::from_rgb(28, 31, 38);
    v.extreme_bg_color = Color32::from_rgb(14, 15, 19);
    v.selection.bg_fill = Color32::from_rgb(46, 125, 50);
    let r = egui::Rounding::same(8.0);
    v.widgets.noninteractive.rounding = r;
    v.widgets.inactive.rounding = r;
    v.widgets.hovered.rounding = r;
    v.widgets.active.rounding = r;
    v.widgets.open.rounding = r;
    ctx.set_visuals(v);

    let mut s = (*ctx.style()).clone();
    s.spacing.item_spacing = egui::vec2(10.0, 10.0);
    s.spacing.button_padding = egui::vec2(14.0, 8.0);
    ctx.set_style(s);
}

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([980.0, 640.0])
            .with_min_inner_size([760.0, 480.0])
            .with_title("Nexia Launcher"),
        ..Default::default()
    };
    eframe::run_native(
        "Nexia Launcher",
        opts,
        Box::new(|cc| {
            setup_style(&cc.egui_ctx);
            Ok(Box::new(App::new(cc)))
        }),
    )
}
