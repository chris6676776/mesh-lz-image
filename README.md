# Mesh-LZ Image Codec

Mesh-LZ is a custom image compression format written in Rust, featuring an Interleaved rANS entropy coder. It supports highly efficient lossless compression as well as several lossy quantization and color-subsampling modes, making it highly competitive with PNG and JPEG.

## Setup & Installation

Ensure you have Rust and Cargo installed.

# Clone the repository
git clone https://github.com/chris6676776/mesh-lz-image.git
cd newform

# Build the release binary
Mesh-LZ includes a hardware-accelerated desktop application to visually compress and compare images.

```
cargo run --release
```
```
# Run the CLI
target/release/mesh_lz_codec.exe --help
```
### Running the Web GUI Dashboard
Mesh-LZ includes a visual browser dashboard to interactively compress and view images.
```
cargo run --release --bin mesh_lz_codec -- gui --port 8080
```

---

## Compression Modes

Mesh-LZ supports multiple modes to give you fine-grained control over the Rate-Distortion tradeoff.

### 1. Lossless Mode (Default)
**Command:** `mesh_lz_codec compress -i in.png -o out.mlz -q 100 -b 16`

The default mode (quality 100) provides mathematically lossless compression. In benchmarking, Mesh-LZ 16x16 achieves **11.5 bpp**, outperforming standard PNG (15.3 bpp) by roughly 25%.

### 2. Lossy RGB (Quantization)
**Command:** `mesh_lz_codec compress -i in.png -o out.mlz -q <1-99>`

Applies pure scalar quantization to the spatial residuals or frequency domain. Lower quality values yield smaller files.

### 3. Palette Mode (NeuQuant)
**Command:** `mesh_lz_codec compress -i in.png -o out.mlz --palette`

Reduces the 24-bit TrueColor image down to an optimized 8-bit (256 color) palette before lossless encoding. This is highly efficient for graphics but introduces lossy color banding in photographs.

### 4. YCoCg-R Color Decorrelation
**Command:** `mesh_lz_codec compress -i in.png -o out.mlz -y -q <1-99>`

Instead of compressing in RGB, the image is converted to the reversible YCoCg-R color space. This separates brightness (Luma) from color (Chroma), which typically compresses much better mathematically.

### 5. Chroma Subsampling (YCoCg 4:2:0)
**Command:** `mesh_lz_codec compress -i in.png -o out.mlz -y -s -q <1-99>`

The ultimate mode for web compression. Human eyes are less sensitive to color resolution than brightness. This mode preserves full brightness detail but discards 75% of the color resolution. It drastically reduces file size with minimal visual impact.

---

### Rate-Distortion Curve
*(Note: Plot generated locally via the benchmarking script)*

![Rate-Distortion Curve](benchmark_rd_curve.png)

### Benchmark Data Table


