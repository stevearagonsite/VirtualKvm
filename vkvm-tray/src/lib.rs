//! Menu bar (macOS) / system tray (Windows) icon for VirtualKvm.
//!
//! `lan-mouse tray` runs in the background, owns the daemon as a child process and
//! offers a single switch to turn mouse & keyboard sharing on or off. The choice is
//! remembered across restarts. Running the daemon as a child of the same binary keeps
//! the macOS Accessibility / Input Monitoring permission attached to one executable.

mod icon;

/// RGBA pixels of the tray icon (`high_contrast`: white glyph with black outline,
/// otherwise black template glyph). Used to export the icon files.
pub fn icon_rgba(on: bool, size: u32, high_contrast: bool) -> Vec<u8> {
    let style = if high_contrast {
        icon::Style::HighContrast
    } else {
        icon::Style::Template
    };
    icon::rgba(on, size, style)
}

use std::{io, path::PathBuf};
use thiserror::Error;

#[derive(clap::Args, Clone, Debug, PartialEq, Eq)]
pub struct TrayArgs {
    /// file for the daemon's log output (default: next to the lan-mouse config)
    #[arg(long)]
    log: Option<PathBuf>,
    /// lan-mouse executable to run as daemon / web UI (default: this executable).
    /// On macOS a separate, stable tray executable keeps the Accessibility
    /// permission while the daemon binary gets updated: TCC attributes the
    /// child to the tray, its responsible process.
    #[arg(long)]
    daemon_exe: Option<PathBuf>,
}

#[derive(Debug, Error)]
pub enum TrayError {
    #[error("the tray icon is only available on macOS and Windows")]
    Unsupported,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("tray icon: {0}")]
    Tray(String),
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn run(_args: TrayArgs) -> Result<(), TrayError> {
    Err(TrayError::Unsupported)
}

#[cfg(any(target_os = "macos", windows))]
pub use imp::run;

#[cfg(any(target_os = "macos", windows))]
mod imp {
    use super::{TrayArgs, TrayError, icon};
    use std::{
        fs::{self, OpenOptions},
        io::{BufRead, BufReader},
        path::PathBuf,
        process::{Child, Command, Stdio},
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };
    use tao::{
        event::{Event, StartCause},
        event_loop::{ControlFlow, EventLoopBuilder},
    };
    use tray_icon::{
        Icon, TrayIcon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    };

    #[cfg(target_os = "macos")]
    const STYLE: icon::Style = icon::Style::Template;
    #[cfg(target_os = "macos")]
    const ICON_SIZE: u32 = 44; // 22 pt @2x
    #[cfg(windows)]
    const STYLE: icon::Style = icon::Style::HighContrast;
    #[cfg(windows)]
    const ICON_SIZE: u32 = 32;

    enum UserEvent {
        Menu(MenuEvent),
    }

