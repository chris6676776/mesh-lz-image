use mesh_lz_codec::codec::{compress, decompress};

fn verify_roundtrip(width: u32, height: u32, channels: u8, data: &[u8], block_size: usize) {
    let compressed = compress(width, height, channels, data, block_size, 100)
        .unwrap_or_else(|e| panic!("Compression failed for size {}x{}x{}: {}", width, height, channels, e));
    let (_dec_width, _dec_height, _dec_channels, decoded) = decompress(&compressed)
        .unwrap_or_else(|e| panic!("Decompression failed for size {}x{}x{}: {}", width, height, channels, e));

    if decoded != data {
        println!("Data content mismatch for {}x{} (channels={})!", width, height, channels);
        let mut count = 0;
        for i in 0..data.len() {
            if decoded[i] != data[i] {
                println!("Mismatch at byte {}: original={}, decoded={}", i, data[i], decoded[i]);
                count += 1;
                if count > 20 {
                    break;
                }
            }
        }
        panic!("Data mismatch");
    }
}

#[test]
fn test_single_block_debug() {
    let width = 8;
    let height = 8;
    let channels = 1;
    let mut data = vec![0u8; 64];
    for y in 0..8 {
        for x in 0..8 {
            data[y * 8 + x] = (y + x) as u8;
        }
    }
    verify_roundtrip(width, height, channels, &data, 8);
}

#[test]
fn test_flat_grayscale() {
    let width = 64;
    let height = 64;
    let channels = 1;
    let data = vec![128u8; (width * height) as usize];
    verify_roundtrip(width, height, channels, &data, 8);
    verify_roundtrip(width, height, channels, &data, 16);
}

#[test]
fn test_flat_rgb() {
    let width = 64;
    let height = 64;
    let channels = 3;
    let mut data = vec![0u8; (width * height * 3) as usize];
    for idx in 0..(width * height) as usize {
        data[idx * 3] = 100;
        data[idx * 3 + 1] = 150;
        data[idx * 3 + 2] = 200;
    }
    verify_roundtrip(width, height, channels, &data, 8);
    verify_roundtrip(width, height, channels, &data, 16);
}

#[test]
fn test_gradient_grayscale() {
    let width = 128;
    let height = 128;
    let channels = 1;
    let mut data = vec![0u8; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            data[idx] = (x + y) as u8;
        }
    }
    verify_roundtrip(width, height, channels, &data, 8);
    verify_roundtrip(width, height, channels, &data, 16);
}

#[test]
fn test_gradient_rgb() {
    let width = 128;
    let height = 128;
    let channels = 3;
    let mut data = vec![0u8; (width * height * 3) as usize];
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            data[idx * 3] = (x * 2) as u8;
            data[idx * 3 + 1] = (y * 2) as u8;
            data[idx * 3 + 2] = (x + y) as u8;
        }
    }
    verify_roundtrip(width, height, channels, &data, 8);
    verify_roundtrip(width, height, channels, &data, 16);
}

#[test]
fn test_noise_grayscale() {
    let width = 64;
    let height = 64;
    let channels = 1;
    let mut data = vec![0u8; (width * height) as usize];
    // Simple LCG random generator for reproducibility
    let mut state = 42u64;
    for i in 0..data.len() {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        data[i] = (state >> 32) as u8;
    }
    verify_roundtrip(width, height, channels, &data, 8);
    verify_roundtrip(width, height, channels, &data, 16);
}

#[test]
fn test_noise_rgb() {
    let width = 64;
    let height = 64;
    let channels = 3;
    let mut data = vec![0u8; (width * height * 3) as usize];
    let mut state = 42u64;
    for i in 0..data.len() {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        data[i] = (state >> 32) as u8;
    }
    verify_roundtrip(width, height, channels, &data, 8);
    verify_roundtrip(width, height, channels, &data, 16);
}

#[test]
fn test_padding_and_edge_sizes() {
    // Test sizes that are not multiples of block size
    let sizes = vec![(17, 17), (23, 31), (9, 9), (3, 47)];
    let channels_list = vec![1, 3];
    let block_sizes = vec![8, 16];

    let mut state = 99u64;

    for &(w, h) in &sizes {
        for &ch in &channels_list {
            for &b in &block_sizes {
                let size = (w * h * ch as u32) as usize;
                let mut data = vec![0u8; size];
                for i in 0..size {
                    state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    data[i] = (state >> 32) as u8;
                }
                verify_roundtrip(w, h, ch, &data, b);
            }
        }
    }
}

