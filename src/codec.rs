use anyhow::{anyhow, Result};
use rayon::prelude::*;
use crate::rans::{
    FreqTable, InterleavedDecoder, encode_single, decode_single, encode_interleaved,
};
use crate::stencil;
use crate::mesh_lz::{Pixel, LzCommand, encode_block_lz};

pub fn zig_zag(val: i8) -> u8 {
    ((val << 1) ^ (val >> 7)) as u8
}

pub fn unzip_zag(val: u8) -> i8 {
    ((val >> 1) as i8) ^ (-((val & 1) as i8))
}

#[inline(always)]
pub fn quantize_residual(orig: u8, prev: u8, q: u16) -> i8 {
    if q == 1 {
        (orig as i16 - prev as i16) as i8
    } else {
        let res = orig as i16 - prev as i16;
        (res as f32 / q as f32).round() as i8
    }
}

#[inline(always)]
pub fn dequantize_residual(quant_res: i8, prev: u8, q: u16) -> u8 {
    if q == 1 {
        prev.wrapping_add(quant_res as u8)
    } else {
        let dequant_res = quant_res as i16 * q as i16;
        (prev as i16 + dequant_res).clamp(0, 255) as u8
    }
}

/// Compress an image into the custom MLZ format.
pub fn compress(
    width: u32,
    height: u32,
    channels: u8,
    data: &[u8],
    block_size: usize,
    quality: u8,
    use_palette: bool,
    use_ycocg: bool,
    subsample: bool,
) -> Result<Vec<u8>> {
    if channels != 1 && channels != 3 {
        return Err(anyhow!("Only 1-channel (Grayscale) and 3-channel (RGB) images are supported"));
    }

    let b = block_size;
    let cols = (width as usize + b - 1) / b;
    let rows = (height as usize + b - 1) / b;
    let w_pad = cols * b;
    let h_pad = rows * b;

    let (eff_channels, eff_data, palette_rgb) = if use_palette && channels == 3 {
        // First, count unique colors
        let mut unique_colors = std::collections::HashSet::new();
        for chunk in data.chunks_exact(3) {
            unique_colors.insert([chunk[0], chunk[1], chunk[2]]);
            if unique_colors.len() > 256 {
                break;
            }
        }

        if unique_colors.len() <= 256 {
            // Exact palette
            let mut palette = Vec::new();
            let mut color_to_idx = std::collections::HashMap::new();
            for (i, &color) in unique_colors.iter().enumerate() {
                palette.extend_from_slice(&color);
                color_to_idx.insert(color, i as u8);
            }
            // Pad to 768 bytes
            while palette.len() < 768 {
                palette.push(0);
            }
            let mut indices = Vec::with_capacity(data.len() / 3);
            for chunk in data.chunks_exact(3) {
                indices.push(*color_to_idx.get(&[chunk[0], chunk[1], chunk[2]]).unwrap());
            }
            (1, indices, Some(palette))
        } else {
            // Convert RGB to RGBA for NeuQuant
            let mut rgba_data = Vec::with_capacity(data.len() / 3 * 4);
            for chunk in data.chunks_exact(3) {
                rgba_data.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            let nq = color_quant::NeuQuant::new(10, 256, &rgba_data);
            let palette = nq.color_map_rgb();
            let mut indices = Vec::with_capacity(data.len() / 3);
            for chunk in data.chunks_exact(3) {
                indices.push(nq.index_of(&[chunk[0], chunk[1], chunk[2], 255]) as u8);
            }
            (1, indices, Some(palette))
        }
    } else {
        (channels, data.to_vec(), None)
    };

    // Quality-to-quantization-step mapping (JPEG-like curve):
    //   100% → q=1 (lossless),  90% → q=2,  80% → q=3,  70% → q=4,  50% → q=6,  10% → q=10
    let q = if eff_channels == 1 && palette_rgb.is_some() {
        1u16 // Force lossless encoding for palette indices
    } else if quality >= 100 { 
        1u16 
    } else { 
        ((100u16.saturating_sub(quality as u16)) / 10 + 1).max(1) 
    };

    // 1. Pad the image using clamp padding and convert to our Pixel struct
    let mut padded_pixels = vec![Pixel { channels: [0, 0, 0], count: eff_channels }; w_pad * h_pad];
    for y in 0..h_pad {
        let sy = y.min(height as usize - 1);
        for x in 0..w_pad {
            let sx = x.min(width as usize - 1);
            let idx = sy * width as usize + sx;
            if eff_channels == 1 {
                padded_pixels[y * w_pad + x] = Pixel::new_gray(eff_data[idx]);
            } else {
                let r = eff_data[idx * 3] as i16;
                let g = eff_data[idx * 3 + 1] as i16;
                let b = eff_data[idx * 3 + 2] as i16;
                
                if use_ycocg {
                    // Lossy YCoCg-R (shifted to fit in 8 bits)
                    let co = r - b;
                    let t = b + (co >> 1);
                    let cg = g - t;
                    let y_val = t + (cg >> 1);
                    
                    let co_8 = (co / 2 + 128).clamp(0, 255) as u8;
                    let cg_8 = (cg / 2 + 128).clamp(0, 255) as u8;
                    padded_pixels[y * w_pad + x] = Pixel::new_rgb(y_val as u8, co_8, cg_8);
                } else {
                    // Green-decorrelation: Y = G, U = R - G, V = B - G
                    let y_val = g as u8;
                    let u_val = (r - g) as u8;
                    let v_val = (b - g) as u8;
                    padded_pixels[y * w_pad + x] = Pixel::new_rgb(y_val, u_val, v_val);
                }
            }
        }
    }

    if use_ycocg && subsample && eff_channels == 3 {
        // 4:2:0 Subsampling: Average 2x2 chroma blocks and duplicate
        for y in (0..h_pad).step_by(2) {
            for x in (0..w_pad).step_by(2) {
                let mut sum_co = 0u32;
                let mut sum_cg = 0u32;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let pix = &padded_pixels[(y + dy) * w_pad + (x + dx)];
                        sum_co += pix.channels[1] as u32;
                        sum_cg += pix.channels[2] as u32;
                    }
                }
                let avg_co = (sum_co / 4) as u8;
                let avg_cg = (sum_cg / 4) as u8;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let pix = &mut padded_pixels[(y + dy) * w_pad + (x + dx)];
                        pix.channels[1] = avg_co;
                        pix.channels[2] = avg_cg;
                    }
                }
            }
        }
    }

    // 2. Perform stencil selection and generate LZ commands for all blocks
    struct RowData {
        stencils: Vec<u8>,
        commands: Vec<Vec<LzCommand>>,
        count_stencil: [u32; 8],
        count_command: [u32; 2],
        count_residual_0: [u32; 256],
        count_residual_1: [u32; 256],
        count_residual_2: [u32; 256],
        count_offset: [u32; 257],
        count_length: [u32; 257],
    }

    let row_data_vec: Vec<RowData> = (0..rows).into_par_iter().map(|by| {
        let mut stencils = Vec::with_capacity(cols);
        let mut commands = Vec::with_capacity(cols);
        
        let mut count_stencil = [0u32; 8];
        let mut count_command = [0u32; 2];
        let mut count_residual_0 = [0u32; 256];
        let mut count_residual_1 = [0u32; 256];
        let mut count_residual_2 = [0u32; 256];
        let mut count_offset = [0u32; 257];
        let mut count_length = [0u32; 257];

        for bx in 0..cols {
            // Extract BxB block pixels
            let mut block = vec![Pixel { channels: [0, 0, 0], count: eff_channels }; b * b];
            for y in 0..b {
                for x in 0..b {
                    block[y * b + x] = padded_pixels[(by * b + y) * w_pad + (bx * b + x)];
                }
            }

            // Heuristic: Calculate horizontal and vertical gradient sum on the first channel (luminance)
            let mut var_h = 0u32;
            let mut var_v = 0u32;
            for y in 0..b {
                for x in 0..(b - 1) {
                    var_h += block[y * b + x].r().abs_diff(block[y * b + x + 1].r()) as u32;
                }
            }
            for y in 0..(b - 1) {
                for x in 0..b {
                    var_v += block[y * b + x].r().abs_diff(block[(y + 1) * b + x].r()) as u32;
                }
            }

            // Select stencil based on gradient variance
            let stencil_idx = if var_h > (var_v * 3 / 2) && var_h > 10 {
                2 // Column Scan
            } else if var_v > (var_h * 3 / 2) && var_v > 10 {
                1 // Raster Scan
            } else {
                0 // Hilbert Curve (default for low/balanced variance)
            };

            stencils.push(stencil_idx);
            count_stencil[stencil_idx as usize] += 1;

            // Generate 1D path using selected stencil
            let path = stencil::get_stencil(stencil_idx, b);
            let mut path_pixels = Vec::with_capacity(b * b);
            for &(py, px) in path {
                path_pixels.push(block[py * b + px]);
            }

            // Encode block using Mesh-LZ
            let cmds = encode_block_lz(&path_pixels, 3);
            
            // Accumulate counts for FreqTables using closed-loop reconstruction
            let mut prev = Pixel { channels: [0, 0, 0], count: eff_channels };
            let mut coded_count = 0;
            let mut recon_pixels = Vec::with_capacity(b * b);
            for cmd in &cmds {
                match cmd {
                    LzCommand::Literal(pix) => {
                        count_command[0] += 1;
                        if eff_channels == 1 {
                            let quant_res = quantize_residual(pix.gray(), prev.gray(), q);
                            let recon = dequantize_residual(quant_res, prev.gray(), q);
                            
                            count_residual_0[zig_zag(quant_res) as usize] += 1;
                            let recon_pix = Pixel::new_gray(recon);
                            recon_pixels.push(recon_pix);
                            prev = recon_pix;
                        } else {
                            let q0 = quantize_residual(pix.channels[0], prev.channels[0], q);
                            let q1 = quantize_residual(pix.channels[1], prev.channels[1], q);
                            let q2 = quantize_residual(pix.channels[2], prev.channels[2], q);
                            
                            let recon_0 = dequantize_residual(q0, prev.channels[0], q);
                            let recon_1 = dequantize_residual(q1, prev.channels[1], q);
                            let recon_2 = dequantize_residual(q2, prev.channels[2], q);
                            
                            count_residual_0[zig_zag(q0) as usize] += 1;
                            count_residual_1[zig_zag(q1) as usize] += 1;
                            count_residual_2[zig_zag(q2) as usize] += 1;
                            
                            let recon_pix = Pixel::new_rgb(recon_0, recon_1, recon_2);
                            recon_pixels.push(recon_pix);
                            prev = recon_pix;
                        }
                        coded_count += 1;
                    }
                    LzCommand::Match { offset, length } => {
                        count_command[1] += 1;
                        count_offset[*offset as usize] += 1;
                        count_length[*length as usize] += 1;
                        
                        let match_len = *length as usize;
                        let offset_idx = *offset as usize;
                        for k in 0..match_len {
                            let val = recon_pixels[coded_count - offset_idx + k];
                            recon_pixels.push(val);
                        }
                        if let Some(&last_pix) = recon_pixels.last() {
                            prev = last_pix;
                        }
                        coded_count += match_len;
                    }
                }
            }
            commands.push(cmds);
        }
        
        RowData {
            stencils,
            commands,
            count_stencil,
            count_command,
            count_residual_0,
            count_residual_1,
            count_residual_2,
            count_offset,
            count_length,
        }
    }).collect();

    // Merge counts and commands
    let mut stencils = Vec::with_capacity(rows * cols);
    let mut block_commands = Vec::with_capacity(rows * cols);
    
    let mut count_stencil = vec![0u32; 8];
    let mut count_command = vec![0u32; 2];
    let mut count_residual_0 = vec![0u32; 256];
    let mut count_residual_1 = vec![0u32; 256];
    let mut count_residual_2 = vec![0u32; 256];
    let mut count_offset = vec![0u32; 257];
    let mut count_length = vec![0u32; 257];

    for rd in row_data_vec {
        stencils.extend(rd.stencils);
        block_commands.extend(rd.commands);
        
        for i in 0..8 { count_stencil[i] += rd.count_stencil[i]; }
        for i in 0..2 { count_command[i] += rd.count_command[i]; }
        for i in 0..256 {
            count_residual_0[i] += rd.count_residual_0[i];
            count_residual_1[i] += rd.count_residual_1[i];
            count_residual_2[i] += rd.count_residual_2[i];
        }
        for i in 0..257 {
            count_offset[i] += rd.count_offset[i];
            count_length[i] += rd.count_length[i];
        }
    }

    // 3. Build Global Frequency Tables
    let table_stencil = FreqTable::new(&count_stencil, 8);
    let table_command = FreqTable::new(&count_command, 2);
    let table_residual_0 = FreqTable::new(&count_residual_0, 256);
    let table_residual_1 = FreqTable::new(&count_residual_1, 256);
    let table_residual_2 = FreqTable::new(&count_residual_2, 256);
    let table_offset = FreqTable::new(&count_offset, 257);
    let table_length = FreqTable::new(&count_length, 257);

    // 4. Encode stencils using single-stream rANS
    let stencil_symbols: Vec<u16> = stencils.iter().map(|&s| s as u16).collect();
    let compressed_stencils = encode_single(&stencil_symbols, &table_stencil);

    // 5. Encode block rows independently (for multi-threaded decoding)
    let mut compressed_rows = vec![Vec::new(); rows];
    compressed_rows.par_iter_mut().enumerate().for_each(|(by, row_bytes)| {
        let mut row_symbols = Vec::new();
        for bx in 0..cols {
            let block_idx = by * cols + bx;
            let cmds = &block_commands[block_idx];
            let mut prev = Pixel { channels: [0, 0, 0], count: eff_channels };
            let mut coded_count = 0;
            let mut recon_pixels = Vec::with_capacity(b * b);

            for cmd in cmds {
                match *cmd {
                    LzCommand::Literal(pix) => {
                        row_symbols.push((0u16, &table_command));
                        if eff_channels == 1 {
                            let quant_res = quantize_residual(pix.gray(), prev.gray(), q);
                            let recon = dequantize_residual(quant_res, prev.gray(), q);
                            
                            row_symbols.push((zig_zag(quant_res) as u16, &table_residual_0));
                            let recon_pix = Pixel::new_gray(recon);
                            recon_pixels.push(recon_pix);
                            prev = recon_pix;
                        } else {
                            let q0 = quantize_residual(pix.channels[0], prev.channels[0], q);
                            let q1 = quantize_residual(pix.channels[1], prev.channels[1], q);
                            let q2 = quantize_residual(pix.channels[2], prev.channels[2], q);
                            
                            let recon_0 = dequantize_residual(q0, prev.channels[0], q);
                            let recon_1 = dequantize_residual(q1, prev.channels[1], q);
                            let recon_2 = dequantize_residual(q2, prev.channels[2], q);
                            
                            row_symbols.push((zig_zag(q0) as u16, &table_residual_0));
                            row_symbols.push((zig_zag(q1) as u16, &table_residual_1));
                            row_symbols.push((zig_zag(q2) as u16, &table_residual_2));
                            
                            let recon_pix = Pixel::new_rgb(recon_0, recon_1, recon_2);
                            recon_pixels.push(recon_pix);
                            prev = recon_pix;
                        }
                        coded_count += 1;
                    }
                    LzCommand::Match { offset, length } => {
                        row_symbols.push((1u16, &table_command));
                        row_symbols.push((offset, &table_offset));
                        row_symbols.push((length as u16, &table_length));

                        let match_len = length as usize;
                        let offset_idx = offset as usize;
                        for k in 0..match_len {
                            let val = recon_pixels[coded_count - offset_idx + k];
                            recon_pixels.push(val);
                        }
                        if let Some(&last_pix) = recon_pixels.last() {
                            prev = last_pix;
                        }
                        coded_count += match_len;
                    }
                }
            }
        }
        *row_bytes = encode_interleaved(&row_symbols);
    });

    // 6. Serialize entire bitstream
    let mut bitstream = Vec::new();
    // Header
    bitstream.extend_from_slice(b"MLZ4");
    bitstream.extend_from_slice(&(width).to_le_bytes());
    bitstream.extend_from_slice(&(height).to_le_bytes());
    bitstream.push(b as u8);
    bitstream.push(channels);
    bitstream.push(quality);
    bitstream.push(if palette_rgb.is_some() { 1 } else { 0 });
    bitstream.push(if use_ycocg { 1 } else { 0 });
    bitstream.push(if subsample { 1 } else { 0 });

    if let Some(ref pal) = palette_rgb {
        bitstream.extend_from_slice(pal);
    }

    bitstream.extend_from_slice(&(rows as u32).to_le_bytes());
    bitstream.extend_from_slice(&(cols as u32).to_le_bytes());

    // Serialize frequency tables
    bitstream.extend_from_slice(&table_stencil.serialize());
    bitstream.extend_from_slice(&table_command.serialize());
    bitstream.extend_from_slice(&table_residual_0.serialize());
    if eff_channels == 3 {
        bitstream.extend_from_slice(&table_residual_1.serialize());
        bitstream.extend_from_slice(&table_residual_2.serialize());
    }
    bitstream.extend_from_slice(&table_offset.serialize());
    bitstream.extend_from_slice(&table_length.serialize());

    // Serialize payloads
    bitstream.extend_from_slice(&(compressed_stencils.len() as u32).to_le_bytes());
    bitstream.extend_from_slice(&compressed_stencils);

    for row_bytes in &compressed_rows {
        bitstream.extend_from_slice(&(row_bytes.len() as u32).to_le_bytes());
        bitstream.extend_from_slice(row_bytes);
    }

    Ok(bitstream)
}

