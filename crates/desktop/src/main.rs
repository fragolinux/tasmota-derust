#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Tasmota Derust")
            .with_inner_size(
                if cfg!(feature = "screenshots") && std::env::var_os("DERUST_SCREENSHOTS").is_some()
                {
                    [1280.0, 560.0] // README images: no empty space below the 8 simulated devices
                } else {
                    [1400.0, 820.0]
                },
            )
            .with_min_inner_size([800.0, 400.0])
            .with_icon(icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Tasmota Derust",
        options,
        Box::new(|cc| Ok(Box::new(app::DerustApp::new(cc)))),
    )
}

/// Window and taskbar icon (macOS takes the bundle's .icns, Windows the executable's .ico).
fn icon() -> egui::IconData {
    let png = include_bytes!("../assets/icon-256.png");
    let mut reader = png::Decoder::new(std::io::Cursor::new(&png[..]))
        .read_info()
        .expect("embedded icon");
    let mut rgba = vec![0; reader.output_buffer_size().expect("icon size")];
    let info = reader.next_frame(&mut rgba).expect("embedded icon");
    rgba.truncate(info.buffer_size());
    egui::IconData {
        rgba,
        width: info.width,
        height: info.height,
    }
}
