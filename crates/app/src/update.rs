//! Update checks and the separate window that installs an update after Airframe exits.
use eframe::egui::{self, Align, CentralPanel, Context, Id, Layout, Modal, ProgressBar, RichText};
use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs::{self, File},
    io::{Read, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    time::{Duration, Instant},
};
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0},
    System::Threading::{CREATE_NO_WINDOW, OpenProcess, WaitForSingleObject},
};

const REPO: &str = "Hola3Dprint/Airframe";
const SETUP: &str = "Airframe-Setup-x64.exe";
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const MAX_DOWNLOAD: u64 = 128 * 1024 * 1024;
type UpdateError = Box<dyn std::error::Error + Send + Sync>;

pub fn installed_root() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Airframe");
    let current = std::env::current_exe().ok()?.canonicalize().ok()?;
    let installed = root.join("Airframe.exe").canonicalize().ok()?;
    (current == installed).then_some(root)
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
    size: u64,
}

impl Release {
    fn update(&self, current: &Version) -> Result<Option<(&Asset, &str)>, UpdateError> {
        let version = Version::parse(self.tag_name.trim_start_matches('v'))?;
        if &version <= current || !version.pre.is_empty() {
            return Ok(None);
        }
        let asset = self
            .assets
            .iter()
            .find(|asset| asset.name == SETUP)
            .ok_or("The release has no Windows installer")?;
        let expected = format!(
            "https://github.com/{REPO}/releases/download/{}/{SETUP}",
            self.tag_name
        );
        if asset.browser_download_url != expected || asset.size == 0 || asset.size > MAX_DOWNLOAD {
            return Err("The release installer has invalid metadata".into());
        }
        let hash = asset
            .digest
            .as_deref()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("The release does not provide an installer checksum")?;
        Ok(Some((asset, hash)))
    }
}

pub struct Download {
    dir: PathBuf,
    version: String,
    digest: String,
}

enum Event {
    Current,
    Available(Update),
    Progress { version: String, fraction: f32 },
    Ready(Download),
    Failed(String),
}

enum CheckStatus {
    Unchecked,
    Current,
    Available(Update),
    Failed(String),
}

#[derive(Clone)]
struct Update {
    version: String,
    url: String,
    size: u64,
    digest: String,
}

pub struct Updater {
    root: Option<PathBuf>,
    next_check: Instant,
    receiver: Option<mpsc::Receiver<Event>>,
    progress: Option<(String, f32)>,
    manual: bool,
    ready: Option<Download>,
    status: CheckStatus,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            root: installed_root(),
            next_check: if std::env::var_os("AIRFRAME_SKIP_UPDATE_CHECK").is_some() {
                Instant::now() + CHECK_INTERVAL
            } else {
                Instant::now()
            },
            receiver: None,
            progress: None,
            manual: false,
            ready: None,
            status: CheckStatus::Unchecked,
        }
    }
}

impl Updater {
    pub fn installed(&self) -> bool {
        self.root.is_some()
    }

    pub fn check(&mut self, ctx: &Context, manual: bool) {
        if self.receiver.is_some() {
            self.manual |= manual;
            return;
        }
        if self.ready.is_some() {
            return;
        }
        if !self.installed() {
            return;
        }
        if !manual && Instant::now() < self.next_check {
            ctx.request_repaint_after(self.next_check.saturating_duration_since(Instant::now()));
            return;
        }
        self.next_check = Instant::now() + CHECK_INTERVAL;
        self.manual = manual;
        self.start(ctx, |_| match latest() {
            Ok(Some(update)) => Event::Available(update),
            Ok(None) => Event::Current,
            Err(error) => Event::Failed(error.to_string()),
        });
    }