#[test]
fn test_lossy_roundtrip() {
    let width = 64;
    let height = 64;
    let channels = 1;
    let size = (width * height * channels as u32) as usize;
    let mut data = vec![0u8; size];
    for y in 0..height {
        for x in 0..width {
            data[(y * width + x) as usize] = (y * 3 + x * 2) as u8;
        }
    }

    let compressed_lossless = compress(width, height, channels, &data, 8, 100).unwrap();
    let (_, _, _, decoded_lossless) = decompress(&compressed_lossless).unwrap();
    assert_eq!(decoded_lossless, data, "Lossless quality 100 must be bit-perfect");

    let compressed_lossy = compress(width, height, channels, &data, 8, 70).unwrap();
    let (_, _, _, decoded_lossy) = decompress(&compressed_lossy).unwrap();

    let mut sum_sq_diff = 0.0;
    for i in 0..data.len() {
        let diff = data[i] as f64 - decoded_lossy[i] as f64;
        sum_sq_diff += diff * diff;
    }
    let mse = sum_sq_diff / (data.len() as f64);
    let psnr = 20.0 * 255.0f64.log10() - 10.0 * mse.log10();
    
    println!("Lossy (Q=70) Compressed size: {} bytes (lossless: {} bytes)", compressed_lossy.len(), compressed_lossless.len());
    println!("Lossy (Q=70) PSNR: {:.2} dB", psnr);
    
    assert!(compressed_lossy.len() <= compressed_lossless.len(), "Lossy bitstream should be smaller or equal to lossless");
    assert!(psnr >= 35.0, "PSNR should be high for quality 70, got {:.2} dB", psnr);
}

#[test]
fn test_large_noise_grayscale() {
    let width = 256u32;
    let height = 256u32;
    let channels = 1u8;
    let size = (width * height * channels as u32) as usize;
    let mut data = vec![0u8; size];
    let mut state = 777u64;
    for i in 0..size {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        data[i] = (state >> 32) as u8;
    }
    verify_roundtrip(width, height, channels, &data, 8);
}

#[test]
fn test_large_noise_rgb() {
    let width = 256u32;
    let height = 256u32;
    let channels = 3u8;
    let size = (width * height * channels as u32) as usize;
    let mut data = vec![0u8; size];
    let mut state = 888u64;
    for i in 0..size {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        data[i] = (state >> 32) as u8;
    }
    verify_roundtrip(width, height, channels, &data, 8);
}

#[test]
fn test_xlarge_noise_rgb() {
    // Stress-test at ~real-image scale (512x512 RGB, 8-bit, lossless)
    let width = 512u32;
    let height = 512u32;
    let channels = 3u8;
    let size = (width * height * channels as u32) as usize;
    let mut data = vec![0u8; size];
    let mut state = 999u64;
    for i in 0..size {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        data[i] = (state >> 32) as u8;
    }
    // Only test block_size=8 to keep test time reasonable
    verify_roundtrip(width, height, channels, &data, 8);
}

/// Verify the zig-zag/unzip-zag round-trips for every possible residual value.
#[test]
fn test_zig_zag_all_values() {
    use mesh_lz_codec::codec::{zig_zag, unzip_zag};
    for v in i8::MIN..=i8::MAX {
        let encoded = zig_zag(v);
        let decoded = unzip_zag(encoded);
        assert_eq!(v, decoded, "zig_zag round-trip failed for {}", v);
    }
}

/// Test that lossless DPCM correctly round-trips for all (orig, prev) pairs.
#[test]
fn test_dpcm_all_pairs() {
    use mesh_lz_codec::codec::{quantize_residual, dequantize_residual};
    // Sample a subset to keep test fast: all prev in [0,255], orig in [0,255]
    for prev in 0u8..=255 {
        for orig in 0u8..=255 {
            let q = quantize_residual(orig, prev, 1);
            let recon = dequantize_residual(q, prev, 1);
            assert_eq!(orig, recon, "DPCM round-trip failed: orig={}, prev={}", orig, prev);
        }
    }
}
