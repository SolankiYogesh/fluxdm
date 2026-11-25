//! Small filesystem helpers.

use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(unix)]
pub fn write_at(file: &File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, buf, offset)
}

#[cfg(windows)]
pub fn write_at(file: &File, mut buf: &[u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = file.seek_write(buf, offset)?;
        if n == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        buf = &buf[n..];
        offset += n as u64;
    }
    Ok(())
}

/// Replace `path` atomically. The file is private to the current user because it
/// can contain cookies and passwords.
pub fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    f.write_all(data)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

/// Move `from` into `dir` under `name`, never overwriting an existing file.
/// Returns the path actually used.
pub fn move_unique(from: &Path, dir: &Path, name: &str) -> Result<PathBuf> {
    for n in 0..10_000 {
        let target = dir.join(crate::naming::numbered(name, n));
        // Claim the name first so a concurrent download cannot take it.
        match OpenOptions::new().write(true).create_new(true).open(&target) {
            Ok(_) => {
                if let Err(e) = std::fs::rename(from, &target) {
                    let _ = std::fs::remove_file(&target);
                    return Err(e).context("rename finished file");
                }
                return Ok(target);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).context("create destination file"),
        }
    }
    anyhow::bail!("too many files named {name}")
}
