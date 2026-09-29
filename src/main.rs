use std::path::PathBuf;
use std::io::{self, BufRead, Write};
use chrono::Local;
use regex::Regex;
use tempfile::NamedTempFile;
use image::{DynamicImage, ImageBuffer, Luma};
use image::imageops;
use colored::Colorize;

fn get_timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn log_info(message: &str) {
    println!("{}", format!("{} - INFO - {}", get_timestamp(), message).blue());
}

fn log_warning(message: &str) {
    println!("{}", format!("{} - WARNING - {}", get_timestamp(), message).yellow());
}

fn log_error(message: &str) {
    eprintln!("{}", format!("{} - ERROR - {}", get_timestamp(), message).red());
}

fn is_url(path: &str) -> bool {
    let url_pattern = Regex::new(
        r"^https?://[\w-]+(\.[\w-]+)+(/\S*)?\.(png|jpg|jpeg|gif|bmp|webp)(\?\S*)?$"
    ).unwrap();
    url_pattern.is_match(path)
}

fn download_image(url: &str) -> Option<PathBuf> {
    if !is_url(url) {
        log_error(&format!("无效的 URL: {}", url));
        return None;
    }

    log_info(&format!("正在下载图片: {}", url));

    let client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .ok()?;

    let response = client.get(url).send().ok()?;
    
    if !response.status().is_success() {
        log_error(&format!("HTTP 错误: {}", response.status()));
        return None;
    }

    let content_type = response.headers()
        .get("Content-Type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    
    if !content_type.starts_with("image/") {
        log_warning(&format!("URL 内容不是图片类型: {}", content_type));
    }

    let bytes = response.bytes().ok()?;
    
    let mut temp_file = NamedTempFile::new().ok()?;
    use std::io::Write;
    temp_file.write_all(&bytes).ok()?;
    
    let path = temp_file.path().to_path_buf();
    std::mem::forget(temp_file);
    
    log_info(&format!("图片下载成功: {} ({} KB)", path.display(), bytes.len() / 1024));
    
    Some(path)
}

fn preprocess_image(img: &DynamicImage) -> Vec<(String, DynamicImage)> {
    let mut images = Vec::new();

    images.push(("原始图像".to_string(), img.clone()));

    let gray_img = img.to_luma8();
    images.push(("灰度图像".to_string(), DynamicImage::ImageLuma8(gray_img.clone())));

    let contrast_img = imageops::contrast(&gray_img, 2.0);
    images.push(("高对比度图像".to_string(), DynamicImage::ImageLuma8(contrast_img.clone())));

    let threshold = 128u8;
    let binary_img: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::from_fn(
        contrast_img.width(),
        contrast_img.height(),
        |x, y| {
            let pixel = contrast_img.get_pixel(x, y);
            if pixel[0] < threshold {
                Luma([0u8])
            } else {
                Luma([255u8])
            }
        }
    );
    images.push(("二值化图像".to_string(), DynamicImage::ImageLuma8(binary_img)));

    for threshold in [64u8, 192u8] {
        let binary_img: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::from_fn(
            contrast_img.width(),
            contrast_img.height(),
            |x, y| {
                let pixel = contrast_img.get_pixel(x, y);
                if pixel[0] < threshold {
                    Luma([0u8])
                } else {
                    Luma([255u8])
                }
            }
        );
        images.push((format!("二值化图像(阈值{})", threshold), DynamicImage::ImageLuma8(binary_img)));
    }

    let blurred = imageops::blur(&gray_img, 1.0);
    let blurred_binary: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::from_fn(
        blurred.width(),
        blurred.height(),
        |x, y| {
            let pixel = blurred.get_pixel(x, y);
            if pixel[0] < 128 {
                Luma([0u8])
            } else {
                Luma([255u8])
            }
        }
    );
    images.push(("高斯模糊+二值化".to_string(), DynamicImage::ImageLuma8(blurred_binary)));

    let sharpened = img.to_rgba8();
    let sharpened = imageops::unsharpen(&sharpened, 1.0, 0);
    images.push(("锐化图像".to_string(), DynamicImage::ImageRgba8(sharpened)));

    let inverted: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::from_fn(
        gray_img.width(),
        gray_img.height(),
        |x, y| {
            let pixel = gray_img.get_pixel(x, y);
            Luma([255u8 - pixel[0]])
        }
    );
    images.push(("颜色反转图像".to_string(), DynamicImage::ImageLuma8(inverted)));

    let large_img = img.resize(img.width() * 2, img.height() * 2, imageops::FilterType::Lanczos3);
    images.push(("放大图像".to_string(), large_img));

    images
}

fn decode_qr_image(img: &DynamicImage) -> Option<String> {
    let results = bardecoder::default_decoder().decode(img);
    
    for result in results {
        if let Ok(text) = result {
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    
    None
}

fn decode_qrcode(image_path: &PathBuf) {
    log_info(&format!("开始解码二维码: {}", image_path.display()));

    let mut img = match image::open(image_path) {
        Ok(img) => img,
        Err(e) => {
            log_error(&format!("无法打开文件: {}", e));
            return;
        }
    };

    let processed_images = preprocess_image(&img);
    
    for (name, processed_img) in &processed_images {
        if let Some(text) = decode_qr_image(processed_img) {
            log_info(&format!("使用 {} 成功解码", name));
            log_info(&format!("二维码内容: {}", text));
            return;
        }
    }

    if let Some(text) = decode_qr_image(&img) {
        log_info(&format!("二维码内容: {}", text));
        return;
    }

    log_warning("标准方法未能解码二维码，尝试额外的处理方法");

    let width = img.width();
    let height = img.height();
    
    let crops = [
        ("左上角", (0, 0, width / 2, height / 2)),
        ("右上角", (width / 2, 0, width, height / 2)),
        ("左下角", (0, height / 2, width / 2, height)),
        ("右下角", (width / 2, height / 2, width, height)),
    ];

    for (name, (x, y, x2, y2)) in &crops {
        let cropped = img.crop(*x, *y, x2 - x, y2 - y);
        if let Some(text) = decode_qr_image(&cropped) {
            log_info(&format!("在裁剪区域 {} 中成功解码", name));
            log_info(&format!("二维码内容: {}", text));
            return;
        }
    }

    log_error("经过所有尝试后仍然无法解码二维码");
    log_info("建议：确保二维码清晰完整，或尝试使用专门的二维码应用扫描");
}

fn process_input(input_path: &str) {
    let mut downloaded_path: Option<PathBuf> = None;
    
    if is_url(input_path) {
        log_info("检测到 URL，开始下载图片...");
        if let Some(path) = download_image(input_path) {
            decode_qrcode(&path);
            downloaded_path = Some(path);
        } else {
            log_error("图片下载失败");
        }
    } else {
        decode_qrcode(&PathBuf::from(input_path));
    }

    if let Some(path) = downloaded_path {
        let _ = std::fs::remove_file(path);
    }
}

fn main() {
    print!("请输入二维码图片的路径或 URL（多个用逗号分隔）: ");
    io::stdout().flush().expect("无法刷新输出缓冲区");
    
    let stdin = io::stdin();
    let mut input = String::new();
    stdin.lock().read_line(&mut input).expect("无法读取输入");
    let input = input.trim();

    if input.is_empty() {
        log_error("未提供输入路径");
        std::process::exit(1);
    }

    let paths: Vec<&str> = input.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();

    if paths.is_empty() {
        log_error("未提供有效的路径");
        std::process::exit(1);
    }

    for (i, path_str) in paths.iter().enumerate() {
        if paths.len() > 1 {
            log_info(&format!("正在处理第 {} 个: {}", i + 1, path_str));
        }
        process_input(path_str);
    }
}