    fn start(
        &mut self,
        ctx: &Context,
        task: impl FnOnce(&dyn Fn(Event)) -> Event + Send + 'static,
    ) {
        let (send, receive) = mpsc::channel();
        self.receiver = Some(receive);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let report = |event| {
                let _ = send.send(event);
                ctx.request_repaint();
            };
            report(task(&report));
        });
    }

    fn install(&mut self, ctx: &Context, update: Update) {
        let Some(root) = self.root.clone() else {
            return;
        };
        self.progress = Some((update.version.clone(), 0.0));
        self.start(ctx, move |report| match download(&root, &update, report) {
            Ok(ready) => Event::Ready(ready),
            Err(error) => Event::Failed(error.to_string()),
        });
    }

    pub fn poll(&mut self) -> Option<(String, bool)> {
        let rx = self.receiver.as_ref()?;
        loop {
            let event = match rx.try_recv() {
                Ok(event) => event,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Event::Failed("The update task ended unexpectedly".into())
                }
            };
            match event {
                Event::Progress { version, fraction } => self.progress = Some((version, fraction)),
                Event::Ready(download) => {
                    self.ready = Some(download);
                    self.receiver = None;
                    break;
                }
                Event::Current => {
                    self.receiver = None;
                    self.status = CheckStatus::Current;
                    if self.manual {
                        return Some(("Airframe is up to date".into(), false));
                    }
                    break;
                }
                Event::Available(update) => {
                    self.receiver = None;
                    let version = update.version.clone();
                    self.status = CheckStatus::Available(update);
                    if self.manual {
                        return Some((format!("Airframe v{version} is available"), false));
                    }
                    break;
                }
                Event::Failed(error) => {
                    self.receiver = None;
                    self.status = CheckStatus::Failed(error.clone());
                    let downloading = self.progress.take().is_some();
                    if self.manual || downloading {
                        return Some((format!("Update unavailable · {error}"), true));
                    }
                    break;
                }
            }
        }
        None
    }

    pub fn controls(&mut self, ui: &mut egui::Ui) -> bool {
        if !self.installed() {
            ui.hyperlink_to(
                "Windows releases",
                format!("https://github.com/{REPO}/releases/latest"),
            );
            return false;
        }
        if let Some((version, _)) = &self.progress {
            ui.label(format!("Downloading v{version}…"));
        } else if self.receiver.is_some() {
            ui.label("Checking for updates…");
        } else if let CheckStatus::Available(update) = &self.status {
            if ui
                .button(format!("Update to v{}", update.version))
                .clicked()
            {
                self.install(ui.ctx(), update.clone());
                return true;
            }
        } else {
            match &self.status {
                CheckStatus::Current => {
                    ui.label("Up to date");
                }
                CheckStatus::Failed(error) => {
                    ui.label("Update check failed").on_hover_text(error);
                }
                _ => {}
            }
            let label = if matches!(self.status, CheckStatus::Current) {
                "Check again"
            } else {
                "Check for updates"
            };
            if ui.button(label).clicked() {
                self.check(ui.ctx(), true);
            }
        }
        false
    }

    pub fn ready(&self) -> bool {
        self.ready.is_some()
    }

    pub fn show(&self, ctx: &Context) {
        if let Some((version, fraction)) = &self.progress {
            Modal::new(Id::new("airframe-update")).show(ctx, |ui| {
                ui.set_min_width(300.0);
                ui.heading(format!("Updating Airframe to {version}"));
                ui.label("Downloading the Windows update…");
                ui.add(ProgressBar::new(*fraction).show_percentage());
            });
        }
    }

    pub fn launch(
        &mut self,
        project: Option<&Path>,
        resume: Option<&Path>,
    ) -> Result<(), UpdateError> {
        let ready = self.ready.take().ok_or("No update is ready")?;
        self.progress = None;
        let mut command = Command::new(ready.dir.join("update.exe"));
        command
            .arg("--apply-update")
            .arg(std::process::id().to_string())
            .arg(&ready.version)
            .arg(&ready.digest);
        if let Some(path) = resume {
            command.arg("--resume-update").arg(path);
        } else if let Some(path) = project {
            command.arg(path);
        }
        if let Err(error) = command.spawn() {
            self.status = CheckStatus::Failed(error.to_string());
            let _ = fs::remove_file(ready.dir.join(SETUP));
            let _ = fs::remove_file(ready.dir.join("update.exe"));
            let _ = fs::remove_dir(&ready.dir);
            return Err(error.into());
        }
        Ok(())
    }

    pub fn defer(&mut self) {
        if let Some(ready) = self.ready.take() {
            let _ = fs::remove_file(ready.dir.join(SETUP));
            let _ = fs::remove_file(ready.dir.join("update.exe"));
            let _ = fs::remove_dir(&ready.dir);
        }
        self.progress = None;
    }
}

