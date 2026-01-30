use clap::{Parser, Subcommand};
use chrono::{Duration, Local, NaiveDateTime, TimeZone, LocalResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufReader, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use md5::compute;
use serde_json::json;
use indicatif::{ProgressBar, ProgressStyle, MultiProgress};
use memmap2::MmapOptions;
use fs2;
use libc;
use std::cmp;
use bytesize::ByteSize;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

const DEFAULT_EXPIRE_DAYS: i64 = 7;
const MAX_LOG_AGE_DAYS: i64 = 30;
const PROTECTED_PATHS: [&str; 8] = ["/bin", "/sbin", "/etc", "/usr", "/lib", "/lib64", "/root", "/boot"];
const SHORT_ID_LENGTH: usize = 6;
const PROGRESS_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;
const PROGRESS_THRESHOLD_ITEMS: usize = 5;
const MAX_RECURSION_DEPTH: usize = 1000;
const MAX_FILE_SPACE_RATIO: f64 = 0.8;
const MMAP_CHUNK_SIZE: usize = 4 * 1024 * 1024;

fn get_srm_base() -> PathBuf {
    let exe_path = std::env::current_exe().expect("Failed to get srm executable path");
    let exe_dir = exe_path.parent().expect("Failed to get srm parent directory");
    exe_dir.join(".srm")
}

#[derive(Serialize, Deserialize, Debug)]
struct LogEntry {
    timestamp: String,
    level: String,
    message: String,
    details: Option<serde_json::Value>,
}

fn secure_create_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o700);
    fs::set_permissions(path, perms)
}

fn secure_create_file(path: &Path) -> io::Result<std::fs::File> {
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms)?;
    Ok(file)
}

fn log_event(level: &str, message: &str, details: Option<serde_json::Value>) {
    let base = get_srm_base();
    let log_path = base.join("srm.log");

    if let Err(e) = secure_create_dir(&base) {
        eprintln!("⚠️  Failed to create log directory: {}", e);
        return;
    }

    let entry = LogEntry {
        timestamp: Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
        level: level.into(),
        message: message.into(),
        details,
    };

    if let Ok(json) = serde_json::to_string(&entry) {
        if let Ok(mut file) = secure_create_file(&log_path) {
            let _ = writeln!(file, "{}", json);
        }
    }
}

fn rotate_logs(base: &Path) {
    let log_path = base.join("srm.log");
    if !log_path.exists() {
        return;
    }

    let cutoff = Local::now() - Duration::days(MAX_LOG_AGE_DAYS);
    let temp_log = base.join("srm.log.tmp");

    // 打开源日志和临时日志文件
    let src_file = match fs::File::open(&log_path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let mut dst_file = match secure_create_file(&temp_log) {
        Ok(f) => f,
        Err(_) => return,
    };

    let mut reader = BufReader::new(src_file);
    let mut line = String::new();

    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            line.clear();
            continue;
        }

        if let Ok(entry) = serde_json::from_str::<LogEntry>(trimmed) {
            if let Ok(ts) = NaiveDateTime::parse_from_str(&entry.timestamp, "%Y-%m-%d %H:%M:%S%.3f") {
                match Local.from_local_datetime(&ts) {
                    LocalResult::Single(dt) if dt >= cutoff => {
                        let _ = writeln!(dst_file, "{}", trimmed);
                    }
                    _ => (),
                }
            }
        }
        line.clear();
    }

    drop((reader, dst_file));
    let _ = fs::rename(&temp_log, &log_path);
}

fn same_filesystem(path1: &Path, path2: &Path) -> bool {
    match (fs::metadata(path1), fs::metadata(path2)) {
        (Ok(m1), Ok(m2)) => m1.dev() == m2.dev(),
        _ => false,
    }
}

fn canonicalize_safe(path: &Path) -> io::Result<PathBuf> {
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    abs_path.canonicalize()
}

#[cfg(target_os = "linux")]
fn try_reflink_copy(src: &Path, dst: &Path) -> io::Result<bool> {
    use std::os::unix::io::AsRawFd;
    let src_file = fs::File::open(src)?;
    let dst_file = fs::File::create(dst)?;

    const FICLONE: u64 = 0x40049409;
    let ret = unsafe {
        libc::ioctl(dst_file.as_raw_fd(), FICLONE as _, src_file.as_raw_fd())
    };

    Ok(ret == 0)
}

#[cfg(not(target_os = "linux"))]
fn try_reflink_copy(_src: &Path, _dst: &Path) -> io::Result<bool> {
    Ok(false)
}

fn mmap_copy(src: &Path, dst: &Path, size: u64, show_progress: bool) -> io::Result<u64> {
    let src_file = fs::File::open(src)?;
    let mut dst_file = fs::File::create(dst)?;
    dst_file.set_len(size)?;

    let mmap = unsafe { MmapOptions::new().map(&src_file)? };
    let mut offset = 0usize;

    let pb = if show_progress && size > PROGRESS_THRESHOLD_BYTES {
        let pb = ProgressBar::new(size);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("█▓▒░ "));
        Some(pb)
    } else {
        None
    };

    let result = loop {
        if INTERRUPTED.load(Ordering::Relaxed) {
            if let Some(p) = &pb {
                p.abandon_with_message("Interrupted");
            }
            break Err(io::Error::new(io::ErrorKind::Interrupted, "Operation interrupted"));
        }

        if offset >= mmap.len() {
            break Ok(size);
        }

        let end = cmp::min(offset + MMAP_CHUNK_SIZE, mmap.len());
        dst_file.write_all(&mmap[offset..end])?;
        offset = end;

        if let Some(p) = &pb {
            p.set_position(offset as u64);
        }
    };

    drop(mmap);
    if let Some(p) = pb {
        p.finish_with_message("Done");
    }

    result
}

