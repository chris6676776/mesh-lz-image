use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use image::{codecs::png::{PngEncoder, PngDecoder}, ImageEncoder, ImageDecoder, DynamicImage, ColorType, GenericImageView};
use std::io::Cursor;
use mesh_lz_codec::codec;
use std::path::PathBuf;

fn load_test_image() -> (u32, u32, u8, Vec<u8>) {
    // Try to load a known test image, or generate a gradient if it doesn't exist
    let path = PathBuf::from("bench_tmp/kodim01.png");
    if path.exists() {
        let img = image::open(&path).unwrap();
        let (width, height) = img.dimensions();
        let rgb = img.to_rgb8();
        (width, height, 3, rgb.into_raw())
    } else {
        // Generate a 512x512 synthetic gradient image
        let width = 512;
        let height = 512;
        let mut data = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                data.push((x % 256) as u8);
                data.push((y % 256) as u8);
                data.push(((x + y) % 256) as u8);
            }
        }
        (width, height, 3, data)
    }
}

fn bench_encode(c: &mut Criterion) {
    let (width, height, channels, raw_data) = load_test_image();
    let mut group = c.benchmark_group("Encode");

    group.bench_function("MLZ Lossless 8x8", |b| {
        b.iter(|| {
            // Encode purely in memory
            let _ = codec::compress(
                width, height, channels, &raw_data,
                8, // block_size
                100, // quality (lossless)
                false, // palette
                false, // ycocg
                false, // subsample
            ).unwrap();
        });
    });

    group.bench_function("PNG", |b| {
        b.iter(|| {
            let mut png_bytes = Vec::new();
            let encoder = PngEncoder::new(&mut png_bytes);
            encoder.write_image(&raw_data, width, height, ColorType::Rgb8).unwrap();
        });
    });

    group.finish();
}

fn bench_decode(c: &mut Criterion) {
    let (width, height, channels, raw_data) = load_test_image();
    
    // Pre-encode MLZ
    let mlz_bytes = codec::compress(
        width, height, channels, &raw_data,
        8, 100, false, false, false
    ).unwrap();

    // Pre-encode PNG
    let mut png_bytes = Vec::new();
    let encoder = PngEncoder::new(&mut png_bytes);
    encoder.write_image(&raw_data, width, height, ColorType::Rgb8).unwrap();

    let mut group = c.benchmark_group("Decode");

    group.bench_function("MLZ Lossless 8x8", |b| {
        b.iter(|| {
            // Decode purely from memory buffer
            let _ = codec::decompress(&mlz_bytes).unwrap();
        });
    });

    group.bench_function("PNG", |b| {
        b.iter(|| {
            let cursor = Cursor::new(&png_bytes);
            let mut decoder = PngDecoder::new(cursor).unwrap();
            let mut decoded_pixels = vec![0; decoder.total_bytes() as usize];
            decoder.read_image(&mut decoded_pixels).unwrap();
        });
    });

    group.finish();
}

criterion_group!(benches, bench_encode, bench_decode);
criterion_main!(benches);
