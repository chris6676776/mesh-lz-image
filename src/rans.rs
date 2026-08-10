use anyhow::{anyhow, Result};

pub const L_PRECISION: u32 = 12;
pub const M_SCALE: u32 = 1 << L_PRECISION; // 4096
pub const X_MIN: u32 = 1 << 16; // 65536

#[derive(Clone, Debug)]
pub struct FreqTable {
    pub alphabet_size: usize,
    pub freq: Vec<u16>,
    pub cum_freq: Vec<u16>,
    pub lookup: Vec<u16>, // size M_SCALE
}

impl FreqTable {
    /// Create a new FreqTable from raw counts, scaling them to sum to M_SCALE.
    pub fn new(counts: &[u32], alphabet_size: usize) -> Self {
        let mut freq = vec![0u16; alphabet_size];
        let total_count: u64 = counts.iter().map(|&c| c as u64).sum();

        if total_count == 0 {
            // Uniform fallback
            let active_symbols = alphabet_size.min(M_SCALE as usize);
            let base_freq = M_SCALE / active_symbols as u32;
            let rem = M_SCALE % active_symbols as u32;
            for i in 0..active_symbols {
                freq[i] = (base_freq + if (i as u32) < rem { 1 } else { 0 }) as u16;
            }
        } else {
            // Scale counts so they sum to M_SCALE, ensuring any symbol with count > 0 gets freq >= 1
            let mut active = Vec::new();
            for (i, &c) in counts.iter().enumerate().take(alphabet_size) {
                if c > 0 {
                    active.push(i);
                }
            }

            if active.is_empty() {
                freq[0] = M_SCALE as u16;
            } else {
                let mut sum = 0;
                let mut errors = Vec::new();

                for &idx in &active {
                    let c = counts[idx] as u64;
                    // Initial scaled freq (at least 1)
                    let f = (c * M_SCALE as u64 / total_count).max(1) as u32;
                    freq[idx] = f as u16;
                    sum += f;
                    
                    // Track fractional part for rounding adjustments
                    let expected = c as f64 * M_SCALE as f64 / total_count as f64;
                    let err = expected - f as f64;
                    errors.push((idx, err));
                }

                if sum < M_SCALE {
                    // Sort by rounding error descending and add 1 to the largest error symbols
                    errors.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                    let diff = (M_SCALE - sum) as usize;
                    for i in 0..diff.min(errors.len()) {
                        freq[errors[i].0] += 1;
                    }
                } else if sum > M_SCALE {
                    // Sort by rounding error ascending and subtract 1 from the largest positive error symbols (freq > 1)
                    errors.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                    let mut diff = (sum - M_SCALE) as usize;
                    while diff > 0 {
                        let mut reduced = false;
                        for &(idx, _) in &errors {
                            if diff == 0 {
                                break;
                            }
                            if freq[idx] > 1 {
                                freq[idx] -= 1;
                                diff -= 1;
                                reduced = true;
                            }
                        }
                        if !reduced {
                            break;
                        }
                    }
                }
            }
        }

        // Build cumulative frequencies and lookup table
        let mut cum_freq = vec![0u16; alphabet_size + 1];
        let mut lookup = vec![0u16; M_SCALE as usize];
        let mut current_cum = 0;
        for i in 0..alphabet_size {
            cum_freq[i] = current_cum;
            let f = freq[i];
            for slot in current_cum..(current_cum + f) {
                if (slot as usize) < lookup.len() {
                    lookup[slot as usize] = i as u16;
                }
            }
            current_cum += f;
        }
        cum_freq[alphabet_size] = current_cum;

        Self {
            alphabet_size,
            freq,
            cum_freq,
            lookup,
        }
    }

    pub fn freq(&self, sym: u16) -> u16 {
        self.freq[sym as usize]
    }

    pub fn cum_freq(&self, sym: u16) -> u16 {
        self.cum_freq[sym as usize]
    }

    pub fn lookup(&self, slot: u16) -> u16 {
        self.lookup[slot as usize]
    }

    /// Serialize the frequency table to a compact binary format.
    /// Bitmask (32 bytes for alphabet size 256) + non-zero frequency values (1 or 3 bytes each).
    pub fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::new();
        let mask_bytes = (self.alphabet_size + 7) / 8;
        let mut bitmask = vec![0u8; mask_bytes];

