use anyhow::{Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

/// Copy text to system clipboard with Linux fallback
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    // Try wl-copy first (Wayland)
    if try_wl_copy(text).is_ok() {
        return Ok(());
    }

    // Try xclip (X11)
    if try_xclip(text).is_ok() {
        return Ok(());
    }

    // Try xsel (X11 alternative)
    if try_xsel(text).is_ok() {
        return Ok(());
    }

    // Fallback to arboard
    try_arboard(text)
}

/// Copy using wl-copy (Wayland)
fn try_wl_copy(text: &str) -> Result<()> {
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("wl-copy not available")?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(text.as_bytes())?;
    }

    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("wl-copy failed")
    }
}

/// Copy using xclip (X11)
fn try_xclip(text: &str) -> Result<()> {
    let mut child = Command::new("xclip")
        .arg("-selection")
        .arg("clipboard")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("xclip not available")?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(text.as_bytes())?;
    }

    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("xclip failed")
    }
}

/// Copy using xsel (X11 alternative)
fn try_xsel(text: &str) -> Result<()> {
    let mut child = Command::new("xsel")
        .arg("--clipboard")
        .arg("--input")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("xsel not available")?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(text.as_bytes())?;
    }

    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("xsel failed")
    }
}

/// Copy using arboard (cross-platform, but may not persist on Linux)
fn try_arboard(text: &str) -> Result<()> {
    use arboard::Clipboard;
    let mut clipboard = Clipboard::new().context("Failed to access clipboard")?;
    clipboard
        .set_text(text.to_string())
        .context("Failed to copy to clipboard")?;
    Ok(())
}

/// Get text from system clipboard
#[allow(dead_code)]
pub fn get_from_clipboard() -> Result<String> {
    // Try wl-paste first (Wayland)
    if let Ok(text) = try_wl_paste() {
        return Ok(text);
    }

    // Try xclip (X11)
    if let Ok(text) = try_xclip_paste() {
        return Ok(text);
    }

    // Fallback to arboard
    use arboard::Clipboard;
    let mut clipboard = Clipboard::new().context("Failed to access clipboard")?;
    clipboard.get_text().context("Failed to read clipboard")
}

/// Paste using wl-paste (Wayland)
fn try_wl_paste() -> Result<String> {
    let output = Command::new("wl-paste")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .context("wl-paste not available")?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        anyhow::bail!("wl-paste failed")
    }
}

/// Paste using xclip (X11)
fn try_xclip_paste() -> Result<String> {
    let output = Command::new("xclip")
        .arg("-selection")
        .arg("clipboard")
        .arg("-o")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .context("xclip not available")?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        anyhow::bail!("xclip failed")
    }
}
