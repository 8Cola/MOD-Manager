use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

fn image_result(encoded: &str) -> Result<Value, String> {
    let encoded = encoded.trim();
    if encoded.len() > 28 * 1024 * 1024 {
        return Err("剪贴板图片过大，请选择小于 20 MB 的图片".into());
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| "无法读取剪贴板图片，请重新复制图片")?;
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || &bytes[12..16] != b"IHDR" {
        return Err("剪贴板中没有可用图片，请先复制图片或截图".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > 40_000_000
        || bytes.len() > 20 * 1024 * 1024
    {
        return Err("剪贴板图片过大，请选择小于 20 MB、4000 万像素的图片".into());
    }
    Ok(json!({"url":format!("data:image/png;base64,{encoded}")}))
}

#[cfg(windows)]
pub fn read() -> Result<Value, String> {
    use std::{
        io::Read,
        os::windows::process::CommandExt,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    // STA is required by Windows Clipboard. Nothing is written until crop confirmation.
    let script = r#"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$image=$null; $stream=$null
try {
  if (-not [System.Windows.Forms.Clipboard]::ContainsImage()) { exit 10 }
  $image=[System.Windows.Forms.Clipboard]::GetImage()
  if ($null -eq $image) { exit 10 }
  if ([long]$image.Width * [long]$image.Height -gt 40000000) { exit 12 }
  $stream=New-Object System.IO.MemoryStream
  $image.Save($stream,[System.Drawing.Imaging.ImageFormat]::Png)
  if ($stream.Length -gt 20971520) { exit 12 }
  [Console]::Out.Write([Convert]::ToBase64String($stream.ToArray()))
} catch { exit 11 } finally {
  if ($null -ne $image) { $image.Dispose() }
  if ($null -ne $stream) { $stream.Dispose() }
}
"#;
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Sta", "-Command", script])
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "无法读取系统剪贴板，请尝试选择图片文件")?;
    let stdout = child.stdout.take().ok_or("无法读取剪贴板图片")?;
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        stdout
            .take(28 * 1024 * 1024 + 1)
            .read_to_end(&mut output)
            .map(|_| output)
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < Duration::from_secs(8) => {
                std::thread::sleep(Duration::from_millis(25))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err("剪贴板读取超时，请重新复制图片后再试".into());
            }
        }
    };
    let output = reader
        .join()
        .map_err(|_| "剪贴板读取失败")?
        .map_err(|_| "剪贴板读取失败")?;
    match status.code() {
        Some(0) => image_result(std::str::from_utf8(&output).map_err(|_| "剪贴板图片格式异常")?),
        Some(10) => Err("剪贴板中没有图片，请先复制图片或截图".into()),
        Some(12) => Err("剪贴板图片过大，请选择小于 20 MB、4000 万像素的图片".into()),
        _ => Err("暂时无法读取剪贴板，请重新复制图片后再试".into()),
    }
}
#[cfg(not(windows))]
pub fn read() -> Result<Value, String> {
    Err("此平台暂不支持剪贴板图片，请选择图片文件".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_empty_and_text() {
        assert!(image_result("").is_err());
        assert!(image_result(&STANDARD.encode(b"plain text")).is_err());
    }
    #[test]
    fn accepts_actual_png() {
        let result =
            image_result(&STANDARD.encode(include_bytes!("../tests/fixtures/crop512.png")))
                .unwrap();
        assert!(result["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }
    #[test]
    fn rejects_invalid_dimensions() {
        let mut png = include_bytes!("../tests/fixtures/crop512.png").to_vec();
        png[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(image_result(&STANDARD.encode(png)).is_err());
    }
}
