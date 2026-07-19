use std::net::{TcpListener, TcpStream};
use std::io::{Read, Write};
use anyhow::{anyhow, Result};
use image::GenericImageView;

pub fn start_server(port: u16) -> Result<()> {
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port))?;
    println!("MLZC Web GUI Dashboard server running on http://127.0.0.1:{}", port);
    println!("Press Ctrl+C to stop the server.");

    // Open browser automatically
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/C", "start", &format!("http://127.0.0.1:{}", port)]).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(format!("http://127.0.0.1:{}", port)).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(format!("http://127.0.0.1:{}", port)).spawn();

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = handle_connection(stream) {
                    eprintln!("Error handling connection: {:?}", e);
                }
            }
            Err(e) => eprintln!("Connection failed: {:?}", e),
        }
    }
    Ok(())
}

fn handle_connection(mut stream: TcpStream) -> Result<()> {
    let mut header_buf = Vec::new();
    let mut temp = [0u8; 1024];
    
    let mut content_length = 0;
    let mut request_line = String::new();
    
    loop {
        let n = stream.read(&mut temp)?;
        if n == 0 {
            break;
        }
        header_buf.extend_from_slice(&temp[..n]);
        
        if let Some(pos) = find_subslice(&header_buf, b"\r\n\r\n") {
            let headers_str = String::from_utf8_lossy(&header_buf[..pos]);
            let mut lines = headers_str.lines();
            if let Some(req_line) = lines.next() {
                request_line = req_line.to_string();
            }
            for line in lines {
                if line.to_lowercase().starts_with("content-length:") {
                    if let Some(val_str) = line.split(':').nth(1) {
                        content_length = val_str.trim().parse::<usize>().unwrap_or(0);
                    }
                }
            }
            
            let body_start = pos + 4;
            let mut body_bytes = header_buf[body_start..].to_vec();
            
            while body_bytes.len() < content_length {
                let mut chunk = vec![0u8; (content_length - body_bytes.len()).min(4096)];
                let n = stream.read(&mut chunk)?;
                if n == 0 {
                    break;
                }
                body_bytes.extend_from_slice(&chunk[..n]);
            }
            
            return respond(stream, &request_line, &body_bytes);
        }
    }
    Ok(())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn respond(mut stream: TcpStream, request_line: &str, body: &[u8]) -> Result<()> {
    if request_line.starts_with("POST /api/compress") {
        let mut quality = 85;
        let mut block_size = 8;
        
        if let Some(pos) = request_line.find('?') {
            let query_str = request_line[pos..].split_whitespace().next().unwrap_or("");
            for param in query_str.trim_start_matches('?').split('&') {
                let mut parts = param.split('=');
                if let (Some(key), Some(val)) = (parts.next(), parts.next()) {
                    if key == "quality" {
                        quality = val.parse::<u8>().unwrap_or(85);
                    } else if key == "block_size" {
                        block_size = val.parse::<usize>().unwrap_or(8);
                    }
                }
            }
        }

        // 1. Load image from memory
        let img = image::load_from_memory(body)
            .map_err(|e| anyhow!("Failed to load uploaded image: {}", e))?;
        let (width, height) = img.dimensions();
        let (channels, raw_data) = match img.color() {
            image::ColorType::L8 | image::ColorType::La8 => {
                let luma = img.to_luma8();
                (1u8, luma.into_raw())
            }
            _ => {
                let rgb = img.to_rgb8();
                (3u8, rgb.into_raw())
            }
        };

        // 2. Compress to MLZC
        let compressed_bytes = crate::codec::compress(width, height, channels, &raw_data, block_size, quality)?;

        // 3. Decompress back to verify and generate visual preview
        let (_, _, _, recon_data) = crate::codec::decompress(&compressed_bytes)?;

        // 4. Encode reconstructed pixels back to standard PNG
        let mut png_bytes = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut png_bytes);
        if channels == 1 {
            let luma_img = image::ImageBuffer::<image::Luma<u8>, _>::from_raw(width, height, recon_data.clone())
                .ok_or_else(|| anyhow!("Failed to build luma image buffer"))?;
            image::DynamicImage::ImageLuma8(luma_img).write_to(&mut cursor, image::ImageFormat::Png)?;
        } else {
            let rgb_img = image::ImageBuffer::<image::Rgb<u8>, _>::from_raw(width, height, recon_data.clone())
                .ok_or_else(|| anyhow!("Failed to build rgb image buffer"))?;
            image::DynamicImage::ImageRgb8(rgb_img).write_to(&mut cursor, image::ImageFormat::Png)?;
        }

        // 5. Compute statistics
        let ratio = (raw_data.len() as f64) / (compressed_bytes.len() as f64);
        let bpp = (compressed_bytes.len() as f64 * 8.0) / (width * height) as f64;
        let psnr = calculate_psnr(&raw_data, &recon_data);

        // 6. Generate JSON Response
        let mlz_b64 = to_base64(&compressed_bytes);
        let png_b64 = to_base64(&png_bytes);
        let response_json = format!(
            "{{\"original_size\":{},\"compressed_size\":{},\"ratio\":{},\"bpp\":{},\"psnr\":{},\"mlz_b64\":\"{}\",\"png_b64\":\"{}\"}}",
            raw_data.len(),
            compressed_bytes.len(),
            ratio,
            bpp,
            psnr,
            mlz_b64,
            png_b64
        );

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_json.len(),
            response_json
        );
        stream.write_all(response.as_bytes())?;
    } else if request_line.starts_with("POST /api/decompress") {
        // 1. Decompress
        let (width, height, channels, recon_data) = crate::codec::decompress(body)?;

        // 2. Encode to PNG
        let mut png_bytes = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut png_bytes);
        if channels == 1 {
            let luma_img = image::ImageBuffer::<image::Luma<u8>, _>::from_raw(width, height, recon_data)
                .ok_or_else(|| anyhow!("Failed to build luma image buffer"))?;
            image::DynamicImage::ImageLuma8(luma_img).write_to(&mut cursor, image::ImageFormat::Png)?;
        } else {
            let rgb_img = image::ImageBuffer::<image::Rgb<u8>, _>::from_raw(width, height, recon_data)
                .ok_or_else(|| anyhow!("Failed to build rgb image buffer"))?;
            image::DynamicImage::ImageRgb8(rgb_img).write_to(&mut cursor, image::ImageFormat::Png)?;
        }

        // 3. Generate JSON response
        let png_b64 = to_base64(&png_bytes);
        let response_json = format!(
            "{{\"width\":{},\"height\":{},\"channels\":{},\"png_b64\":\"{}\"}}",
            width,
            height,
            channels,
            png_b64
        );

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_json.len(),
            response_json
        );
        stream.write_all(response.as_bytes())?;
    } else {
        // Default serve UI dashboard
        let html = include_str!("viewer.html");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        stream.write_all(response.as_bytes())?;
    }
    Ok(())
}

