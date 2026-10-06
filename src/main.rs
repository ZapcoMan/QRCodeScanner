use std::path::PathBuf;
use std::ffi::OsStr;
use std::io::{self, BufRead, Write};
use chrono::Local;
use regex::Regex;
use tempfile::NamedTempFile;
use image::{DynamicImage, ImageBuffer, Luma};
use image::imageops;
use colored::Colorize;
use rxing::helpers::detect_in_luma_slice;
use rxing::BarcodeFormat;

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
/// 支持 http/https 协议，大小写不敏感的 png/jpg/jpeg/gif/bmp/webp 扩展名
/// 同时兼容带端口、预签名 query、页内锚点 fragment 等常见形式
fn is_url(path: &str) -> bool {
    let url_pattern = Regex::new(
        r"(?i)^https?://\S+\.(png|jpg|jpeg|gif|bmp|webp)(\?\S*)?(#\S*)?$"
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

/// 解码结果结构体，包含文本内容和条码格式
struct DecodeResult {
    text: String,
    format: BarcodeFormat,
}

/// 将 DynamicImage 转换为灰度数据并尝试解码（二维码 + 条形码）
/// 成功时返回解码结果（含格式信息），失败时返回 None
fn decode_barcode_image(img: &DynamicImage) -> Option<DecodeResult> {
    let luma = img.to_luma8();
    let (width, height) = (luma.width(), luma.height());
    let luma_data: &[u8] = luma.as_raw();

    match detect_in_luma_slice(luma_data, width, height, None) {
        Ok(result) => {
            let text = result.getText().to_string();
            if !text.is_empty() {
                return Some(DecodeResult {
                    text,
                    format: *result.getBarcodeFormat(),
                });
            }
        }
        Err(_) => {}
    }

    None
}

/// 解码指定图片路径中的条码（支持二维码和一维条形码）
/// 采用多种预处理方法和裁剪策略提高识别率
fn decode_image(image_path: &PathBuf) {
    log_info(&format!("开始解码: {}", image_path.display()));

    let mut img = match image::open(image_path) {
        Ok(img) => img,
        Err(e) => {
            log_error(&format!("无法打开文件: {}", e));
            return;
        }
    };

    // 尝试所有预处理后的图像（列表首项即为"原始图像"，无需额外重复解码）
    let processed_images = preprocess_image(&img);

    for (name, processed_img) in &processed_images {
        if let Some(result) = decode_barcode_image(processed_img) {
            log_info(&format!("使用 {} 成功解码", name));
            log_info(&format!("条码格式: {}", format_display_name(&result.format)));
            log_info(&format!("解码内容: {}", result.text));
            return;
        }
    }

    // 预处理失败，尝试裁剪图像四角
    log_warning("标准方法未能解码，尝试额外的裁剪处理方法");

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
        if let Some(result) = decode_barcode_image(&cropped) {
            log_info(&format!("在裁剪区域 {} 中成功解码", name));
            log_info(&format!("条码格式: {}", format_display_name(&result.format)));
            log_info(&format!("解码内容: {}", result.text));
            return;
        }
    }

    // 水平条带扫描（针对一维条形码常出现在图像中某个水平条带的情况）
    log_warning("裁剪方法未能解码，尝试水平条带扫描");

    let strip_height = (height / 6).max(1);
    let mut y = 0u32;
    while y + strip_height <= height {
        let strip = img.crop(0, y, width, strip_height);
        if let Some(result) = decode_barcode_image(&strip) {
            log_info(&format!("在水平条带(y={}, h={})中成功解码", y, strip_height));
            log_info(&format!("条码格式: {}", format_display_name(&result.format)));
            log_info(&format!("解码内容: {}", result.text));
            return;
        }
        y += strip_height / 2; // 50% 重叠滑动
    }

    log_error("经过所有尝试后仍然无法解码");
    log_info("建议：确保条码清晰完整，光照均匀，或尝试使用专业扫码应用");
}

/// 将 BarcodeFormat 枚举转为易读的中文显示名称
fn format_display_name(fmt: &BarcodeFormat) -> &'static str {
    match fmt {
        BarcodeFormat::QR_CODE => "二维码 (QR Code)",
        BarcodeFormat::MICRO_QR_CODE => "微型二维码 (Micro QR)",
        BarcodeFormat::RECTANGULAR_MICRO_QR_CODE => "矩形微型二维码 (rMQR)",
        BarcodeFormat::EAN_13 => "EAN-13 商品条码",
        BarcodeFormat::EAN_8 => "EAN-8 商品条码",
        BarcodeFormat::UPC_A => "UPC-A 商品条码",
        BarcodeFormat::UPC_E => "UPC-E 商品条码",
        BarcodeFormat::CODE_128 => "Code 128 条码",
        BarcodeFormat::CODE_39 => "Code 39 条码",
        BarcodeFormat::CODE_93 => "Code 93 条码",
        BarcodeFormat::CODABAR => "Codabar 条码",
        BarcodeFormat::ITF => "ITF (二五码) 条码",
        BarcodeFormat::DATA_MATRIX => "Data Matrix 码",
        BarcodeFormat::AZTEC => "Aztec 码",
        BarcodeFormat::PDF_417 => "PDF417 条码",
        BarcodeFormat::RSS_14 => "RSS-14 条码",
        BarcodeFormat::RSS_EXPANDED => "RSS-Expanded 条码",
        BarcodeFormat::MAXICODE => "MaxiCode 条码",
        BarcodeFormat::TELEPEN => "Telepen 条码",
        BarcodeFormat::UPC_EAN_EXTENSION => "UPC/EAN 扩展码",
        _ => "未知格式",
    }
}

