use anyhow::{anyhow, Result};
use reqwest::blocking::Client;
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};

pub const UA: &str = "NexiaLauncher/0.1 (open-source minecraft launcher)";

/// Một HTTP client dùng chung (connection pool, keep-alive) cho cả chương trình.
pub fn client() -> &'static Client {
    static C: OnceLock<Client> = OnceLock::new();
    C.get_or_init(|| {
        Client::builder()
            .user_agent(UA)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .build()
            .expect("không tạo được HTTP client")
    })
}

pub fn get_json(url: &str) -> Result<Value> {
    Ok(client().get(url).send()?.error_for_status()?.json()?)
}

pub fn sha1_file(p: &Path) -> Option<String> {
    let mut f = fs::File::open(p).ok()?;
    let mut h = Sha1::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(hex::encode(h.finalize()))
}

#[derive(Clone)]
pub struct Job {
    pub url: String,
    pub path: PathBuf,
    pub sha1: Option<String>,
}

/// Tải một file. Nếu file đã tồn tại thì bỏ qua (tên file đã nằm trong thư mục
/// theo hash/phiên bản, và ta chỉ ghi file sau khi đã kiểm tra sha1 xong).
pub fn download(job: &Job) -> Result<()> {
    if job.path.exists() {
        return Ok(());
    }
    if let Some(p) = job.path.parent() {
        fs::create_dir_all(p)?;
    }
    let mut tmp_name = job.path.as_os_str().to_owned();
    tmp_name.push(".part");
    let tmp = PathBuf::from(tmp_name);

    let mut resp = client().get(&job.url).send()?.error_for_status()?;
    {
        let mut f = fs::File::create(&tmp)?;
        std::io::copy(&mut resp, &mut f)?;
    }
    if let Some(want) = &job.sha1 {
        let got = sha1_file(&tmp).unwrap_or_default();
        if !got.eq_ignore_ascii_case(want) {
            let _ = fs::remove_file(&tmp);
            return Err(anyhow!("sai sha1 khi tải {}", job.url));
        }
    }
    fs::rename(&tmp, &job.path)?;
    Ok(())
}

pub struct Progress {
    pub total: AtomicUsize,
    pub done: AtomicUsize,
    status: Mutex<String>,
}

impl Progress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            total: AtomicUsize::new(0),
            done: AtomicUsize::new(0),
            status: Mutex::new(String::new()),
        })
    }
    pub fn set(&self, s: &str) {
        *self.status.lock().unwrap() = s.to_string();
    }
    pub fn snapshot(&self) -> (String, usize, usize) {
        (
            self.status.lock().unwrap().clone(),
            self.done.load(Ordering::Relaxed),
            self.total.load(Ordering::Relaxed),
        )
    }
}

/// Tải song song nhiều file bằng một nhóm thread (I/O bound nên dùng nhiều thread hơn số core).
pub fn download_many(jobs: Vec<Job>, prog: &Progress) -> Result<()> {
    prog.total.store(jobs.len(), Ordering::Relaxed);
    prog.done.store(0, Ordering::Relaxed);
    let queue = Mutex::new(jobs);
    let errors: Mutex<Vec<String>> = Mutex::new(Vec::new());

    std::thread::scope(|s| {
        for _ in 0..16 {
            s.spawn(|| loop {
                let job = queue.lock().unwrap().pop();
                let Some(job) = job else { break };
                let mut last_err = None;
                for _ in 0..3 {
                    match download(&job) {
                        Ok(()) => {
                            last_err = None;
                            break;
                        }
                        Err(e) => last_err = Some(format!("{e:#}")),
                    }
                }
                if let Some(e) = last_err {
                    errors.lock().unwrap().push(e);
                }
                prog.done.fetch_add(1, Ordering::Relaxed);
            });
        }
    });

    let errs = errors.into_inner().unwrap();
    if let Some(first) = errs.first() {
        return Err(anyhow!("{} file tải lỗi, ví dụ: {first}", errs.len()));
    }
    Ok(())
}