        for i in 0..self.alphabet_size {
            if self.freq[i] > 0 {
                bitmask[i / 8] |= 1 << (i % 8);
            }
        }
        data.extend_from_slice(&bitmask);

        for i in 0..self.alphabet_size {
            let f = self.freq[i];
            if f > 0 {
                if f < 255 {
                    data.push(f as u8);
                } else {
                    data.push(255);
                    data.extend_from_slice(&f.to_le_bytes());
                }
            }
        }
        data
    }

    /// Deserialize a frequency table from the binary stream.
    pub fn deserialize(data: &[u8], offset: &mut usize, alphabet_size: usize) -> Result<Self> {
        let mask_bytes = (alphabet_size + 7) / 8;
        if *offset + mask_bytes > data.len() {
            return Err(anyhow!("Unexpected EOF reading frequency table bitmask"));
        }
        let bitmask = &data[*offset..(*offset + mask_bytes)];
        *offset += mask_bytes;

        let mut freq = vec![0u16; alphabet_size];
        for i in 0..alphabet_size {
            if (bitmask[i / 8] & (1 << (i % 8))) != 0 {
                if *offset >= data.len() {
                    return Err(anyhow!("Unexpected EOF reading frequency value"));
                }
                let f_byte = data[*offset];
                *offset += 1;
                if f_byte < 255 {
                    freq[i] = f_byte as u16;
                } else {
                    if *offset + 2 > data.len() {
                        return Err(anyhow!("Unexpected EOF reading 16-bit frequency"));
                    }
                    let f_val = u16::from_le_bytes([data[*offset], data[*offset + 1]]);
                    freq[i] = f_val;
                    *offset += 2;
                }
            }
        }

        // Build cumulative frequencies and lookup table
        let mut cum_freq = vec![0u16; alphabet_size + 1];
        let mut lookup = vec![0u16; M_SCALE as usize];
        let mut current_cum = 0;
        for i in 0..alphabet_size {
            cum_freq[i] = current_cum;
            let f = freq[i];
            for slot in current_cum..(current_cum + f) {
                if (slot as usize) < lookup.len() {
                    lookup[slot as usize] = i as u16;
                }
            }
            current_cum += f;
        }
        cum_freq[alphabet_size] = current_cum;

        Ok(Self {
            alphabet_size,
            freq,
            cum_freq,
            lookup,
        })
    }
}

