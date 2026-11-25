//! Registers the native messaging host with Chromium-based browsers.

use anyhow::{bail, Context, Result};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

const HOST_NAME: &str = "com.fluxdm.host";
/// Fixed by the public key in the extension's manifest.
pub const EXTENSION_ID: &str = "gookccndiheljhpjlenagbfgklkefjgh";

fn manifest(id: &str) -> Result<Vec<u8>> {
    if id.len() != 32 || !id.bytes().all(|b| (b'a'..=b'p').contains(&b)) {
        bail!("an extension id is 32 letters from a to p");
    }
    Ok(serde_json::to_vec_pretty(&serde_json::json!({
        "name": HOST_NAME,
        "description": "FluxDM browser helper",
        "type": "stdio",
        "path": std::env::current_exe()?,
        "allowed_origins": [format!("chrome-extension://{id}/")],
    }))?)
}

#[cfg(unix)]
fn browser_dirs() -> Result<Vec<(&'static str, PathBuf)>> {
    let list: &[(&str, &str)] = if cfg!(target_os = "macos") {
        &[
            ("Chrome", "Google/Chrome"),
            ("Edge", "Microsoft Edge"),
            ("Brave", "BraveSoftware/Brave-Browser"),
            ("Chromium", "Chromium"),
            ("Vivaldi", "Vivaldi"),
        ]
    } else {
        &[
            ("Chrome", "google-chrome"),
            ("Edge", "microsoft-edge"),
            ("Brave", "BraveSoftware/Brave-Browser"),
            ("Chromium", "chromium"),
            ("Vivaldi", "vivaldi"),
        ]
    };
    let base = dirs::config_dir().context("no configuration folder")?;
    Ok(list.iter().map(|(n, d)| (*n, base.join(d))).collect())
}

fn write(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, data).with_context(|| format!("write {}", path.display()))
}

#[cfg(unix)]
pub fn install(id: &str) -> Result<()> {
    let data = manifest(id)?;
    let mut count = 0;
    for (name, dir) in browser_dirs()? {
        if dir.is_dir() {
            let path = dir.join("NativeMessagingHosts").join(format!("{HOST_NAME}.json"));
            write(&path, &data)?;
            println!("registered for {name}: {}", path.display());
            count += 1;
        }
    }
    if count == 0 {
        bail!("no supported browser profile was found");
    }
    Ok(())
}

#[cfg(unix)]
pub fn uninstall() -> Result<()> {
    for (name, dir) in browser_dirs()? {
        let path = dir.join("NativeMessagingHosts").join(format!("{HOST_NAME}.json"));
        if std::fs::remove_file(path).is_ok() {
            println!("removed from {name}");
        }
    }
    Ok(())
}

#[cfg(windows)]
const REGISTRY_ROOTS: [&str; 2] = [
    r"HKCU\Software\Google\Chrome\NativeMessagingHosts",
    r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts",
];

#[cfg(windows)]
pub fn install(id: &str) -> Result<()> {
    let path = fluxdm_core::manager::default_data_dir().join(format!("{HOST_NAME}.json"));
    write(&path, &manifest(id)?)?;
    for root in REGISTRY_ROOTS {
        let status = std::process::Command::new("reg")
            .args(["add", &format!(r"{root}\{HOST_NAME}"), "/ve", "/t", "REG_SZ", "/d"])
            .arg(&path)
            .arg("/f")
            .status()?;
        if !status.success() {
            bail!("could not write the registry");
        }
    }
    println!("registered for Chrome and Edge");
    Ok(())
}

#[cfg(windows)]
pub fn uninstall() -> Result<()> {
    for root in REGISTRY_ROOTS {
        let _ = std::process::Command::new("reg")
            .args(["delete", &format!(r"{root}\{HOST_NAME}"), "/f"])
            .status();
    }
    Ok(())
}
