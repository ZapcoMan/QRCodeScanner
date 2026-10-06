use std::path::PathBuf;
use std::ffi::OsStr;
use std::cell::RefCell;
use std::io::{self, BufRead, Cursor, Read, Write};
use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use chrono::Local;
use regex::Regex;
use tempfile::NamedTempFile;
use image::{DynamicImage, ImageBuffer, ImageDecoder, ImageFormat, Luma};
use image::imageops;
use colored::Colorize;
use rxing::helpers::detect_in_luma_slice_with_hints;
use rxing::{BarcodeFormat, DecodeHints};

/// 获取当前时间戳，格式为 "年-月-日 时:分:秒"
fn get_timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 一条已格式化的日志：着色后的文本 + 应输出到标准输出还是标准错误
struct CapturedLog {
    line: String,
    to_stderr: bool,
}

thread_local! {
    /// 当前线程的日志捕获栈。并行处理多图时，每张图片在自己的工作线程内
    /// 开启一层捕获，将日志累积到栈顶缓冲，结束后由主线程按输入顺序统一打印。
    /// 栈为空时（串行/普通调用）日志直接打印，行为与以往一致；栈结构天然支持嵌套。
    static LOG_CAPTURE: RefCell<Vec<Vec<CapturedLog>>> = const { RefCell::new(Vec::new()) };
}

/// 输出一行日志：若当前线程处于捕获模式则存入栈顶缓冲，否则直接打印
fn emit(line: String, to_stderr: bool) {
    let captured = LOG_CAPTURE.with(|c| {
        let mut stack = c.borrow_mut();
        if let Some(top) = stack.last_mut() {
            top.push(CapturedLog { line: line.clone(), to_stderr });
            true
        } else {
            false
        }
    });
    if !captured {
        if to_stderr {
            eprintln!("{}", line);
        } else {
            println!("{}", line);
        }
    }
}

/// 输出蓝色 INFO 级别日志
fn log_info(message: &str) {
    emit(format!("{} - INFO - {}", get_timestamp(), message).blue().to_string(), false);
}

/// 输出黄色 WARNING 级别日志
fn log_warning(message: &str) {
    emit(format!("{} - WARNING - {}", get_timestamp(), message).yellow().to_string(), false);
}

/// 输出红色 ERROR 级别日志（到标准错误流）
fn log_error(message: &str) {
    emit(format!("{} - ERROR - {}", get_timestamp(), message).red().to_string(), true);
}

/// 在捕获模式下运行 f：期间所有 log_* 输出被收集而非直接打印
/// 返回 f 的结果与被捕获的日志列表（供主线程按序打印）
fn capture_logs<R>(f: impl FnOnce() -> R) -> (R, Vec<CapturedLog>) {
    LOG_CAPTURE.with(|c| c.borrow_mut().push(Vec::new()));
    let result = f();
    let logs = LOG_CAPTURE.with(|c| c.borrow_mut().pop()).unwrap_or_default();
    (result, logs)
}