fn update_client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .user_agent(concat!("Airframe/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::custom(|redirect| {
            if redirect.previous().len() < 5
                && redirect.url().scheme() == "https"
                && matches!(
                    redirect.url().host_str(),
                    Some(
                        "github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
            {
                redirect.follow()
            } else {
                redirect.error("Unexpected release download redirect")
            }
        }))
        .build()
}

fn latest() -> Result<Option<Update>, UpdateError> {
    let client = update_client()?;
    let api = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let release: Release = client
        .get(api)
        .header("Accept", "application/vnd.github+json")
        .send()?
        .error_for_status()?
        .json()?;
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let Some((asset, digest)) = release.update(&current)? else {
        return Ok(None);
    };
    Ok(Some(Update {
        version: release.tag_name.trim_start_matches('v').to_owned(),
        url: asset.browser_download_url.clone(),
        size: asset.size,
        digest: digest.into(),
    }))
}

fn download(root: &Path, update: &Update, report: &dyn Fn(Event)) -> Result<Download, UpdateError> {
    let client = update_client()?;
    let Update {
        version,
        url,
        size,
        digest,
    } = update;
    let dir = root.join("updates").join(Uuid::new_v4().to_string());
    fs::create_dir_all(&dir)?;
    let result = (|| {
        report(Event::Progress {
            version: version.clone(),
            fraction: 0.0,
        });
        let mut response = client.get(url).send()?.error_for_status()?;
        let mut file = File::create(dir.join(SETUP))?;
        let mut checksum = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = [0; 32 * 1024];
        let mut last_progress = Instant::now();
        loop {
            let read = response.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            bytes += read as u64;
            if bytes > *size {
                return Err("The installer is larger than expected".into());
            }
            file.write_all(&buffer[..read])?;
            checksum.update(&buffer[..read]);
            if last_progress.elapsed() >= Duration::from_millis(80) {
                report(Event::Progress {
                    version: version.clone(),
                    fraction: bytes as f32 / *size as f32,
                });
                last_progress = Instant::now();
            }
        }
        if bytes != *size || !format!("{:x}", checksum.finalize()).eq_ignore_ascii_case(digest) {
            return Err("The installer checksum does not match the GitHub release".into());
        }
        file.sync_all()?;
        fs::copy(std::env::current_exe()?, dir.join("update.exe"))?;
        Ok(Download {
            dir: dir.clone(),
            version: version.clone(),
            digest: digest.clone(),
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(dir.join(SETUP));
        let _ = fs::remove_file(dir.join("update.exe"));
        let _ = fs::remove_dir(dir);
    }
    result
}

fn wait_for_parent(pid: u32) -> Result<(), UpdateError> {
    // SYNCHRONIZE access is enough to wait for the old process to release its executable.
    const SYNCHRONIZE: u32 = 0x0010_0000;
    let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            return Ok(());
        }
        return Err(error.into());
    }
    let result = unsafe {
        let status = WaitForSingleObject(handle, 30_000);
        CloseHandle(handle);
        status
    };
    if result != WAIT_OBJECT_0 {
        return Err("Airframe did not close in time".into());
    }
    Ok(())
}

pub fn apply(mut args: impl Iterator<Item = OsString>) -> eframe::Result {
    let Some(pid) = args
        .next()
        .and_then(|pid| pid.to_str()?.parse::<u32>().ok())
    else {
        return Ok(());
    };
    let Some(version) = args.next().and_then(|value| value.into_string().ok()) else {
        return Ok(());
    };
    let Some(digest) = args.next().and_then(|value| value.into_string().ok()) else {
        return Ok(());
    };
    let reopen_args: Vec<_> = args.collect();
    let Ok(helper) = std::env::current_exe() else {
        return Ok(());
    };
    let Some(dir) = helper.parent().map(Path::to_path_buf) else {
        return Ok(());
    };
    let Some(root) =
        std::env::var_os("LOCALAPPDATA").map(|path| PathBuf::from(path).join("Airframe"))
    else {
        return Ok(());
    };
    let Ok(expected) = root.join("updates").canonicalize() else {
        return Ok(());
    };
    if dir
        .parent()
        .and_then(|path| path.canonicalize().ok())
        .as_ref()
        != Some(&expected)
    {
        return Ok(());
    }
    eframe::run_native(
        "Updating Airframe",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([420.0, 180.0])
                .with_resizable(false),
            ..Default::default()
        },
        Box::new(move |cc| {
            crate::style::install(&cc.egui_ctx, false);
            let (send, inbox) = mpsc::channel();
            let ctx = cc.egui_ctx.clone();
            let app = root.join("Airframe.exe");
            let next_app = app.clone();
            let next_args = reopen_args.clone();
            std::thread::spawn(move || {
                let result =
                    install(&dir, pid, &digest).and_then(|()| reopen(&next_app, &next_args, false));
                let _ = send.send(result.map_err(|error| error.to_string()));
                ctx.request_repaint();
            });
            Ok(Box::new(InstallScreen {
                version,
                inbox,
                error: None,
                app,
                reopen_args,
            }))
        }),
    )
}