fn calculate_psnr(orig: &[u8], recon: &[u8]) -> f64 {
    if orig.len() != recon.len() || orig.is_empty() {
        return 0.0;
    }
    let mut sum_sq_diff = 0.0;
    for (o, r) in orig.iter().zip(recon.iter()) {
        let diff = *o as f64 - *r as f64;
        sum_sq_diff += diff * diff;
    }
    let mse = sum_sq_diff / (orig.len() as f64);
    if mse == 0.0 {
        return 99.9;
    }
    20.0 * 255.0f64.log10() - 10.0 * mse.log10()
}

fn to_base64(data: &[u8]) -> String {
    const CHARSET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        match chunk.len() {
            3 => {
                let b = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
                result.push(CHARSET[((b >> 18) & 63) as usize] as char);
                result.push(CHARSET[((b >> 12) & 63) as usize] as char);
                result.push(CHARSET[((b >> 6) & 63) as usize] as char);
                result.push(CHARSET[(b & 63) as usize] as char);
            }
            2 => {
                let b = ((chunk[0] as u32) << 8) | (chunk[1] as u32);
                result.push(CHARSET[((b >> 10) & 63) as usize] as char);
                result.push(CHARSET[((b >> 4) & 63) as usize] as char);
                result.push(CHARSET[((b << 2) & 63) as usize] as char);
                result.push('=');
            }
            1 => {
                let b = chunk[0] as u32;
                result.push(CHARSET[((b >> 2) & 63) as usize] as char);
                result.push(CHARSET[((b << 4) & 63) as usize] as char);
                result.push('=');
                result.push('=');
            }
            _ => unreachable!(),
        }
    }
    result
}
