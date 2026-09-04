//! Native "reveal in file browser" hook, mirrored from go-daemon/oshooks.go.
//! Lives here too because the browser-based frontend's HTTP `/api/reveal`
//! call is served directly by the engine, not proxied through the daemon —
//! the daemon's copy remains for parity if a future native-client path
//! calls IndexService/SearchService directly instead of the HTTP shim.

use anyhow::{bail, Result};
use std::process::Command;

pub fn reveal(path: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg("-R").arg(path).status()?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer.exe").arg(format!("/select,{path}")).status()?;
        return Ok(());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        bail!("reveal-in-file-browser is only supported on macOS and Windows");
    }
}