    fn config_dir() -> PathBuf {
        #[cfg(unix)]
        let base = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
            });
        #[cfg(windows)]
        let base = std::env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        base.join("virtualkvm")
    }

    /// remembered on/off switch
    struct Switch(PathBuf);

    impl Switch {
        fn get(&self) -> bool {
            fs::read_to_string(&self.0)
                .map(|s| s.trim() != "off")
                .unwrap_or(true)
        }
        fn set(&self, on: bool) {
            let _ = fs::create_dir_all(self.0.parent().expect("parent"));
            let _ = fs::write(&self.0, if on { "on\n" } else { "off\n" });
        }
    }

    static DAEMON_EXE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

    fn command(args: &[&str]) -> Command {
        let exe = DAEMON_EXE
            .get()
            .cloned()
            .unwrap_or_else(|| std::env::current_exe().expect("current exe"));
        let mut cmd = Command::new(exe);
        cmd.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd
    }

    fn stop_child(child: &mut Child) {
        #[cfg(unix)]
        unsafe {
            // graceful: lets the daemon release keys and close its connections
            libc::kill(child.id() as libc::pid_t, libc::SIGINT);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    struct Daemon {
        child: Option<Child>,
        log: PathBuf,
    }

    impl Daemon {
        fn running(&mut self) -> bool {
            match &mut self.child {
                Some(c) => match c.try_wait() {
                    Ok(None) => true,
                    _ => {
                        self.child = None;
                        false
                    }
                },
                None => false,
            }
        }

        fn start(&mut self) {
            if self.running() {
                return;
            }
            let _ = fs::create_dir_all(self.log.parent().expect("parent"));
            let open = || OpenOptions::new().create(true).append(true).open(&self.log);
            let mut cmd = command(&["daemon"]);
            if std::env::var_os("RUST_LOG").is_none() {
                cmd.env("RUST_LOG", "info");
            }
            match (open(), open()) {
                (Ok(out), Ok(err)) => {
                    cmd.stdout(out).stderr(err);
                }
                _ => {
                    cmd.stdout(Stdio::null()).stderr(Stdio::null());
                }
            }
            match cmd.spawn() {
                Ok(c) => {
                    log::info!("daemon started (pid {})", c.id());
                    self.child = Some(c);
                }
                Err(e) => log::error!("could not start the daemon: {e}"),
            }
        }

        fn stop(&mut self) {
            if let Some(mut c) = self.child.take() {
                stop_child(&mut c);
                log::info!("daemon stopped");
            }
        }
    }

    /// `lan-mouse web` child; its URL (with the token) is read from stdout
    #[derive(Default)]
    struct Web {
        child: Option<Child>,
        url: Arc<Mutex<Option<String>>>,
    }

    impl Web {
        fn open(&mut self) {
            let alive = matches!(self.child.as_mut().map(|c| c.try_wait()), Some(Ok(None)));
            if alive {
                if let Some(url) = self.url.lock().expect("lock").clone() {
                    open_url(&url);
                }
                return;
            }
            *self.url.lock().expect("lock") = None;
            let mut cmd = command(&["web", "--no-open"]);
            cmd.stdout(Stdio::piped()).stderr(Stdio::null());
            match cmd.spawn() {
                Ok(mut c) => {
                    let stdout = c.stdout.take().expect("stdout");
                    let url = self.url.clone();
                    thread::spawn(move || {
                        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                            if let Some(u) = line.split_whitespace().find(|w| w.starts_with("http"))
                            {
                                *url.lock().expect("lock") = Some(u.to_owned());
                                open_url(u);
                            }
                        }
                    });
                    self.child = Some(c);
                }
                Err(e) => log::error!("could not start the web UI: {e}"),
            }
        }

        fn stop(&mut self) {
            if let Some(mut c) = self.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }

    fn open_url(url: &str) {
        #[cfg(target_os = "macos")]
        let r = Command::new("open").arg(url).spawn();
        #[cfg(windows)]
        let r = {
            use std::os::windows::process::CommandExt;
            Command::new("cmd")
                .args(["/C", "start", "", url])
                .creation_flags(0x0800_0000)
                .spawn()
        };
        if let Err(e) = r {
            log::warn!("could not open {url}: {e}");
        }
    }

    fn make_icon(on: bool) -> Icon {
        Icon::from_rgba(icon::rgba(on, ICON_SIZE, STYLE), ICON_SIZE, ICON_SIZE).expect("icon")
    }

    /// user interface language from the shared config (`language = "en"`)
    fn language() -> &'static str {
        let config = fs::read_to_string(config_dir().join("config.toml")).unwrap_or_default();
        let english = config.lines().any(|l| {
            let l = l.replace(' ', "");
            l == "language=\"en\"" || l == "language='en'"
        });
        if english { "en" } else { "es" }
    }

    struct Texts {
        external: &'static str,
        sharing: &'static str,
        starting: &'static str,
        off: &'static str,
        turn_off: &'static str,
        turn_on: &'static str,
        displays: &'static str,
        quit: &'static str,
    }

    fn texts(lang: &str) -> Texts {
        match lang {
            "en" => Texts {
                external: "VirtualKvm is already running outside the tray",
                sharing: "Sharing mouse and keyboard",
                starting: "Starting…",
                off: "Off",
                turn_off: "Turn off",
                turn_on: "Turn on",
                displays: "Displays & options…",
                quit: "Quit",
            },
            _ => Texts {
                external: "VirtualKvm ya corre fuera de la bandeja",
                sharing: "Compartiendo mouse y teclado",
                starting: "Iniciando…",
                off: "Apagado",
                turn_off: "Apagar",
                turn_on: "Prender",
                displays: "Monitores y opciones…",
                quit: "Salir",
            },
        }
    }

    struct Ui {
        tray: TrayIcon,
        status: MenuItem,
        toggle: MenuItem,
        monitors: MenuItem,
        quit: MenuItem,
        shown: Option<(bool, &'static str)>,
    }

    impl Ui {
        fn show(&mut self, running: bool, wanted: bool, external: bool) {
            let lang = language();
            let t = texts(lang);
            let key = Some((running, lang));
            let label = if external {
                t.external
            } else if running {
                t.sharing
            } else if wanted {
                t.starting
            } else {
                t.off
            };
            self.status.set_text(label);
            self.toggle
                .set_text(if wanted { t.turn_off } else { t.turn_on });
            self.toggle.set_enabled(!external);
            self.monitors.set_text(t.displays);
            self.quit.set_text(t.quit);
            if self.shown != key {
                #[cfg(target_os = "macos")]
                let _ = self.tray.set_icon_templated(Some(make_icon(running)));
                #[cfg(windows)]
                let _ = self.tray.set_icon(Some(make_icon(running)));
                let _ = self.tray.set_tooltip(Some(format!("VirtualKvm · {label}")));
                self.shown = key;
            }
        }
    }

    pub fn run(args: TrayArgs) -> Result<(), TrayError> {
        if let Some(exe) = args.daemon_exe.clone() {
            let _ = DAEMON_EXE.set(exe);
        }
        let switch = Switch(config_dir().join("vkvm-tray-state"));
        let mut daemon = Daemon {
            child: None,
            log: args.log.unwrap_or_else(|| config_dir().join("daemon.log")),
        };
        let mut web = Web::default();

        #[allow(unused_mut)]
        let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
        #[cfg(target_os = "macos")]
        {
            use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
            event_loop.set_activation_policy(ActivationPolicy::Accessory);
        }
        let proxy = event_loop.create_proxy();
        MenuEvent::set_event_handler(Some(move |e| {
            let _ = proxy.send_event(UserEvent::Menu(e));
        }));

        let status = MenuItem::new("…", false, None);
        let toggle = MenuItem::new("Apagar", true, None);
        let monitors = MenuItem::new("Configurar monitores…", true, None);
        let quit = MenuItem::new("Salir", true, None);
        let menu = Menu::new();
        menu.append_items(&[
            &status,
            &PredefinedMenuItem::separator(),
            &toggle,
            &monitors,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .map_err(|e| TrayError::Tray(e.to_string()))?;

        let mut ui: Option<Ui> = None;
        let mut external = false;

        event_loop.run(move |event, _, control_flow| {
            *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_secs(2));
            match event {
                Event::NewEvents(StartCause::Init) => {
                    let builder = TrayIconBuilder::new()
                        .with_menu(Box::new(menu.clone()))
                        .with_tooltip("VirtualKvm");
                    #[cfg(target_os = "macos")]
                    let builder = builder.with_icon_templated(make_icon(false));
                    #[cfg(windows)]
                    let builder = builder.with_icon(make_icon(false));
                    let tray = builder.build();
                    match tray {
                        Ok(tray) => {
                            ui = Some(Ui {
                                tray,
                                status: status.clone(),
                                toggle: toggle.clone(),
                                monitors: monitors.clone(),
                                quit: quit.clone(),
                                shown: None,
                            })
                        }
                        Err(e) => {
                            log::error!("could not create the tray icon: {e}");
                            *control_flow = ControlFlow::Exit;
                            return;
                        }
                    }
                    external = lan_mouse_ipc::is_service_running();
                    if external {
                        log::warn!(
                            "a lan-mouse daemon is already running; not starting another one"
                        );
                    } else if switch.get() {
                        daemon.start();
                    }
                }
                Event::UserEvent(UserEvent::Menu(e)) => {
                    if e.id == toggle.id() {
                        let on = !switch.get();
                        switch.set(on);
                        if on {
                            daemon.start();
                        } else {
                            daemon.stop();
                        }
                    } else if e.id == monitors.id() {
                        web.open();
                    } else if e.id == quit.id() {
                        web.stop();
                        daemon.stop();
                        *control_flow = ControlFlow::Exit;
                        return;
                    }
                }
                _ => {}
            }
            // keep the daemon in the wanted state (restart if it crashed)
            let wanted = switch.get();
            if !external && wanted && !daemon.running() {
                daemon.start();
            }
            if external && !lan_mouse_ipc::is_service_running() {
                external = false;
            }
            let running = external || daemon.running();
            if let Some(ui) = ui.as_mut() {
                ui.show(running, wanted, external);
            }
        });
    }
}
