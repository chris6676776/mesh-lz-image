pub use crate::encoder::compress;
pub use crate::decoder::decompress;

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