/// 处理单个输入路径（本地路径或 URL）
fn process_input(input_path: &str) {
    let mut downloaded_path: Option<PathBuf> = None;
    
    if is_url(input_path) {
        log_info("检测到 URL，开始下载图片...");
        if let Some(path) = download_image(input_path) {
            decode_image(&path);
            downloaded_path = Some(path);
        } else {
            log_error("图片下载失败");
        }
    } else {
        decode_image(&PathBuf::from(input_path));
    }

    // 清理下载的临时文件
    if let Some(path) = downloaded_path {
        let _ = std::fs::remove_file(path);
    }
}

/// 判断当前控制台是否只由本进程独占
/// 双击启动时 Windows 会为进程新建控制台，列表里只有我们自己；
/// 从已有终端启动时，宿主 shell 也在同一控制台内，数量大于 1
#[cfg(windows)]
fn owns_console_alone() -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleProcessList(process_list: *mut u32, max_count: u32) -> u32;
    }

    let mut pids = [0u32; 4];
    let count = unsafe { GetConsoleProcessList(pids.as_mut_ptr(), pids.len() as u32) };
    count == 1
}

/// 在当前控制台内另起一个交互式 PowerShell，使本进程退出后窗口仍然保留
/// CREATE_NEW_PROCESS_GROUP 让新 shell 不响应本进程收到的 Ctrl+C 广播
#[cfg(windows)]
fn spawn_persistent_shell(program: &std::ffi::OsStr) -> io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0200;

    std::process::Command::new(program)
        .arg("-NoExit")
        .creation_flags(CREATE_NEW_PROCESS_GROUP)
        .spawn()
}

/// 选择可用的 PowerShell：优先 PowerShell 7（pwsh），回退系统自带的 Windows PowerShell 5.1
#[cfg(windows)]
fn keep_console_open() {
    if !owns_console_alone() {
        return;
    }

    // pwsh 没有固定安装路径，只能靠 PATH 查找；起不来就说明未安装，直接回退
    if spawn_persistent_shell(OsStr::new("pwsh")).is_ok() {
        return;
    }

    // 5.1 用绝对路径，避免用户改过 PATH 后找不到
    let legacy = std::env::var("SystemRoot")
        .map(|root| {
            PathBuf::from(root)
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        })
        .unwrap_or_else(|_| PathBuf::from("powershell.exe"));

    let _ = spawn_persistent_shell(legacy.as_os_str());
}

/// 控制台事件回调：先留下可用的 shell，再交回系统默认终止流程
#[cfg(windows)]
unsafe extern "system" fn console_event_handler(_event_type: u32) -> i32 {
    keep_console_open();
    0
}

/// 注册 Ctrl+C / 关闭事件回调
#[cfg(windows)]
fn install_console_guard() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }
    unsafe { SetConsoleCtrlHandler(Some(console_event_handler), 1) };
}

#[cfg(not(windows))]
fn install_console_guard() {}

#[cfg(not(windows))]
fn keep_console_open() {}

