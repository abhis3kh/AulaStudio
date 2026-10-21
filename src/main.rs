#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod audio_capture;
mod keyboard_scene;
mod layout;
mod live_rgb;
mod music;
mod music_dsp;
mod profiles;
mod scene;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|argument| argument == "--probe") {
        let result =
            aula_studio::device::Keyboard::connect().and_then(|mut keyboard| keyboard.status());
        match result {
            Ok(status) => println!(
                "READ-ONLY PROBE OK\nProduct: {}\nTransport: {:?}\nLighting: {:?}\nSleep: {:?}",
                status.product, status.transport, status.lighting, status.sleep_seconds
            ),
            Err(error) => {
                eprintln!("READ-ONLY PROBE FAILED: {error}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let window_size = [1440.0, 900.0];
    #[cfg(feature = "capture")]
    let window_size = match std::env::var("AULA_CAPTURE_SIZE").as_deref() {
        Ok("compact") => [1200.0, 780.0],
        Ok("minimum") => [920.0, 650.0],
        _ => window_size,
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size(window_size)
            .with_min_inner_size([920.0, 650.0])
            .with_title("Aula Studio | F75"),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Aula Studio",
        options,
        Box::new(|cc| Ok(Box::new(app::Studio::new(cc)))),
    )?;
    Ok(())
}