fn fast_file_copy(src: &Path, dst: &Path, show_progress: bool) -> io::Result<u64> {
    let src_meta = fs::metadata(src)?;
    let size = src_meta.len();

    #[cfg(target_os = "linux")]
    {
        if try_reflink_copy(src, dst)? {
            return Ok(size);
        }
    }

    if same_filesystem(src, dst) {
        if fs::hard_link(src, dst).is_ok() {
            return Ok(size);
        }
    }

    if size > 10 * 1024 * 1024 {
        return mmap_copy(src, dst, size, show_progress);
    }

    let mut reader = fs::File::open(src)?;
    let mut writer = fs::File::create(dst)?;

    if show_progress && size > PROGRESS_THRESHOLD_BYTES {
        let pb = ProgressBar::new(size);
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("█▓▒░ "));

        let mut buffer = [0u8; 256 * 1024];
        let mut total = 0u64;

        loop {
            if INTERRUPTED.load(Ordering::Relaxed) {
                pb.abandon_with_message("Interrupted");
                return Err(io::Error::new(io::ErrorKind::Interrupted, "Operation interrupted"));
            }

            let n = reader.read(&mut buffer)?;
            if n == 0 { break; }
            writer.write_all(&buffer[..n])?;
            total += n as u64;
            pb.set_position(total);
        }
        pb.finish_with_message("Done");
        Ok(total)
    } else {
        io::copy(&mut reader, &mut writer)
    }
}

fn safe_move_with_progress(src: &Path, dst: &Path, show_progress: bool) -> io::Result<u64> {
    let src_meta = fs::symlink_metadata(src)?;
    let src_size = src_meta.len();

    if fs::rename(src, dst).is_ok() {
        return Ok(src_size);
    }

    if src_meta.file_type().is_symlink() {
        let target = fs::read_link(src)?;
        std::os::unix::fs::symlink(target, dst)?;
        fs::remove_file(src)?;
        return Ok(0);
    }

    if src_meta.is_dir() {
        return move_directory_with_progress(src, dst, show_progress);
    }

    let size = fast_file_copy(src, dst, show_progress)?;
    fs::remove_file(src)?;
    Ok(size)
}

fn is_dir_empty(path: &Path) -> io::Result<bool> {
    let mut entries = fs::read_dir(path)?;
    Ok(entries.next().is_none())
}

fn move_directory_with_progress(src: &Path, dst: &Path, show_progress: bool) -> io::Result<u64> {
    fs::create_dir_all(dst)?;

    let mut stack: Vec<(PathBuf, PathBuf, usize)> = vec![(src.to_path_buf(), dst.to_path_buf(), 0)];
    let mut total_size = 0u64;
    let mut processed_items = 0usize;
    let mut pb: Option<ProgressBar> = None;

    if show_progress {
        let (bytes, items) = calculate_dir_stats(src)?;
        if bytes > PROGRESS_THRESHOLD_BYTES || items > PROGRESS_THRESHOLD_ITEMS {
            let mp = MultiProgress::new();
            let progress_bar = mp.add(ProgressBar::new(items as u64));
            progress_bar.set_style(ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] {pos}/{len} items [{wide_bar:.cyan/blue}] {percent}% ({eta})")
                .unwrap()
                .progress_chars("█▓▒░ "));
            progress_bar.set_position(0);
            progress_bar.set_message(format!("Moving: {}", src.file_name().unwrap_or_default().to_string_lossy()));
            pb = Some(progress_bar);
        }
    }

    while let Some((current_src, current_dst, depth)) = stack.pop() {
        if depth > MAX_RECURSION_DEPTH {
            return Err(io::Error::new(io::ErrorKind::Other, "Directory depth exceeds safety limit (1000)"));
        }

        if INTERRUPTED.load(Ordering::Relaxed) {
            if let Some(p) = &pb {
                p.abandon_with_message("Interrupted");
            }
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Operation interrupted"));
        }

        if let Ok(entries) = fs::read_dir(&current_src) {
            for entry in entries.flatten() {
                let src_path = entry.path();
                let dst_path = current_dst.join(entry.file_name());

                if let Ok(meta) = fs::symlink_metadata(&src_path) {
                    if meta.is_dir() {
                        fs::create_dir_all(&dst_path)?;
                        stack.push((src_path, dst_path, depth + 1));
                    } else {
                        let size = safe_move_with_progress(&src_path, &dst_path, false)?;
                        total_size += size;
                        processed_items += 1;

                        if let Some(p) = &pb {
                            p.inc(1);
                        }
                    }
                }
            }
        }

        if is_dir_empty(&current_src)? {
            fs::remove_dir(&current_src)?;
        } else {
            return Err(io::Error::new(io::ErrorKind::Other, format!("Failed to delete non-empty directory: {}", current_src.display())));
        }
    }

    if let Some(p) = pb {
        p.finish_with_message(format!("Done ({} items, {})", processed_items, ByteSize(total_size)));
    }

    Ok(total_size)
}