/// Decompress an MLZ bitstream.
pub fn decompress(bitstream: &[u8]) -> Result<(u32, u32, u8, Vec<u8>)> {
    if bitstream.len() < 24 {
        return Err(anyhow!("Invalid bitstream: too short"));
    }

    if &bitstream[0..4] != b"MLZ4" {
        return Err(anyhow!("Invalid magic bytes: must be MLZ4"));
    }

    let mut offset = 4;
    let width = u32::from_le_bytes([bitstream[offset], bitstream[offset + 1], bitstream[offset + 2], bitstream[offset + 3]]);
    let height = u32::from_le_bytes([bitstream[offset + 4], bitstream[offset + 5], bitstream[offset + 6], bitstream[offset + 7]]);
    offset += 8;

    let b = bitstream[offset] as usize;
    let channels = bitstream[offset + 1];
    let quality = bitstream[offset + 2];
    let palette_flag = bitstream[offset + 3];
    let ycocg_flag = bitstream[offset + 4];
    let _subsample_flag = bitstream[offset + 5];
    offset += 6;

    if b != 8 && b != 16 {
        return Err(anyhow!("Unsupported block size {}", b));
    }
    if channels != 1 && channels != 3 {
        return Err(anyhow!("Unsupported channel count {}", channels));
    }

    let mut palette = None;
    if palette_flag == 1 {
        if bitstream.len() < offset + 768 {
            return Err(anyhow!("Invalid bitstream: missing palette data"));
        }
        let mut pal = vec![0u8; 768];
        pal.copy_from_slice(&bitstream[offset..offset+768]);
        palette = Some(pal);
        offset += 768;
    }

    let eff_channels = if palette.is_some() { 1 } else { channels };

    let q = if eff_channels == 1 && palette.is_some() {
        1u16
    } else if quality >= 100 { 
        1u16 
    } else { 
        ((100u16.saturating_sub(quality as u16)) / 10 + 1).max(1) 
    };

    let rows = u32::from_le_bytes([bitstream[offset], bitstream[offset + 1], bitstream[offset + 2], bitstream[offset + 3]]) as usize;
    let cols = u32::from_le_bytes([bitstream[offset + 4], bitstream[offset + 5], bitstream[offset + 6], bitstream[offset + 7]]) as usize;
    offset += 8;

    // Deserialize frequency tables
    let table_stencil = FreqTable::deserialize(bitstream, &mut offset, 8)?;
    let table_command = FreqTable::deserialize(bitstream, &mut offset, 2)?;
    let table_residual_0 = FreqTable::deserialize(bitstream, &mut offset, 256)?;
    let table_residual_1 = if eff_channels == 3 {
        Some(FreqTable::deserialize(bitstream, &mut offset, 256)?)
    } else {
        None
    };
    let table_residual_2 = if eff_channels == 3 {
        Some(FreqTable::deserialize(bitstream, &mut offset, 256)?)
    } else {
        None
    };
    let table_offset = FreqTable::deserialize(bitstream, &mut offset, 257)?;
    let table_length = FreqTable::deserialize(bitstream, &mut offset, 257)?;

    // Deserialize compressed stencils
    if offset + 4 > bitstream.len() {
        return Err(anyhow!("Unexpected EOF reading stencil block size"));
    }
    let _stencils_size = u32::from_le_bytes([bitstream[offset], bitstream[offset + 1], bitstream[offset + 2], bitstream[offset + 3]]) as usize;
    offset += 4;

    let stencil_symbols = decode_single(bitstream, &mut offset, rows * cols, &table_stencil)?;
    let stencils: Vec<u8> = stencil_symbols.iter().map(|&s| s as u8).collect();

    // Read row payloads
    let mut row_payloads = Vec::with_capacity(rows);
    for _ in 0..rows {
        if offset + 4 > bitstream.len() {
            return Err(anyhow!("Unexpected EOF reading row size"));
        }
        let row_size = u32::from_le_bytes([bitstream[offset], bitstream[offset + 1], bitstream[offset + 2], bitstream[offset + 3]]) as usize;
        offset += 4;
        if offset + row_size > bitstream.len() {
            return Err(anyhow!("Unexpected EOF reading row payload"));
        }
        row_payloads.push(&bitstream[offset..(offset + row_size)]);
        offset += row_size;
    }

    // Decode rows in parallel
    let w_pad = cols * b;
    let h_pad = rows * b;
    let mut padded_pixels = vec![Pixel { channels: [0, 0, 0], count: eff_channels }; w_pad * h_pad];

    let row_results: Result<()> = padded_pixels
        .par_chunks_exact_mut(cols * b * b)
        .zip(row_payloads)
        .enumerate()
        .map(|(by, (row_pixels, row_data))| {
            let mut r_offset = 0;
            let mut decoder = InterleavedDecoder::new(row_data, &mut r_offset)?;

            let mut symbol_idx = 0;
            for bx in 0..cols {
                let block_idx = by * cols + bx;
                let stencil_idx = stencils[block_idx];
                let path = stencil::get_stencil(stencil_idx, b);

                // Decode Mesh-LZ commands for the block
                let mut block_pixels = Vec::with_capacity(b * b);
                let mut prev = Pixel { channels: [0, 0, 0], count: eff_channels };

                while block_pixels.len() < b * b {
                    let cmd = decoder.decode_symbol(symbol_idx, &table_command);
                    symbol_idx += 1;

                    if cmd == 0 {
                        // Literal
                        let pix = if eff_channels == 1 {
                            let r0 = decoder.decode_symbol(symbol_idx, &table_residual_0);
                            symbol_idx += 1;
                            let quant_res = unzip_zag(r0 as u8);
                            let recon = dequantize_residual(quant_res, prev.gray(), q);
                            Pixel::new_gray(recon)
                        } else {
                            let r0 = decoder.decode_symbol(symbol_idx, &table_residual_0);
                            symbol_idx += 1;
                            let r1 = decoder.decode_symbol(symbol_idx, table_residual_1.as_ref().unwrap());
                            symbol_idx += 1;
                            let r2 = decoder.decode_symbol(symbol_idx, table_residual_2.as_ref().unwrap());
                            symbol_idx += 1;

                            let q0 = unzip_zag(r0 as u8);
                            let q1 = unzip_zag(r1 as u8);
                            let q2 = unzip_zag(r2 as u8);

                            let y = dequantize_residual(q0, prev.channels[0], q);
                            let u = dequantize_residual(q1, prev.channels[1], q);
                            let v = dequantize_residual(q2, prev.channels[2], q);
                            Pixel::new_rgb(y, u, v)
                        };
                        block_pixels.push(pix);
                        prev = pix;
                    } else {
                        // Match
                        let match_offset = decoder.decode_symbol(symbol_idx, &table_offset);
                        symbol_idx += 1;
                        let match_length = decoder.decode_symbol(symbol_idx, &table_length);
                        symbol_idx += 1;

                        let offset_idx = match_offset as usize;
                        let match_len = match_length as usize;

                        for _ in 0..match_len {
                            if block_pixels.len() >= offset_idx {
                                let val = block_pixels[block_pixels.len() - offset_idx];
                                block_pixels.push(val);
                            } else {
                                block_pixels.push(Pixel { channels: [0, 0, 0], count: eff_channels });
                            }
                        }
                        if let Some(&last_pix) = block_pixels.last() {
                            prev = last_pix;
                        }
                    }
                }

                // Place decoded block pixels directly into the padded image buffer
                for (idx, &(py, px)) in path.iter().enumerate() {
                    row_pixels[py * w_pad + bx * b + px] = block_pixels[idx];
                }
            }

            Ok(())
        })
        .collect();

    row_results?;

    // 7. Crop the padded image and reconstruct color values
    let mut out_data = vec![0u8; (width * height) as usize * channels as usize];
    for y in 0..(height as usize) {
        for x in 0..(width as usize) {
            let src_idx = y * w_pad + x;
            let dst_idx = y * width as usize + x;
            let pix = padded_pixels[src_idx];
            
            if let Some(ref pal) = palette {
                // Palettized RGB: Map index back to RGB
                let idx = pix.gray() as usize;
                let r = pal.get(idx * 3).copied().unwrap_or(0);
                let g = pal.get(idx * 3 + 1).copied().unwrap_or(0);
                let b = pal.get(idx * 3 + 2).copied().unwrap_or(0);
                out_data[dst_idx * 3] = r;
                out_data[dst_idx * 3 + 1] = g;
                out_data[dst_idx * 3 + 2] = b;
            } else if channels == 1 {
                out_data[dst_idx] = pix.gray();
            } else {
                let y_val = pix.channels[0];
                let co_val = pix.channels[1];
                let cg_val = pix.channels[2];
                
                if ycocg_flag == 1 {
                    // Inverse Lossy YCoCg-R
                    let y = y_val as i16;
                    let co = (co_val as i16 - 128) * 2;
                    let cg = (cg_val as i16 - 128) * 2;
                    
                    let t = y - (cg >> 1);
                    let g = cg + t;
                    let b = t - (co >> 1);
                    let r = b + co;
                    
                    out_data[dst_idx * 3] = r.clamp(0, 255) as u8;
                    out_data[dst_idx * 3 + 1] = g.clamp(0, 255) as u8;
                    out_data[dst_idx * 3 + 2] = b.clamp(0, 255) as u8;
                } else {
                    // Inverse Green-decorrelation:
                    // G = Y, R = U + G, B = V + G
                    let g = y_val;
                    let r = co_val.wrapping_add(g);
                    let b = cg_val.wrapping_add(g);
                    out_data[dst_idx * 3] = r;
                    out_data[dst_idx * 3 + 1] = g;
                    out_data[dst_idx * 3 + 2] = b;
                }
            }
        }
    }

    Ok((width, height, channels, out_data))
}
