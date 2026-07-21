import os
import subprocess
import time
import urllib.request
import numpy as np
import json
from PIL import Image
import matplotlib.pyplot as plt
from skimage.metrics import structural_similarity as ssim
import imagecodecs

def get_ssim(ref_img_path, test_img_path):
    ref = np.array(Image.open(ref_img_path).convert('RGB'))
    test = np.array(Image.open(test_img_path).convert('RGB'))
    score, _ = ssim(ref, test, channel_axis=-1, full=True, data_range=255)
    return score

def run_mlz(input_png, output_mlz, decoded_png, block_size, quality, palette, ycocg, subsample):
    args_comp = ['target/release/mesh_lz_codec.exe', 'compress', '-i', input_png, '-o', output_mlz, '-b', str(block_size)]
    if quality is not None:
        args_comp.extend(['-q', str(quality)])
    if palette:
        args_comp.append('--palette')
    if ycocg:
        args_comp.append('-y')
    if subsample:
        args_comp.append('-s')
    subprocess.run(args_comp, check=True, stdout=subprocess.DEVNULL)
    
    args_decomp = ['target/release/mesh_lz_codec.exe', 'decompress', '-i', output_mlz, '-o', decoded_png]
    subprocess.run(args_decomp, check=True, stdout=subprocess.DEVNULL)

def run_jpeg(input_png, output_jpg, decoded_png, quality):
    img = Image.open(input_png).convert('RGB')
    img.save(output_jpg, 'JPEG', quality=quality)
    img_dec = Image.open(output_jpg)
    img_dec.load()
    img_dec.save(decoded_png)

def run_webp(input_png, output_webp, decoded_png, quality=None, lossless=False):
    img = Image.open(input_png).convert('RGB')
    if lossless:
        img.save(output_webp, 'WEBP', lossless=True)
    else:
        img.save(output_webp, 'WEBP', quality=quality)
    img_dec = Image.open(output_webp)
    img_dec.load()
    img_dec.save(decoded_png)

def run_jxl(input_png, output_jxl, decoded_png, effort=7, distance=1.0, lossless=False):
    img_data = np.array(Image.open(input_png).convert('RGB'))
    t0 = time.perf_counter()
    if lossless:
        jxl_bytes = imagecodecs.jpegxl_encode(img_data, effort=effort, distance=0.0)
    else:
        jxl_bytes = imagecodecs.jpegxl_encode(img_data, effort=effort, distance=distance)
    with open(output_jxl, 'wb') as f:
        f.write(jxl_bytes)
    t1 = time.perf_counter()
    enc_t = (t1 - t0) * 1000.0
    
    t0 = time.perf_counter()
    with open(output_jxl, 'rb') as f:
        jxl_read = f.read()
    dec_data = imagecodecs.jpegxl_decode(jxl_read)
    t1 = time.perf_counter()
    dec_t = (t1 - t0) * 1000.0
    
    Image.fromarray(dec_data).save(decoded_png)
    return enc_t, dec_t

def run_png(input_png, output_png):
    img = Image.open(input_png).convert('RGB')
    img.save(output_png, 'PNG')

def get_criterion_time(action, group, name):
    try:
        # Action is "Encode" or "Decode"
        path = f"target/criterion/{action} {group}/{name}/new/estimates.json"
        with open(path, 'r') as f:
            data = json.load(f)
            # convert nanoseconds to milliseconds
            return data["mean"]["point_estimate"] / 1_000_000.0
    except Exception as e:
        return 0.0