fn calculate_dir_stats(path: &Path) -> io::Result<(u64, usize)> {
    let mut stack = vec![path.to_path_buf()];
    let mut total_size = 0u64;
    let mut total_items = 0usize;

    while let Some(current) = stack.pop() {
        if INTERRUPTED.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Operation interrupted"));
        }
        if let Ok(entries) = fs::read_dir(&current) {
            for entry in entries.flatten() {
                total_items += 1;
                if let Ok(meta) = fs::symlink_metadata(entry.path()) {
                    if meta.is_dir() {
                        stack.push(entry.path());
                    } else {
                        total_size += meta.len();
                    }
                }
            }
        }
    }

    Ok((total_size, total_items))
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct FileMeta {
    original_path: String,
    trash_path: String,
    delete_time: String,
    expire_days: i64,
    file_type: FileType,
    permissions: Option<u32>,
    uid: Option<u32>,
    gid: Option<u32>,
    short_id: String,
    size_bytes: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
enum FileType {
    File,
    Dir,
    Symlink,
}

impl std::fmt::Display for FileType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            FileType::File => write!(f, "file"),
            FileType::Dir => write!(f, "dir"),
            FileType::Symlink => write!(f, "symlink"),
        }
    }
}

fn atomic_save_meta(name: &str, meta: &FileMeta, meta_dir: &Path) -> io::Result<()> {
    let tmp_path = meta_dir.join(format!("{}.meta.tmp", name));
    let final_path = meta_dir.join(format!("{}.meta", name));

    let mut tmp_file = fs::File::create(&tmp_path)?;
    write!(tmp_file, "{}", serde_json::to_string_pretty(meta)?)?;
    tmp_file.sync_all()?;
    drop(tmp_file);

    fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

fn remove_meta(name: &str, meta_dir: &Path) {
    let _ = fs::remove_file(meta_dir.join(format!("{}.meta", name)));
}

fn list_all_meta(meta_dir: &Path, trash_dir: &Path) -> HashMap<String, FileMeta> {
    let mut map = HashMap::new();
    if let Ok(entries) = fs::read_dir(meta_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()).and_then(|s| s.strip_suffix(".meta")) else {
                continue;
            };
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(mut meta) = serde_json::from_str::<FileMeta>(&content) {
                    let trash_file = PathBuf::from(&meta.trash_path);
                    if !trash_file.exists() || !same_filesystem(&trash_file, trash_dir) {
                        eprintln!("⚠️  清理无效元数据：{}", name);
                        remove_meta(name, meta_dir);
                        continue;
                    }
                    if meta.short_id.is_empty() {
                        meta.short_id = name.to_string();
                    }
                    map.insert(name.to_string(), meta);
                } else {
                    eprintln!("⚠️  清理损坏元数据：{}", name);
                    remove_meta(name, meta_dir);
                }
            }
        }
    }
    map
}

fn generate_short_id(trash_id: &str, file_type: FileType, existing: &HashSet<String>) -> String {
    let hash = compute(trash_id.as_bytes());
    let hex = format!("{:x}", hash);
    let type_prefix = match file_type {
        FileType::File => "f",
        FileType::Dir => "d",
        FileType::Symlink => "l",
    };

    let mut candidate = format!("{}{}", type_prefix, &hex[..SHORT_ID_LENGTH]);
    let mut counter = 1;

    while existing.contains(&candidate) {
        candidate = format!("{}_{}", candidate, counter);
        counter += 1;
    }

    candidate
}

fn setup_interrupt_handler() {
    ctrlc::set_handler(|| {
        INTERRUPTED.store(true, Ordering::Relaxed);
    }).ok();
}

fn check_disk_space(trash_dir: &Path, required_bytes: u64, is_single_file: bool) -> io::Result<()> {
    let available = fs2::available_space(trash_dir)?;
    if available == 0 {
        return Err(io::Error::new(io::ErrorKind::Other, "No disk space available"));
    }

    let base_required = if required_bytes < 100 * 1024 * 1024 {
        required_bytes
    } else {
        required_bytes
            .checked_mul(120)
            .and_then(|v| v.checked_div(100))
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "File size too large, calculation overflow"))?
    };

    if is_single_file {
        let max_allowed = (available as f64 * MAX_FILE_SPACE_RATIO) as u64;
        if required_bytes > max_allowed {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("Single file too large: {} exceeds {}% of available space ({})",
                        ByteSize(required_bytes),
                        (MAX_FILE_SPACE_RATIO * 100.0) as u8,
                        ByteSize(available))
            ));
        }
    }

    if base_required > available {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("Insufficient disk space. Need {} but only {} available",
                    ByteSize(base_required),
                    ByteSize(available))
        ));
    }

    Ok(())
}

