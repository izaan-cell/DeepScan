//! Native "reveal in file browser" hook, mirrored from go-daemon/oshooks.go.
//! Lives here too because the browser-based frontend's HTTP `/api/reveal`
//! call is served directly by the engine, not proxied through the daemon —
//! the daemon's copy remains for parity if a future native-client path
//! calls IndexService/SearchService directly instead of the HTTP shim.

use anyhow::Result;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
use anyhow::bail;
use std::process::Command;

pub fn reveal(path: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        // .spawn(), not .status() — status() blocks the calling thread
        // until `open` fully exits, and this runs straight inside an async
        // HTTP handler with no spawn_blocking wrapper, so it was stalling
        // the engine's whole tokio runtime (search, status, everything)
        // for however long Finder took to actually respond, not just the
        // near-instant fork+exec this only needs. We don't care about the
        // exit code — reveal-in-Finder is inherently fire-and-forget.
        Command::new("open").arg("-R").arg(path).spawn()?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer.exe").arg(format!("/select,{path}")).spawn()?;
        return Ok(());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        bail!("reveal-in-file-browser is only supported on macOS and Windows");
    }
}