/// 主函数：程序入口
fn main() {
    install_console_guard();

    print!("请输入条码图片的路径或 URL（支持二维码和条形码，多个用逗号分隔）: ");
    io::stdout().flush().expect("无法刷新输出缓冲区");
    
    let stdin = io::stdin();
    let mut input = String::new();
    stdin.lock().read_line(&mut input).expect("无法读取输入");
    let input = input.trim();

    if input.is_empty() {
        log_error("未提供输入路径");
        keep_console_open();
        std::process::exit(1);
    }

    // 按逗号分割输入，去除空白并过滤空字符串
    let paths: Vec<&str> = input.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();

    if paths.is_empty() {
        log_error("未提供有效的路径");
        keep_console_open();
        std::process::exit(1);
    }

    // 依次处理每个路径
    for (i, path_str) in paths.iter().enumerate() {
        if paths.len() > 1 {
            log_info(&format!("正在处理第 {} 个: {}", i + 1, path_str));
        }
        process_input(path_str);
    }

    keep_console_open();
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    /// 构造一张纯色小图，用于不依赖网络的单元测试
    fn solid_image(w: u32, h: u32, color: [u8; 3]) -> DynamicImage {
        let buf: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(w, h, Rgb(color));
        DynamicImage::ImageRgb8(buf)
    }

    #[test]
    fn test_get_timestamp_format() {
        let ts = get_timestamp();
        // 预期格式：YYYY-MM-DD HH:MM:SS，固定 19 个字符
        assert_eq!(ts.len(), 19, "时间戳长度应为 19，实际: {}", ts);
        let b = ts.as_bytes();
        assert_eq!(b[4], b'-');
        assert_eq!(b[7], b'-');
        assert_eq!(b[10], b' ');
        assert_eq!(b[13], b':');
        assert_eq!(b[16], b':');
        // 前 4 位与中间日期位都应为数字
        assert!(b[..4].iter().all(|c| c.is_ascii_digit()));
        assert!(b[11..13].iter().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_is_url_accepts_common_image_urls() {
        assert!(is_url("https://example.com/qrcode.png"));
        assert!(is_url("http://example.com/a.jpg"));
        assert!(is_url("https://cdn.example.com/dir/sub/file.jpeg"));
        assert!(is_url("https://sub.example.com/file.gif"));
        assert!(is_url("https://example.com/file.bmp"));
        assert!(is_url("https://example.com/file.webp"));
    }

    #[test]
    fn test_is_url_supports_query_fragment_port_and_case() {
        // 带查询参数（如预签名 URL）
        assert!(is_url("https://example.com/image.png?token=abc123"));
        assert!(is_url(
            "https://s3.amazonaws.com/bucket/key.png?X-Amz-Signature=abc&X-Amz-Expires=3600"
        ));
        // 带 fragment
        assert!(is_url("https://example.com/image.png#section"));
        // 带端口
        assert!(is_url("https://example.com:8080/image.png"));
        // 大写扩展名（本次修复重点）
        assert!(is_url("https://example.com/IMAGE.PNG"));
        assert!(is_url("https://example.com/photo.JPEG"));
    }

    #[test]
    fn test_is_url_rejects_invalid_inputs() {
        assert!(!is_url(""));
        assert!(!is_url("not a url"));
        assert!(!is_url("/local/path/file.png"));
        assert!(!is_url("C:\\path\\to\\file.png"));
        assert!(!is_url("ftp://example.com/a.png"));
        assert!(!is_url("https://example.com/a.txt"));
        assert!(!is_url("https://example.com/a.png.txt"));
        assert!(!is_url("https://example.com"));
        assert!(!is_url("https://"));
    }

    #[test]
    fn test_preprocess_image_returns_ten_variants() {
        // 包含：原始、灰度、高对比度、二值化(128)、二值化(64)、二值化(192)、高斯模糊+二值化、锐化、反色、放大
        let img = solid_image(8, 8, [128, 128, 128]);
        let processed = preprocess_image(&img);
        assert_eq!(processed.len(), 10);

        let names: Vec<&str> = processed.iter().map(|(n, _)| n.as_str()).collect();
        // 验证首项为原始图像，避免回归删除之前重复解码时丢失基础入口
        assert_eq!(names[0], "原始图像");
        for expected in [
            "灰度图像",
            "高对比度图像",
            "二值化图像",
            "二值化图像(阈值64)",
            "二值化图像(阈值192)",
            "高斯模糊+二值化",
            "锐化图像",
            "颜色反转图像",
            "放大图像",
        ] {
            assert!(names.contains(&expected), "缺少预处理变体: {}", expected);
        }
    }

    #[test]
    fn test_preprocess_image_upscaled_doubles_size() {
        // 使用正方形图像，避免 resize 保持纵横比时带来的浮点边界不确定性
        let img = solid_image(16, 16, [0, 0, 0]);
        let processed = preprocess_image(&img);
        let large = processed
            .iter()
            .find(|(n, _)| n == "放大图像")
            .expect("应存在放大图像变体");
        assert_eq!(large.1.width(), 32);
        assert_eq!(large.1.height(), 32);
    }

    #[test]
    fn test_decode_barcode_image_blank_returns_none() {
        // 全白图不会包含任何条码
        let img = solid_image(32, 32, [255, 255, 255]);
        assert!(decode_barcode_image(&img).is_none());
    }

    #[test]
    fn test_format_display_name_common_variants() {
        assert_eq!(format_display_name(&BarcodeFormat::QR_CODE), "二维码 (QR Code)");
        assert_eq!(format_display_name(&BarcodeFormat::EAN_13), "EAN-13 商品条码");
        assert_eq!(format_display_name(&BarcodeFormat::CODE_128), "Code 128 条码");
        assert_eq!(format_display_name(&BarcodeFormat::CODE_39), "Code 39 条码");
        assert_eq!(format_display_name(&BarcodeFormat::UPC_A), "UPC-A 商品条码");
        assert_eq!(format_display_name(&BarcodeFormat::ITF), "ITF (二五码) 条码");
    }

    #[test]
    fn test_format_display_name_unknown_fallback() {
        // 未并列的变体应回退到"未知格式"
        assert_eq!(format_display_name(&BarcodeFormat::UNSUPORTED_FORMAT), "未知格式");
    }

    #[test]
    fn test_download_image_rejects_invalid_url_without_network() {
        // 非法 URL 应直接拒绝，不会发起实际网络请求
        assert!(download_image("not-a-valid-url").is_none());
        assert!(download_image("/local/path.png").is_none());
    }
}