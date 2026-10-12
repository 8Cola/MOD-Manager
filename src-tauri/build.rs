fn main() {
    // Keep the icon embedded in the standalone EXE in sync with the app's sidebar mark.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    let attributes = tauri_build::Attributes::new().windows_attributes(
        tauri_build::WindowsAttributes::new().window_icon_path("icons/icon.ico"),
    );
    tauri_build::try_build(attributes).expect("failed to build Tauri application resources");
}
