use criterion::{criterion_group, criterion_main, Criterion};
use image::{codecs::png::{PngEncoder, PngDecoder}, codecs::jpeg::{JpegEncoder, JpegDecoder}, ImageEncoder, ImageDecoder, ColorType};
use std::io::Cursor;
use mesh_lz_codec::codec;
use std::path::PathBuf;
use webp::{Encoder as WebPEncoder, Decoder as WebPDecoder};

fn load_test_image() -> (u32, u32, u8, Vec<u8>) {
    let path = PathBuf::from("bench_tmp/kodim01.png");
    if path.exists() {
        let img = image::open(&path).unwrap();
        let width = img.width();
        let height = img.height();
        let rgb = img.to_rgb8();
        (width, height, 3, rgb.into_raw())
    } else {
        (512, 512, 3, vec![0; 512 * 512 * 3])
    }
}

fn bench_all(c: &mut Criterion) {
    let (width, height, channels, raw_data) = load_test_image();

    // Helper macro to benchmark MLZ
    macro_rules! bench_mlz {
        ($group:expr, $name:expr, $bs:expr, $q:expr, $pal:expr, $ycocg:expr, $sub:expr) => {
            {
                let enc_name = format!("Encode {}", $group);
                let dec_name = format!("Decode {}", $group);
                
                let mut enc_g = c.benchmark_group(&enc_name);
                enc_g.sample_size(10); // keep it fast
                enc_g.bench_function($name, |b| {
                    b.iter(|| {
                        let _ = codec::compress(width, height, channels, &raw_data, $bs, $q, $pal, $ycocg, $sub).unwrap();
                    });
                });
                enc_g.finish();

                let compressed = codec::compress(width, height, channels, &raw_data, $bs, $q, $pal, $ycocg, $sub).unwrap();
                let mut dec_g = c.benchmark_group(&dec_name);
                dec_g.sample_size(10);
                dec_g.bench_function($name, |b| {
                    b.iter(|| {
                        let _ = codec::decompress(&compressed).unwrap();
                    });
                });
                dec_g.finish();
            }
        };
    }

    // Lossless
    bench_mlz!("Lossless", "MLZ 8x8", 8, 100, false, false, false);
    bench_mlz!("Lossless", "MLZ 16x16", 16, 100, false, false, false);
    
    // Palette
    bench_mlz!("Palette", "MLZ 8x8", 8, 100, true, false, false);
    bench_mlz!("Palette", "MLZ 16x16", 16, 100, true, false, false);

    // Lossy variants
    let qs = [10, 30, 50, 70, 90];
    for &q in &qs {
        let name = format!("q={}", q);
        bench_mlz!("Lossy RGB 8x8", &name, 8, q, false, false, false);
        bench_mlz!("Lossy RGB 16x16", &name, 16, q, false, false, false);
        bench_mlz!("Lossy YCoCg 8x8", &name, 8, q, false, true, false);
        bench_mlz!("Lossy YCoCg 16x16", &name, 16, q, false, true, false);
        bench_mlz!("Lossy Chroma 8x8", &name, 8, q, false, true, true);
        bench_mlz!("Lossy Chroma 16x16", &name, 16, q, false, true, true);
    }

    // Benchmark JPEG
    for &q in &qs {
        let name = format!("q={}", q);
        
        let mut enc_g = c.benchmark_group("Encode JPEG");
        enc_g.sample_size(10);
        enc_g.bench_function(&name, |b| {
            b.iter(|| {
                let mut out = Vec::new();
                let mut encoder = JpegEncoder::new_with_quality(&mut out, q);
                encoder.encode(&raw_data, width, height, ColorType::Rgb8).unwrap();
            });
        });
        enc_g.finish();

        let mut out = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut out, q);
        encoder.encode(&raw_data, width, height, ColorType::Rgb8).unwrap();
        
        let mut dec_g = c.benchmark_group("Decode JPEG");
        dec_g.sample_size(10);
        dec_g.bench_function(&name, |b| {
            b.iter(|| {
                let mut decoder = JpegDecoder::new(Cursor::new(&out)).unwrap();
                let mut decoded = vec![0; decoder.total_bytes() as usize];
                decoder.read_image(&mut decoded).unwrap();
            });
        });
        dec_g.finish();
    }

    // Benchmark WebP
    for &q in &qs {
        let name = format!("q={}", q);
        
        let mut enc_g = c.benchmark_group("Encode WebP");
        enc_g.sample_size(10);
        enc_g.bench_function(&name, |b| {
            b.iter(|| {
                let encoder = WebPEncoder::from_rgb(&raw_data, width, height);
                let _ = encoder.encode(q as f32);
            });
        });
        enc_g.finish();

        let encoder = WebPEncoder::from_rgb(&raw_data, width, height);
        let webp_mem = encoder.encode(q as f32);
        
        let mut dec_g = c.benchmark_group("Decode WebP");
        dec_g.sample_size(10);
        dec_g.bench_function(&name, |b| {
            b.iter(|| {
                let decoder = WebPDecoder::new(&webp_mem);
                let _ = decoder.decode();
            });
        });
        dec_g.finish();
    }

    // WebP Lossless
    {
        let mut enc_g = c.benchmark_group("Encode Lossless");
        enc_g.sample_size(10);
        enc_g.bench_function("WebP Lossless", |b| {
            b.iter(|| {
                let encoder = WebPEncoder::from_rgb(&raw_data, width, height);
                let _ = encoder.encode_lossless();
            });
        });
        enc_g.finish();

        let encoder = WebPEncoder::from_rgb(&raw_data, width, height);
        let webp_mem = encoder.encode_lossless();
        let mut dec_g = c.benchmark_group("Decode Lossless");
        dec_g.sample_size(10);
        dec_g.bench_function("WebP Lossless", |b| {
            b.iter(|| {
                let decoder = WebPDecoder::new(&webp_mem);
                let _ = decoder.decode();
            });
        });
        dec_g.finish();
    }

    // PNG
    {
        let mut enc_g = c.benchmark_group("Encode Lossless");
        enc_g.sample_size(10);
        enc_g.bench_function("PNG", |b| {
            b.iter(|| {
                let mut out = Vec::new();
                let encoder = PngEncoder::new(&mut out);
                encoder.write_image(&raw_data, width, height, ColorType::Rgb8).unwrap();
            });
        });
        enc_g.finish();

        let mut out = Vec::new();
        let encoder = PngEncoder::new(&mut out);
        encoder.write_image(&raw_data, width, height, ColorType::Rgb8).unwrap();

        let mut dec_g = c.benchmark_group("Decode Lossless");
        dec_g.sample_size(10);
        dec_g.bench_function("PNG", |b| {
            b.iter(|| {
                let mut decoder = PngDecoder::new(Cursor::new(&out)).unwrap();
                let mut decoded = vec![0; decoder.total_bytes() as usize];
                decoder.read_image(&mut decoded).unwrap();
            });
        });
        dec_g.finish();
    }
}

criterion_group!(benches, bench_all);
criterion_main!(benches);
