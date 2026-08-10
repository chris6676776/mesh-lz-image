#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pixel {
    pub channels: [u8; 3],
    pub count: u8, // 1 for Grayscale, 3 for RGB
}

impl Pixel {
    pub fn new_gray(g: u8) -> Self {
        Self {
            channels: [g, 0, 0],
            count: 1,
        }
    }

    pub fn new_rgb(r: u8, g: u8, b: u8) -> Self {
        Self {
            channels: [r, g, b],
            count: 3,
        }
    }

    pub fn r(&self) -> u8 {
        self.channels[0]
    }

    pub fn g(&self) -> u8 {
        self.channels[1]
    }

    pub fn b(&self) -> u8 {
        self.channels[2]
    }

    pub fn gray(&self) -> u8 {
        self.channels[0]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LzCommand {
    Literal(Pixel),
    Match { offset: u16, length: u8 },
}

/// Greedy match finder that runs Mesh-LZ on a 1D sequence of pixels.
pub fn encode_block_lz(pixels: &[Pixel], min_match_len: usize) -> Vec<LzCommand> {
    let n = pixels.len();
    let mut commands = Vec::new();
    
    if n == 0 {
        return commands;
    }

    const HASH_SIZE: usize = 1024;
    const MAX_CHAIN_DEPTH: usize = 8;
    
    let mut head = vec![usize::MAX; HASH_SIZE];
    let mut next = vec![usize::MAX; n];
    
    let hash_pixels = |idx: usize| -> usize {
        if idx + 2 < n {
            let p0 = &pixels[idx];
            let p1 = &pixels[idx + 1];
            let p2 = &pixels[idx + 2];
            let v0 = (p0.channels[0] as u32) | ((p0.channels[1] as u32) << 8) | ((p0.channels[2] as u32) << 16);
            let v1 = (p1.channels[0] as u32) | ((p1.channels[1] as u32) << 8) | ((p1.channels[2] as u32) << 16);
            let v2 = (p2.channels[0] as u32) | ((p2.channels[1] as u32) << 8) | ((p2.channels[2] as u32) << 16);
            let h = v0.wrapping_mul(0x1e35a7bd) ^ v1.wrapping_mul(0x5b3731) ^ v2.wrapping_mul(0x937);
            ((h ^ (h >> 16)) as usize) & (HASH_SIZE - 1)
        } else {
            0
        }
    };

    let mut i = 0;
    while i < n {
        let mut best_len = 0;
        let mut best_offset = 0;

        if i + 2 < n {
            let h = hash_pixels(i);
            let mut curr = head[h];
            let mut depth = 0;

            while curr != usize::MAX && depth < MAX_CHAIN_DEPTH {
                if curr < i { // Only match backwards
                    let mut len = 0;
                    while i + len < n && pixels[curr + len] == pixels[i + len] {
                        len += 1;
                    }
                    if len > best_len {
                        best_len = len;
                        best_offset = (i - curr) as u16;
                    }
                }
                curr = next[curr];
                depth += 1;
            }
        }

        if best_len >= min_match_len {
            commands.push(LzCommand::Match {
                offset: best_offset,
                length: best_len as u8,
            });
            // Update hash chains for skipped pixels
            for k in 0..best_len {
                if i + k + 2 < n {
                    let h = hash_pixels(i + k);
                    next[i + k] = head[h];
                    head[h] = i + k;
                }
            }
            i += best_len;
        } else {
            commands.push(LzCommand::Literal(pixels[i]));
            if i + 2 < n {
                let h = hash_pixels(i);
                next[i] = head[h];
                head[h] = i;
            }
            i += 1;
        }
    }

    commands
}

/// Decode the sequence of commands back to a 1D list of pixels.
pub fn decode_block_lz(commands: &[LzCommand], expected_size: usize) -> Vec<Pixel> {
    let mut pixels = Vec::with_capacity(expected_size);

    for cmd in commands {
        match *cmd {
            LzCommand::Literal(pixel) => {
                pixels.push(pixel);
            }
            LzCommand::Match { offset, length } => {
                let offset = offset as usize;
                for _ in 0..length {
                    if pixels.len() >= offset {
                        let val = pixels[pixels.len() - offset];
                        pixels.push(val);
                    } else {
                        // Guard against malformed streams
                        pixels.push(Pixel {
                            channels: [0, 0, 0],
                            count: 1,
                        });
                    }
                }
            }
        }
    }

    pixels
}
