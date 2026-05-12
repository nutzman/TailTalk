use std::path::PathBuf;

#[cfg(any(feature = "ethertalk", feature = "tashtalk"))]
use std::rc::Rc;

#[cfg(any(feature = "ethertalk", feature = "tashtalk"))]
use slint::SharedString;
use serde::{Deserialize, Serialize};
use tailtalk::ShutdownHandle;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{filter::LevelFilter, prelude::*};

slint::include_modules!();

// ── Persisted user configuration ──────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct AppConfig {
    server_name: Option<String>,
    volume_name: Option<String>,
    volume_path: Option<String>,
    // Stored as the port path string so it survives device re-enumeration.
    tashtalk_port: Option<String>,
    ethernet_interface: Option<String>,
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("TailTalk").join("config.toml"))
}

fn load_config() -> AppConfig {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_config(config: &AppConfig) {
    let Some(path) = config_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = toml::to_string(config) {
        let _ = std::fs::write(path, s);
    }
}

// ── Commands sent from the UI thread to the tokio server task ─────────────────

enum ServerCommand {
    Start {
        server_name: String,
        volume_name: String,
        #[cfg(feature = "ethertalk")]
        ethernet: Option<String>,
        #[cfg(feature = "tashtalk")]
        tashtalk: Option<String>,
        volume: PathBuf,
    },
    Stop,
}

// ── Interface enumeration ─────────────────────────────────────────────────────

#[cfg(feature = "ethertalk")]
fn enumerate_ethernet() -> Vec<String> {
    let mut names = vec!["None".to_string()];
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        let mut seen = std::collections::HashSet::new();
        for iface in ifaces {
            if !iface.is_loopback() && seen.insert(iface.name.clone()) {
                names.push(iface.name);
            }
        }
    }
    names
}

#[cfg(feature = "tashtalk")]
const TASHTALK_USB_VID: u16 = 0x10c4;
#[cfg(feature = "tashtalk")]
const TASHTALK_USB_PID: u16 = 0xea60;

#[cfg(feature = "tashtalk")]
struct SerialDevice {
    label: String,
    path: String,
}

#[cfg(feature = "tashtalk")]
fn enumerate_serial() -> Vec<SerialDevice> {
    let mut devices = Vec::new();
    if let Ok(available) = serialport::available_ports() {
        for p in available {
            #[cfg(target_os = "macos")]
            if p.port_name.starts_with("/dev/tty.") {
                continue;
            }
            if let serialport::SerialPortType::UsbPort(ref info) = p.port_type
                && info.vid == TASHTALK_USB_VID
                && info.pid == TASHTALK_USB_PID
            {
                let product = info.product.as_deref().unwrap_or("TashTalk USB");
                devices.push(SerialDevice {
                    label: format!("{} - {}", product, p.port_name),
                    path: p.port_name,
                });
            }
        }
    }
    devices
}

// ── Logging ───────────────────────────────────────────────────────────────────

