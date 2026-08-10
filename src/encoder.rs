use anyhow::{anyhow, Result};
use rayon::prelude::*;
use crate::rans::{FreqTable, encode_single, encode_interleaved};
use crate::stencil;
use crate::mesh_lz::{Pixel, LzCommand, encode_block_lz};
use crate::codec::{zig_zag, quantize_residual, dequantize_residual};

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
