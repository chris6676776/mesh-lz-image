use std::sync::OnceLock;

static STENCILS_8: [OnceLock<Vec<(usize, usize)>>; 8] = [
    OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new(),
    OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new(),
];

static STENCILS_16: [OnceLock<Vec<(usize, usize)>>; 8] = [
    OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new(),
    OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new(),
];

pub fn get_stencil(stencil_idx: u8, b: usize) -> &'static [(usize, usize)] {
    let idx = (stencil_idx % 8) as usize;
    if b == 8 {
        STENCILS_8[idx].get_or_init(|| generate_stencil(idx as u8, b)).as_slice()
    } else if b == 16 {
        STENCILS_16[idx].get_or_init(|| generate_stencil(idx as u8, b)).as_slice()
    } else {
        panic!("Unsupported block size for stencil caching: {}", b);
    }
}

fn generate_stencil(stencil_idx: u8, b: usize) -> Vec<(usize, usize)> {
    match stencil_idx {
        0 => generate_hilbert(b),
        1 => generate_raster(b),
        2 => generate_column(b),
        3 => generate_snake_raster(b),
        4 => generate_column_snake(b),
        5 => generate_zigzag(b),
        6 => generate_spiral(b),
        7 => generate_morton(b),
        _ => generate_hilbert(b), // Fallback
    }
}

fn generate_raster(b: usize) -> Vec<(usize, usize)> {
    let mut path = Vec::with_capacity(b * b);
    for y in 0..b {
        for x in 0..b {
            path.push((y, x));
        }
    }
    path
}

fn generate_column(b: usize) -> Vec<(usize, usize)> {
    let mut path = Vec::with_capacity(b * b);
    for x in 0..b {
        for y in 0..b {
            path.push((y, x));
        }
    }
    path
}

fn generate_snake_raster(b: usize) -> Vec<(usize, usize)> {
    let mut path = Vec::with_capacity(b * b);
    for y in 0..b {
        if y % 2 == 0 {
            for x in 0..b {
                path.push((y, x));
            }
        } else {
            for x in (0..b).rev() {
                path.push((y, x));
            }
        }
    }
    path
}

fn generate_column_snake(b: usize) -> Vec<(usize, usize)> {
    let mut path = Vec::with_capacity(b * b);
    for x in 0..b {
        if x % 2 == 0 {
            for y in 0..b {
                path.push((y, x));
            }
        } else {
            for y in (0..b).rev() {
                path.push((y, x));
            }
        }
    }
    path
}

fn generate_zigzag(b: usize) -> Vec<(usize, usize)> {
    let mut path = Vec::with_capacity(b * b);
    let limit = 2 * b - 1;
    for d in 0..limit {
        if d % 2 == 0 {
            // Even diagonal: bottom-left to top-right
            let start = d.min(b - 1);
            let end = if d >= b { d - b + 1 } else { 0 };
            let mut y = start;
            loop {
                let x = d - y;
                path.push((y, x));
                if y == end {
                    break;
                }
                y -= 1;
            }
        } else {
            // Odd diagonal: top-right to bottom-left
            let start = d.min(b - 1);
            let end = if d >= b { d - b + 1 } else { 0 };
            let mut x = start;
            loop {
                let y = d - x;
                path.push((y, x));
                if x == end {
                    break;
                }
                x -= 1;
            }
        }
    }
    path
}

fn generate_spiral(b: usize) -> Vec<(usize, usize)> {
    let mut path = Vec::with_capacity(b * b);
    if b == 0 {
        return path;
    }
    let mut ymin = 0;
    let mut ymax = b - 1;
    let mut xmin = 0;
    let mut xmax = b - 1;

    while ymin <= ymax && xmin <= xmax {
        // Go right
        for x in xmin..=xmax {
            path.push((ymin, x));
        }
        if ymin == ymax {
            break;
        }
        ymin += 1;

        // Go down
        for y in ymin..=ymax {
            path.push((y, xmax));
        }
        if xmin == xmax {
            break;
        }
        xmax -= 1;

        // Go left
        for x in (xmin..=xmax).rev() {
            path.push((ymax, x));
        }
        ymax -= 1;

        // Go up
        for y in (ymin..=ymax).rev() {
            path.push((y, xmin));
        }
        xmin += 1;
    }
    path
}

fn generate_morton(b: usize) -> Vec<(usize, usize)> {
    let mut path = Vec::with_capacity(b * b);
    for i in 0..(b * b) {
        let mut x = 0;
        let mut y = 0;
        for bit in 0..8 {
            x |= ((i >> (2 * bit)) & 1) << bit;
            y |= ((i >> (2 * bit + 1)) & 1) << bit;
        }
        path.push((y, x));
    }
    path
}

fn generate_hilbert(b: usize) -> Vec<(usize, usize)> {
    let mut path = Vec::with_capacity(b * b);
    for d in 0..(b * b) {
        path.push(d2xy(b, d));
    }
    path
}

fn d2xy(n: usize, mut d: usize) -> (usize, usize) {
    let mut x = 0;
    let mut y = 0;
    let mut s = 1;
    while s < n {
        let rx = 1 & (d / 2);
        let ry = 1 & (d ^ rx);
        rot(s, &mut x, &mut y, rx, ry);
        x += s * rx;
        y += s * ry;
        d /= 4;
        s *= 2;
    }
    (y, x)
}

fn rot(n: usize, x: &mut usize, y: &mut usize, rx: usize, ry: usize) {
    if ry == 0 {
        if rx == 1 {
            *x = n - 1 - *x;
            *y = n - 1 - *y;
        }
        let temp = *x;
        *x = *y;
        *y = temp;
    }
}
