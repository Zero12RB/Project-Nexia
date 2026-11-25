//! Tải phiên bản Minecraft (vanilla hoặc Fabric): version json, libraries, natives, assets.

use crate::{
    config::data_dir,
    net::{download, download_many, get_json, Job, Progress},
};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

const MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

pub struct Prepared {
    pub version_id: String,
    pub main_class: String,
    pub classpath: String,
    pub jvm_args: Vec<String>,
    pub game_args: Vec<String>,
    pub asset_index: String,
    pub natives_dir: PathBuf,
    pub java_major: u64,
}

pub fn os_name() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

pub fn cp_sep() -> &'static str {
    if cfg!(windows) {
        ";"
    } else {
        ":"
    }
}

pub fn list_versions() -> Result<Vec<String>> {
    let m = get_json(MANIFEST)?;
    Ok(m["versions"]
        .as_array()
        .ok_or_else(|| anyhow!("manifest không hợp lệ"))?
        .iter()
        .filter(|v| v["type"] == "release")
        .filter_map(|v| v["id"].as_str().map(String::from))
        .collect())
}

fn os_matches(os: Option<&Value>) -> bool {
    let Some(os) = os else { return true };
    if let Some(n) = os["name"].as_str() {
        if n != os_name() {
            return false;
        }
    }
    if let Some(a) = os["arch"].as_str() {
        let cur = std::env::consts::ARCH;
        let ok = match a {
            "x86" => cur == "x86",
            "arm64" => cur == "aarch64",
            _ => true,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// Đánh giá mảng `rules` theo quy tắc của launcher Mojang (rule có `features` bị bỏ qua).
fn rules_ok(rules: Option<&Value>) -> bool {
    let Some(arr) = rules.and_then(|r| r.as_array()) else {
        return true;
    };
    let mut allowed = false;
    for r in arr {
        if os_matches(r.get("os")) && r.get("features").is_none() {
            allowed = r["action"] == "allow";
        }
    }
    allowed
}

fn maven_path(name: &str) -> String {
    let mut p = name.split(':');
    let g = p.next().unwrap_or("").replace('.', "/");
    let a = p.next().unwrap_or("");
    let v = p.next().unwrap_or("");
    match p.next() {
        Some(c) => format!("{g}/{a}/{v}/{a}-{v}-{c}.jar"),
        None => format!("{g}/{a}/{v}/{a}-{v}.jar"),
    }
}

fn collect_args(v: Option<&Value>) -> Vec<String> {
    let mut out = Vec::new();
    for a in v.and_then(|x| x.as_array()).into_iter().flatten() {
        match a {
            Value::String(s) => out.push(s.clone()),
            Value::Object(_) => {
                if rules_ok(a.get("rules")) {
                    match &a["value"] {
                        Value::String(s) => out.push(s.clone()),
                        Value::Array(arr) => {
                            out.extend(arr.iter().filter_map(|x| x.as_str().map(String::from)))
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn merge_fabric(ver: &mut Value, mc: &str) -> Result<()> {
    let list = get_json(&format!("https://meta.fabricmc.net/v2/versions/loader/{mc}"))?;
    let arr = list.as_array().ok_or_else(|| anyhow!("Fabric trả về dữ liệu lạ"))?;
    let entry = arr
        .iter()
        .find(|e| e["loader"]["stable"] == true)
        .or_else(|| arr.first())
        .ok_or_else(|| anyhow!("Fabric chưa hỗ trợ Minecraft {mc}"))?;
    let lv = entry["loader"]["version"]
        .as_str()
        .ok_or_else(|| anyhow!("không đọc được phiên bản Fabric loader"))?;
    let prof = get_json(&format!(
        "https://meta.fabricmc.net/v2/versions/loader/{mc}/{lv}/profile/json"
    ))?;

    ver["mainClass"] = prof["mainClass"].clone();

    let mut libs = prof["libraries"].as_array().cloned().unwrap_or_default();
    libs.extend(ver["libraries"].as_array().cloned().unwrap_or_default());
    ver["libraries"] = Value::Array(libs);

    for k in ["jvm", "game"] {
        if let Some(extra) = prof["arguments"][k].as_array() {
            let mut merged = ver["arguments"][k].as_array().cloned().unwrap_or_default();
            merged.extend(extra.iter().cloned());
            ver["arguments"][k] = Value::Array(merged);
        }
    }
    Ok(())
}

fn extract_natives(jar: &Path, out: &Path) -> Result<()> {
    let mut z = zip::ZipArchive::new(fs::File::open(jar)?)?;
    for i in 0..z.len() {
        let mut f = z.by_index(i)?;
        let name = f.name().to_string();
        if f.is_dir() || name.starts_with("META-INF") {
            continue;
        }
        let Some(fname) = Path::new(&name).file_name() else {
            continue;
        };
        let mut o = fs::File::create(out.join(fname))?;
        std::io::copy(&mut f, &mut o)?;
    }
    Ok(())
}

pub fn prepare(mc: &str, loader: &str, prog: &Progress) -> Result<Prepared> {
    let root = data_dir();
    let libs_dir = root.join("libraries");
    let assets_dir = root.join("assets");
    let ver_dir = root.join("versions").join(mc);
    fs::create_dir_all(&ver_dir)?;

    prog.set("Đang lấy thông tin phiên bản…");
    let manifest = get_json(MANIFEST)?;
    let entry = manifest["versions"]
        .as_array()
        .and_then(|a| a.iter().find(|v| v["id"] == mc))
        .ok_or_else(|| anyhow!("Không tìm thấy phiên bản {mc}"))?;
    let vjson_path = ver_dir.join(format!("{mc}.json"));
    download(&Job {
        url: entry["url"].as_str().ok_or_else(|| anyhow!("thiếu url version"))?.into(),
        path: vjson_path.clone(),
        sha1: entry["sha1"].as_str().map(String::from),
    })?;
    let mut ver: Value = serde_json::from_slice(&fs::read(&vjson_path)?)?;

    if loader == "fabric" {
        prog.set("Đang lấy Fabric loader…");
        merge_fabric(&mut ver, mc)?;
    }

    let natives_dir = ver_dir.join("natives");
    fs::create_dir_all(&natives_dir)?;

    let mut jobs: Vec<Job> = Vec::new();
    let mut classpath: Vec<PathBuf> = Vec::new();
    let mut native_jars: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // ---- Libraries ----
    for lib in ver["libraries"].as_array().into_iter().flatten() {
        if !rules_ok(lib.get("rules")) {
            continue;
        }
        let name = lib["name"].as_str().unwrap_or("");

        // Natives kiểu cũ (classifiers) — các bản trước 1.19.
        if let Some(nat) = lib["natives"][os_name()].as_str() {
            let key = nat.replace(
                "${arch}",
                if cfg!(target_pointer_width = "64") { "64" } else { "32" },
            );
            if let Some(a) = lib["downloads"]["classifiers"].get(&key) {
                if let (Some(url), Some(path)) = (a["url"].as_str(), a["path"].as_str()) {
                    let p = libs_dir.join(path);
                    jobs.push(Job {
                        url: url.into(),
                        path: p.clone(),
                        sha1: a["sha1"].as_str().map(String::from),
                    });
                    native_jars.push(p);
                }
            }
        }

        let (url, rel, sha1): (String, String, Option<String>) =
            if let Some(a) = lib["downloads"].get("artifact") {
                match (a["url"].as_str(), a["path"].as_str()) {
                    (Some(u), Some(p)) if !u.is_empty() => {
                        (u.into(), p.into(), a["sha1"].as_str().map(String::from))
                    }
                    _ => continue,
                }
            } else if let Some(base) = lib["url"].as_str() {
                let p = maven_path(name);
                (
                    format!("{}/{}", base.trim_end_matches('/'), p),
                    p,
                    lib["sha1"].as_str().map(String::from),
                )
            } else {
                continue;
            };

        if seen.insert(rel.clone()) {
            let path = libs_dir.join(&rel);
            jobs.push(Job { url, path: path.clone(), sha1 });
            classpath.push(path);
        }
    }

    // ---- client.jar ----
    let cj = &ver["downloads"]["client"];
    let client_jar = ver_dir.join(format!("{mc}.jar"));
    jobs.push(Job {
        url: cj["url"].as_str().ok_or_else(|| anyhow!("thiếu client.jar"))?.into(),
        path: client_jar.clone(),
        sha1: cj["sha1"].as_str().map(String::from),
    });
    classpath.push(client_jar);

    // ---- Assets ----
    prog.set("Đang lấy danh sách tài nguyên…");
    let ai = &ver["assetIndex"];
    let ai_id = ai["id"].as_str().unwrap_or(mc).to_string();
    let ai_path = assets_dir.join("indexes").join(format!("{ai_id}.json"));
    download(&Job {
        url: ai["url"].as_str().ok_or_else(|| anyhow!("thiếu assetIndex.url"))?.into(),
        path: ai_path.clone(),
        sha1: ai["sha1"].as_str().map(String::from),
    })?;
    let idx: Value = serde_json::from_slice(&fs::read(&ai_path)?)?;
    let mut seen_hash: HashSet<String> = HashSet::new();
    if let Some(objs) = idx["objects"].as_object() {
        for obj in objs.values() {
            let Some(h) = obj["hash"].as_str() else { continue };
            if h.len() < 2 || !seen_hash.insert(h.to_string()) {
                continue;
            }
            let pre = &h[..2];
            jobs.push(Job {
                url: format!("https://resources.download.minecraft.net/{pre}/{h}"),
                path: assets_dir.join("objects").join(pre).join(h),
                sha1: Some(h.to_string()),
            });
        }
    }

    // Chỉ giữ lại những file còn thiếu → lần chạy sau khởi động gần như tức thì.
    jobs.retain(|j| !j.path.exists());
    prog.set("Đang tải thư viện & tài nguyên…");
    download_many(jobs, prog)?;

    for jar in &native_jars {
        extract_natives(jar, &natives_dir)?;
    }

    // ---- Arguments ----
    let (mut jvm, game) = if ver.get("arguments").is_some() {
        (
            collect_args(ver["arguments"].get("jvm")),
            collect_args(ver["arguments"].get("game")),
        )
    } else {
        (
            vec![
                "-Djava.library.path=${natives_directory}".to_string(),
                "-cp".to_string(),
                "${classpath}".to_string(),
            ],
            ver["minecraftArguments"]
                .as_str()
                .unwrap_or("")
                .split_whitespace()
                .map(String::from)
                .collect(),
        )
    };
    if !jvm.iter().any(|a| a.contains("${classpath}")) {
        jvm.push("-cp".into());
        jvm.push("${classpath}".into());
    }
    if !jvm.iter().any(|a| a.contains("java.library.path")) {
        jvm.push("-Djava.library.path=${natives_directory}".into());
    }

    Ok(Prepared {
        version_id: mc.to_string(),
        main_class: ver["mainClass"]
            .as_str()
            .ok_or_else(|| anyhow!("thiếu mainClass"))?
            .to_string(),
        classpath: classpath
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(cp_sep()),
        jvm_args: jvm,
        game_args: game,
        asset_index: ai_id,
        natives_dir,
        java_major: ver["javaVersion"]["majorVersion"].as_u64().unwrap_or(8),
    })
}
