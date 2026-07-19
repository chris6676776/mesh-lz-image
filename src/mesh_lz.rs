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
    let mut i = 0;

    while i < n {
        let mut best_len = 0;
        let mut best_offset = 0;

        // Search for the longest match in the history pixels[0..i]
        for j in 0..i {
            let offset = (i - j) as u16;
            let mut len = 0;
            while i + len < n && pixels[j + len] == pixels[i + len] {
                len += 1;
            }
            if len > best_len {
                best_len = len;
                best_offset = offset;
            }
        }

        if best_len >= min_match_len {
            commands.push(LzCommand::Match {
                offset: best_offset,
                length: best_len as u8,
            });
            i += best_len;
        } else {
            commands.push(LzCommand::Literal(pixels[i]));
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
