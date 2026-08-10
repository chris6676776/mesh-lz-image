use eframe::egui;
use egui::{ColorImage, TextureHandle};
use std::path::PathBuf;
use std::fs;
use std::time::Instant;
use image::GenericImageView;
use crate::codec;


#[derive(PartialEq)]
enum Tab {
    Encoder,
    Viewer,
}

pub struct MlzApp {
    current_tab: Tab,
    
    // Encoder State
    enc_input_path: Option<PathBuf>,
    enc_output_path: Option<PathBuf>,
    enc_block_size: usize,
    enc_quality: u8,
    enc_palette: bool,
    enc_ycocg: bool,
    enc_subsample: bool,
    enc_status: String,
    
    // Viewer State
    view_input_path: Option<PathBuf>,
    view_texture: Option<TextureHandle>,
    view_zoom: f32,
    view_status: String,
}

impl Default for MlzApp {
    fn default() -> Self {
        Self {
            current_tab: Tab::Encoder,
            enc_input_path: None,
            enc_output_path: None,
            enc_block_size: 8,
            enc_quality: 100,
            enc_palette: false,
            enc_ycocg: false,
            enc_subsample: false,
            enc_status: String::new(),
            view_input_path: None,
            view_texture: None,
            view_zoom: 1.0,
            view_status: String::new(),
        }
    }
}

impl eframe::App for MlzApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.current_tab, Tab::Encoder, "Encoder");
                ui.selectable_value(&mut self.current_tab, Tab::Viewer, "Viewer");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            match self.current_tab {
                Tab::Encoder => self.show_encoder(ui),
                Tab::Viewer => self.show_viewer(ui),
            }
        });
    }
}

