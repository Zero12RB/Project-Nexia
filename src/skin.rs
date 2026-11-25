//! Xem và đổi skin qua Minecraft Services API.

use crate::{auth, net::client};
use anyhow::{anyhow, bail, Result};
use image::{Rgba, RgbaImage};
use reqwest::blocking::multipart::Form;
use std::path::Path;

/// Đọc file skin và kiểm tra kích thước hợp lệ (64x64 hoặc 64x32).
pub fn load_local(p: &Path) -> Result<RgbaImage> {
    let img = image::open(p)
        .map_err(|e| anyhow!("Không đọc được ảnh: {e}"))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    if !(w == 64 && (h == 64 || h == 32)) {
        bail!("Skin phải là PNG 64x64 hoặc 64x32 (file này là {w}x{h})");
    }
    Ok(img)
}

/// Tải skin đang dùng của tài khoản. Trả về (ảnh, có phải kiểu tay nhỏ "slim").
pub fn current(token: &str) -> Result<(RgbaImage, bool)> {
    let p = auth::profile(token)?;
    let skin = p["skins"]
        .as_array()
        .and_then(|a| a.iter().find(|s| s["state"] == "ACTIVE"))
        .ok_or_else(|| anyhow!("Tài khoản chưa có skin"))?;
    let url = skin["url"].as_str().ok_or_else(|| anyhow!("skin không có url"))?;
    let slim = skin["variant"] == "SLIM";
    let bytes = client().get(url).send()?.error_for_status()?.bytes()?;
    let img = image::load_from_memory(&bytes)?.to_rgba8();
    Ok((img, slim))
}

pub fn upload(token: &str, path: &Path, slim: bool) -> Result<()> {
    load_local(path)?; // kiểm tra trước khi gửi
    let form = Form::new()
        .text("variant", if slim { "slim" } else { "classic" })
        .file("file", path)?;
    let r = client()
        .post("https://api.minecraftservices.com/minecraft/profile/skins")
        .bearer_auth(token)
        .multipart(form)
        .send()?;
    if !r.status().is_success() {
        let code = r.status();
        bail!("Đổi skin thất bại ({code}): {}", r.text().unwrap_or_default());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn blit(
    dst: &mut RgbaImage,
    src: &RgbaImage,
    sx: u32,
    sy: u32,
    w: u32,
    h: u32,
    dx: u32,
    dy: u32,
    overlay: bool,
) {
    if sx + w > src.width() || sy + h > src.height() {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let s = src.get_pixel(sx + x, sy + y).0;
            let d = dst.get_pixel_mut(dx + x, dy + y);
            if overlay {
                let a = s[3] as u32;
                if a == 0 {
                    continue;
                }
                for i in 0..3 {
                    d.0[i] = ((s[i] as u32 * a + d.0[i] as u32 * (255 - a)) / 255) as u8;
                }
                d.0[3] = 255;
            } else {
                *d = Rgba([s[0], s[1], s[2], 255]);
            }
        }
    }
}

/// Ghép ảnh nhân vật nhìn từ phía trước (16x32 px) từ texture skin, gồm cả lớp áo ngoài.
pub fn front_preview(skin: &RgbaImage, slim: bool) -> RgbaImage {
    let mut out = RgbaImage::new(16, 32);
    let modern = skin.height() >= 64;
    let aw = if slim { 3 } else { 4 };

    blit(&mut out, skin, 8, 8, 8, 8, 4, 0, false); // đầu
    blit(&mut out, skin, 20, 20, 8, 12, 4, 8, false); // thân
    blit(&mut out, skin, 44, 20, aw, 12, 4 - aw, 8, false); // tay phải
    let (lax, lay) = if modern { (36, 52) } else { (44, 20) };
    blit(&mut out, skin, lax, lay, aw, 12, 12, 8, false); // tay trái
    blit(&mut out, skin, 4, 20, 4, 12, 4, 20, false); // chân phải
    let (llx, lly) = if modern { (20, 52) } else { (4, 20) };
    blit(&mut out, skin, llx, lly, 4, 12, 8, 20, false); // chân trái

    if modern {
        blit(&mut out, skin, 40, 8, 8, 8, 4, 0, true); // mũ/tóc
        blit(&mut out, skin, 20, 36, 8, 12, 4, 8, true); // áo
        blit(&mut out, skin, 44, 36, aw, 12, 4 - aw, 8, true); // tay áo phải
        blit(&mut out, skin, 52, 52, aw, 12, 12, 8, true); // tay áo trái
        blit(&mut out, skin, 4, 36, 4, 12, 4, 20, true); // quần phải
        blit(&mut out, skin, 4, 52, 4, 12, 8, 20, true); // quần trái
    }
    out
}