/// Set up tracing to write to both stdout and a rolling daily log file.
/// Returns a guard that must be kept alive for the duration of the process;
/// dropping it flushes and closes the file writer.
fn init_logging() -> Option<WorkerGuard> {
    #[cfg(target_os = "macos")]
    {
        let log_dir = dirs::home_dir()
            .map(|h| h.join("Library/Logs/TailTalk"))
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/TailTalk"));
        let _ = std::fs::create_dir_all(&log_dir);

        let file_appender = tracing_appender::rolling::daily(&log_dir, "tailtalk.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

        tracing_subscriber::registry()
            .with(LevelFilter::INFO)
            .with(tracing_subscriber::fmt::layer()) // stdout
            .with(tracing_subscriber::fmt::layer().with_writer(non_blocking)) // file
            .init();

        return Some(guard);
    }

    #[allow(unreachable_code)]
    {
        tracing_subscriber::registry()
            .with(LevelFilter::INFO)
            .with(tracing_subscriber::fmt::layer())
            .init();
        None
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<ServerCommand>(4);

    let _log_guard = init_logging();

    let ui = AppWindow::new()?;

    // Inform the UI which transport sections to show
    ui.set_feature_ethertalk(cfg!(feature = "ethertalk"));
    ui.set_feature_tashtalk(cfg!(feature = "tashtalk"));

    #[cfg(feature = "ethertalk")]
    let ethernet_names = enumerate_ethernet();
    #[cfg(feature = "tashtalk")]
    let tashtalk_devices = enumerate_serial();

    #[cfg(feature = "ethertalk")]
    {
        let eth_model: slint::ModelRc<SharedString> = Rc::new(slint::VecModel::from(
            ethernet_names
                .iter()
                .map(|s| SharedString::from(s.as_str()))
                .collect::<Vec<_>>(),
        ))
        .into();
        ui.set_ethernet_interfaces(eth_model);
    }

    #[cfg(feature = "tashtalk")]
    {
        let tash_model: slint::ModelRc<SharedString> = Rc::new(slint::VecModel::from(
            std::iter::once(SharedString::from("None"))
                .chain(
                    tashtalk_devices
                        .iter()
                        .map(|d| SharedString::from(d.label.as_str())),
                )
                .collect::<Vec<_>>(),
        ))
        .into();
        ui.set_tashtalk_ports(tash_model);
    }

    // Restore previously saved settings
    {
        let config = load_config();
        if let Some(ref v) = config.server_name {
            ui.set_server_name(v.as_str().into());
        }
        if let Some(ref v) = config.volume_name {
            ui.set_volume_name(v.as_str().into());
        }
        if let Some(ref v) = config.volume_path {
            ui.set_volume_path(v.as_str().into());
        }
        #[cfg(feature = "ethertalk")]
        if let Some(ref iface) = config.ethernet_interface {
            if let Some(idx) = ethernet_names.iter().position(|n| n == iface) {
                ui.set_selected_ethernet(idx as i32);
            }
        }
        #[cfg(feature = "tashtalk")]
        if let Some(ref port) = config.tashtalk_port {
            // index 0 is "None"; device entries start at 1
            if let Some(idx) = tashtalk_devices.iter().position(|d| &d.path == port) {
                ui.set_selected_tashtalk((idx + 1) as i32);
            }
        }
    }

    let ui_handle = ui.as_weak();
    std::thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(server_loop(cmd_rx, ui_handle));
    });

    let ui_weak = ui.as_weak();
    #[cfg(feature = "ethertalk")]
    let eth_names = ethernet_names.clone();
    #[cfg(feature = "tashtalk")]
    let tash_devices = tashtalk_devices;

    ui.on_start_stop(move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        if ui.get_running() {
            let _ = cmd_tx.try_send(ServerCommand::Stop);
        } else {
            #[cfg(feature = "ethertalk")]
            let ethernet = {
                let eth_idx = ui.get_selected_ethernet() as usize;
                eth_names
                    .get(eth_idx)
                    .filter(|s| s.as_str() != "None")
                    .cloned()
            };

            #[cfg(feature = "tashtalk")]
            let tashtalk = {
                let tash_idx = ui.get_selected_tashtalk() as usize;
                // index 0 is "None"; devices start at 1
                tash_idx
                    .checked_sub(1)
                    .and_then(|i| tash_devices.get(i))
                    .map(|d| d.path.clone())
            };

            #[allow(unused_mut)]
            let mut any_transport = false;
            #[cfg(feature = "ethertalk")]
            if ethernet.is_some() {
                any_transport = true;
            }
            #[cfg(feature = "tashtalk")]
            if tashtalk.is_some() {
                any_transport = true;
            }
            if !any_transport {
                tracing::error!("At least one transport must be selected");
                return;
            }

            let volume = PathBuf::from(ui.get_volume_path().as_str());
            if volume.as_os_str().is_empty() {
                tracing::error!("No volume path selected");
                return;
            }

            save_config(&AppConfig {
                server_name: Some(ui.get_server_name().to_string()),
                volume_name: Some(ui.get_volume_name().to_string()),
                volume_path: Some(ui.get_volume_path().to_string()),
                tashtalk_port: {
                    #[cfg(feature = "tashtalk")]
                    { tashtalk.clone() }
                    #[cfg(not(feature = "tashtalk"))]
                    { None }
                },
                ethernet_interface: {
                    #[cfg(feature = "ethertalk")]
                    { ethernet.clone() }
                    #[cfg(not(feature = "ethertalk"))]
                    { None }
                },
            });

            let _ = cmd_tx.try_send(ServerCommand::Start {
                server_name: ui.get_server_name().to_string(),
                volume_name: ui.get_volume_name().to_string(),
                #[cfg(feature = "ethertalk")]
                ethernet,
                #[cfg(feature = "tashtalk")]
                tashtalk,
                volume,
            });
        }
    });

    let ui_weak = ui.as_weak();
    ui.on_browse_volume(move || {
        let ui_weak = ui_weak.clone();
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                let path_str: slint::SharedString = path.to_string_lossy().into_owned().into();
                slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_volume_path(path_str);
                    }
                })
                .ok();
            }
        });
    });

    ui.run()?;
    Ok(())
}

// ── Server loop (runs on the background tokio thread) ─────────────────────────

async fn server_loop(
    mut cmd_rx: tokio::sync::mpsc::Receiver<ServerCommand>,
    ui_weak: slint::Weak<AppWindow>,
) {
    let mut shutdown_handle: Option<ShutdownHandle> = None;

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            ServerCommand::Start {
                server_name,
                volume_name,
                #[cfg(feature = "ethertalk")]
                ethernet,
                #[cfg(feature = "tashtalk")]
                tashtalk,
                volume,
            } => {
                if let Some(h) = shutdown_handle.take() {
                    h.shutdown();
                }

                let ui_w = ui_weak.clone();
                let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<ShutdownHandle>();
                tokio::spawn(run_server(
                    server_name,
                    volume_name,
                    #[cfg(feature = "ethertalk")]
                    ethernet,
                    #[cfg(feature = "tashtalk")]
                    tashtalk,
                    volume,
                    ready_tx,
                    ui_w.clone(),
                ));

                // Wait for the stack to finish initialising (AARP probe etc.)
                // before flipping the UI to "Running".
                if let Ok(handle) = ready_rx.await {
                    shutdown_handle = Some(handle);
                    slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_w.upgrade() {
                            ui.set_running(true);
                        }
                    })
                    .ok();
                }
                // If ready_rx errors the stack failed to build; run_server
                // already logged the error and reset the UI.
            }

            ServerCommand::Stop => {
                if let Some(h) = shutdown_handle.take() {
                    h.shutdown();
                }
                let ui_w = ui_weak.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_w.upgrade() {
                        ui.set_running(false);
                    }
                })
                .ok();
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn show_error(ui_weak: slint::Weak<AppWindow>, message: String) {
    slint::invoke_from_event_loop(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_error_message(message.into());
        }
    })
    .ok();
}