fn handle_delete_batch(paths: Vec<PathBuf>, expire_days: i64, force: bool, trash_dir: &Path, meta_dir: &Path) {
    setup_interrupt_handler();

    if !force {
        for path in &paths {
            let path_str = path.to_string_lossy();
            if path_str.contains("..") || path_str.starts_with('-') {
                eprintln!("❌  Path traversal detected ('{}'). Use -f to override safety checks.", path_str);
                std::process::exit(1);
            }
        }
    }

    let mut items_to_delete = Vec::new();
    let mut skipped = Vec::new();
    let mut total_required_space = 0u64;

    for path in &paths {
        let abs_path = if path.is_absolute() {
            path.clone()
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(path),
                Err(e) => {
                    skipped.push((path.display().to_string(), format!("Failed to get current directory: {}", e)));
                    continue;
                }
            }
        };

        if let Ok(meta) = fs::symlink_metadata(&abs_path) {
            if meta.file_type().is_symlink() {
                if let Ok(target) = fs::read_link(&abs_path) {
                    let target_str = target.to_string_lossy();
                    if !force && PROTECTED_PATHS.iter().any(|&p| target_str.starts_with(p)) {
                        skipped.push((abs_path.display().to_string(), format!("Symlink targets protected path: {}", target_str)));
                        continue;
                    }
                }
            }
        }

        if !abs_path.exists() {
            skipped.push((abs_path.display().to_string(), "Not found".into()));
            continue;
        }

        let meta = match fs::symlink_metadata(&abs_path) {
            Ok(m) => m,
            Err(e) => {
                skipped.push((abs_path.display().to_string(), format!("{}", e)));
                continue;
            }
        };

        let is_symlink = meta.file_type().is_symlink();
        let file_type = if is_symlink { FileType::Symlink } else if meta.is_dir() { FileType::Dir } else { FileType::File };

        if !force && !is_symlink {
            let canon_path = match canonicalize_safe(&abs_path) {
                Ok(p) => p,
                Err(e) => {
                    skipped.push((abs_path.display().to_string(), format!("Invalid path: {}", e)));
                    continue;
                }
            };
            let path_str = canon_path.to_string_lossy();
            if PROTECTED_PATHS.iter().any(|&p| {
                path_str.starts_with(p) &&
                (path_str.len() == p.len() || path_str.as_bytes()[p.len()] == b'/')
            }) {
                skipped.push((abs_path.display().to_string(), "Protected system path (use -f to override)".into()));
                continue;
            }
        }

        let (size_bytes, _) = if file_type == FileType::Dir {
            match calculate_dir_stats(&abs_path) {
                Ok(s) => s,
                Err(e) => {
                    skipped.push((abs_path.display().to_string(), format!("Failed to get dir stats: {}", e)));
                    continue;
                }
            }
        } else {
            (meta.len(), 0)
        };

        if file_type != FileType::Dir && size_bytes > 0 {
            if let Err(e) = check_disk_space(trash_dir, size_bytes, true) {
                skipped.push((abs_path.display().to_string(), format!("{}", e)));
                continue;
            }
        }

        total_required_space += size_bytes;
        items_to_delete.push((abs_path, meta, file_type, size_bytes));
    }

    if let Err(e) = check_disk_space(trash_dir, total_required_space, false) {
        eprintln!("❌  {}", e);
        std::process::exit(1);
    }

    log_event("INFO", "Delete command started", Some(json!({
        "paths_count": paths.len(),
        "expire_days": expire_days,
        "force": force,
        "items_to_delete": items_to_delete.len(),
        "skipped": skipped.len(),
        "total_size_bytes": total_required_space
    })));

    if items_to_delete.is_empty() && skipped.is_empty() {
        println!("ℹ️  No items to delete");
        log_event("INFO", "Delete command completed", Some(json!({"success": 0, "skipped": 0, "failed": 0})));
        return;
    }

    println!("🗑️  Deleting {} item(s) ({} total, expire in {} days)...",
             items_to_delete.len(),
             ByteSize(total_required_space),
             expire_days);

    let existing_short_ids: HashSet<String> = list_all_meta(meta_dir, trash_dir)
        .values()
        .map(|m| m.short_id.clone())
        .collect();

    let mut moved: Vec<(String, String, String, String, u64)> = Vec::new();
    let mut failed = Vec::new();
    let total_items = items_to_delete.len();
    let show_batch_progress = total_items > PROGRESS_THRESHOLD_ITEMS || total_required_space > PROGRESS_THRESHOLD_BYTES;
    let mut mp_pb = None;
    if show_batch_progress {
        let mp = MultiProgress::new();
        let main_pb = mp.add(ProgressBar::new(total_items as u64));
        main_pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] {pos}/{len} items [{wide_bar:.cyan/blue}] {percent}%")
            .unwrap()
            .progress_chars("█▓▒░ "));
        main_pb.set_message("Deleting items");
        mp_pb = Some((mp, main_pb));
    }

    let start_time = Instant::now();
    let mut processed = 0usize;

    for (abs_path, meta, file_type, size_bytes) in items_to_delete {
        if INTERRUPTED.load(Ordering::Relaxed) {
            println!("\n⚠️  Operation interrupted. Stopping...");
            log_event("WARN", "User interrupted operation", None);
            break;
        }

        let name = abs_path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown");
        let ts = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
        let trash_id = format!("{}_{}", name, ts);
        let trash_path = trash_dir.join(&trash_id);
        let short_id = generate_short_id(&trash_id, file_type, &existing_short_ids);
        let item_count = if file_type == FileType::Dir {
            fs::read_dir(&abs_path).map(|e| e.count()).unwrap_or(0)
        } else {
            0
        };

        if let Some((_, pb)) = &mp_pb {
            pb.set_position(processed as u64);
            pb.set_message(format!("{} → {}", truncate_path(&abs_path.display().to_string(), 30), short_id));
        }

        let show_progress = size_bytes > PROGRESS_THRESHOLD_BYTES || (file_type == FileType::Dir && item_count > 100);
        match safe_move_with_progress(&abs_path, &trash_path, show_progress) {
            Ok(_) => {
                let original_str = abs_path.to_string_lossy().into_owned();
                let trash_str = trash_path.to_string_lossy().into_owned();
                let file_meta = FileMeta {
                    original_path: original_str.clone(),
                    trash_path: trash_str.clone(),
                    delete_time: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                    expire_days,
                    file_type,
                    permissions: Some(meta.permissions().mode()),
                    uid: Some(meta.uid()),
                    gid: Some(meta.gid()),
                    short_id: short_id.clone(),
                    size_bytes,
                };

                if let Err(e) = atomic_save_meta(&trash_id, &file_meta, meta_dir) {
                    eprintln!("⚠️  Failed to save metadata for '{}': {}", abs_path.display(), e);
                    let _ = safe_move_with_progress(&trash_path, &abs_path, false);
                    failed.push((original_str, format!("Metadata save failed: {}", e)));
                    continue;
                }

                if !show_batch_progress {
                    let display_name = if file_type == FileType::Dir {
                        format!("{}/", name)
                    } else {
                        name.to_string()
                    };

                    let item_suffix = if item_count > 0 {
                        format!(" ({} item{})", item_count, if item_count > 1 { "s" } else { "" })
                    } else {
                        String::new()
                    };

                    println!("✅ {} → 🆔 {}{} [{}]", display_name, short_id, item_suffix, ByteSize(size_bytes));
                }

                log_event("INFO", "File deleted", Some(json!({
                    "action": "delete",
                    "short_id": short_id,
                    "trash_id": trash_id,
                    "original_path": original_str,
                    "backup_path": trash_str,
                    "file_type": format!("{}", file_type),
                    "size_bytes": size_bytes,
                    "permissions": format!("{:o}", meta.permissions().mode() & 0o777),
                    "expire_days": expire_days,
                    "forced": force,
                    "duration_ms": start_time.elapsed().as_millis()
                })));

                moved.push((trash_id, short_id, original_str, trash_str, size_bytes));
                processed += 1;
            }
            Err(e) => {
                failed.push((abs_path.display().to_string(), format!("{}", e)));
                eprintln!("❌ Failed '{}': {}", abs_path.display(), e);
            }
        }
    }

    let skipped_count = skipped.len();
    for (path, reason) in &skipped {
        println!("⚠️  Skip '{}': {}", path, reason);
        log_event("WARN", "Skipped deletion", Some(json!({
            "path": path,
            "reason": reason,
            "forced": force
        })));
    }

    let failed_count = failed.len();

    if INTERRUPTED.load(Ordering::Relaxed) {
        let rollback_count = moved.len();
        println!("\n🔄 Rolling back {} items...", rollback_count);

        for (trash_id, short_id, orig_str, trash_str, _) in moved.into_iter().rev() {
            let orig_path = PathBuf::from(&orig_str);
            let trash_path = PathBuf::from(&trash_str);
            if trash_path.exists() {
                println!("↩️  Rolling back: {}", orig_path.display());
                let _ = safe_move_with_progress(&trash_path, &orig_path, false);
                remove_meta(&trash_id, meta_dir);
                log_event("INFO", "Rollback performed", Some(json!({
                    "short_id": short_id,
                    "original_path": orig_str
                })));
            }
        }
        println!("✅ Rollback finished.");
        log_event("WARN", "Operation interrupted and rolled back", Some(json!({"rolled_back_count": rollback_count})));
    } else {
        let success_count = moved.len();
        let total_size = moved.iter().map(|(_, _, _, _, s)| s).sum::<u64>();
        let duration = start_time.elapsed();
        let throughput = if duration.as_secs() > 0 {
            total_size / duration.as_secs()
        } else {
            total_size
        };

        if show_batch_progress {
            if let Some((_, pb)) = mp_pb {
                pb.finish_with_message(format!("Done ({} items, {} total, {:.1} MB/s)",
                    success_count,
                    ByteSize(total_size),
                    throughput as f64 / 1024.0 / 1024.0));
            }
        } else {
            println!("\n✅ Deletion completed ({} succeeded, {} skipped, {} failed)",
                     success_count, skipped_count, failed_count);
            println!("   Total: {} in {:.1}s ({:.1} MB/s)",
                     ByteSize(total_size),
                     duration.as_secs_f32(),
                     throughput as f64 / 1024.0 / 1024.0);
        }

        log_event("INFO", "Delete command completed", Some(json!({
            "success_count": success_count,
            "skipped_count": skipped_count,
            "failed_count": failed_count,
            "total_size_bytes": total_size,
            "duration_ms": duration.as_millis(),
            "throughput_bytes_per_sec": throughput
        })));
    }
}

