use std::path::PathBuf;
use std::io::{self, BufRead, Write};
use chrono::Local;
use regex::Regex;
use tempfile::NamedTempFile;
use image::{DynamicImage, ImageBuffer, Luma};
use image::imageops;
use colored::Colorize;

/// 获取当前时间戳，格式为 "年-月-日 时:分:秒"
fn get_timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 输出蓝色 INFO 级别日志
fn log_info(message: &str) {
    println!("{}", format!("{} - INFO - {}", get_timestamp(), message).blue());
}

/// 输出黄色 WARNING 级别日志
fn log_warning(message: &str) {
    println!("{}", format!("{} - WARNING - {}", get_timestamp(), message).yellow());
}

/// 输出红色 ERROR 级别日志（到标准错误流）
fn log_error(message: &str) {
    eprintln!("{}", format!("{} - ERROR - {}", get_timestamp(), message).red());
}

/// 判断字符串是否为有效的图片 URL
/// 支持 http/https 协议，以及 png/jpg/jpeg/gif/bmp/webp 格式
fn is_url(path: &str) -> bool {
    let url_pattern = Regex::new(
        r"^https?://[\w-]+(\.[\w-]+)+(/\S*)?\.(png|jpg|jpeg|gif|bmp|webp)(\?\S*)?$"
    ).unwrap();
    url_pattern.is_match(path)
}

/// 从 URL 下载图片到临时文件
/// 返回临时文件路径，失败时返回 None
fn download_image(url: &str) -> Option<PathBuf> {
    if !is_url(url) {
        log_error(&format!("无效的 URL: {}", url));
        return None;
    }

    log_info(&format!("正在下载图片: {}", url));

    // 创建 HTTP 客户端，设置浏览器 User-Agent 和 30 秒超时
    let client = match reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log_error(&format!("创建 HTTP 客户端失败: {}", e));
            return None;
        }
    };

    let response = match client.get(url).send() {
        Ok(r) => r,
        Err(e) => {
            log_error(&format!("网络请求失败: {}", e));
            return None;
        }
    };
    
    if !response.status().is_success() {
        log_error(&format!("HTTP 错误: {}", response.status()));
        return None;
    }

    // 验证 Content-Type 是否为图片类型
    let content_type = response.headers()
        .get("Content-Type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    
    if !content_type.starts_with("image/") {
        log_warning(&format!("URL 内容不是图片类型: {}", content_type));
    }

    let bytes = match response.bytes() {
        Ok(b) => b,
        Err(e) => {
            log_error(&format!("读取响应体失败: {}", e));
            return None;
        }
    };
    
    // 创建临时文件并写入图片数据
    let mut temp_file = match NamedTempFile::new() {
        Ok(f) => f,
        Err(e) => {
            log_error(&format!("创建临时文件失败: {}", e));
            return None;
        }
    };
    use std::io::Write;
    if let Err(e) = temp_file.write_all(&bytes) {
        log_error(&format!("写入临时文件失败: {}", e));
        return None;
    }
    
    // 获取临时文件路径，防止文件被自动删除
    let path = temp_file.path().to_path_buf();
    std::mem::forget(temp_file);
    
    log_info(&format!("图片下载成功: {} ({} KB)", path.display(), bytes.len() / 1024));
    
    Some(path)
}

/// 对图像进行多种预处理操作，返回预处理后的图像列表
/// 包括：原始、灰度、对比度增强、二值化（多阈值）、模糊+二值化、锐化、颜色反转、放大
fn preprocess_image(img: &DynamicImage) -> Vec<(String, DynamicImage)> {
    let mut images = Vec::new();

    // 原始图像
    images.push(("原始图像".to_string(), img.clone()));

    // 转换为灰度图像
    let gray_img = img.to_luma8();
    images.push(("灰度图像".to_string(), DynamicImage::ImageLuma8(gray_img.clone())));

    // 增强对比度（2.0 倍）
    let contrast_img = imageops::contrast(&gray_img, 2.0);
    images.push(("高对比度图像".to_string(), DynamicImage::ImageLuma8(contrast_img.clone())));

    // 二值化处理（阈值 128）
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

    // 二值化处理（其他阈值：64 和 192）
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

    // 高斯模糊 + 二值化
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

    // 锐化处理
    let sharpened = img.to_rgba8();
    let sharpened = imageops::unsharpen(&sharpened, 1.0, 0);
    images.push(("锐化图像".to_string(), DynamicImage::ImageRgba8(sharpened)));

    // 颜色反转
    let inverted: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::from_fn(
        gray_img.width(),
        gray_img.height(),
        |x, y| {
            let pixel = gray_img.get_pixel(x, y);
            Luma([255u8 - pixel[0]])
        }
    );
    images.push(("颜色反转图像".to_string(), DynamicImage::ImageLuma8(inverted)));

    // 图像放大（2 倍，使用 Lanczos3 算法）
    let large_img = img.resize(img.width() * 2, img.height() * 2, imageops::FilterType::Lanczos3);
    images.push(("放大图像".to_string(), large_img));

    images
}

/// 尝试解码单个图像中的二维码
/// 成功时返回二维码内容，失败时返回 None
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

/// 解码指定图片路径中的二维码
/// 采用多种预处理方法和裁剪策略提高识别率
fn decode_qrcode(image_path: &PathBuf) {
    log_info(&format!("开始解码二维码: {}", image_path.display()));

    let mut img = match image::open(image_path) {
        Ok(img) => img,
        Err(e) => {
            log_error(&format!("无法打开文件: {}", e));
            return;
        }
    };

    // 尝试所有预处理后的图像
    let processed_images = preprocess_image(&img);
    
    for (name, processed_img) in &processed_images {
        if let Some(text) = decode_qr_image(processed_img) {
            log_info(&format!("使用 {} 成功解码", name));
            log_info(&format!("二维码内容: {}", text));
            return;
        }
    }

    // 再次尝试原始图像
    if let Some(text) = decode_qr_image(&img) {
        log_info(&format!("二维码内容: {}", text));
        return;
    }

    // 预处理失败，尝试裁剪图像四角
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

/// 处理单个输入路径（本地路径或 URL）
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

    // 清理下载的临时文件
    if let Some(path) = downloaded_path {
        let _ = std::fs::remove_file(path);
    }
}

/// 主函数：程序入口
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

    // 按逗号分割输入，去除空白并过滤空字符串
    let paths: Vec<&str> = input.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();

    if paths.is_empty() {
        log_error("未提供有效的路径");
        std::process::exit(1);
    }

    // 依次处理每个路径
    for (i, path_str) in paths.iter().enumerate() {
        if paths.len() > 1 {
            log_info(&format!("正在处理第 {} 个: {}", i + 1, path_str));
        }
        process_input(path_str);
    }
}