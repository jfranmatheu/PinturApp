use crate::PinturappUi;
use eframe::egui;

pub fn run() -> eframe::Result<()> {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Pinturapp - 3D Texture Painter")
            .with_inner_size([1200.0, 800.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "Pinturapp - 3D Texture Painter",
        options,
        Box::new(|cc| Ok(Box::new(PinturappUi::new(cc)))),
    )
}
