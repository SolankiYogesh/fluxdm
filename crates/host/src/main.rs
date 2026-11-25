//! `fluxdm-host`: the browser's native messaging helper, its installer, and a
//! small command line downloader.

mod install;

use anyhow::{bail, Context, Result};
use fluxdm_core::ipc::{self, BrowserItem, Command, Reply};
use fluxdm_core::manager::default_data_dir;
use fluxdm_core::{DownloadRequest, Manager, Settings, Status};
use std::io::{Read, Write};
use std::time::Duration;

const MAX_MESSAGE: u32 = 4 * 1024 * 1024;

const HELP: &str = "fluxdm-host
  install [--extension-id ID]   register the browser helper
  uninstall                     remove the registration
  add URL...                    send addresses to the running app
  get URL [options]             download without the app
      -o FOLDER                 destination (default: current folder)
      -n NUMBER                 connections, 1-32 (default 16)
      -l KBPS                   speed limit in KiB/s
      -c TYPE:HEX               verify sha256, sha1 or md5";

fn read_message(input: &mut impl Read) -> Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_ne_bytes(len);
    if len > MAX_MESSAGE {
        bail!("message of {len} bytes is too large");
    }
    let mut data = vec![0; len as usize];
    input.read_exact(&mut data)?;
    Ok(Some(data))
}

fn write_message(output: &mut impl Write, reply: &Reply) -> Result<()> {
    let data = serde_json::to_vec(reply)?;
    output.write_all(&(data.len() as u32).to_ne_bytes())?;
    output.write_all(&data)?;
    output.flush()?;
    Ok(())
}

fn launch_app() -> Result<()> {
    let name = if cfg!(windows) { "fluxdm.exe" } else { "fluxdm" };
    let app = std::env::current_exe()?.with_file_name(name);
    // The helper's stdio belongs to the browser and must not be inherited.
    std::process::Command::new(&app)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("start {}", app.display()))?;
    Ok(())
}

/// Deliver to the app, starting it first if necessary.
fn deliver(command: Command) -> Result<Reply> {
    let dir = default_data_dir();
    if ipc::request(&dir, Command::Ping).is_err() {
        launch_app()?;
        let mut ready = false;
        for _ in 0..60 {
            std::thread::sleep(Duration::from_millis(250));
            if ipc::request(&dir, Command::Ping).is_ok() {
                ready = true;
                break;
            }
        }
        if !ready {
            bail!("FluxDM did not start");
        }
    }
    ipc::request(&dir, command)
}

fn native_messaging() -> Result<()> {
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    while let Some(data) = read_message(&mut input)? {
        let result = serde_json::from_slice::<Command>(&data)
            .context("unrecognised message")
            .and_then(|command| match command {
                // A ping reports on the app without launching it.
                Command::Ping => ipc::request(&default_data_dir(), Command::Ping),
                add => deliver(add),
            });
        let reply = result.unwrap_or_else(|e| Reply { error: Some(format!("{e:#}")), ..Default::default() });
        write_message(&mut output, &reply)?;
    }
    Ok(())
}

fn human(bytes: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let (mut value, mut unit) = (bytes as f64, 0);
    while value >= 1024.0 && unit + 1 < units.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", units[unit])
}

fn get(args: &[String]) -> Result<()> {
    let mut url = None;
    let mut settings = Settings {
        download_dir: std::env::current_dir()?,
        category_folders: false,
        ..Settings::default()
    };
    let mut checksum = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "-o" => settings.download_dir = std::path::absolute(value()?)?,
            "-n" => settings.max_connections = value()?.parse()?,
            "-l" => settings.speed_limit = value()?.parse::<u64>()?.saturating_mul(1024),
            "-c" => checksum = Some(value()?.clone()),
            other if url.is_none() => url = Some(other.to_string()),
            other => bail!("unexpected argument {other}"),
        }
    }
    let url = url.context("which address?")?;
    // A private, temporary list so the app's downloads are left alone.
    let state = std::env::temp_dir().join(format!("fluxdm-cli-{}", std::process::id()));
    let runtime = tokio::runtime::Runtime::new()?;
    let manager = Manager::open(state.clone(), runtime.handle().clone())?;
    manager.update_settings(settings)?;
    let id = manager.add(DownloadRequest { url, checksum, ..Default::default() }, true)?;
    let outcome = loop {
        manager.tick();
        let view = manager.list().into_iter().find(|v| v.task.id == id).context("task vanished")?;
        let task = view.task;
        let percent = task.fraction().map_or_else(String::new, |f| format!("{:5.1}%  ", f * 100.0));
        eprint!(
            "\r{percent}{}  {}/s  connections {}      ",
            human(task.downloaded()),
            human(view.speed as u64),
            task.segments.active_count()
        );
        match task.status {
            // Let the job finish its last save before the state is removed.
            Status::Completed if !manager.is_idle(id) => std::thread::sleep(Duration::from_millis(20)),
            Status::Completed => {
                eprintln!();
                if let Some(note) = &task.note {
                    eprintln!("{note}");
                }
                println!("{}", task.final_path().context("no path")?.display());
                break Ok(());
            }
            Status::Failed => {
                eprintln!();
                manager.remove(id, false);
                std::thread::sleep(Duration::from_millis(300));
                break Err(anyhow::anyhow!(task.error.unwrap_or_default()));
            }
            _ => std::thread::sleep(Duration::from_millis(250)),
        }
    };
    let _ = std::fs::remove_dir_all(state);
    outcome
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let first = args.first().map(String::as_str).unwrap_or("");
    // Chrome passes the calling extension's origin; Windows adds a window handle.
    if first.starts_with("chrome-extension://") {
        return native_messaging();
    }
    match first {
        "install" => {
            let id = match args.get(1).map(String::as_str) {
                Some("--extension-id") => args.get(2).context("missing extension id")?.as_str(),
                None => install::EXTENSION_ID,
                Some(other) => bail!("unexpected argument {other}"),
            };
            install::install(id)
        }
        "uninstall" => install::uninstall(),
        "add" if args.len() > 1 => {
            let items = args[1..]
                .iter()
                .map(|u| BrowserItem { url: u.clone(), ..Default::default() })
                .collect();
            let reply = deliver(Command::Add { items, confirmed: false })?;
            if !reply.ok {
                bail!(reply.error.unwrap_or_else(|| "rejected".into()));
            }
            println!("sent {} download(s)", reply.accepted);
            Ok(())
        }
        "get" => get(&args[1..]),
        _ => {
            println!("{HELP}");
            Ok(())
        }
    }
}