// ── AFP server task ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn run_server(
    server_name: String,
    volume_name: String,
    #[cfg(feature = "ethertalk")] ethernet: Option<String>,
    #[cfg(feature = "tashtalk")] tashtalk: Option<String>,
    volume: PathBuf,
    ready_tx: tokio::sync::oneshot::Sender<ShutdownHandle>,
    ui_weak: slint::Weak<AppWindow>,
) {
    use tailtalk::{
        TalkStack,
        afp::{AfpServer, AfpServerConfig},
    };

    let set_stopped = |ui_weak: slint::Weak<AppWindow>| {
        slint::invoke_from_event_loop(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_running(false);
            }
        })
        .ok();
    };

    #[allow(unused_mut)]
    let mut stack_builder = TalkStack::builder();

    #[cfg(feature = "ethertalk")]
    if let Some(ref intf) = ethernet {
        stack_builder = stack_builder.ethernet(intf);
    }

    #[cfg(feature = "tashtalk")]
    if let Some(ref tty) = tashtalk {
        let features = tailtalk::TashTalkFeatures::new()
            .with_crc_calculation()
            .with_crc_checking();
        stack_builder = stack_builder.localtalk(tty).tashtalk_features(features);
    }

    let stack = match stack_builder.build().await {
        Ok(s) => s,
        Err(e) => {
            let is_perm = e.chain().any(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .map(|io| io.kind() == std::io::ErrorKind::PermissionDenied)
                    .unwrap_or(false)
            });

            if is_perm {
                #[cfg(target_os = "linux")]
                {
                    if let Ok(exe) = std::env::current_exe() {
                        tracing::warn!(
                            "Permission denied — requesting CAP_NET_RAW via pkexec setcap..."
                        );
                        let result = tokio::process::Command::new("pkexec")
                            .args(["setcap", "cap_net_raw+eip"])
                            .arg(&exe)
                            .status()
                            .await;

                        match result {
                            Ok(s) if s.success() => {
                                tracing::info!("Capability granted — relaunching...");
                                std::process::Command::new(&exe).spawn().ok();
                                std::process::exit(0);
                            }
                            _ => {
                                let msg = format!(
                                    "Permission denied. Run manually:\nsudo setcap cap_net_raw+eip {}",
                                    exe.display()
                                );
                                tracing::error!("{msg}");
                                show_error(ui_weak.clone(), msg);
                            }
                        }
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let msg = format!("Permission denied: {e}");
                    tracing::error!("{msg}");
                    show_error(ui_weak.clone(), msg);
                }
            } else {
                let msg = format!("Failed to start AppleTalk stack:\n{e}");
                tracing::error!("{msg}");
                show_error(ui_weak.clone(), msg);
            }

            set_stopped(ui_weak);
            return;
        }
    };

    let afp_config = AfpServerConfig {
        volume_path: volume,
        volume_name,
        server_name,
        ..AfpServerConfig::default()
    };

    let _afp = match AfpServer::spawn(&stack.ddp, &stack.nbp, Some(254), afp_config, stack.token()).await {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("Failed to start AFP server:\n{e}");
            tracing::error!("{msg}");
            show_error(ui_weak.clone(), msg);
            set_stopped(ui_weak);
            return;
        }
    };

    #[cfg(any(feature = "ethertalk", feature = "tashtalk"))]
    {
        #[cfg(all(feature = "ethertalk", feature = "tashtalk"))]
        let transport_desc = match (&ethernet, &tashtalk) {
            (Some(eth), Some(tty)) => format!("{eth} + {tty}"),
            (Some(eth), None) => eth.clone(),
            (None, Some(tty)) => tty.clone(),
            (None, None) => unreachable!(),
        };
        #[cfg(all(feature = "ethertalk", not(feature = "tashtalk")))]
        let transport_desc = ethernet.as_deref().unwrap_or("(none)").to_string();
        #[cfg(all(not(feature = "ethertalk"), feature = "tashtalk"))]
        let transport_desc = tashtalk.as_deref().unwrap_or("(none)").to_string();
        tracing::info!("AFP server running on {transport_desc}");
    }

    // Signal the GUI that the stack is up; hand over the shutdown handle.
    let _ = ready_tx.send(stack.shutdown_handle());

    // Block until shutdown() is called (e.g. the user clicks Stop).
    stack.wait_for_shutdown().await;

    set_stopped(ui_weak);
}