/// Standard single-stream rANS encoder (encodes in reverse order).
pub fn encode_single(symbols: &[u16], table: &FreqTable) -> Vec<u8> {
    let mut x = X_MIN as u64;
    let mut words = Vec::new();

    // Encode in reverse order
    for &sym in symbols.iter().rev() {
        let f = table.freq(sym) as u64;
        let c = table.cum_freq(sym) as u64;
        assert!(f > 0, "Attempted to encode symbol with frequency 0");

        // Normalize
        let limit = ((X_MIN as u64 * f) >> L_PRECISION) << 16;
        while x >= limit {
            words.push((x & 0xffff) as u16);
            x >>= 16;
        }

        // Encode
        x = (x / f) * (M_SCALE as u64) + (x % f) + c;
    }

    // Serialize output: final state x (4 bytes) + count of words (4 bytes) + words
    let mut out = Vec::new();
    out.extend_from_slice(&(x as u32).to_le_bytes());
    out.extend_from_slice(&(words.len() as u32).to_le_bytes());
    for &w in &words {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}

/// Standard single-stream rANS decoder (decodes in forward order).
pub fn decode_single(data: &[u8], offset: &mut usize, num_symbols: usize, table: &FreqTable) -> Result<Vec<u16>> {
    if *offset + 8 > data.len() {
        return Err(anyhow!("Unexpected EOF reading rANS single stream header"));
    }
    let mut x = u32::from_le_bytes([data[*offset], data[*offset + 1], data[*offset + 2], data[*offset + 3]]) as u64;
    let num_words = u32::from_le_bytes([data[*offset + 4], data[*offset + 5], data[*offset + 6], data[*offset + 7]]) as usize;
    *offset += 8;

    if *offset + num_words * 2 > data.len() {
        return Err(anyhow!("Unexpected EOF reading rANS single stream words"));
    }

    let mut words = vec![0u16; num_words];
    for i in 0..num_words {
        words[i] = u16::from_le_bytes([data[*offset + i * 2], data[*offset + i * 2 + 1]]);
    }
    *offset += num_words * 2;

    let mut decoded = vec![0u16; num_symbols];
    for i in 0..num_symbols {
        let slot = (x & (M_SCALE as u64 - 1)) as u16;
        let sym = table.lookup(slot);
        decoded[i] = sym;

        let f = table.freq(sym) as u64;
        let c = table.cum_freq(sym) as u64;

        x = f * (x >> L_PRECISION) + slot as u64 - c;

        if x < X_MIN as u64 {
            let word = words.pop().unwrap_or(0);
            x = (x << 16) | (word as u64);
        }
    }

    Ok(decoded)
}

/// Interleaved 4-state rANS encoder (encodes in reverse order).
/// Takes a slice of symbol-table pairs.
pub fn encode_interleaved(symbols: &[(u16, &FreqTable)]) -> Vec<u8> {
    let mut states = [X_MIN as u64; 4];
    let mut words = Vec::new();

    // Encode in reverse order
    for (i, &(sym, table)) in symbols.iter().enumerate().rev() {
        let state_idx = i % 4;
        let f = table.freq(sym) as u64;
        let c = table.cum_freq(sym) as u64;
        assert!(f > 0, "Attempted to encode symbol with frequency 0");

        let mut x = states[state_idx];
        let limit = ((X_MIN as u64 * f) >> L_PRECISION) << 16;
        while x >= limit {
            words.push((x & 0xffff) as u16);
            x >>= 16;
        }

        states[state_idx] = (x / f) * (M_SCALE as u64) + (x % f) + c;
    }

    // Serialize output: 4 final states (16 bytes) + count of words (4 bytes) + words
    let mut out = Vec::new();
    for &state in &states {
        out.extend_from_slice(&(state as u32).to_le_bytes());
    }
    out.extend_from_slice(&(words.len() as u32).to_le_bytes());
    for &w in &words {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}

/// Interleaved 4-state rANS decoder context.
pub struct InterleavedDecoder<'a> {
    states: [u64; 4],
    words: Vec<u16>,
    _phantom: std::marker::PhantomData<&'a ()>,
}

impl<'a> InterleavedDecoder<'a> {
    pub fn new(data: &[u8], offset: &mut usize) -> Result<Self> {
        if *offset + 20 > data.len() {
            return Err(anyhow!("Unexpected EOF reading interleaved rANS header"));
        }

        let mut states = [0u64; 4];
        for i in 0..4 {
            states[i] = u32::from_le_bytes([
                data[*offset + i * 4],
                data[*offset + i * 4 + 1],
                data[*offset + i * 4 + 2],
                data[*offset + i * 4 + 3],
            ]) as u64;
        }
        let num_words = u32::from_le_bytes([
            data[*offset + 16],
            data[*offset + 17],
            data[*offset + 18],
            data[*offset + 19],
        ]) as usize;
        *offset += 20;

        if *offset + num_words * 2 > data.len() {
            return Err(anyhow!("Unexpected EOF reading interleaved rANS words"));
        }

        let mut words = vec![0u16; num_words];
        for i in 0..num_words {
            words[i] = u16::from_le_bytes([data[*offset + i * 2], data[*offset + i * 2 + 1]]);
        }
        *offset += num_words * 2;

        Ok(Self {
            states,
            words,
            _phantom: std::marker::PhantomData,
        })
    }

    /// Decode the next symbol at step `i` (forward order).
    #[inline(always)]
    pub fn decode_symbol(&mut self, i: usize, table: &FreqTable) -> u16 {
        let state_idx = i % 4;
        let mut x = self.states[state_idx];

        let slot = (x & (M_SCALE as u64 - 1)) as u16;
        let sym = table.lookup(slot);

        let f = table.freq(sym) as u64;
        let c = table.cum_freq(sym) as u64;

        x = f * (x >> L_PRECISION) + slot as u64 - c;

        if x < X_MIN as u64 {
            let word = self.words.pop().unwrap_or(0);
            x = (x << 16) | (word as u64);
        }

        self.states[state_idx] = x;
        sym
    }
}