impl MlzApp {
    fn show_encoder(&mut self, ui: &mut egui::Ui) {
        ui.heading("MLZ Encoder");
        ui.add_space(10.0);
        
        ui.horizontal(|ui| {
            if ui.button("Select Input Image").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.enc_input_path = Some(path);
                    self.enc_status = String::new();
                }
            }
            if let Some(path) = &self.enc_input_path {
                ui.label(path.display().to_string());
            }
        });

        ui.add_space(10.0);
        ui.label("Block Size:");
        ui.radio_value(&mut self.enc_block_size, 8, "8x8");
        ui.radio_value(&mut self.enc_block_size, 16, "16x16");
        
        ui.add_space(10.0);
        ui.add_enabled(!self.enc_palette, egui::Slider::new(&mut self.enc_quality, 1..=100).text("Quality (100 = Lossless)"));

        ui.add_space(10.0);
        if ui.checkbox(&mut self.enc_palette, "Use 256-color palette (NeuQuant) for lossless RGB").changed() {
            if self.enc_palette {
                self.enc_ycocg = false;
                self.enc_subsample = false;
            }
        }

        ui.add_space(5.0);
        let ycocg_btn = egui::Checkbox::new(&mut self.enc_ycocg, "Use YCoCg-R color space (better for RGB)");
        if ui.add_enabled(!self.enc_palette, ycocg_btn).changed() {
            if !self.enc_ycocg {
                self.enc_subsample = false;
            }
        }
        
        ui.add_space(5.0);
        let sub_btn = egui::Checkbox::new(&mut self.enc_subsample, "Use 4:2:0 Chroma Subsampling (requires YCoCg-R, lossy)");
        if ui.add_enabled(!self.enc_palette, sub_btn).changed() {
            if self.enc_subsample {
                self.enc_ycocg = true;
            }
        }

        ui.add_space(20.0);
        if ui.button("Compress & Save").clicked() {
            if let Some(_input_path) = &self.enc_input_path {
                if let Some(output_path) = rfd::FileDialog::new()
                    .add_filter("MLZ File", &["mlz"])
                    .save_file() 
                {
                    self.enc_output_path = Some(output_path);
                    self.perform_compression();
                }
            } else {
                self.enc_status = "Please select an input image first.".to_string();
            }
        }
        
        ui.add_space(10.0);
        if !self.enc_status.is_empty() {
            ui.label(&self.enc_status);
        }
    }

    fn perform_compression(&mut self) {
        let input = self.enc_input_path.as_ref().unwrap();
        let output = self.enc_output_path.as_ref().unwrap();
        
        self.enc_status = "Loading image...".to_string();
        let img = match image::open(input) {
            Ok(img) => img,
            Err(e) => {
                self.enc_status = format!("Error loading image: {}", e);
                return;
            }
        };

        let (width, height) = img.dimensions();
        let (channels, raw_data) = match img.color() {
            image::ColorType::L8 | image::ColorType::La8 => {
                (1u8, img.to_luma8().into_raw())
            }
            _ => {
                (3u8, img.to_rgb8().into_raw())
            }
        };

        let start = Instant::now();
        match codec::compress(width, height, channels, &raw_data, self.enc_block_size, self.enc_quality, self.enc_palette, self.enc_ycocg, self.enc_subsample) {
            Ok(compressed_bytes) => {
                let duration = start.elapsed();
                let orig_size = raw_data.len();
                let comp_size = compressed_bytes.len();
                let ratio = orig_size as f64 / comp_size as f64;
                
                if let Err(e) = fs::write(output, compressed_bytes) {
                    self.enc_status = format!("Error writing file: {}", e);
                } else {
                    self.enc_status = format!(
                        "Success!\nTime: {:.2?}\nRatio: {:.2}x ({} -> {} bytes)",
                        duration, ratio, orig_size, comp_size
                    );
                }
            }
            Err(e) => {
                self.enc_status = format!("Compression failed: {}", e);
            }
        }
    }

    fn show_viewer(&mut self, ui: &mut egui::Ui) {
        ui.heading("MLZ Viewer");
        ui.horizontal(|ui| {
            if ui.button("Open .mlz File").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("MLZ File", &["mlz"])
                    .pick_file() 
                {
                    self.view_input_path = Some(path);
                    self.load_mlz(ui.ctx());
                }
            }
            if let Some(path) = &self.view_input_path {
                ui.label(path.display().to_string());
            }
        });

        ui.add_space(5.0);
        if !self.view_status.is_empty() {
            ui.label(&self.view_status);
        }
        
        ui.horizontal(|ui| {
            ui.label("Zoom:");
            if ui.button("-").clicked() {
                self.view_zoom = (self.view_zoom / 1.2).max(0.1);
            }
            ui.add(egui::Slider::new(&mut self.view_zoom, 0.1..=10.0));
            if ui.button("+").clicked() {
                self.view_zoom = (self.view_zoom * 1.2).min(10.0);
            }
            if ui.button("1:1").clicked() {
                self.view_zoom = 1.0;
            }
        });
        
        ui.add_space(10.0);

        if let Some(texture) = &self.view_texture {
            let size = egui::vec2(texture.size()[0] as f32 * self.view_zoom, texture.size()[1] as f32 * self.view_zoom);
            egui::ScrollArea::both().show(ui, |ui| {
                ui.add(
                    egui::Image::new(texture)
                        .fit_to_exact_size(size)
                );
            });
        }
    }

    fn load_mlz(&mut self, ctx: &egui::Context) {
        let input = self.view_input_path.as_ref().unwrap();
        let bitstream = match fs::read(input) {
            Ok(b) => b,
            Err(e) => {
                self.view_status = format!("Error reading file: {}", e);
                return;
            }
        };

        let start = Instant::now();
        match codec::decompress(&bitstream) {
            Ok((width, height, channels, data)) => {
                let duration = start.elapsed();
                self.view_status = format!("Loaded {}x{} in {:.2?}", width, height, duration);
                
                // Convert raw data to egui::ColorImage
                let mut pixels = Vec::with_capacity((width * height) as usize);
                if channels == 1 {
                    for &l in &data {
                        pixels.push(egui::Color32::from_gray(l));
                    }
                } else {
                    for chunk in data.chunks(3) {
                        pixels.push(egui::Color32::from_rgb(chunk[0], chunk[1], chunk[2]));
                    }
                }
                let image = ColorImage {
                    size: [width as usize, height as usize],
                    pixels,
                };
                
                self.view_texture = Some(ctx.load_texture("mlz_image", image, Default::default()));
                self.view_zoom = 1.0;
            }
            Err(e) => {
                self.view_status = format!("Error decompressing: {}", e);
            }
        }
    }
}