fn install(dir: &Path, pid: u32, digest: &str) -> Result<(), UpdateError> {
    wait_for_parent(pid)?;
    let installer = dir.join(SETUP);
    let mut checksum = Sha256::new();
    std::io::copy(&mut File::open(&installer)?, &mut checksum)?;
    if !format!("{:x}", checksum.finalize()).eq_ignore_ascii_case(digest) {
        return Err("The installer checksum changed".into());
    }
    let status = Command::new(&installer)
        .arg("/S")
        .arg("/UPDATE")
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    let _ = fs::remove_file(installer);
    if !status.success() {
        return Err(format!("Windows setup returned {status}.").into());
    }
    Ok(())
}

fn reopen(app: &Path, args: &[OsString], skip_update: bool) -> Result<(), UpdateError> {
    let mut command = Command::new(app);
    if skip_update {
        command.env("AIRFRAME_SKIP_UPDATE_CHECK", "1");
    }
    command.args(args);
    command.spawn()?;
    Ok(())
}

struct InstallScreen {
    version: String,
    inbox: mpsc::Receiver<Result<(), String>>,
    error: Option<String>,
    app: PathBuf,
    reopen_args: Vec<OsString>,
}

impl eframe::App for InstallScreen {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if let Ok(result) = self.inbox.try_recv() {
            match result {
                Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Err(error) => self.error = Some(error),
            }
        }
        CentralPanel::default().show(ctx, |ui| {
            ui.with_layout(Layout::top_down(Align::Center), |ui| {
                ui.add_space(24.0);
                if let Some(error) = &self.error {
                    ui.heading("Airframe update stopped");
                    ui.label(error);
                    if ui.button("Open Airframe").clicked() {
                        match reopen(&self.app, &self.reopen_args, true) {
                            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    }
                } else {
                    ui.heading(format!("Updating Airframe to {}", self.version));
                    ui.label(
                        RichText::new("Installing the update. Airframe will reopen automatically.")
                            .weak(),
                    );
                    ui.add_space(12.0);
                    ui.spinner();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_check_records_results_without_starting_a_download() {
        let mut updater = Updater::default();
        let deliver = |updater: &mut Updater, event| {
            let (send, receive) = mpsc::channel();
            send.send(event).unwrap();
            updater.receiver = Some(receive);
            updater.poll();
        };
        deliver(&mut updater, Event::Current);
        assert!(matches!(updater.status, CheckStatus::Current));
        deliver(
            &mut updater,
            Event::Available(Update {
                version: "1.0.0".into(),
                url: String::new(),
                size: 1,
                digest: String::new(),
            }),
        );
        assert!(matches!(updater.status, CheckStatus::Available(_)));
        assert!(updater.receiver.is_none() && updater.progress.is_none() && !updater.ready());
        deliver(&mut updater, Event::Failed("offline".into()));
        assert!(matches!(updater.status, CheckStatus::Failed(ref error) if error == "offline"));
    }
}
