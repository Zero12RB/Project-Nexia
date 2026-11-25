use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

/// Azure Client ID mặc định của Nexia. Client ID là định danh công khai, không phải bí mật.
pub const DEFAULT_CLIENT_ID: &str = "dc6c6e5a-47d1-4550-8314-a61d4ef30dd5";

#[derive(Serialize, Deserialize, Clone)]
pub struct Account {
    pub username: String,
    pub uuid: String,
    pub mc_token: String,
    pub refresh_token: String,
    /// Unix timestamp (giây) khi token Minecraft hết hạn.
    pub expires_at: u64,
    /// Tài khoản offline: không có token thật, không cần làm mới.
    #[serde(default)]
    pub offline: bool,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub client_id: String,
    pub ram_mb: u32,
    pub java_path: String,
    pub mc_version: String,
    /// "vanilla" hoặc "fabric"
    pub loader: String,
    pub account: Option<Account>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            client_id: DEFAULT_CLIENT_ID.into(),
            ram_mb: 4096,
            java_path: "java".into(),
            mc_version: "1.21.1".into(),
            loader: "fabric".into(),
            account: None,
        }
    }
}

pub fn data_dir() -> PathBuf {
    let d = ProjectDirs::from("dev", "nexia", "Nexia")
        .map(|p| p.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("nexia-data"));
    let _ = fs::create_dir_all(&d);
    d
}

pub fn game_dir() -> PathBuf {
    let d = data_dir().join("instance");
    let _ = fs::create_dir_all(&d);
    d
}

pub fn mods_dir() -> PathBuf {
    let d = game_dir().join("mods");
    let _ = fs::create_dir_all(&d);
    d
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Config {
    pub fn load() -> Self {
        let mut c: Config = fs::read(data_dir().join("config.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if c.client_id.trim().is_empty() {
            if let Ok(v) = std::env::var("NEXIA_CLIENT_ID") {
                c.client_id = v;
            }
        }
        if c.client_id.trim().is_empty() {
            c.client_id = DEFAULT_CLIENT_ID.into();
        }
        c
    }

    pub fn save(&self) {
        let path = data_dir().join("config.json");
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            let _ = fs::write(&path, b);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // File chứa token đăng nhập: chỉ chủ sở hữu được đọc.
                let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
            }
        }
    }
}