fn confirm_overwrite(path: &Path) -> bool {
    print!("⚠️  Target '{}' exists. Overwrite? [y/N]: ", path.display());
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    input.trim().eq_ignore_ascii_case("y")
}

fn handle_restore(names: Vec<String>, force: bool, target: Option<PathBuf>, meta_dir: &Path, trash_dir: &Path) {
    let all_meta = list_all_meta(meta_dir, trash_dir);
    let short_id_map: HashMap<String, String> = all_meta
        .iter()
        .map(|(trash_id, meta)| (meta.short_id.clone(), trash_id.clone()))
        .collect();

    let mut restored = 0;
    let mut failed = 0;

    for name in names {
        let Some(trash_id) = short_id_map.get(&name).cloned().or_else(|| {
            if all_meta.contains_key(&name) { Some(name.clone()) } else { None }
        }) else {
            eprintln!("❌  '{}' not found in trash (check with `srm ls`)", name);
            failed += 1;
            continue;
        };

        let meta_path = meta_dir.join(format!("{}.meta", trash_id));
        let content = match fs::read_to_string(&meta_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("❌  Failed to read metadata for '{}': {}", name, e);
                failed += 1;
                continue;
            }
        };

        let meta: FileMeta = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("❌  Failed to parse metadata for '{}': {}", name, e);
                failed += 1;
                continue;
            }
        };

        let trash_path = PathBuf::from(&meta.trash_path);
        if !trash_path.exists() {
            eprintln!("❌  Trash file missing for '{}'", name);
            remove_meta(&trash_id, meta_dir);
            failed += 1;
            continue;
        }

        let final_target = if let Some(t) = &target {
            let t_abs = if t.is_absolute() {
                t.clone()
            } else {
                std::env::current_dir().map(|c| c.join(t)).unwrap_or_else(|_| t.clone())
            };
            if t_abs.is_dir() {
                t_abs.join(Path::new(&meta.original_path).file_name().unwrap())
            } else {
                t_abs
            }
        } else {
            PathBuf::from(&meta.original_path)
        };

        if final_target.exists() {
            if !force && !confirm_overwrite(&final_target) {
                println!("✅  Skipped restoring '{}'", name);
                continue;
            }
            let _ = fs::remove_dir_all(&final_target);
        }

        if safe_move_with_progress(&trash_path, &final_target, false).is_err() {
            eprintln!("❌  Failed to restore '{}'", name);
            failed += 1;
            continue;
        }

        if let Some(mode) = meta.permissions {
            let _ = fs::set_permissions(&final_target, fs::Permissions::from_mode(mode));
        }

        remove_meta(&trash_id, meta_dir);
        restored += 1;

        log_event("INFO", "File restored", Some(json!({
            "action": "restore",
            "short_id": meta.short_id,
            "trash_id": trash_id,
            "original_path": meta.original_path,
            "restored_path": final_target.display().to_string(),
            "forced": force
        })));

        println!("✅ Restored: {} → {}", meta.short_id, final_target.display());
    }

    if restored > 0 || failed > 0 {
        println!("\n✅ Restore completed ({} succeeded, {} failed)", restored, failed);
    }
}