| Group | Name | BPP | File Size (Bytes) | SSIM | Enc (ms) | Dec (ms) |
|-------|------|-----|-------------------|------|---------------|---------------|
| Lossless | MLZ 8x8 | 11.704 | 575270 | 1.0000 | 10.15 | 3.70 |
| Lossless | MLZ 16x16 | 11.514 | 565944 | 1.0000 | 12.07 | 3.64 |
| Lossless | WebP Lossless | 10.223 | 502480 | 1.0000 | 297.80 | 8.51 |
| Lossless | PNG | 15.339 | 753927 | 1.0000 | 3.45 | 3.96 |
| Lossless | JPEG XL Lossless | 18.829 | 925506 | 1.0000 | 419.51 | 39.17 |
| Palette | MLZ 8x8 | 6.230 | 306241 | 0.9906 | 61.25 | 2.75 |
| Palette | MLZ 16x16 | 6.161 | 302821 | 0.9906 | 60.11 | 2.61 |
| Lossy RGB 8x8 | q=10 | 5.248 | 257932 | 0.9655 | 18.47 | 3.45 |
| Lossy RGB 8x8 | q=30 | 5.925 | 291238 | 0.9758 | 19.14 | 3.52 |
| Lossy RGB 8x8 | q=50 | 6.672 | 327926 | 0.9838 | 20.58 | 3.57 |
| Lossy RGB 8x8 | q=70 | 7.947 | 390619 | 0.9921 | 21.85 | 4.26 |
| Lossy RGB 8x8 | q=90 | 10.384 | 510379 | 0.9970 | 22.42 | 3.78 |
| Lossy RGB 16x16 | q=10 | 5.082 | 249781 | 0.9655 | 20.21 | 3.35 |
| Lossy RGB 16x16 | q=30 | 5.778 | 283998 | 0.9756 | 20.49 | 3.43 |
| Lossy RGB 16x16 | q=50 | 6.517 | 320306 | 0.9838 | 21.04 | 3.48 |
| Lossy RGB 16x16 | q=70 | 7.774 | 382108 | 0.9921 | 32.07 | 3.60 |
| Lossy RGB 16x16 | q=90 | 10.187 | 500715 | 0.9970 | 23.40 | 3.75 |
| Lossy YCoCg 8x8 | q=10 | 4.664 | 229253 | 0.9513 | 18.51 | 4.10 |
| Lossy YCoCg 8x8 | q=30 | 5.255 | 258295 | 0.9650 | 18.95 | 4.14 |
| Lossy YCoCg 8x8 | q=50 | 6.084 | 299060 | 0.9752 | 20.11 | 4.19 |
| Lossy YCoCg 8x8 | q=70 | 6.978 | 342983 | 0.9872 | 20.32 | 4.38 |
| Lossy YCoCg 8x8 | q=90 | 8.992 | 441992 | 0.9950 | 22.04 | 4.42 |
| Lossy YCoCg 16x16 | q=10 | 4.460 | 219197 | 0.9513 | 19.65 | 4.02 |
| Lossy YCoCg 16x16 | q=30 | 5.046 | 248014 | 0.9650 | 20.27 | 4.01 |
| Lossy YCoCg 16x16 | q=50 | 5.858 | 287955 | 0.9752 | 21.46 | 4.09 |
| Lossy YCoCg 16x16 | q=70 | 6.739 | 331248 | 0.9872 | 21.73 | 4.13 |
| Lossy YCoCg 16x16 | q=90 | 8.736 | 429380 | 0.9950 | 24.84 | 4.27 |
| Lossy Chroma 8x8 | q=10 | 4.469 | 219651 | 0.9508 | 18.82 | 4.06 |
| Lossy Chroma 8x8 | q=30 | 5.071 | 249226 | 0.9635 | 19.18 | 4.17 |
| Lossy Chroma 8x8 | q=50 | 5.824 | 286263 | 0.9731 | 19.82 | 4.19 |
| Lossy Chroma 8x8 | q=70 | 6.672 | 327959 | 0.9845 | 20.44 | 4.24 |
| Lossy Chroma 8x8 | q=90 | 8.654 | 425365 | 0.9923 | 23.10 | 4.38 |
| Lossy Chroma 16x16 | q=10 | 4.265 | 209611 | 0.9507 | 20.17 | 3.98 |
| Lossy Chroma 16x16 | q=30 | 4.860 | 238887 | 0.9635 | 20.35 | 4.01 |
| Lossy Chroma 16x16 | q=50 | 5.608 | 275627 | 0.9730 | 20.98 | 4.22 |
| Lossy Chroma 16x16 | q=70 | 6.443 | 316685 | 0.9845 | 21.76 | 4.10 |
| Lossy Chroma 16x16 | q=90 | 8.425 | 414116 | 0.9923 | 23.07 | 4.23 |
| JPEG | q=10 | 0.440 | 21619 | 0.7074 | 13.76 | 2.81 |
| JPEG | q=30 | 0.922 | 45334 | 0.8506 | 15.02 | 3.15 |
| JPEG | q=50 | 1.257 | 61794 | 0.8911 | 15.89 | 3.31 |
| JPEG | q=70 | 1.708 | 83969 | 0.9275 | 16.85 | 3.58 |
| JPEG | q=90 | 3.153 | 154983 | 0.9313 | 19.77 | 4.41 |
| WebP | q=10 | 0.453 | 22260 | 0.7826 | 33.35 | 3.08 |
| WebP | q=30 | 0.827 | 40638 | 0.8810 | 37.47 | 4.20 |
| WebP | q=50 | 1.160 | 57012 | 0.9244 | 40.82 | 5.14 |
| WebP | q=70 | 1.493 | 73406 | 0.9493 | 43.73 | 6.07 |
| WebP | q=90 | 2.786 | 136962 | 0.9853 | 52.87 | 9.54 |
| JPEG XL | d=5.0 | 0.644 | 31655 | 0.8263 | 481.79 | 47.35 |
| JPEG XL | d=3.0 | 1.058 | 51999 | 0.9028 | 356.01 | 22.34 |
| JPEG XL | d=1.0 | 2.526 | 124151 | 0.9803 | 352.68 | 25.21 |
| JPEG XL | d=0.5 | 3.661 | 179934 | 0.9904 | 342.82 | 18.96 |
| JPEG XL | d=0.1 | 7.457 | 366549 | 0.9978 | 398.39 | 27.06 |