/// 将捕获到的日志按顺序刷回真实输出
fn flush_captured_logs(logs: Vec<CapturedLog>) {
    for entry in logs {
        if entry.to_stderr {
            eprintln!("{}", entry.line);
        } else {
            println!("{}", entry.line);
        }
    }
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
/// 包括：原始、对比度增强、二值化（多阈值）、模糊+二值化、锐化、颜色反转、放大（仅小图）
fn preprocess_image(img: &DynamicImage) -> Vec<(String, DynamicImage)> {
    let mut images = Vec::new();

    // 原始图像（解码器内部会自行转灰度，无需再单独提供一份灰度变体）
    images.push(("原始图像".to_string(), img.clone()));

    // 转换为灰度图像（供后续模糊、反转等变体使用）
    let gray_img = img.to_luma8();

    // 增强对比度（2.0 倍）
    let contrast_img = imageops::contrast(&gray_img, 2.0);
    images.push(("高对比度图像".to_string(), DynamicImage::ImageLuma8(contrast_img.clone())));

    // 二值化处理（阈值 128 / 64 / 192）
    // 直接对原始字节做映射，避免逐像素 get_pixel 的边界检查开销
    let contrast_raw = contrast_img.as_raw();
    let (cw, ch) = (contrast_img.width(), contrast_img.height());
    for threshold in [128u8, 64u8, 192u8] {
        let data: Vec<u8> = contrast_raw
            .iter()
            .map(|&v| if v < threshold { 0u8 } else { 255u8 })
            .collect();
        let binary_img = ImageBuffer::<Luma<u8>, Vec<u8>>::from_raw(cw, ch, data)
            .expect("二值化数据长度应与图像尺寸一致");
        let name = if threshold == 128 {
            "二值化图像".to_string()
        } else {
            format!("二值化图像(阈值{})", threshold)
        };
        images.push((name, DynamicImage::ImageLuma8(binary_img)));
    }

    // 高斯模糊 + 二值化
    let blurred = imageops::blur(&gray_img, 1.0);
    let blurred_data: Vec<u8> = blurred
        .as_raw()
        .iter()
        .map(|&v| if v < 128 { 0u8 } else { 255u8 })
        .collect();
    let blurred_binary =
        ImageBuffer::<Luma<u8>, Vec<u8>>::from_raw(blurred.width(), blurred.height(), blurred_data)
            .expect("模糊二值化数据长度应与图像尺寸一致");
    images.push(("高斯模糊+二值化".to_string(), DynamicImage::ImageLuma8(blurred_binary)));

    // 锐化处理
    let sharpened = img.to_rgba8();
    let sharpened = imageops::unsharpen(&sharpened, 1.0, 0);
    images.push(("锐化图像".to_string(), DynamicImage::ImageRgba8(sharpened)));

    // 颜色反转
    let inverted_data: Vec<u8> = gray_img.as_raw().iter().map(|&v| 255u8 - v).collect();
    let inverted =
        ImageBuffer::<Luma<u8>, Vec<u8>>::from_raw(gray_img.width(), gray_img.height(), inverted_data)
            .expect("反转数据长度应与图像尺寸一致");
    images.push(("颜色反转图像".to_string(), DynamicImage::ImageLuma8(inverted)));

    // 图像放大（2 倍，使用 Lanczos3 算法）
    // 仅对小图有效：大图放大产生 4 倍像素，既拖慢解码又几乎不提升识别率
    if img.width().max(img.height()) < 900 {
        let large_img = img.resize(img.width() * 2, img.height() * 2, imageops::FilterType::Lanczos3);
        images.push(("放大图像".to_string(), large_img));
    }

    images
}

/// 解码结果结构体，包含文本内容和条码格式
struct DecodeResult {
    text: String,
    format: BarcodeFormat,
}

/// 一维条码候选格式（含 PDF417 堆叠式条码）
/// 水平条带扫描时只尝试这些格式，跳过重量级的 2D 解码器，显著提速
fn oned_barcode_formats() -> Vec<BarcodeFormat> {
    vec![
        BarcodeFormat::EAN_13,
        BarcodeFormat::EAN_8,
        BarcodeFormat::UPC_A,
        BarcodeFormat::UPC_E,
        BarcodeFormat::CODE_128,
        BarcodeFormat::CODE_39,
        BarcodeFormat::CODE_93,
        BarcodeFormat::CODABAR,
        BarcodeFormat::ITF,
        BarcodeFormat::RSS_14,
        BarcodeFormat::RSS_EXPANDED,
        BarcodeFormat::TELEPEN,
        BarcodeFormat::UPC_EAN_EXTENSION,
        BarcodeFormat::PDF_417,
    ]
}

/// 按指定候选格式集合解码图像；formats 为 None 时尝试所有格式
/// try_harder 控制 rxing 是否投入更多搜索时间（false = 速度优先）
fn decode_barcode_image_with(
    img: &DynamicImage,
    formats: Option<&[BarcodeFormat]>,
    try_harder: bool,
) -> Option<DecodeResult> {
    let luma = img.to_luma8();
    let (width, height) = (luma.width(), luma.height());

    let mut hints = DecodeHints::default();
    if let Some(fmts) = formats {
        hints.PossibleFormats = Some(fmts.iter().copied().collect::<HashSet<BarcodeFormat>>());
    }
    hints.TryHarder = Some(try_harder);

    if let Ok(result) =
        detect_in_luma_slice_with_hints(luma.as_raw(), width, height, None, &mut hints)
    {
        let text = result.getText().to_string();
        if !text.is_empty() {
            return Some(DecodeResult {
                text,
                format: *result.getBarcodeFormat(),
            });
        }
    }

    None
}

/// 并行级联中的单个解码任务
struct DecodeJob {
    desc: String,
    image: DynamicImage,
    formats: Option<Vec<BarcodeFormat>>,
}

/// 多线程并行执行解码任务：任意线程首个成功即作为结果，
/// 其他线程在任务间检查完成标志后提前退出
fn run_jobs_parallel(jobs: Vec<DecodeJob>) -> Option<(String, DecodeResult)> {
    if jobs.is_empty() {
        return None;
    }

    let thread_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(jobs.len());

    let next_index = AtomicUsize::new(0);
    let finished = AtomicBool::new(false);
    let winner: Mutex<Option<(String, DecodeResult)>> = Mutex::new(None);

    std::thread::scope(|scope| {
        for _ in 0..thread_count {
            scope.spawn(|| {
                loop {
                    if finished.load(Ordering::Relaxed) {
                        break;
                    }
                    let index = next_index.fetch_add(1, Ordering::Relaxed);
                    if index >= jobs.len() {
                        break;
                    }
                    let job = &jobs[index];
                    let result =
                        decode_barcode_image_with(&job.image, job.formats.as_deref(), true);
                    if let Some(decoded) = result {
                        let mut guard = winner.lock().unwrap();
                        if guard.is_none() {
                            *guard = Some((job.desc.clone(), decoded));
                            finished.store(true, Ordering::Relaxed);
                        }
                        break;
                    }
                }
            });
        }
    });

    winner.into_inner().unwrap()
}

/// 执行完整解码策略：缩小快速通道 → 并行级联（预处理/裁剪/条带）→ 全尺寸原图兜底尝试
/// 成功时返回 (策略描述, 解码结果)，全部失败返回 None
fn try_decode_image(img: &mut DynamicImage) -> Option<(String, DecodeResult)> {
    let width = img.width();
    let height = img.height();

    // 级联统一在长边 ≤1600px 的工作图上进行：
    // 手机照片常有数千万像素，全尺寸反复解码是主要耗时来源，
    // 缩小后条码模块宽度通常仍≥ 3px，不影响解码
    // （若加载阶段已采样解码到 ≈CASCADE_MAX_DIM，此处会自然跳过缩放）
    let downscaled = width.max(height) > CASCADE_MAX_DIM;
    let mut work_img = if downscaled {
        img.resize(CASCADE_MAX_DIM, CASCADE_MAX_DIM, imageops::FilterType::Triangle)
    } else {
        img.clone()
    };

    // 快速通道：工作图低强度一次解码（TryHarder 关闭），清晰图片通常在此即命中
    if let Some(result) = decode_barcode_image_with(&work_img, None, false) {
        return Some(("快速通道解码成功（工作图像）".to_string(), result));
    }

    log_warning("快速通道未能解码，进入并行级联尝试");

    // 收集全部候选任务：预处理变体 + 角部裁剪 + 水平条带，一次性并行执行
    let (work_width, work_height) = (work_img.width(), work_img.height());
    let mut jobs: Vec<DecodeJob> = Vec::new();

    for (name, processed_img) in preprocess_image(&work_img) {
        jobs.push(DecodeJob {
            desc: format!("使用 {} 成功解码", name),
            image: processed_img,
            formats: None,
        });
    }

    let crops = [
        ("左上角", (0, 0, work_width / 2, work_height / 2)),
        ("右上角", (work_width / 2, 0, work_width, work_height / 2)),
        ("左下角", (0, work_height / 2, work_width / 2, work_height)),
        ("右下角", (work_width / 2, work_height / 2, work_width, work_height)),
    ];

    for (name, (x, y, x2, y2)) in &crops {
        let cropped = work_img.crop(*x, *y, x2 - *x, y2 - *y);
        jobs.push(DecodeJob {
            desc: format!("在裁剪区域 {} 中成功解码", name),
            image: cropped,
            formats: None,
        });
    }

    // 水平条带只尝试一维格式，跳过重量级 2D 解码器
    let strip_formats = oned_barcode_formats();
    let strip_height = (work_height / 6).max(1);
    let mut y = 0u32;
    while y + strip_height <= work_height {
        let strip = work_img.crop(0, y, work_width, strip_height);
        jobs.push(DecodeJob {
            desc: format!("在水平条带(y={}, h={})中成功解码", y, strip_height),
            image: strip,
            formats: Some(strip_formats.clone()),
        });
        y += strip_height / 2; // 50% 重叠滑动
    }

    if let Some(outcome) = run_jobs_parallel(jobs) {
        return Some(outcome);
    }

    // 最后兜底：在全尺寸原图上做一次高强度解码
    // （针对工作副本中条码过细、缩小后丢失细节的情况）
    if downscaled {
        log_warning("并行级联未能解码，尝试全尺寸原图高强度解码");
        if let Some(result) = decode_barcode_image_with(img, None, true) {
            return Some(("在全尺寸原图上高强度解码成功".to_string(), result));
        }
    }

    None
}

/// 级联工作图的长边上限
const CASCADE_MAX_DIM: u32 = 1600;

/// 计算 JPEG 采样解码所需的缩放因子（1/1 ~ 1/8，分母为 2 的幂）
/// 采样解码在 DCT 域直接输出缩小后的图像，跳过全尺寸解压 + 独立缩放两步
/// 纯函数便于单元测试
fn jpeg_sampling_scale(width: u32, height: u32) -> u32 {
    let max_dim = width.max(height);
    if max_dim <= CASCADE_MAX_DIM {
        return 1;
    }
    // 取 2 的幂分母，使缩放后长边仍 ≥ CASCADE_MAX_DIM（宁可偏大不偏小，保证条码细节）
    let mut scale = 1u32;
    while max_dim / (scale * 2) >= CASCADE_MAX_DIM && scale < 8 {
        scale *= 2;
    }
    scale
}

/// 加载条码图片，对超大 JPEG 使用采样解码提速
/// 手机照片常有数千万像素，全尺寸解压再缩放很慢；这里在读文件时
/// 就按 1/2、1/4 等比例缩小输出。PNG/BMP 等格式没有采样能力，走常规路径。
fn load_barcode_image(path: &PathBuf) -> Result<DynamicImage, image::ImageError> {
    // 1. 一次读入内存：采样解码需要先探查尺寸，且字节缓冲可复用
    let mut buffer = Vec::new();
    std::fs::File::open(path)
        .map_err(image::ImageError::IoError)?
        .read_to_end(&mut buffer)
        .map_err(image::ImageError::IoError)?;

    // 2. 仅对超大 JPEG 启用采样解码（ImageError 未实现 PartialEq，用 matches! 判断格式）
    if matches!(image::guess_format(&buffer), Ok(ImageFormat::Jpeg)) {
        // 同一个解码器先读原始尺寸（仅解析头部），需要时再就地缩小，避免重复建解码器
        let mut decoder = image::codecs::jpeg::JpegDecoder::new(Cursor::new(&buffer))?;
        let (w, h) = decoder.dimensions();
        let scale = jpeg_sampling_scale(w, h);
        if scale > 1 {
            // scale() 会自动选用 ≥ 目标尺寸的最小 2 的幂因子，返回缩放后的尺寸
            let (sw, sh) = decoder.scale((CASCADE_MAX_DIM + 1) as u16, (CASCADE_MAX_DIM + 1) as u16)?;
            let (sw, sh) = (sw as u32, sh as u32);
            // 根据解码器自报的色彩类型确定每像素字节数（JPEG 常为彩色 RGB8 或灰度 L8）
            let bpp = match decoder.color_type() {
                image::ColorType::L8 => 1usize,
                image::ColorType::Rgb8 => 3usize,
                // 其他罕见色彩类型退回全尺寸常规解码，避免手工构造出错
                _ => 0usize,
            };
            if bpp > 0 {
                let mut data = vec![0u8; (sw as usize) * (sh as usize) * bpp];
                decoder.read_image(&mut data)?;
                let img = match bpp {
                    1 => DynamicImage::ImageLuma8(
                        ImageBuffer::<Luma<u8>, Vec<u8>>::from_raw(sw, sh, data)
                            .expect("采样灰度数据长度应与图像尺寸一致")),
                    _ => DynamicImage::ImageRgb8(
                        ImageBuffer::<image::Rgb<u8>, Vec<u8>>::from_raw(sw, sh, data)
                            .expect("采样彩色数据长度应与图像尺寸一致")),
                };
                return Ok(img);
            }
        }
    }

    // 3. 常规路径：全尺寸解码（PNG/BMP 等非超大 JPEG）
    let img = image::io::Reader::new(Cursor::new(&buffer))
        .with_guessed_format()?
        .decode()?;
    Ok(img)
}

/// 解码指定图片路径中的条码（支持二维码和一维条形码）
/// 先走速度优先的快速通道，失败后再进入完整的预处理与扫描级联
fn decode_image(image_path: &PathBuf) {
    log_info(&format!("开始解码: {}", image_path.display()));

    let mut img = match load_barcode_image(image_path) {
        Ok(img) => img,
        Err(e) => {
            log_error(&format!("无法打开文件: {}", e));
            return;
        }
    };

    match try_decode_image(&mut img) {
        Some((how, result)) => {
            log_info(&how);
            log_info(&format!("条码格式: {}", format_display_name(&result.format)));
            log_info(&format!("解码内容: {}", result.text));
        }
        None => {
            log_error("经过所有尝试后仍然无法解码");
            log_info("建议：确保条码清晰完整，光照均匀，或尝试使用专业扫码应用");
        }
    }
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

    if paths.len() == 1 {
        // 单图（最常见）直接处理，日志即时打印，无额外开销
        process_input(paths[0]);
    } else {
        // 多图并行解码：每张图片在自己的线程里跑完整流程并捕获日志，
        // 主线程按输入顺序统一打印，既提速又保证输出不交错
        let captured: Vec<Vec<CapturedLog>> = std::thread::scope(|scope| {
            let handles: Vec<_> = paths
                .iter()
                .enumerate()
                .map(|(i, path_str)| {
                    let path_owned = (*path_str).to_string();
                    let index = i + 1;
                    scope.spawn(move || {
                        let (_, logs) = capture_logs(|| {
                            log_info(&format!("正在处理第 {} 个: {}", index, path_owned));
                            process_input(&path_owned);
                        });
                        logs
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
        });
        for logs in captured {
            flush_captured_logs(logs);
        }
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
    fn test_capture_logs_collects_instead_of_printing() {
        // 捕获模式下：日志不直接输出，而是按顺序收集，且保留级别与目标流
        let (ret, logs) = capture_logs(|| {
            log_info("hello info");
            log_warning("a warning");
            log_error("an error");
            42
        });
        assert_eq!(ret, 42);
        assert_eq!(logs.len(), 3);
        assert!(logs[0].line.contains("INFO"));
        assert!(!logs[0].to_stderr);
        assert!(logs[1].line.contains("WARNING"));
        assert!(!logs[1].to_stderr);
        assert!(logs[2].line.contains("ERROR"));
        assert!(logs[2].to_stderr, "ERROR 应标记为输出到标准错误流");
    }

    #[test]
    fn test_capture_logs_nested_and_restores_state() {
        // 嵌套捕获：内层结束后外层不应受影响；整体结束后线程回到直接打印模式
        let (outer_len, _) = capture_logs(|| {
            log_info("outer-1");
            let inner_logs = capture_logs(|| log_info("inner")).1;
            log_info("outer-2");
            inner_logs.len()
        });
        assert_eq!(outer_len, 1, "内层捕获不应污染外层缓冲");
        // 退出捕获模式后再调用日志应直接打印（不 panic、不残留缓冲）
        let (_, logs) = capture_logs(|| log_info("fresh"));
        assert_eq!(logs.len(), 1);
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
    fn test_preprocess_image_returns_nine_variants() {
        // 包含：原始、高对比度、二值化(128)、二值化(64)、二值化(192)、高斯模糊+二值化、锐化、反色、放大（小图）
        // 注意：灰度变体已移除（解码器内部自行转灰度，属于重复工作）
        let img = solid_image(8, 8, [128, 128, 128]);
        let processed = preprocess_image(&img);
        assert_eq!(processed.len(), 9);

        let names: Vec<&str> = processed.iter().map(|(n, _)| n.as_str()).collect();
        // 验证首项为原始图像，避免回归删除之前重复解码时丢失基础入口
        assert_eq!(names[0], "原始图像");
        assert!(!names.contains(&"灰度图像"), "重复的灰度变体应已移除");
        for expected in [
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
    fn test_preprocess_image_skips_upscale_for_large_images() {
        // 长边超过 900px 的大图不应再生成放大变体（耗时且无收益）
        let img = solid_image(1000, 950, [128, 128, 128]);
        let processed = preprocess_image(&img);
        let names: Vec<&str> = processed.iter().map(|(n, _)| n.as_str()).collect();
        assert!(!names.contains(&"放大图像"), "大图不应包含放大变体");
        assert_eq!(processed.len(), 8);
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
        assert!(decode_barcode_image_with(&img, None, true).is_none());
    }

    #[test]
    fn test_run_jobs_parallel_empty_jobs_returns_none() {
        assert!(run_jobs_parallel(Vec::new()).is_none());
    }

    #[test]
    fn test_run_jobs_parallel_blank_images_return_none() {
        // 混合全格式与一维格式任务，空白图均无法解码，不应 panic 或死锁
        let jobs = vec![
            DecodeJob {
                desc: "a".to_string(),
                image: solid_image(32, 32, [255, 255, 255]),
                formats: None,
            },
            DecodeJob {
                desc: "b".to_string(),
                image: solid_image(16, 16, [0, 0, 0]),
                formats: Some(oned_barcode_formats()),
            },
        ];
        assert!(run_jobs_parallel(jobs).is_none());
    }

    #[test]
    fn test_oned_barcode_formats_contents() {
        let formats = oned_barcode_formats();
        // 应包含常见一维格式
        for expected in [
            BarcodeFormat::EAN_13,
            BarcodeFormat::UPC_A,
            BarcodeFormat::CODE_128,
            BarcodeFormat::PDF_417,
        ] {
            assert!(formats.contains(&expected), "缺少一维格式: {}", expected);
        }
        // 不应包含重量级 2D 格式（条带扫描提速的关键）
        for excluded in [
            BarcodeFormat::QR_CODE,
            BarcodeFormat::AZTEC,
            BarcodeFormat::DATA_MATRIX,
        ] {
            assert!(!formats.contains(&excluded), "不应包含 2D 格式: {}", excluded);
        }
    }

    #[test]
    fn test_decode_barcode_image_with_restricted_formats() {
        // 限制格式集后对空白图同样返回 None，不应 panic
        let img = solid_image(32, 32, [255, 255, 255]);
        let formats = oned_barcode_formats();
        assert!(decode_barcode_image_with(&img, Some(&formats), false).is_none());
        assert!(decode_barcode_image_with(&img, None, true).is_none());
    }

    #[test]
    fn test_jpeg_sampling_scale_thresholds() {
        // 小图不缩放
        assert_eq!(jpeg_sampling_scale(100, 100), 1);
        assert_eq!(jpeg_sampling_scale(1600, 1200), 1);
        assert_eq!(jpeg_sampling_scale(1601, 100), 2);
        // 典型手机照片：2412px → 1/2 后 1206px
        assert_eq!(jpeg_sampling_scale(1080, 2412), 2);
        // 4000px → 1/4 后 1000px（1/8 会低于 1600 下限）
        assert_eq!(jpeg_sampling_scale(3000, 4000), 4);
        // 8000px → 1/8 后 1000px（分母封顶 8）
        assert_eq!(jpeg_sampling_scale(8000, 6000), 8);
        assert_eq!(jpeg_sampling_scale(20000, 10000), 8);
    }

    #[test]
    fn test_load_barcode_image_missing_file_returns_io_error() {
        // 不存在的文件应返回 IO 错误，不应 panic
        let result = load_barcode_image(&PathBuf::from("Z:\\not_exist_for_test.jpg"));
        assert!(matches!(result, Err(image::ImageError::IoError(_))));
    }

    #[test]
    fn test_load_barcode_image_reads_small_jpeg_from_tempfile() {
        // 小尺寸 JPEG 不会触发采样（scale==1），应走常规解码路径并原尺寸返回
        let img = solid_image(64, 48, [200, 100, 50]);
        let mut tmp = NamedTempFile::new().expect("创建临时文件失败");
        img.save_with_format(&mut tmp, ImageFormat::Jpeg)
            .expect("写入临时 JPEG 失败");
        let path = tmp.path().to_path_buf();
        let loaded = load_barcode_image(&path).expect("应能加载临时 JPEG");
        assert_eq!(loaded.width(), 64);
        assert_eq!(loaded.height(), 48);
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