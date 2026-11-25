use crate::{
    config::{data_dir, game_dir, Account, Config},
    mojang::{cp_sep, Prepared},
};
use anyhow::{anyhow, bail, Result};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::Arc,
};

pub type Logger = Arc<dyn Fn(String) + Send + Sync>;

#[cfg(windows)]
fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}
#[cfg(not(windows))]
fn hide_console(_: &mut Command) {}

/// Đọc phiên bản Java chính (8, 17, 21…) từ `java -version`.
pub fn java_major(java: &str) -> Result<u64> {
    let mut cmd = Command::new(java);
    cmd.arg("-version");
    hide_console(&mut cmd);
    let out = cmd.output().map_err(|e| {
        anyhow!("Không chạy được Java ('{java}'): {e}. Hãy cài Java hoặc đặt đường dẫn trong Cài đặt.")
    })?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stderr),
        String::from_utf8_lossy(&out.stdout)
    );
    let start = text
        .find("version \"")
        .ok_or_else(|| anyhow!("Không đọc được phiên bản Java"))?
        + 9;
    let rest = &text[start..];
    let end = rest.find('"').unwrap_or(rest.len());
    let mut parts = rest[..end].split(|c| c == '.' || c == '_' || c == '-');
    let first: u64 = parts
        .next()
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| anyhow!("Không đọc được phiên bản Java"))?;
    if first == 1 {
        parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or_else(|| anyhow!("Không đọc được phiên bản Java"))
    } else {
        Ok(first)
    }
}

fn sub(s: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = s.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("${{{k}}}"), v);
    }
    out
}

pub fn launch(cfg: &Config, acc: &Account, p: &Prepared, log: Logger) -> Result<Child> {
    let have = java_major(&cfg.java_path)?;
    if have < p.java_major {
        bail!(
            "Minecraft {} cần Java {}+, nhưng máy đang dùng Java {}. Hãy cài Java mới hơn hoặc chỉnh đường dẫn Java trong Cài đặt.",
            p.version_id,
            p.java_major,
            have
        );
    }

    let game = game_dir();
    let assets_root = data_dir().join("assets");
    let s = |p: &std::path::Path| p.to_string_lossy().into_owned();

    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("auth_player_name", acc.username.clone());
    vars.insert("version_name", p.version_id.clone());
    vars.insert("game_directory", s(&game));
    vars.insert("assets_root", s(&assets_root));
    vars.insert("game_assets", s(&assets_root.join("virtual").join("legacy")));
    vars.insert("assets_index_name", p.asset_index.clone());
    vars.insert("auth_uuid", acc.uuid.clone());
    vars.insert("auth_access_token", acc.mc_token.clone());
    vars.insert("auth_session", format!("token:{}:{}", acc.mc_token, acc.uuid));
    vars.insert("clientid", String::new());
    vars.insert("auth_xuid", String::new());
    vars.insert("user_type", if acc.offline { "legacy" } else { "msa" }.into());
    vars.insert("user_properties", "{}".into());
    vars.insert("version_type", "release".into());
    vars.insert("natives_directory", s(&p.natives_dir));
    vars.insert("launcher_name", "Nexia".into());
    vars.insert("launcher_version", env!("CARGO_PKG_VERSION").into());
    vars.insert("classpath", p.classpath.clone());
    vars.insert("library_directory", s(&data_dir().join("libraries")));
    vars.insert("classpath_separator", cp_sep().into());

    let mut args: Vec<String> = vec![
        format!("-Xmx{}M", cfg.ram_mb),
        format!("-Xms{}M", (cfg.ram_mb / 2).max(512)),
        // Bộ cờ GC nhẹ, giảm giật lag cho client.
        "-XX:+UseG1GC".into(),
        "-XX:+ParallelRefProcEnabled".into(),
        "-XX:MaxGCPauseMillis=130".into(),
        "-XX:+UnlockExperimentalVMOptions".into(),
        "-XX:+DisableExplicitGC".into(),
        "-XX:G1NewSizePercent=30".into(),
        "-XX:G1MaxNewSizePercent=40".into(),
        "-XX:G1HeapRegionSize=8M".into(),
        "-XX:G1ReservePercent=20".into(),
        "-XX:InitiatingHeapOccupancyPercent=15".into(),
        "-XX:+PerfDisableSharedMem".into(),
    ];
    args.extend(p.jvm_args.iter().map(|a| sub(a, &vars)));
    args.push(p.main_class.clone());
    args.extend(p.game_args.iter().map(|a| sub(a, &vars)));

    let mut cmd = Command::new(&cfg.java_path);
    cmd.args(&args)
        .current_dir(&game)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| anyhow!("Không khởi động được Java: {e}"))?;

    if let Some(o) = child.stdout.take() {
        let l = log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(o).lines().map_while(Result::ok) {
                l(line);
            }
        });
    }
    if let Some(e) = child.stderr.take() {
        let l = log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(e).lines().map_while(Result::ok) {
                l(line);
            }
        });
    }
    Ok(child)
}