fn format_duration(duration: chrono::Duration) -> String {
    let days = duration.num_days();
    let hours = duration.num_hours() % 24;

    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, duration.num_minutes() % 60)
    } else {
        format!("{}m", duration.num_minutes())
    }
}

fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        path.to_string()
    } else {
        let half = (max_len - 3) / 2;
        let start = &path[..half.min(path.len())];
        let end = &path[(path.len() - half).max(half)..];
        format!("{}...{}", start, end)
    }
}

fn handle_list(meta_dir: &Path, trash_dir: &Path, expired: bool, verbose: bool) {
    let now = Local::now();
    let all_meta = list_all_meta(meta_dir, trash_dir);
    if all_meta.is_empty() {
        println!("📭 Trash is empty");
        return;
    }

    let mut active = Vec::new();
    let mut expired_items = Vec::new();

    for (trash_id, meta) in all_meta {
        let Some(delete_naive) = NaiveDateTime::parse_from_str(&meta.delete_time, "%Y-%m-%d %H:%M:%S").ok() else {
            eprintln!("⚠️  跳过无效元数据：{}", meta.short_id);
            remove_meta(&trash_id, meta_dir);
            continue;
        };
        let delete_time = match Local.from_local_datetime(&delete_naive) {
            LocalResult::Single(dt) => dt,
            _ => {
                eprintln!("⚠️  跳过时间无效的元数据：{}", meta.short_id);
                remove_meta(&trash_id, meta_dir);
                continue;
            }
        };
        let expire_time = delete_time + Duration::days(meta.expire_days);
        let is_expired = now > expire_time;

        if expired && !is_expired { continue; }

        let duration = if is_expired {
            now.signed_duration_since(expire_time)
        } else {
            expire_time.signed_duration_since(now)
        };

        let item = (trash_id, meta, is_expired, duration);
        if is_expired {
            expired_items.push(item);
        } else {
            active.push(item);
        }
    }

    if !active.is_empty() && !expired {
        println!("📦 Active items ({}):", active.len());
        if !verbose {
            println!("{:<12} {:<45} {:<12} {}", "🆔 SHORT", "ORIGINAL PATH", "EXPIRES IN", "SIZE");
            println!("{:-<12} {:-<45} {:-<12} {:-<15}", "", "", "", "");
        }

        for (_, meta, _, duration) in &active {
            if verbose {
                println!("\n🆔 {} ({})", meta.short_id, meta.trash_path);
                println!("   Original: {}", meta.original_path);
                let Some(delete_naive) = NaiveDateTime::parse_from_str(&meta.delete_time, "%Y-%m-%d %H:%M:%S").ok() else {
                    continue;
                };
                let delete_chrono = match Local.from_local_datetime(&delete_naive) {
                    LocalResult::Single(dt) => dt,
                    _ => continue,
                };
                println!("   Deleted:  {} ({} ago)", meta.delete_time, format_duration(now.signed_duration_since(delete_chrono)));
                println!("   Expires:  in {} (on {})", format_duration(*duration),
                    (Local::now() + *duration).format("%Y-%m-%d %H:%M:%S"));
                println!("   Type:     {} | Size: {} | Perm: {:o}",
                    meta.file_type,
                    ByteSize(meta.size_bytes),
                    meta.permissions.unwrap_or(0) & 0o777);
            } else {
                let size_display = if meta.file_type == FileType::Dir {
                    format!("{} (dir)", ByteSize(meta.size_bytes))
                } else {
                    ByteSize(meta.size_bytes).to_string()
                };
                println!("{:<12} {:<45} {:<12} {}",
                    meta.short_id,
                    truncate_path(&meta.original_path, 43),
                    format_duration(*duration),
                    size_display);
            }
        }
    }

    if !expired_items.is_empty() {
        if !verbose && !active.is_empty() && !expired { println!(); }
        println!("🗑️  {}", if expired { format!("Expired items ({})", expired_items.len()) } else { format!("Expired items ({})", expired_items.len()) });
        if !verbose {
            println!("{:<12} {:<45} {:<12} {}", "🆔 SHORT", "ORIGINAL PATH", "EXPIRED", "SIZE");
            println!("{:-<12} {:-<45} {:-<12} {:-<15}", "", "", "", "");
        }

        for (_, meta, _, duration) in &expired_items {
            if verbose {
                println!("\n🆔 {} (EXPIRED)", meta.short_id);
                println!("   Original: {}", meta.original_path);
                let Some(delete_naive) = NaiveDateTime::parse_from_str(&meta.delete_time, "%Y-%m-%d %H:%M:%S").ok() else {
                    continue;
                };
                let delete_chrono = match Local.from_local_datetime(&delete_naive) {
                    LocalResult::Single(dt) => dt,
                    _ => continue,
                };
                println!("   Deleted:  {} ({} ago)", meta.delete_time, format_duration(now.signed_duration_since(delete_chrono)));
                println!("   Expired:  {} ago (on {})", format_duration(*duration),
                    (Local::now() - *duration).format("%Y-%m-%d %H:%M:%S"));
                println!("   Type:     {} | Size: {} | Perm: {:o}",
                    meta.file_type,
                    ByteSize(meta.size_bytes),
                    meta.permissions.unwrap_or(0) & 0o777);
            } else {
                let size_display = if meta.file_type == FileType::Dir {
                    format!("{} (dir)", ByteSize(meta.size_bytes))
                } else {
                    ByteSize(meta.size_bytes).to_string()
                };
                println!("{:<12} {:<45} {:<12} {}",
                    meta.short_id,
                    truncate_path(&meta.original_path, 43),
                    format!("{} ago", format_duration(*duration)),
                    size_display);
            }
        }
    }

    if !verbose && !expired {
        println!("\n💡 Use `srm ls -v` for detailed view, `srm ls --expired` for expired items");
    }
}

