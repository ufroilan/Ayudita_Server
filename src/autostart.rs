//! Arrancar con el sistema (al iniciar sesión en el ordenador), minimizada en la bandeja.

use auto_launch::{AutoLaunchBuilder, LinuxLaunchMode, MacOSLaunchMode, WindowsEnableMode};

pub const HIDDEN_ARG: &str = "--minimizado";

pub fn apply(enabled: bool) -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .display()
        .to_string();
    // En desarrollo no se registra nada: compilación debug, datos aislados o binario de cargo.
    let in_cargo_target = exe.contains("/target/") || exe.contains("\\target\\");
    if cfg!(debug_assertions) || std::env::var_os("AYUDITA_DATOS").is_some() || in_cargo_target {
        log::info!("(desarrollo) arrancar con el sistema = {enabled}; no se registra");
        return Ok(());
    }
    let launcher = AutoLaunchBuilder::new()
        .set_app_name("Ayudita Comandas")
        .set_app_path(&exe)
        .set_args(&[HIDDEN_ARG])
        .set_macos_launch_mode(MacOSLaunchMode::LaunchAgent)
        .set_windows_enable_mode(WindowsEnableMode::CurrentUser)
        .set_linux_launch_mode(LinuxLaunchMode::XdgAutostart)
        .build()
        .map_err(|e| e.to_string())?;
    let current = launcher.is_enabled().unwrap_or(false);
    match (enabled, current) {
        (true, false) => launcher.enable().map_err(|e| e.to_string()),
        (false, true) => launcher.disable().map_err(|e| e.to_string()),
        _ => Ok(()),
    }
}
