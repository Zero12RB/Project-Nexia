//! Đăng nhập Microsoft → Xbox Live → XSTS → Minecraft Services.
//! Dùng "device code flow": launcher hiện một mã, người dùng nhập mã đó trên trình duyệt.
//! Cách này không cần client secret, không cần mở cổng localhost.

use crate::{
    config::{now, Account},
    net::client,
};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

const TENANT: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0";
const SCOPE: &str = "XboxLive.signin offline_access";

#[derive(Clone)]
pub struct DeviceCode {
    pub user_code: String,
    pub device_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_in: u64,
}

pub fn start_device_flow(client_id: &str) -> Result<DeviceCode> {
    if client_id.trim().is_empty() {
        bail!("Chưa có Azure Client ID. Vào tab Cài đặt để nhập (xem README để biết cách tạo).");
    }
    let v: Value = client()
        .post(format!("{TENANT}/devicecode"))
        .form(&[("client_id", client_id), ("scope", SCOPE)])
        .send()?
        .json()?;
    if let Some(e) = v["error_description"].as_str() {
        bail!("{e}");
    }
    Ok(DeviceCode {
        user_code: v["user_code"].as_str().ok_or_else(|| anyhow!("thiếu user_code"))?.into(),
        device_code: v["device_code"].as_str().ok_or_else(|| anyhow!("thiếu device_code"))?.into(),
        verification_uri: v["verification_uri"]
            .as_str()
            .unwrap_or("https://microsoft.com/link")
            .into(),
        interval: v["interval"].as_u64().unwrap_or(5),
        expires_in: v["expires_in"].as_u64().unwrap_or(900),
    })
}

/// Chặn cho tới khi người dùng đăng nhập xong, hết hạn, hoặc `cancel` được bật.
pub fn poll_device_flow(client_id: &str, dc: &DeviceCode, cancel: &AtomicBool) -> Result<Account> {
    let mut interval = dc.interval.max(1);
    let deadline = now() + dc.expires_in;
    loop {
        // Ngủ từng nhịp 250ms để huỷ phản hồi nhanh.
        let mut waited = 0u64;
        while waited < interval * 1000 {
            if cancel.load(Ordering::Relaxed) {
                bail!("đã huỷ");
            }
            std::thread::sleep(Duration::from_millis(250));
            waited += 250;
        }
        if now() > deadline {
            bail!("Mã đã hết hạn, hãy thử đăng nhập lại");
        }
        let v: Value = client()
            .post(format!("{TENANT}/token"))
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", client_id),
                ("device_code", dc.device_code.as_str()),
            ])
            .send()?
            .json()?;
        if let Some(at) = v["access_token"].as_str() {
            return finish(at, v["refresh_token"].as_str().unwrap_or(""));
        }
        match v["error"].as_str() {
            Some("authorization_pending") => continue,
            Some("slow_down") => interval += 5,
            Some(_) => bail!("{}", v["error_description"].as_str().unwrap_or("Đăng nhập thất bại")),
            None => bail!("Phản hồi đăng nhập không hợp lệ"),
        }
    }
}

pub fn refresh(client_id: &str, refresh_token: &str) -> Result<Account> {
    let v: Value = client()
        .post(format!("{TENANT}/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("scope", SCOPE),
        ])
        .send()?
        .json()?;
    let at = v["access_token"]
        .as_str()
        .ok_or_else(|| anyhow!("Phiên đăng nhập đã hết hạn, hãy đăng nhập lại"))?;
    finish(at, v["refresh_token"].as_str().unwrap_or(refresh_token))
}

fn finish(ms_access: &str, ms_refresh: &str) -> Result<Account> {
    let c = client();

    // 1) Xbox Live
    let xbl: Value = c
        .post("https://user.auth.xboxlive.com/user/authenticate")
        .json(&json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={ms_access}")
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        }))
        .send()?
        .json()?;
    let xbl_token = xbl["Token"].as_str().ok_or_else(|| anyhow!("Xbox Live từ chối đăng nhập"))?;
    let uhs = xbl["DisplayClaims"]["xui"][0]["uhs"]
        .as_str()
        .ok_or_else(|| anyhow!("Xbox Live không trả về uhs"))?;

    // 2) XSTS
    let r = c
        .post("https://xsts.auth.xboxlive.com/xsts/authorize")
        .json(&json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl_token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT"
        }))
        .send()?;
    let ok = r.status().is_success();
    let xsts: Value = r.json()?;
    if !ok {
        let msg = match xsts["XErr"].as_u64().unwrap_or(0) {
            2148916233 => "Tài khoản Microsoft này chưa có tài khoản Xbox. Hãy tạo tại xbox.com rồi thử lại.",
            2148916235 => "Xbox Live không khả dụng ở quốc gia/vùng của tài khoản này.",
            2148916238 => "Tài khoản trẻ em cần được người lớn thêm vào Microsoft Family trước.",
            _ => "XSTS từ chối đăng nhập.",
        };
        bail!("{}", msg);
    }
    let xsts_token = xsts["Token"].as_str().ok_or_else(|| anyhow!("XSTS không trả về token"))?;

    // 3) Minecraft Services
    let mc: Value = c
        .post("https://api.minecraftservices.com/authentication/login_with_xbox")
        .json(&json!({ "identityToken": format!("XBL3.0 x={uhs};{xsts_token}") }))
        .send()?
        .json()?;
    let mc_token = match mc["access_token"].as_str() {
        Some(t) => t.to_string(),
        None => bail!(
            "Minecraft từ chối đăng nhập: {}. Nếu là 'Invalid app registration', Azure app của bạn \
             chưa được Mojang duyệt (xem README).",
            mc["errorMessage"].as_str().or(mc["error"].as_str()).unwrap_or("không rõ lý do")
        ),
    };
    let expires_in = mc["expires_in"].as_u64().unwrap_or(86400);

    // 4) Hồ sơ người chơi (đồng thời kiểm tra có sở hữu game)
    let p = profile(&mc_token)?;
    Ok(Account {
        username: p["name"].as_str().unwrap_or("Player").to_string(),
        uuid: p["id"].as_str().unwrap_or_default().to_string(),
        mc_token,
        refresh_token: ms_refresh.to_string(),
        expires_at: now() + expires_in.saturating_sub(60),
        offline: false,
    })
}

/// Tạo tài khoản offline. UUID tính theo cách vanilla dùng cho offline-mode
/// (MD5 của "OfflinePlayer:<tên>", UUID v3) nên giữ nguyên dữ liệu người chơi giữa các launcher.
pub fn offline_account(name: &str) -> Account {
    let mut h = md5::compute(format!("OfflinePlayer:{name}")).0;
    h[6] = (h[6] & 0x0f) | 0x30;
    h[8] = (h[8] & 0x3f) | 0x80;
    Account {
        username: name.to_string(),
        uuid: hex::encode(h),
        mc_token: "0".into(),
        refresh_token: String::new(),
        expires_at: u64::MAX,
        offline: true,
    }
}

pub fn profile(token: &str) -> Result<Value> {
    let r = client()
        .get("https://api.minecraftservices.com/minecraft/profile")
        .bearer_auth(token)
        .send()?;
    if r.status().as_u16() == 404 {
        bail!("Tài khoản này chưa sở hữu Minecraft: Java Edition");
    }
    Ok(r.error_for_status()?.json()?)
}
