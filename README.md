# Nexia Launcher

Launcher Minecraft Java viết bằng Rust, giao diện native (egui, không Electron, không webview).

## Tính năng

- **Đăng nhập Microsoft** (device code flow → Xbox Live → XSTS → Minecraft), tự làm mới token.
- **Tải mod từ Modrinth**: tìm kiếm, lọc theo phiên bản + loader, tự cài mod phụ thuộc bắt buộc, kiểm tra SHA-1.
- **Đổi skin**: xem skin hiện tại, xem trước skin mới (kể cả lớp áo ngoài), chọn Classic/Slim, tải lên.
- **Vanilla & Fabric**: tự tải version, libraries, natives, assets.
- **Tối ưu hiệu năng**: tải song song 16 luồng, dùng lại file đã có (lần chạy sau gần như tức thì),
  binary build với LTO + strip, bộ cờ G1GC nhẹ cho game.

### Chế độ offline

Chưa có Azure Client ID? Ở thanh bên trái, nhập tên (3–16 ký tự: chữ, số, `_`) rồi bấm **Chơi offline**.
Chế độ này chạy được game một người chơi và tải mod, nhưng **không vào được server online-mode và không đổi được skin**.

## Build

Cần Rust ≥ 1.80 và Java (8/17/21 tuỳ phiên bản Minecraft; 1.20.5+ cần Java 21).

```bash
cargo run --release
```

Linux cần thêm thư viện hệ thống cho egui, ví dụ trên Debian/Ubuntu:

```bash
sudo apt install libxkbcommon-dev libwayland-dev libgl1-mesa-dev libxcb-render0-dev \
  libxcb-shape0-dev libxcb-xfixes0-dev
```

## Thiết lập đăng nhập Microsoft (bắt buộc, làm một lần)

Nexia đã có sẵn Client ID mặc định trong `src/config.rs` (`DEFAULT_CLIENT_ID`); bạn có thể ghi đè trong
tab **Cài đặt**. Nếu muốn dùng app Azure của riêng bạn:

1. Vào <https://portal.azure.com> → *App registrations* → *New registration*.
2. *Supported account types*: chọn loại có **personal Microsoft accounts**.
3. Tab *Authentication* → *Allow public client flows* = **Yes**.
4. Copy *Application (client) ID*, dán vào tab **Cài đặt** của launcher
   (hoặc đặt biến môi trường `NEXIA_CLIENT_ID`).

> **Lưu ý quan trọng:** Mojang yêu cầu app mới phải được duyệt trước khi gọi Minecraft API.
> Nếu đăng nhập báo `Invalid app registration`, hãy gửi form xin quyền truy cập tại
> <https://aka.ms/AppRegInfo> và chờ được duyệt.

## Dữ liệu

Mọi thứ nằm trong thư mục dữ liệu của hệ điều hành (nút *Mở thư mục dữ liệu* trong Cài đặt):
`libraries/`, `assets/`, `versions/`, `instance/` (chứa `mods/`, saves…) và `config.json`.
`config.json` chứa token đăng nhập (trên Linux/macOS file được đặt quyền 600). Đừng chia sẻ file này.

## Cấu trúc mã nguồn

| File | Vai trò |
|---|---|
| `src/main.rs` | Giao diện egui, điều phối thread nền |
| `src/auth.rs` | Chuỗi đăng nhập Microsoft → Minecraft |
| `src/mojang.rs` | Version manifest, Fabric, libraries, natives, assets |
| `src/launch.rs` | Kiểm tra Java, dựng lệnh, chạy game |
| `src/modrinth.rs` | Tìm/cài mod, xử lý dependency |
| `src/skin.rs` | Tải/đổi skin, dựng ảnh xem trước |
| `src/net.rs` | HTTP client dùng chung, tải song song + SHA-1 |

## Hạn chế hiện tại

- Chưa tự tải Java: cần cài sẵn và (nếu cần) trỏ đường dẫn trong Cài đặt.
- Chỉ hỗ trợ bản *release*, loader Vanilla và Fabric; một instance duy nhất.
- Chưa hiện icon mod và chưa có cape.
