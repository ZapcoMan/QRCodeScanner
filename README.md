# QR Code Scanner

一个基于 Rust 的命令行二维码扫描工具，支持本地图片和 URL 解码。

## 功能特点

- **交互式输入** — 支持本地文件路径或网络 URL
- **批量处理** — 逗号分隔多个输入，依次解码
- **自动下载** — 识别 http/https 图片链接并下载到临时文件
- **智能增强** — 内置 10 种图像预处理 + 4 种裁剪策略，最大化识别率
- **详细日志** — 彩色分级日志（INFO / WARNING / ERROR），精确到每个失败环节
- **双击可用** — 双击 exe 启动时结束后自动保留控制台窗口，不再闪退
- **单元测试** — 覆盖 URL 判定、图像预处理、解码器等核心逻辑

## 支持的图片格式

| 类型 | 扩展名（大小写不敏感） |
|------|----------------------|
| 图片 | PNG, JPG, JPEG, GIF, BMP, WEBP |

URL 同时兼容带端口（`:8080`）、查询参数（`?token=xxx`）、页内锚点（`#section`）等形式。

## 快速开始

### 环境要求

- Rust 工具链（Edition 2024）
- 网络连接（仅在解码 URL 图片时需要）

### 构建

```bash
cargo build
```

### 运行

```bash
cargo run
```

也可以直接双击 `QRCodeScanner.exe`：程序结束后会在同一窗口留下一个 PowerShell 提示符，输出不再随窗口消失。

> 输入只走标准输入，程序不解析命令行参数，因此"拖图片到 exe 上"不会触发解码。

### 测试

```bash
cargo test
```

### 输入示例

```
请输入二维码图片的路径或 URL（多个用逗号分隔）:
```

```text
# 单个本地图片
C:\path\to\qrcode.png

# 多个本地图片
C:\path\to\qr1.png, C:\path\to\qr2.jpg

# URL 图片
https://example.com/qrcode.png

# 带签名参数的 CDN URL
https://s3.amazonaws.com/bucket/key.png?X-Amz-Signature=abc

# 混合输入
C:\path\to\local.png, https://example.com/remote.jpg
```

## 项目结构

```
QRCodeScanner/
├── src/main.rs          ← 全部业务逻辑 + 单元测试
├── docs/详细说明.md      ← 技术细节与流程文档
├── README.md             ← 本文件
├── Cargo.toml            ← 依赖配置
└── Cargo.lock
```

## 依赖库

| 库 | 版本 | 用途 |
|----|------|------|
| `bardecoder` | 0.4 | 二维码/条形码解码 |
| `image` | 0.24 | 图像加载与预处理 |
| `reqwest` | 0.12 | HTTP 下载（blocking 模式） |
| `regex` | 1.11 | URL 格式校验 |
| `tempfile` | 3.14 | 临时文件管理 |
| `chrono` | 0.4 | 日志时间戳 |
| `colored` | 2.1 | 终端彩色输出 |

## 更多信息

详细的技术说明、解码流程、输出示例和注意事项，请参阅 [详细说明.md](docs/详细说明.md)。