fn clean_trash(meta_dir: &Path, trash_dir: &Path, all: bool) {
    let now = Local::now();
    let mut cleaned = 0;
    let mut total_size = 0u64;

    for (trash_id, meta) in list_all_meta(meta_dir, trash_dir) {
        let should_clean = if all {
            true
        } else {
            let Some(delete_naive) = NaiveDateTime::parse_from_str(&meta.delete_time, "%Y-%m-%d %H:%M:%S").ok() else {
                eprintln!("⚠️  清理无效元数据：{}", meta.short_id);
                remove_meta(&trash_id, meta_dir);
                continue;
            };
            let delete_time = match Local.from_local_datetime(&delete_naive) {
                LocalResult::Single(dt) => dt,
                _ => {
                    remove_meta(&trash_id, meta_dir);
                    continue;
                }
            };
            now > (delete_time + Duration::days(meta.expire_days))
        };

        if should_clean {
            let trash_path = PathBuf::from(&meta.trash_path);
            let file_size = meta.size_bytes;
            total_size += file_size;
            let _ = fs::remove_dir_all(&trash_path);
            remove_meta(&trash_id, meta_dir);
            cleaned += 1;

            log_event("INFO", "Item cleaned from trash", Some(json!({
                "action": "clean",
                "short_id": meta.short_id,
                "trash_id": trash_id,
                "original_path": meta.original_path,
                "size_bytes": file_size,
                "cleaned_all": all
            })));

            println!("🗑️  Cleaned: {} ({})", meta.short_id, truncate_path(&meta.original_path, 40));
        }
    }

    if cleaned > 0 {
        println!("\n✅ Clean completed! {} item(s) removed ({} total)", cleaned, ByteSize(total_size));
    } else {
        println!("📭 Nothing to clean");
    }
}

