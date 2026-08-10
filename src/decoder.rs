use anyhow::{anyhow, Result};
use rayon::prelude::*;
use crate::rans::{FreqTable, InterleavedDecoder, decode_single};
use crate::stencil;
use crate::mesh_lz::Pixel;
use crate::codec::{unzip_zag, dequantize_residual};

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