def main():
    os.makedirs('bench_tmp', exist_ok=True)
    print("Building mesh_lz_codec...")
    subprocess.run(['cargo', 'build', '--release'], check=True)
    
    test_img = 'bench_tmp/kodim01.png'
    if not os.path.exists(test_img):
        print("Downloading test image (kodim01.png)...")
        urllib.request.urlretrieve('http://r0k.us/graphics/kodak/kodak/kodim01.png', test_img)
        
    img_pil = Image.open(test_img)
    pixels = img_pil.width * img_pil.height
    
    results = {}
    def add_result(group, name, file_size, ssim_val, enc_time=0.0, dec_time=0.0):
        if group not in results:
            results[group] = []
        bpp = (file_size * 8.0) / pixels
        results[group].append({'name': name, 'bpp': bpp, 'size': file_size, 'ssim': ssim_val, 'enc': enc_time, 'dec': dec_time})

    print("Generating compressed assets and measuring sizes/SSIM...")
    
    for bs in [8, 16]:
        run_mlz(test_img, f'bench_tmp/mlz_l_{bs}.mlz', f'bench_tmp/mlz_l_{bs}.png', block_size=bs, quality=100, palette=False, ycocg=False, subsample=False)
        sz = os.path.getsize(f'bench_tmp/mlz_l_{bs}.mlz')
        add_result('Lossless', f'MLZ {bs}x{bs}', sz, 1.0)
        
    for bs in [8, 16]:
        run_mlz(test_img, f'bench_tmp/mlz_p_{bs}.mlz', f'bench_tmp/mlz_p_{bs}.png', block_size=bs, quality=None, palette=True, ycocg=False, subsample=False)
        sz = os.path.getsize(f'bench_tmp/mlz_p_{bs}.mlz')
        ssim_v = get_ssim(test_img, f'bench_tmp/mlz_p_{bs}.png')
        add_result('Palette', f'MLZ {bs}x{bs}', sz, ssim_v)

    for bs in [8, 16]:
        for q in [10, 30, 50, 70, 90]:
            run_mlz(test_img, f'bench_tmp/mlz_rgb_{bs}_{q}.mlz', f'bench_tmp/mlz_rgb_{bs}_{q}.png', block_size=bs, quality=q, palette=False, ycocg=False, subsample=False)
            sz = os.path.getsize(f'bench_tmp/mlz_rgb_{bs}_{q}.mlz')
            ssim_v = get_ssim(test_img, f'bench_tmp/mlz_rgb_{bs}_{q}.png')
            add_result(f'Lossy RGB {bs}x{bs}', f'q={q}', sz, ssim_v)
            
    for bs in [8, 16]:
        for q in [10, 30, 50, 70, 90]:
            run_mlz(test_img, f'bench_tmp/mlz_y_{bs}_{q}.mlz', f'bench_tmp/mlz_y_{bs}_{q}.png', block_size=bs, quality=q, palette=False, ycocg=True, subsample=False)
            sz = os.path.getsize(f'bench_tmp/mlz_y_{bs}_{q}.mlz')
            ssim_v = get_ssim(test_img, f'bench_tmp/mlz_y_{bs}_{q}.png')
            add_result(f'Lossy YCoCg {bs}x{bs}', f'q={q}', sz, ssim_v)

    for bs in [8, 16]:
        for q in [10, 30, 50, 70, 90]:
            run_mlz(test_img, f'bench_tmp/mlz_ys_{bs}_{q}.mlz', f'bench_tmp/mlz_ys_{bs}_{q}.png', block_size=bs, quality=q, palette=False, ycocg=True, subsample=True)
            sz = os.path.getsize(f'bench_tmp/mlz_ys_{bs}_{q}.mlz')
            ssim_v = get_ssim(test_img, f'bench_tmp/mlz_ys_{bs}_{q}.png')
            add_result(f'Lossy Chroma {bs}x{bs}', f'q={q}', sz, ssim_v)

    for q in [10, 30, 50, 70, 90]:
        run_jpeg(test_img, f'bench_tmp/comp_{q}.jpg', f'bench_tmp/comp_{q}_dec.jpg', q)
        sz = os.path.getsize(f'bench_tmp/comp_{q}.jpg')
        ssim_v = get_ssim(test_img, f'bench_tmp/comp_{q}_dec.jpg')
        add_result('JPEG', f'q={q}', sz, ssim_v)

    for q in [10, 30, 50, 70, 90]:
        run_webp(test_img, f'bench_tmp/comp_{q}.webp', f'bench_tmp/comp_{q}_dec.png', q)
        sz = os.path.getsize(f'bench_tmp/comp_{q}.webp')
        ssim_v = get_ssim(test_img, f'bench_tmp/comp_{q}_dec.png')
        add_result('WebP', f'q={q}', sz, ssim_v)
        
    run_webp(test_img, f'bench_tmp/comp_l.webp', f'bench_tmp/comp_l_dec.png', lossless=True)
    sz = os.path.getsize(f'bench_tmp/comp_l.webp')
    add_result('Lossless', 'WebP Lossless', sz, 1.0)
    
    run_png(test_img, f'bench_tmp/comp.png')
    sz = os.path.getsize(f'bench_tmp/comp.png')
    add_result('Lossless', 'PNG', sz, 1.0)
    
    try:
        for d in [5.0, 3.0, 1.0, 0.5, 0.1]:
            enc_t, dec_t = run_jxl(test_img, f'bench_tmp/comp_{d}.jxl', f'bench_tmp/comp_{d}_dec.png', distance=d)
            sz = os.path.getsize(f'bench_tmp/comp_{d}.jxl')
            ssim_v = get_ssim(test_img, f'bench_tmp/comp_{d}_dec.png')
            add_result('JPEG XL', f'd={d}', sz, ssim_v, enc_time=enc_t, dec_time=dec_t)
            
        enc_t, dec_t = run_jxl(test_img, f'bench_tmp/comp_l.jxl', f'bench_tmp/comp_l_dec.png', lossless=True)
        sz = os.path.getsize(f'bench_tmp/comp_l.jxl')
        add_result('Lossless', 'JPEG XL Lossless', sz, 1.0, enc_time=enc_t, dec_time=dec_t)
    except Exception as e:
        print(f"Failed to run JPEG XL benchmarking: {e}")

    print("\nRunning Criterion master bench to measure precise in-memory timings...")
    subprocess.run(['cargo', 'bench', '--bench', 'master_bench'], check=True)

    print("\nMerging timing data from Criterion estimates...")
    print(f"\n--- Master Benchmarks for {test_img} ({img_pil.width}x{img_pil.height}) ---")
    print(f"{'Group':20} | {'Name':15} | {'BPP':>10} | {'SSIM':>11} | {'Enc':>9} | {'Dec':>9}")
    print("-" * 85)

    for group, group_res in results.items():
        for r in group_res:
            if group == 'JPEG XL' or (group == 'Lossless' and r['name'] == 'JPEG XL Lossless'):
                pass # Keeps the manually measured jxl times
            else:
                r['enc'] = get_criterion_time('Encode', group, r['name'])
                r['dec'] = get_criterion_time('Decode', group, r['name'])
            
            print(f"{group:20} | {r['name']:15} | {r['bpp']:6.3f} bpp | SSIM: {r['ssim']:.4f} | Enc: {r['enc']:7.1f}ms | Dec: {r['dec']:7.1f}ms")

    plt.figure(figsize=(12, 8))
    colors = plt.cm.tab10(np.linspace(0, 1, 10))
    c_idx = 0
    
    for group, group_res in results.items():
        if group == 'Lossless' or group == 'Palette':
            continue
            
        bpps = [r['bpp'] for r in group_res]
        ssims = [r['ssim'] for r in group_res]
        pts = sorted(zip(bpps, ssims))
        if pts:
            x, y = zip(*pts)
            plt.plot(x, y, marker='o', label=group, color=colors[c_idx])
            c_idx += 1

    if 'Palette' in results:
        bpps = [r['bpp'] for r in results['Palette']]
        ssims = [r['ssim'] for r in results['Palette']]
        plt.scatter(bpps, ssims, marker='*', s=150, color='gold', label='Palette', zorder=5)

    plt.xlabel('Bits Per Pixel (bpp) - Lower is better')
    plt.ylabel('SSIM - Higher is better')
    plt.title('Rate-Distortion Comparison (Mesh-LZ vs Standard Formats)')
    plt.legend()
    plt.grid(True)
    plt.savefig('benchmark_rd_curve.png')
    print("\nSaved master plot to benchmark_rd_curve.png")
    
    md_content = ["| Group | Name | BPP | File Size (Bytes) | SSIM | True Enc (ms) | True Dec (ms) |", 
                  "|-------|------|-----|-------------------|------|---------------|---------------|"]
    for group, group_res in results.items():
        for r in group_res:
            md_content.append(f"| {group} | {r['name']} | {r['bpp']:.3f} | {r['size']} | {r['ssim']:.4f} | {r['enc']:.2f} | {r['dec']:.2f} |")
            
    with open('benchmark_table.md', 'w', encoding='utf-8') as f:
        f.write("\n".join(md_content))
    print("Saved master table to benchmark_table.md")
    
if __name__ == '__main__':
    main()
