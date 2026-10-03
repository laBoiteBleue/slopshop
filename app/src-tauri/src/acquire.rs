//! File > Import from Device: an image from a scanner or a camera, through Windows' own
//! scanning dialog (WIA), opened as a new untitled document.
//!
//! HACK(no WIA binding): the dialog is driven by a short PowerShell script through WIA's COM
//! automation objects, which saves the image as a PNG in the temporary folder; the engine then
//! opens it like any file. This needs no new dependency and no `unsafe` code (the maintainer's
//! choice, 2026-10-03); a direct binding (the `windows` crate) would avoid starting PowerShell.
//! macOS (ImageCaptureCore) and Linux (SANE) are not supported yet.

use serde::Serialize;
use tauri::AppHandle;

/// How an import ended, for the UI to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AcquireOutcome {
    /// The image is open in a new tab.
    Opened,
    /// The user closed the dialog.
    Cancelled,
    /// No scanner or camera is connected.
    NoDevice,
    /// The image could not be opened; the open's own `open-failed` event said why.
    OpenFailed,
}

/// The script: the image in `$env:SLOPSHOP_ACQUIRE` (PNG); exit code 0, 2 when cancelled, 3
/// without a device. WIA's own dialogs choose the device and the scan settings.
#[cfg(windows)]
const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$png = '{B96B3CAF-0728-11D3-9D7B-0000F81EF32E}'
try {
  $dialog = New-Object -ComObject WIA.CommonDialog
  # Any device type, the default intent and bias, PNG, the device list only when several.
  $image = $dialog.ShowAcquireImage(0, 0, 0, $png, $false, $true, $false)
} catch {
  if ($_.Exception.HResult -eq -2145320939) { exit 3 }  # WIA_S_NO_DEVICE_AVAILABLE
  throw
}
if ($null -eq $image) { exit 2 }
if ($image.FormatID -ne $png) {
  $process = New-Object -ComObject WIA.ImageProcess
  $process.Filters.Add($process.FilterInfos.Item('Convert').FilterID)
  $process.Filters.Item(1).Properties.Item('FormatID').Value = $png
  $image = $process.Apply($image)
}
$image.SaveFile($env:SLOPSHOP_ACQUIRE)
"#;

/// Run the scanning dialog; the image's path when one was saved.
#[cfg(windows)]
fn scan() -> Result<Result<std::path::PathBuf, AcquireOutcome>, String> {
    use std::os::windows::process::CommandExt;
    /// No console window flashes over the app.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let path = std::env::temp_dir().join(format!("slopshop-scan-{stamp}.png"));
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
        ])
        .args(["-Command", SCRIPT])
        .env("SLOPSHOP_ACQUIRE", &path)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("cannot start PowerShell: {e}"))?;
    match output.status.code() {
        Some(0) if path.is_file() => Ok(Ok(path)),
        Some(2) => Ok(Err(AcquireOutcome::Cancelled)),
        Some(3) => Ok(Err(AcquireOutcome::NoDevice)),
        _ => Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
    }
}

/// File > Import from Device: scan (or take from a camera) and open the image in a new tab,
/// untitled. Blocks a worker while Windows' dialog is shown.
#[tauri::command]
pub async fn acquire_image(app: AppHandle) -> Result<AcquireOutcome, String> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let path = match scan()? {
                Ok(path) => path,
                Err(outcome) => return Ok(outcome),
            };
            let opened = crate::open_scanned(&app, &path);
            // Decoded into memory: the temporary file can go.
            let _ = std::fs::remove_file(&path);
            Ok(if opened {
                AcquireOutcome::Opened
            } else {
                AcquireOutcome::OpenFailed
            })
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("importing from a device is only supported on Windows for now".to_owned())
    }
}
