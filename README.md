# Barcode Scanner

![Rust](https://img.shields.io/badge/Rust-Edition%202024-orange)
![Platform](https://img.shields.io/badge/Platform-Windows-blue)
![Version](https://img.shields.io/badge/Version-v1.2.0-brightgreen)
![Tests](https://img.shields.io/badge/Tests-10%20passed-success)
![License](https://img.shields.io/badge/License-MIT-yellow)

**基于 Rust 的高性能命令行条码解码工具，支持二维码与一维条形码，内置 10 级图像预处理 + 四角裁剪 + 水平条带扫描**

[快速开始](#-快速开始) • [使用说明](#-使用说明) • [技术细节](#-技术细节) • [测试说明](#-测试说明)

---

## 📌 项目简介

Barcode Scanner 是一款专注于识别率的命令行条码解码工具，支持二维码（QR Code）和一维条形码（EAN-13、Code 128、UPC-A 等 20+ 种格式），支持本地图片与网络 URL 双通道输入，通过多级图像预处理策略大幅提升复杂场景下的解码成功率。

### ✨ 核心特性

- 📊 **多格式支持**：QR Code、EAN-13/8、UPC-A/E、Code 128/39/93、ITF、Data Matrix、Aztec、PDF417 等 20+ 种条码格式
- 🖥️ **双通道输入**：支持本地文件路径与 http / https URL，两者可混合批量使用
- 📦 **批量并行处理**：英文逗号分隔多个输入，多张图片跨线程并行解码，输出仍按输入顺序无交错
- 🔧 **10 级图像预处理**：原始 → 灰度 → 高对比度 → 多阈值二值化 → 高斯模糊 → 锐化 → 反色 → 2× 放大，逐级降级重试
- ✂️ **四角裁剪 + 水平条带扫描**：标准预处理失败后，自动裁剪四角并滑动水平条带重试
- 🏷️ **格式识别输出**：解码成功时显示条码类型（如 “EAN-13 商品条码”、“Code 128 条码”）
- 🌐 **自动下载**：识别 URL 后自动下载至临时文件，解码结束后自动清理
- 🎨 **彩色分级日志**：INFO（蓝）/ WARNING（黄）/ ERROR（红），精确到每个失败环节
- 🪟 **双击可用**：Windows 双击启动时由 PowerShell 接管窗口，解码输出不会闪退消失
- ✅ **完整单元测试**：10 项测试覆盖 URL 判定、图像预处理、解码器与格式名称映射

### 🛠 技术栈

| 分类           | 技术                                                           |
|----------------|----------------------------------------------------------------|
| **语言**       | Rust（Edition 2024）                                           |
| **解码核心**   | rxing 0.9（ZXing Rust 移植，支持 20+ 条码格式）          |
| **图像处理**   | image 0.24（PNG / JPG / GIF / BMP / WEBP）                     |
| **网络请求**   | reqwest 0.12（blocking 模式，30 s 超时）                       |
| **URL 校验**   | regex 1.11                                                     |
| **临时文件**   | tempfile 3.14                                                  |
| **日志时间戳** | chrono 0.4                                                     |
| **终端着色**   | colored 2.1                                                    |
| **控制台守卫** | Win32 API（`GetConsoleProcessList` / `SetConsoleCtrlHandler`） |

---

## 🚀 快速开始

### 前置要求

- Rust 工具链（Edition 2024，建议通过 [rustup](https://rustup.rs) 安装）
- 网络连接（仅解码 URL 图片时需要）

### 构建

```bash
# 调试构建（输出至 target/debug/）
cargo build

# 发布构建（输出至 target/release/，推荐）
cargo build --release
```

### 运行

```bash
# 直接在终端运行
cargo run

# 或运行发布版本
./target/release/QRCodeScanner
```

> 💡 **Windows 用户**：也可直接双击 `target\release\QRCodeScanner.exe`，程序结束后窗口将由 PowerShell 接管，不会闪退。

---

## 📖 使用说明

程序启动后出现提示符，输入一行文本（多个路径以英文逗号分隔）：

```text
请输入条码图片的路径或 URL（支持二维码和条形码，多个用逗号分隔）:
```

### 输入格式

| 场景                 | 输入示例                                                      |
|----------------------|---------------------------------------------------------------|
| 单个本地图片         | `C:\path\to\qrcode.png`                                       |
| 多个本地图片         | `C:\path\to\qr1.png, C:\path\to\qr2.jpg`                      |
| 网络图片             | `https://example.com/qrcode.png`                              |
| 带签名参数的 CDN URL | `https://s3.amazonaws.com/bucket/key.png?X-Amz-Signature=abc` |
| 本地与网络混合       | `C:\path\to\local.png, https://example.com/remote.jpg`        |

### 支持的图片类型

**PNG、JPG、JPEG、GIF、BMP、WEBP**，扩展名大小写不敏感。

URL 同时兼容带端口（`:8080`）、查询参数（`?token=xxx`）与页内锚点（`#section`）等形式。

### 输出示例

**✅ 成功解码**
```text
2026-10-05 12:00:01 - INFO - 开始解码: C:\path\to\qrcode.png
2026-10-05 12:00:01 - INFO - 使用 原始图像 成功解码
2026-10-05 12:00:01 - INFO - 条码格式: 二维码 (QR Code)
2026-10-05 12:00:01 - INFO - 解码内容: HelloQRCode
```

**✅ 条形码解码**
```text
2026-10-05 12:00:01 - INFO - 开始解码: C:\path\to\barcode.jpg
2026-10-05 12:00:02 - INFO - 使用 灰度图像 成功解码
2026-10-05 12:00:02 - INFO - 条码格式: EAN-13 商品条码
2026-10-05 12:00:02 - INFO - 解码内容: 6901234567892
```

**⚠️ 解码失败**
```text
2026-10-05 12:00:01 - INFO - 开始解码: C:\path\to\damaged.png
2026-10-05 12:00:05 - WARNING - 标准方法未能解码，尝试额外的裁剪处理方法
2026-10-05 12:00:06 - WARNING - 裁剪方法未能解码，尝试水平条带扫描
2026-10-05 12:00:07 - ERROR - 经过所有尝试后仍然无法解码
2026-10-05 12:00:07 - INFO - 建议：确保条码清晰完整，光照均匀，或尝试使用专业扫码应用
```

---

## 📂 项目结构

```
QRCodeScanner/
├── 📄 README.md                    # 项目说明（本文件）
├── 📄 Cargo.toml                   # 包与依赖配置
├── 📄 Cargo.lock                   # 依赖版本锁定
├── 📁 src/
│   └── 📄 main.rs                  # 全部业务逻辑与单元测试
└── 📁 docs/
    └── 📄 详细说明.md               # 架构、流程与技术细节
```

---

## 🔬 技术细节

### 执行流程

```text
main()
  ├─ install_console_guard()    注册控制台事件回调（Ctrl+C 场景）
  ├─ 读取用户输入（逗号分隔多个路径 / URL）
  ├─ 对每个输入调用 process_input()
  │     ├─ is_url()             判断 URL 还是本地路径
  │     ├─ [URL]  download_image() → 临时文件 → decode_image() → 清理
  │     └─ [本地] decode_image()
  └─ keep_console_open()        双击启动时把窗口交给 PowerShell
```

### 图像预处理策略

`preprocess_image()` 按以下顺序生成 **10 个候选图像**，逐一送入解码器，命中即止：

| #  | 名称                   | 处理方法                      |
|----|------------------------|-------------------------------|
| 1  | 原始图像               | 直接 clone 输入图像           |
| 2  | 灰度图像               | `to_luma8()`                  |
| 3  | 高对比度图像           | 灰度 + `contrast(2.0)`        |
| 4  | 二值化图像（阈值 128） | 高对比度 + 阈值二值化         |
| 5  | 二值化图像（阈值 64）  | 高对比度 + 低阈值二值化       |
| 6  | 二值化图像（阈值 192） | 高对比度 + 高阈值二值化       |
| 7  | 高斯模糊 + 二值化      | 灰度 `blur(1.0)` + 阈值二值化 |
| 8  | 锐化图像               | RGBA `unsharpen(1.0, 0)`      |
| 9  | 颜色反转图像           | 灰度每像素 `255 - v`          |
| 10 | 放大图像               | 2× Lanczos3 放大              |

### 四角裁剪 + 水平条带扫描

若所有预处理变体均失败，输出 WARNING 日志后执行：

1. **四角裁剪**：对图像四角各裁剪 1/4 区域重试
2. **水平条带扫描**：将图像按 1/6 高度切片，50% 重叠滑动扫描（特别适用于一维条形码）

| 区域   | 裁剪坐标 (x, y, x2, y2)            |
|--------|------------------------------------|
| 左上角 | (0, 0, width/2, height/2)          |
| 右上角 | (width/2, 0, width, height/2)      |
| 左下角 | (0, height/2, width/2, height)     |
| 右下角 | (width/2, height/2, width, height) |

### URL 识别规则

`is_url()` 使用以下正则（大小写不敏感）：

```regex
(?i)^https?://\S+\.(png|jpg|jpeg|gif|bmp|webp)(\?\S*)?(#\S*)?$
```

| 场景         | 示例                                   | 是否匹配 |
|--------------|----------------------------------------|----------|
| 普通图片 URL | `https://example.com/qr.png`           | ✅       |
| 大写扩展名   | `https://example.com/QR.PNG`           | ✅       |
| 带端口       | `https://example.com:8080/qr.png`      | ✅       |
| 带查询参数   | `https://example.com/qr.png?token=abc` | ✅       |
| 多级路径     | `https://cdn.example.com/a/b/c.png`    | ✅       |
| 本地路径     | `C:\path\to\qr.png`                    | ❌       |

### 日志格式

```text
{YYYY-MM-DD HH:MM:SS} - {级别} - {消息}
```

| 级别    | 颜色    | 输出流 |
|---------|---------|--------|
| INFO    | 🔵 蓝色 | stdout |
| WARNING | 🟡 黄色 | stdout |
| ERROR   | 🔴 红色 | stderr |

### 控制台窗口保留机制

双击 exe 启动时，`keep_console_open()` 通过 Win32 `GetConsoleProcessList` 检测是否独占控制台（进程数为 1），若是则 spawn 一个带 `-NoExit` 参数的 PowerShell 交互式会话接管窗口。

| 启动方式               | 是否派生 shell |
|------------------------|----------------|
| 双击 exe（新建控制台） | ✅ 是          |
| 在已有终端中运行       | ❌ 否          |
| `cargo run`            | ❌ 否          |
| Git Bash 直接运行      | ❌ 否          |

优先使用 PATH 中的 `pwsh`（PowerShell 7），失败则回退至 Windows PowerShell 5.1。

---

## 🧪 测试说明

项目包含 **10 个不依赖网络的单元测试**，覆盖 URL 判定、图像预处理、解码器与格式名称映射。

| 测试函数                                                  | 验证内容                                   |
|-----------------------------------------------------------|--------------------------------------------|
| `test_get_timestamp_format`                               | 时间戳格式与分隔符位置                     |
| `test_is_url_accepts_common_image_urls`                   | 6 种常见图片扩展名的正例                   |
| `test_is_url_supports_query_fragment_port_and_case`       | 端口 / 查询参数 / fragment / 大写扩展名    |
| `test_is_url_rejects_invalid_inputs`                      | 9 个反例（本地路径、错误协议、无扩展名等） |
| `test_preprocess_image_returns_ten_variants`              | 预处理列表长度为 10，首项为“原始图像”      |
| `test_preprocess_image_upscaled_doubles_size`             | 16×16 → 32×32 放大语义                     |
| `test_decode_barcode_image_blank_returns_none`            | 空白图应返回 None                          |
| `test_format_display_name_common_variants`                | 常见条码格式的中文名称映射                 |
| `test_format_display_name_unknown_fallback`               | 未定义格式回退为“未知格式”                 |
| `test_download_image_rejects_invalid_url_without_network` | 非法 URL 不发网络请求即返回 None           |

**快速运行：**

```bash
cargo test                # 运行所有测试
cargo test -- --nocapture # 显示测试输出
```

---

## ❓ 常见问题

### 1. 双击 exe 后窗口一闪而过

v1.0.0 已内置控制台守卫，正常情况下不会闪退。若仍有问题，改为在终端中运行 `QRCodeScanner.exe`。

### 2. 将图片拖放到 exe 上没有反应

程序不解析命令行参数，拖放操作不会触发解码。请在终端中启动程序后手动输入路径。

### 3. 路径中包含英文逗号

英文逗号是批量输入的分隔符，路径本身不能包含逗号，否则会被拆分为两个输入。

### 4. URL 图片下载失败

检查网络连接，确认可访问目标 URL。程序 HTTP 超时设置为 30 秒，部分慢速 CDN 可能超时。

### 5. 条码解码失败

确保图片清晰完整、无严重遮挡。一维条形码拍摄时建议保持图片水平，避免倾斜过大。低质量图片可能需要多级预处理重试，耐心等待日志输出即可。

---

## 📋 已知限制

- 不解析命令行参数，仅接受标准输入
- 无图形界面，仅控制台彩色输出
- 逗号是批量输入分隔符，路径本身不能包含英文逗号
- 控制台窗口保留功能为 Windows 专属

---

## 🤝 贡献指南

1. Fork 本仓库
2. 创建特性分支：`git checkout -b feature/AmazingFeature`
3. 提交更改：`git commit -m 'Add some AmazingFeature'`
4. 推送分支：`git push origin feature/AmazingFeature`
5. 提交 Pull Request

---

## 📄 许可证

本项目仅供学习交流使用

---

## 📬 联系方式

如有问题或建议，欢迎提 Issue：[github.com/ZapcoMan/QRCodeScanner](https://github.com/ZapcoMan/QRCodeScanner)

---

*最后更新时间：2026-10-06*
*详细技术文档：[docs/详细说明.md](docs/详细说明.md)*
