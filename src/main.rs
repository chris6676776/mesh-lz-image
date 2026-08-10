use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::fs;
use std::time::Instant;
use anyhow::{anyhow, Result};
use image::{DynamicImage, GenericImageView};
use mesh_lz_codec::{codec, web};

mod app;


#[derive(Parser)]
#[command(name = "mesh_lz_codec")]
#[command(about = "Mesh-LZ Image Codec with Interleaved rANS", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Compress a standard image (PNG, JPG, BMP) to the custom .mlz format.
    Compress {
        /// Path to the input image file
        #[arg(short, long)]
        input: PathBuf,

        /// Path to the output compressed file
        #[arg(short, long)]
        output: PathBuf,

        /// Block size to use (8 or 16)
        #[arg(short, long, default_value_t = 8)]
        block_size: usize,

        /// Quality of compression (1-100, where 100 is lossless)
        #[arg(short, long, default_value_t = 100)]
        quality: u8,
        
        /// Use a 256-color palette (NeuQuant) to compress RGB images further losslessly.
        #[arg(short, long, default_value_t = false)]
        palette: bool,

        /// Use YCoCg-R color space for better decorrelation (RGB only).
        #[arg(short = 'y', long, default_value_t = false)]
        ycocg: bool,

        /// Use 4:2:0 chroma subsampling for higher compression (requires ycocg).
        #[arg(short = 's', long, default_value_t = false)]
        subsample: bool,
    },

    /// Decompress a .mlz file back to a standard image.
    Decompress {
        /// Path to the input compressed .mlz file
        #[arg(short, long)]
        input: PathBuf,

        /// Path to the output decompressed image file
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Launch the interactive browser visual viewer & compressor.
    Gui {
        /// Local web server port
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Compress { input, output, block_size, quality, palette, ycocg, subsample }) => {
            if block_size != 8 && block_size != 16 {
                return Err(anyhow!("Block size must be either 8 or 16"));
            }
            if quality < 1 || quality > 100 {
                return Err(anyhow!("Quality must be between 1 and 100"));
            }

            if subsample && !ycocg {
                return Err(anyhow!("--subsample requires --ycocg to be enabled"));
            }
            if palette && (ycocg || subsample) {
                return Err(anyhow!("--palette cannot be used together with --ycocg or --subsample"));
            }

            println!("Loading input image: {:?}", input);
            let img_load_start = Instant::now();
            let img = image::open(&input)
                .map_err(|e| anyhow!("Failed to open input image: {}", e))?;
            println!("Image loaded in {:.2?}", img_load_start.elapsed());

            let (width, height) = img.dimensions();
            let (channels, raw_data) = match img.color() {
                image::ColorType::L8 | image::ColorType::La8 => {
                    let luma = img.to_luma8();
                    (1u8, luma.into_raw())
                }
                _ => {
                    let rgb = img.to_rgb8();
                    (3u8, rgb.into_raw())
                }
            };

            let color_mode_str = if channels == 1 { "Grayscale" } else { "RGB" };
            let quality_str = if quality >= 100 { "Lossless".to_string() } else { format!("Lossy ({}%)", quality) };
            println!(
                "Compressing image ({}x{}, color: {}, block size: {}, quality: {}, palette: {})...",
                width, height, color_mode_str, block_size, quality_str, palette
            );

            let comp_start = Instant::now();
            let compressed_bytes = codec::compress(width, height, channels, &raw_data, block_size, quality, palette, ycocg, subsample)?;
            let comp_duration = comp_start.elapsed();

            let orig_size = raw_data.len();
            let comp_size = compressed_bytes.len();
            let bpp = (comp_size as f64 * 8.0) / (width * height) as f64;
            let ratio = (orig_size as f64) / (comp_size as f64);

            fs::write(&output, compressed_bytes)
                .map_err(|e| anyhow!("Failed to write compressed file: {}", e))?;

            println!("\nCompression completed in {:.2?}", comp_duration);
            println!("Original size:    {:>10} bytes", orig_size);
            println!("Compressed size:  {:>10} bytes", comp_size);
            println!("Compression ratio: {:>10.2}x", ratio);
            println!("Bits per pixel:    {:>10.3} bpp", bpp);
            println!("Saved compressed payload to: {:?}", output);
        }

        Some(Commands::Decompress { input, output }) => {
            println!("Loading compressed payload: {:?}", input);
            let bitstream = fs::read(&input)
                .map_err(|e| anyhow!("Failed to read compressed file: {}", e))?;

            println!("Decompressing image...");
            let decomp_start = Instant::now();
            let (width, height, channels, decoded_data) = codec::decompress(&bitstream)?;
            let decomp_duration = decomp_start.elapsed();

            println!("Decompression completed in {:.2?}", decomp_duration);
            println!("Reconstructed size: {}x{} (channels: {})", width, height, channels);

            // Reconstruct the DynamicImage
            let dyn_img = if channels == 1 {
                let img_buf = image::ImageBuffer::<image::Luma<u8>, _>::from_raw(width, height, decoded_data)
                    .ok_or_else(|| anyhow!("Failed to create image buffer"))?;
                DynamicImage::ImageLuma8(img_buf)
            } else {
                let img_buf = image::ImageBuffer::<image::Rgb<u8>, _>::from_raw(width, height, decoded_data)
                    .ok_or_else(|| anyhow!("Failed to create image buffer"))?;
                DynamicImage::ImageRgb8(img_buf)
            };

            println!("Saving decompressed image to: {:?}", output);
            dyn_img.save(&output)
                .map_err(|e| anyhow!("Failed to save output image: {}", e))?;
            println!("Image saved successfully!");
        }

        Some(Commands::Gui { port }) => {
            web::start_server(port)?;
        }
        
        None => {
            // Launch Native GUI if no subcommand is provided
            let options = eframe::NativeOptions {
                viewport: eframe::egui::ViewportBuilder::default()
                    .with_inner_size([800.0, 600.0]),
                ..Default::default()
            };
            eframe::run_native(
                "MLZ Codec Studio",
                options,
                Box::new(|_cc| Box::new(app::MlzApp::default()) as Box<dyn eframe::App>),
            ).map_err(|e| anyhow!("Failed to run native GUI: {}", e))?;
        }
    }

    Ok(())
}
