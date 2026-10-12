use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// Best effort local Steam cover or Windows EXE icon. Never accesses a network API.
pub fn local_game_image(exe: &Path, output_dir: &Path) -> Option<PathBuf> {
    if !exe.is_file() || exe.extension()?.to_string_lossy().to_lowercase() != "exe" {
        return None;
    }
    for ancestor in exe.ancestors() {
        if ancestor
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("steamapps"))
        {
            if let Ok(entries) = std::fs::read_dir(ancestor) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("appmanifest_"))
                    {
                        continue;
                    }
                    if let Ok(manifest) = std::fs::read_to_string(&path) {
                        let field = |name: &str| {
                            manifest.lines().find_map(|line| {
                                let values: Vec<_> = line.split('"').collect();
                                if values.get(1) == Some(&name) {
                                    values.get(3).map(|s| s.to_string())
                                } else {
                                    None
                                }
                            })
                        };
                        if let (Some(dir), Some(id)) = (field("installdir"), field("appid")) {
                            if !id.chars().all(|c| c.is_ascii_digit()) {
                                continue;
                            }
                            if exe.starts_with(ancestor.join("common").join(dir)) {
                                if let Some(steam) = ancestor.parent() {
                                    for name in [
                                        format!("{id}_icon.jpg"),
                                        format!("{id}_library_600x900.jpg"),
                                        format!("{id}_header.jpg"),
                                    ] {
                                        let image = steam.join("appcache/librarycache").join(name);
                                        if image.is_file() {
                                            return Some(image);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::fs::create_dir_all(output_dir).ok()?;
        let output = output_dir.join(format!("exe-{}.png", uuid::Uuid::new_v4()));
        // Paths travel in environment values, not PowerShell source interpolation.
        let code="Add-Type -AssemblyName System.Drawing; $icon=[System.Drawing.Icon]::ExtractAssociatedIcon($env:MM_ICON_EXE); if($null -eq $icon){exit 1}; $bitmap=$icon.ToBitmap(); try {$bitmap.Save($env:MM_ICON_OUTPUT,[System.Drawing.Imaging.ImageFormat]::Png)} finally {$bitmap.Dispose(); $icon.Dispose()}";
        let result = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", code])
            .env("MM_ICON_EXE", exe)
            .env("MM_ICON_OUTPUT", &output)
            .creation_flags(0x08000000)
            .output()
            .ok()?;
        if result.status.success() && output.is_file() {
            return Some(output);
        }
    }
    None
}