fn handle_empty(yes: bool, trash_dir: &Path, meta_dir: &Path) {
    if !yes {
        print!("⚠️  Empty trash permanently? This cannot be undone! [y/N]: ");
        io::stdout().flush().ok();
        let mut input = String::new();
        io::stdin().read_line(&mut input).ok();
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("✅ Canceled");
            return;
        }
    }

    let all_meta = list_all_meta(meta_dir, trash_dir);
    let item_count = all_meta.len();
    let total_size = all_meta.values().map(|m| m.size_bytes).sum::<u64>();

    log_event("WARN", "Trash emptied permanently", Some(json!({
        "action": "empty",
        "item_count": item_count,
        "total_size_bytes": total_size
    })));

    let _ = fs::remove_dir_all(trash_dir);
    let _ = fs::remove_dir_all(meta_dir);
    secure_create_dir(trash_dir).ok();
    secure_create_dir(meta_dir).ok();

    println!("✅ Trash emptied! {} item(s) permanently deleted ({} total)", item_count, ByteSize(total_size));
}

#[derive(Parser, Debug)]
#[command(
    name = "srm",
    author = "Meitao Lin <mtl>",
    version = "1.2.1",
    about = "Safe rm alternative with audit trail and TB-scale performance",
    long_about = r#"srm prevents accidental deletion with enterprise-grade safety and performance.

🚀 Performance Optimizations:
  • Same-filesystem: instant rename (no copy)
  • Cross-filesystem: reflink (CoW) on Btrfs/XFS/ZFS (Linux)
  • Large files: memory-mapped I/O with progress tracking
  • Directories: iterative traversal (no stack overflow)

💡 Typical Workflow:
  $ srm del report.pdf             # Gets short ID like f_a3b4c5
  $ srm del /data/large_dataset/   # Shows real-time progress bar
  $ srm ls                         # List with sizes and expiry
  $ srm res f_a3b4c5               # Restore using short ID
  $ srm cln                        # Clean expired items

🔒 All operations are securely logged to .srm/srm.log (30-day retention) in srm's directory.
"#
)]
struct Cli {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(alias = "del", about = "Move files/dirs to trash with progress tracking")]
    Delete {
        #[arg(required = true, help = "Paths to delete")]
        paths: Vec<PathBuf>,
        #[arg(short = 'd', long, default_value_t = DEFAULT_EXPIRE_DAYS, help = "Expiration days before auto-cleanup")]
        expire_days: i64,
        #[arg(short = 'f', long, help = "Force delete protected paths and disable safety checks")]
        force: bool,
    },
    #[command(alias = "res", about = "Restore files from trash using short ID")]
    Restore {
        #[arg(required = true, help = "Short IDs or full trash IDs (from `srm ls`)")]
        names: Vec<String>,
        #[arg(short = 'f', long, help = "Force overwrite existing files")]
        force: bool,
        #[arg(short = 't', long = "target", help = "Custom restore path")]
        target: Option<PathBuf>,
    },
    #[command(alias = "ls", about = "List trash contents with sizes")]
    List {
        #[arg(long, help = "Only show expired items")]
        expired: bool,
        #[arg(short = 'v', long, help = "Verbose mode with full metadata")]
        verbose: bool,
    },
    #[command(alias = "cln", about = "Clean expired items")]
    Clean {
        #[arg(short = 'a', long, help = "Clean all items (not just expired)")]
        all: bool,
    },
    #[command(alias = "empty", about = "Permanently empty entire trash")]
    Empty {
        #[arg(short = 'y', long, help = "Skip confirmation prompt")]
        yes: bool,
    },
}

fn main() {
    let base = get_srm_base();
    let trash_dir = base.join("trash");
    let meta_dir = base.join("meta");

    secure_create_dir(&trash_dir).expect("Failed to create trash dir");
    secure_create_dir(&meta_dir).expect("Failed to create meta dir");

    rotate_logs(&base);

    match Cli::parse().cmd {
        Commands::Delete { paths, expire_days, force } => {
            handle_delete_batch(paths, expire_days, force, &trash_dir, &meta_dir);
        }
        Commands::Restore { names, force, target } => {
            handle_restore(names, force, target, &meta_dir, &trash_dir);
        }
        Commands::List { expired, verbose } => {
            handle_list(&meta_dir, &trash_dir, expired, verbose);
        }
        Commands::Clean { all } => {
            clean_trash(&meta_dir, &trash_dir, all);
        }
        Commands::Empty { yes } => {
            handle_empty(yes, &trash_dir, &meta_dir);
        }
    }
}
