<div align="center">

<img src="logo" alt="Nexia Launcher" width="120">

# Nexia Launcher

**Launcher Minecraft Java viết bằng Rust — giao diện native, nhẹ, không Electron, không webview.**

![Rust](https://img.shields.io/badge/Rust-stable-orange?logo=rust)
![egui](https://img.shields.io/badge/UI-egui%20%2F%20eframe-blue)
![Platform](https://img.shields.io/badge/Windows-ch%C3%ADnh-0078D6?logo=windows)

</div>

Nexia Launcher là launcher Minecraft Java dành cho cộng đồng **NCA** ([nonca.tokyo](https://nonca.tokyo/)), giao diện tối theo phong cách nonca.tokyo: nền navy, accent xanh thép, card bo tròn, animation nhẹ.

## Tính năng

- **Viết bằng Rust**: tiết kiệm bộ nhớ, khởi động nhanh và tối ưu hơn so với launcher dùng Electron/webview.
- **Loader đầy đủ**: Vanilla, Fabric, Quilt, Forge, NeoForge.
- **Mod Library**: tìm và cài mod từ Modrinth, tự lọc theo phiên bản + loader và tự cài dependency.

## Tải về

Vào mục **Releases** để tải `Nexia-Launcher-Setup-x.y.z.exe` (installer) hoặc `Nexia-Launcher.exe` (chạy trực tiếp).

> Windows SmartScreen có thể cảnh báo vì file chưa có chữ ký số. Chọn **More info → Run anyway**.

## Build từ mã nguồn

Cần [Rust stable](https://rustup.rs/).

**Windows**

```cmd
BUILD-AND-RUN.cmd
```

Script build bản release, tạo `Nexia-Launcher.exe` ngay trong thư mục và mở launcher.

**Linux / macOS**

```bash
cargo run --release
```

Linux cần thêm thư viện cho egui (Debian/Ubuntu):

```bash
sudo apt install libxkbcommon-dev libwayland-dev libgl1-mesa-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
```

Linux/macOS hiện cần tự cài Java; chỉ Windows có tự tải Java.

**Kiểm thử**

```bash
cargo check --locked
cargo test --locked
```

### Tạo installer (Windows)

1. Cài [Inno Setup 6](https://jrsoftware.org/isdl.php).
2. Chạy `BUILD-AND-RUN.cmd` để có `Nexia-Launcher.exe`.
3. Chạy `BUILD-INSTALLER.cmd` → kết quả ở `dist\Nexia-Launcher-Setup-<version>.exe`.

Script installer nằm ở `installer/nexia.iss`. Dữ liệu người dùng nằm trong `%APPDATA%` nên gỡ cài đặt không xóa tài khoản hay cấu hình.

## Cấu hình

### Client ID Microsoft

Client ID mặc định nằm trong `src/config.rs` (`DEFAULT_CLIENT_ID`). Nếu dùng ứng dụng Entra riêng:

1. Vào <https://entra.microsoft.com/> → **App registrations**, chọn hoặc tạo ứng dụng hỗ trợ **personal Microsoft accounts**.
2. **Authentication** → thêm nền tảng **Mobile and desktop applications** với redirect URI `http://localhost`.
3. Bật **Allow public client flows**. Không dùng client secret.
4. Đặt Client ID vào `DEFAULT_CLIENT_ID` rồi build lại.

Mojang có thể yêu cầu phê duyệt ứng dụng trước khi cho gọi Minecraft Services API (lỗi `Invalid app registration`, xem <https://aka.ms/AppRegInfo>). Tài khoản cần sở hữu Minecraft Java Edition.

### Avatar Credits (GIF động)

Tab Credits hiển thị avatar Discord của nhà phát triển. Discord CDN cần hash avatar nên chỉ ID là không đủ. Để hiện ảnh thật, dán URL avatar vào `assets/credits-avatar-url.txt` rồi build lại:

```
https://cdn.discordapp.com/avatars/<user_id>/a_<hash>.gif
```

Hash bắt đầu bằng `a_` sẽ tải bản GIF động; hash thường tải PNG tĩnh. Nếu để trống, launcher dùng avatar mặc định của Discord. Không cần token Discord.

## Dữ liệu người dùng

Lưu trong thư mục dữ liệu của hệ điều hành (nút **Mở thư mục dữ liệu** trong Cài đặt):

| Thư mục / file | Nội dung |
|---|---|
| `instance/` | Thư mục game: `mods/`, `saves/`, `resourcepacks/`… |
| `libraries/`, `assets/`, `versions/` | File game do launcher quản lý |
| `config.json` | Cấu hình và phiên đăng nhập |

Trên Windows, token được mã hóa bằng DPAPI theo người dùng hiện tại. Trên Linux/macOS, file có quyền `600` nhưng token không mã hóa. **Không chia sẻ `config.json`.**

## Cấu trúc mã nguồn

| File | Vai trò |
|---|---|
| `src/main.rs` | Giao diện egui, điều phối thread nền |
| `src/auth.rs` | Chuỗi đăng nhập Microsoft → Xbox → Minecraft, Ely.by |
| `src/browser_auth.rs` | PKCE S256, callback loopback |
| `src/token_store.rs` | Mã hóa token bằng Windows DPAPI |
| `src/mojang.rs` | Version manifest, libraries, natives, assets |
| `src/loaders.rs` | Fabric, Quilt, Forge, NeoForge |
| `src/launch.rs` | Dựng lệnh và chạy game |
| `src/java_runtime.rs` | Tìm/tải Java, xác thực SHA-256 |
| `src/modrinth.rs` | Tìm/cài mod, dependency |
| `src/skin.rs`, `src/skin_game.rs` | Skin và tích hợp CustomSkinLoader |
| `src/servers.rs` | Thêm server NCA vào `servers.dat` |
| `src/credits.rs` | Avatar Discord, link credits |
| `src/net.rs` | HTTP client chung, tải song song + SHA-1 |

## Hạn chế hiện tại

- Tự tải Java chỉ hỗ trợ Windows.
- Chỉ hỗ trợ bản *release*; một instance duy nhất.
- Forge trước 1.13 và NeoForge 1.20.1 chưa hỗ trợ; chưa hỗ trợ OptiFine độc lập, LiteLoader, Legacy Fabric.
- Skin Offline/Ely.by trong game chỉ hoạt động với Fabric.
- Skin cục bộ chỉ hiển thị ở client của bạn, không vượt qua xác thực server online-mode.
- Chưa có cape và icon mod.

Chi tiết các thay đổi theo từng phiên bản xem trong [`UI-CHANGES.md`](UI-CHANGES.md).

## Credits

- **Rei** — nhà phát triển Nexia Launcher · [stellarei.id.vn](https://stellarei.id.vn/)
- Danh sách đầy đủ: <https://nonca.tokyo/credits/>

Xây dựng với [Rust](https://www.rust-lang.org/), [egui / eframe](https://github.com/emilk/egui), [Modrinth](https://modrinth.com/), [Eclipse Adoptium](https://adoptium.net/), [CustomSkinLoader](https://github.com/xfl03/MCCustomSkinLoader).

## Tuyên bố

Nexia Launcher là dự án cộng đồng, **không liên kết với Mojang Studios, Microsoft hay Ely.by**. Minecraft là thương hiệu của Mojang Studios.

## Giấy phép
[MIT](https://github.com/Zero12RB/Project-Nexia/tree/main?tab=MIT-1-ov-file)
