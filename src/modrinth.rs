//! Tìm và cài mod từ Modrinth (API v2), tự cài các mod phụ thuộc bắt buộc.

use crate::{
    config::mods_dir,
    net::{client, download, Job},
};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::{collections::HashSet, fs, path::Path};

#[derive(Clone)]
pub struct Hit {
    pub id: String,
    pub title: String,
    pub desc: String,
    pub author: String,
    pub downloads: u64,
}

pub fn search(query: &str, mc: &str, loader: &str) -> Result<Vec<Hit>> {
    let mut facets = vec![r#"["project_type:mod"]"#.to_string()];
    if !mc.is_empty() {
        facets.push(format!(r#"["versions:{mc}"]"#));
    }
    if loader != "vanilla" {
        facets.push(format!(r#"["categories:{loader}"]"#));
    }
    let facets = format!("[{}]", facets.join(","));
    let index = if query.trim().is_empty() { "downloads" } else { "relevance" };

    let v: Value = client()
        .get("https://api.modrinth.com/v2/search")
        .query(&[
            ("query", query),
            ("facets", facets.as_str()),
            ("limit", "20"),
            ("index", index),
        ])
        .send()?
        .error_for_status()?
        .json()?;

    Ok(v["hits"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|h| Hit {
            id: h["project_id"].as_str().unwrap_or("").to_string(),
            title: h["title"].as_str().unwrap_or("?").to_string(),
            desc: h["description"].as_str().unwrap_or("").to_string(),
            author: h["author"].as_str().unwrap_or("").to_string(),
            downloads: h["downloads"].as_u64().unwrap_or(0),
        })
        .collect())
}

pub fn install(
    project_id: &str,
    mc: &str,
    loader: &str,
    visited: &mut HashSet<String>,
    log: &dyn Fn(String),
) -> Result<()> {
    if !visited.insert(project_id.to_string()) {
        return Ok(());
    }
    let loaders = format!("[\"{loader}\"]");
    let gvs = format!("[\"{mc}\"]");
    let vs: Value = client()
        .get(format!("https://api.modrinth.com/v2/project/{project_id}/version"))
        .query(&[("loaders", loaders.as_str()), ("game_versions", gvs.as_str())])
        .send()?
        .error_for_status()?
        .json()?;
    let list = vs.as_array().ok_or_else(|| anyhow!("Modrinth trả về dữ liệu lạ"))?;
    let ver = list
        .iter()
        .find(|v| v["version_type"] == "release")
        .or_else(|| list.first())
        .ok_or_else(|| anyhow!("Không có bản mod tương thích với Minecraft {mc} / {loader}"))?;

    let files = ver["files"].as_array().ok_or_else(|| anyhow!("phiên bản không có file"))?;
    let f = files
        .iter()
        .find(|f| f["primary"] == true)
        .or_else(|| files.first())
        .ok_or_else(|| anyhow!("phiên bản không có file"))?;

    // Chỉ lấy tên file, chặn path traversal.
    let raw_name = f["filename"].as_str().ok_or_else(|| anyhow!("thiếu filename"))?;
    let filename = Path::new(raw_name)
        .file_name()
        .ok_or_else(|| anyhow!("tên file không hợp lệ"))?
        .to_string_lossy()
        .into_owned();

    log(format!("Đang tải {filename}…"));
    download(&Job {
        url: f["url"].as_str().ok_or_else(|| anyhow!("thiếu url"))?.into(),
        path: mods_dir().join(&filename),
        sha1: f["hashes"]["sha1"].as_str().map(String::from),
    })?;

    for d in ver["dependencies"].as_array().into_iter().flatten() {
        if d["dependency_type"] == "required" {
            if let Some(pid) = d["project_id"].as_str() {
                if let Err(e) = install(pid, mc, loader, visited, log) {
                    log(format!("Không cài được mod phụ thuộc {pid}: {e:#}"));
                }
            }
        }
    }
    Ok(())
}

pub fn list_installed() -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(mods_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".jar"))
        .collect();
    v.sort_by_key(|s| s.to_lowercase());
    v
}

pub fn remove(name: &str) {
    if let Some(n) = Path::new(name).file_name() {
        let _ = fs::remove_file(mods_dir().join(n));
    }
}
