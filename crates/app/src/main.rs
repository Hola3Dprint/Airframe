#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod canvas;
mod component_help;
mod inspector;
mod layout;
mod presets;
mod recovery;
mod results;
mod startup;
mod style;
#[cfg(windows)]
mod update;
mod workbench;

fn program_folder() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.ancestors()
                .skip(1)
                .find(|dir| dir.join("crates/app/Cargo.toml").is_file())
                .map(std::path::Path::to_path_buf)
        })
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

fn local_data_folder() -> std::path::PathBuf {
    #[cfg(windows)]
    if let Some(root) = update::installed_root() {
        return root.join("data");
    }
    program_folder().join("projects/local")
}

fn main() -> eframe::Result {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    #[cfg(windows)]
    if first.as_deref() == Some(std::ffi::OsStr::new("--apply-update")) {
        return update::apply(args);
    }
    let mut path = first.map(std::path::PathBuf::from);
    let resume = if path.as_deref() == Some(std::path::Path::new("--resume-update")) {
        path = None;
        args.next().map(std::path::PathBuf::from)
    } else {
        None
    };
    eframe::run_native(
        "Airframe · Ventilation Workbench",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([1480.0, 920.0])
                .with_min_inner_size([1000.0, 670.0]),
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        },
        Box::new(move |cc| {
            let mut workbench = workbench::Workbench::new(&cc.egui_ctx, path);
            if let Some(file) = resume {
                workbench.restore_recovery(&cc.egui_ctx, &file);
            }
            Ok(Box::new(workbench))
        }),
    )
}
