use serde::Serialize;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[derive(Clone, Serialize)]
struct LogPayload {
    line: String,
}

#[derive(Clone, Serialize)]
struct StatusPayload {
    success: bool,
    code: i32,
}

#[derive(Clone, Serialize)]
struct UpdateStatusPayload {
    success: bool,
    message: String,
}

#[derive(Clone, Serialize)]
struct DownloadProgressPayload {
    percent: f64,
    downloaded_mb: f64,
    total_mb: f64,
    speed_mbps: f64,
    done: bool,
}

static RUNNING: AtomicBool = AtomicBool::new(false);
static UPDATE_CANCEL: AtomicBool = AtomicBool::new(false);

/// Get the directory where this exe resides
fn get_app_dir() -> Option<std::path::PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// Find syncthing.exe in the same directory as this executable
#[tauri::command]
fn find_syncthing() -> Option<String> {
    if let Some(dir) = get_app_dir() {
        let candidate = dir.join("syncthing.exe");
        if candidate.exists() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    let candidate = std::path::Path::new("syncthing.exe");
    if candidate.exists() {
        return Some(std::fs::canonicalize(candidate).unwrap_or_default().to_string_lossy().to_string());
    }
    None
}

/// Get syncthing version string (only version number, e.g. "v1.27.3")
#[tauri::command]
fn get_syncthing_version() -> Result<String, String> {
    let path = find_syncthing().ok_or("syncthing.exe 未找到")?;
    let mut cmd = Command::new(&path);
    cmd.arg("--version");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let output = cmd.output().map_err(|e| format!("执行失败: {}", e))?;
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        Err("无法获取版本信息".to_string())
    } else {
        Ok(parse_version_number(&raw))
    }
}

/// Extract version number from full version string
/// e.g. "syncthing v1.27.3 Windows (64-bit Intel/AMD64) ..." -> "v1.27.3"
fn parse_version_number(full: &str) -> String {
    for token in full.split_whitespace() {
        let t = token.trim_start_matches('v');
        if !t.is_empty() && t.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            return if token.starts_with('v') { token.to_string() } else { format!("v{}", token) };
        }
    }
    full.trim().to_string()
}

/// Compare two version tuples, returns true if a >= b
fn version_gte(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split('.')
            .take(3)
            .map(|p| p.split('-').next().unwrap_or("0").parse::<u64>().unwrap_or(0))
            .collect()
    };
    let va = parse(a);
    let vb = parse(b);
    for i in 0..3 {
        let x = va.get(i).copied().unwrap_or(0);
        let y = vb.get(i).copied().unwrap_or(0);
        if x != y { return x > y; }
    }
    true // equal
}

/// Start folder decryption
#[tauri::command]
fn decrypt(
    app: AppHandle,
    syncthing_path: String,
    folder_id: String,
    password: String,
    source: String,
    dest: String,
) -> Result<(), String> {
    if RUNNING.load(Ordering::SeqCst) {
        return Err("解密正在进行中，请等待完成".to_string());
    }
    RUNNING.store(true, Ordering::SeqCst);

    std::thread::spawn(move || {
        // Output to dest/decrypted subfolder
        let dest_final = std::path::Path::new(&dest).join("decrypted");
        let dest_final = dest_final.to_string_lossy().to_string();

        let mut args: Vec<String> = vec!["decrypt".into()];
        if !folder_id.is_empty() {
            args.push("--folder-id".into());
            args.push(folder_id.clone());
        }
        args.push("--password".into());
        args.push(password.clone());
        args.push("--to".into());
        args.push(dest_final.clone());
        args.push("--continue".into());
        args.push("--verbose".into());
        args.push(source.clone());

        let _ = app.emit("decrypt-log", LogPayload {
            line: format!("[信息] 解密结果将输出到: {}", dest_final),
        });

        let result = run_syncthing(&app, &syncthing_path, &args);
        emit_status(&app, result);
        RUNNING.store(false, Ordering::SeqCst);
    });
    Ok(())
}

/// Decrypt multiple files using temp directory workaround
#[tauri::command]
fn decrypt_files(
    app: AppHandle,
    syncthing_path: String,
    folder_id: String,
    password: String,
    files: Vec<String>,
    dest: String,
) -> Result<(), String> {
    if RUNNING.load(Ordering::SeqCst) {
        return Err("解密正在进行中，请等待完成".to_string());
    }
    if files.is_empty() {
        return Err("未选择文件".to_string());
    }
    RUNNING.store(true, Ordering::SeqCst);

    std::thread::spawn(move || {
        let result = run_decrypt_files(&app, &syncthing_path, &folder_id, &password, &files, &dest);
        emit_status(&app, result);
        RUNNING.store(false, Ordering::SeqCst);
    });
    Ok(())
}

fn run_decrypt_files(
    app: &AppHandle,
    syncthing_path: &str,
    folder_id: &str,
    password: &str,
    files: &[String],
    dest: &str,
) -> Result<i32, String> {
    // Create temp directory
    let temp_dir = std::env::temp_dir().join(format!("scdecrypt_{}", std::process::id()));
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir).ok();
    }
    std::fs::create_dir_all(&temp_dir).map_err(|e| format!("创建临时目录失败: {}", e))?;

    let _ = app.emit("decrypt-log", LogPayload {
        line: format!("[信息] 文件解密模式: {} 个文件", files.len()),
    });

    // Copy each file into temp dir (flat, just filenames)
    for file_str in files {
        let file = std::path::Path::new(file_str);
        let file_name = file.file_name().ok_or("无效文件路径")?;
        let dest_file = temp_dir.join(file_name);
        std::fs::copy(file, &dest_file).map_err(|e| format!("复制文件失败: {}", e))?;
    }

    // Run decrypt on temp dir with explicit folder-id (required for file mode)
    // Output to dest/decrypted subfolder
    let dest_final = std::path::Path::new(dest).join("decrypted");
    let dest_final_str = dest_final.to_string_lossy().to_string();
    let _ = app.emit("decrypt-log", LogPayload {
        line: format!("[信息] 解密结果将输出到: {}", dest_final_str),
    });

    let temp_str = temp_dir.to_string_lossy().to_string();
    let mut args: Vec<String> = vec!["decrypt".into()];
    args.push("--folder-id".into());
    args.push(folder_id.to_string());
    args.push("--password".into());
    args.push(password.to_string());
    args.push("--to".into());
    args.push(dest_final_str);
    args.push("--continue".into());
    args.push("--verbose".into());
    args.push(temp_str);

    let result = run_syncthing(app, syncthing_path, &args);

    // Cleanup temp dir
    std::fs::remove_dir_all(&temp_dir).ok();

    result
}

/// Update syncthing by downloading from github or mirror (cross-platform, no shell dependency)
#[tauri::command]
fn update_syncthing(app: AppHandle, use_mirror: bool, proxy_mode: String, proxy_addr: String) -> Result<(), String> {
    UPDATE_CANCEL.store(false, Ordering::SeqCst);
    std::thread::spawn(move || {
        let result = do_update_syncthing(&app, use_mirror, &proxy_mode, &proxy_addr);
        let message = match &result {
            Ok(msg) => msg.clone(),
            Err(e) => e.clone(),
        };
        let _ = app.emit("update-status", UpdateStatusPayload {
            success: result.is_ok(),
            message,
        });
    });
    Ok(())
}

/// Cancel an in-progress update download
#[tauri::command]
fn cancel_update() -> Result<(), String> {
    UPDATE_CANCEL.store(true, Ordering::SeqCst);
    Ok(())
}

fn do_update_syncthing(app: &AppHandle, use_mirror: bool, proxy_mode: &str, proxy_addr: &str) -> Result<String, String> {
    let app_dir = get_app_dir().ok_or("无法获取程序目录")?;
    let syncthing_exe = app_dir.join("syncthing.exe");

    // Get local version for comparison
    let local_version = if syncthing_exe.exists() {
        let mut cmd = Command::new(&syncthing_exe);
        cmd.arg("--version");
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        cmd.output().ok()
            .map(|o| parse_version_number(&String::from_utf8_lossy(&o.stdout).trim()))
    } else {
        None
    };

    if let Some(ref lv) = local_version {
        let _ = app.emit("update-log", LogPayload {
            line: format!("[信息] 本地版本: {}", lv),
        });
    } else {
        let _ = app.emit("update-log", LogPayload {
            line: "[信息] 本地未检测到 syncthing，将下载最新版本".to_string(),
        });
    }

    let _ = app.emit("update-log", LogPayload {
        line: format!("[信息] 正在查询最新版本... (源: {}, 代理: {})",
            if use_mirror { "镜像站" } else { "GitHub直连" },
            match proxy_mode { "none" => "无", "custom" => proxy_addr, _ => "系统代理" }
        ),
    });

    // Build HTTP client with proxy configuration (cross-platform)
    let mut client_builder = reqwest::blocking::Client::builder()
        .user_agent("scdecrypt-gui")
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(600));
    match proxy_mode {
        "none" => {
            client_builder = client_builder.no_proxy();
        }
        "custom" => {
            if !proxy_addr.is_empty() {
                let proxy = reqwest::Proxy::all(proxy_addr)
                    .map_err(|e| format!("代理地址无效: {}", e))?;
                client_builder = client_builder.proxy(proxy);
            }
        }
        _ => {
            // System proxy: read from OS settings (Windows registry), fallback to env vars
            let sys_proxy = get_system_proxy();
            if let Some(ref addr) = sys_proxy {
                let _ = app.emit("update-log", LogPayload {
                    line: format!("[信息] 检测到系统代理: {}", addr),
                });
                if let Ok(proxy) = reqwest::Proxy::all(addr) {
                    client_builder = client_builder.proxy(proxy);
                }
            } else {
                let _ = app.emit("update-log", LogPayload {
                    line: "[信息] 未检测到系统代理，尝试环境变量...".to_string(),
                });
                // reqwest auto-detects HTTP_PROXY/HTTPS_PROXY env vars by default
            }
        }
    }
    let client = client_builder.build().map_err(|e| format!("初始化网络组件失败: {}", e))?;

    // Check cancellation before network operations
    if UPDATE_CANCEL.load(Ordering::SeqCst) {
        return Err("已取消更新".to_string());
    }

    // Query latest version from Syncthing official upgrade server
    // (syncthing.net infrastructure - stable, no GitHub API dependency)
    let api_url = "https://upgrades.syncthing.net/meta.json";
    let resp = client.get(api_url).send()
        .map_err(|e| format!("无法获取最新版本号: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("无法获取最新版本号: HTTP {}", resp.status()));
    }
    let releases: Vec<serde_json::Value> = resp.json()
        .map_err(|e| format!("解析版本信息失败: {}", e))?;
    // Pick the latest stable (non-prerelease) release
    let latest = releases.iter()
        .find(|r| r["prerelease"].as_bool() == Some(false))
        .ok_or("无法获取最新版本号: 未找到稳定版本")?;
    let tag = latest["tag_name"].as_str().unwrap_or("").to_string();
    if tag.is_empty() {
        return Err("无法获取最新版本号: 返回结果为空".to_string());
    }

    let _ = app.emit("update-log", LogPayload {
        line: format!("[信息] 最新版本: {}", tag),
    });

    // Compare local vs latest version
    if let Some(ref lv) = local_version {
        if version_gte(lv, &tag) {
            let msg = format!("当前已是最新版本 ({})，无需更新", lv);
            let _ = app.emit("update-log", LogPayload {
                line: format!("[完成] {}", msg),
            });
            return Ok(msg);
        }
        let _ = app.emit("update-log", LogPayload {
            line: format!("[信息] 发现新版本: {} → {}", lv, tag),
        });
    }

    // Resolve download URL from the release assets (fallback to constructed URL)
    let version_num = tag.trim_start_matches('v');
    let asset_name = format!("syncthing-windows-amd64-v{}.zip", version_num);
    let base_url = latest["assets"].as_array()
        .and_then(|assets| assets.iter().find(|a| a["name"].as_str() == Some(asset_name.as_str())))
        .and_then(|a| a["url"].as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!(
            "https://github.com/syncthing/syncthing/releases/download/{}/syncthing-windows-amd64-v{}.zip",
            tag, version_num
        ));
    let download_url = if use_mirror {
        format!("https://gh-proxy.com/{}", base_url)
    } else {
        base_url
    };

    let _ = app.emit("update-log", LogPayload {
        line: format!("[信息] 正在下载: {}", download_url),
    });

    // Check cancellation before download
    if UPDATE_CANCEL.load(Ordering::SeqCst) {
        return Err("已取消更新".to_string());
    }

    // Download zip with progress reporting
    let resp = client.get(&download_url).send()
        .map_err(|e| format!("下载失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status()));
    }
    let total_size = resp.content_length().unwrap_or(0);
    let mut zip_bytes: Vec<u8> = Vec::with_capacity(total_size as usize);
    let mut chunk_buf = [0u8; 65536];
    let mut downloaded: u64 = 0;
    let start_time = Instant::now();
    let mut last_emit = Instant::now();
    let mut resp = resp;

    loop {
        // Check cancellation
        if UPDATE_CANCEL.load(Ordering::SeqCst) {
            let _ = app.emit("update-progress", DownloadProgressPayload {
                percent: 0.0, downloaded_mb: 0.0, total_mb: 0.0, speed_mbps: 0.0, done: true,
            });
            return Err("已取消下载".to_string());
        }
        let n = resp.read(&mut chunk_buf)
            .map_err(|e| format!("下载失败: 读取数据出错 {}", e))?;
        if n == 0 {
            break;
        }
        zip_bytes.extend_from_slice(&chunk_buf[..n]);
        downloaded += n as u64;

        // Throttle progress events to ~4 per second
        if last_emit.elapsed().as_millis() >= 250 {
            last_emit = Instant::now();
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed = if elapsed > 0.1 { downloaded as f64 / 1048576.0 / elapsed } else { 0.0 };
            let percent = if total_size > 0 { downloaded as f64 / total_size as f64 * 100.0 } else { 0.0 };
            let _ = app.emit("update-progress", DownloadProgressPayload {
                percent,
                downloaded_mb: downloaded as f64 / 1048576.0,
                total_mb: total_size as f64 / 1048576.0,
                speed_mbps: speed,
                done: false,
            });
        }
    }

    // Final progress event (reset frontend progress line)
    let _ = app.emit("update-progress", DownloadProgressPayload {
        percent: 100.0,
        downloaded_mb: downloaded as f64 / 1048576.0,
        total_mb: total_size as f64 / 1048576.0,
        speed_mbps: 0.0,
        done: true,
    });

    let _ = app.emit("update-log", LogPayload {
        line: format!("[信息] 下载完成 ({:.1} MB)，正在解压...", zip_bytes.len() as f64 / 1048576.0),
    });

    // Extract zip (native Rust, cross-platform)
    let extract_dir = app_dir.join("syncthing_extract");
    if extract_dir.exists() { std::fs::remove_dir_all(&extract_dir).ok(); }
    std::fs::create_dir_all(&extract_dir).map_err(|e| format!("创建解压目录失败: {}", e))?;

    let cursor = std::io::Cursor::new(&zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| format!("解压失败: 压缩包格式错误 ({})", e))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)
            .map_err(|e| format!("解压失败: {}", e))?;
        let out_path = match file.enclosed_name() {
            Some(p) => extract_dir.join(p),
            None => continue,
        };
        if file.is_dir() {
            std::fs::create_dir_all(&out_path).ok();
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            let mut outfile = std::fs::File::create(&out_path)
                .map_err(|e| format!("解压失败: 写入文件出错 {}", e))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("解压失败: 写入文件出错 {}", e))?;
            // Preserve executable permission on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_permissions() {
                    std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode)).ok();
                }
            }
        }
    }

    // Find syncthing binary in extracted dir
    let exe_name = if cfg!(windows) { "syncthing.exe" } else { "syncthing" };
    let extracted_exe = find_file_recursive(&extract_dir, exe_name)
        .ok_or("解压后未找到 syncthing 可执行文件")?;

    // Replace old binary
    if syncthing_exe.exists() {
        std::fs::remove_file(&syncthing_exe).map_err(|e| format!("删除旧版本失败: {}", e))?;
    }
    std::fs::copy(&extracted_exe, &syncthing_exe).map_err(|e| format!("复制新版本失败: {}", e))?;

    // Cleanup
    std::fs::remove_dir_all(&extract_dir).ok();

    let msg = format!("已更新到 {}", tag);
    let _ = app.emit("update-log", LogPayload {
        line: format!("[完成] {}", msg),
    });

    Ok(msg)
}

/// Get system proxy address from OS settings.
/// Windows: reads HKCU\...\Internet Settings\ProxyServer (set by Clash/v2rayN etc.)
/// Other platforms: falls back to HTTP_PROXY/HTTPS_PROXY env vars.
fn get_system_proxy() -> Option<String> {
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok(settings) = hkcu.open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings") {
            let enable: u32 = settings.get_value("ProxyEnable").unwrap_or(0);
            if enable != 0 {
                if let Ok(server) = settings.get_value::<String, _>("ProxyServer") {
                    if !server.is_empty() {
                        // Format: "host:port" or "http=host:port;https=host:port;..."
                        if server.contains('=') {
                            // Prefer https entry, then http
                            for part in server.split(';') {
                                if let Some(addr) = part.trim().strip_prefix("https=") {
                                    return Some(format!("http://{}", addr));
                                }
                            }
                            for part in server.split(';') {
                                if let Some(addr) = part.trim().strip_prefix("http=") {
                                    return Some(format!("http://{}", addr));
                                }
                            }
                        } else {
                            return Some(format!("http://{}", server));
                        }
                    }
                }
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        // On other platforms, check env vars (reqwest also does this automatically)
        std::env::var("HTTPS_PROXY")
            .or_else(|_| std::env::var("HTTP_PROXY"))
            .or_else(|_| std::env::var("https_proxy"))
            .or_else(|_| std::env::var("http_proxy"))
            .ok()
    }
}

fn find_file_recursive(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.file_name().map(|f| f == name).unwrap_or(false) {
                return Some(path);
            } else if path.is_dir() {
                if let Some(found) = find_file_recursive(&path, name) {
                    return Some(found);
                }
            }
        }
    }
    None
}

// ===== Helpers =====

fn run_syncthing(app: &AppHandle, syncthing_path: &str, args: &[String]) -> Result<i32, String> {
    // Build display command with password masked
    let mut display_args: Vec<String> = Vec::new();
    let mut mask_next = false;
    for a in args {
        if mask_next {
            display_args.push("***".to_string());
            mask_next = false;
        } else if a == "--password" {
            display_args.push(a.clone());
            mask_next = true;
        } else {
            display_args.push(a.clone());
        }
    }
    let _ = app.emit("decrypt-log", LogPayload {
        line: format!("[CMD] syncthing.exe {}", display_args.join(" ")),
    });

    let mut cmd = Command::new(syncthing_path);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    let mut child = cmd.spawn().map_err(|e| format!("无法启动 syncthing.exe: {}", e))?;

    let _ = app.emit("decrypt-log", LogPayload {
        line: "[信息] 解密进程已启动 (verbose 模式)...".to_string(),
    });

    // ===== Output analysis =====
    let mut file_count: usize = 0;
    let mut error_lines: Vec<String> = Vec::new();
    let mut warning_count: usize = 0;

    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines() {
            if let Ok(line) = line {
                if line.trim().is_empty() {
                    continue;
                }
                let lower = line.to_lowercase();

                // Classify output lines
                let is_error = lower.contains("[error]") || lower.contains("error:")
                    || lower.contains("fatal") || lower.contains("panic")
                    || lower.contains("failed:") || lower.contains("failure");
                let is_warning = lower.contains("[warn]") || lower.contains("warning");
                let is_file_op = lower.contains("decrypt") && (lower.contains("copied")
                    || lower.contains("copy") || lower.contains("file")
                    || lower.contains("writing") || lower.contains("→"));

                if is_error {
                    error_lines.push(line.clone());
                    let _ = app.emit("decrypt-log", LogPayload { line: format!("[stderr] {}", line) });
                } else if is_warning {
                    warning_count += 1;
                    let _ = app.emit("decrypt-log", LogPayload { line: format!("[警告] {}", line) });
                } else {
                    if is_file_op {
                        file_count += 1;
                    }
                    let _ = app.emit("decrypt-log", LogPayload { line });
                }
            }
        }
    }

    let mut stderr_lines: Vec<String> = Vec::new();
    if let Some(stderr) = child.stderr.take() {
        for line in BufReader::new(stderr).lines() {
            if let Ok(line) = line {
                if !line.trim().is_empty() {
                    stderr_lines.push(line.clone());
                    let _ = app.emit("decrypt-log", LogPayload { line: format!("[stderr] {}", line) });
                }
            }
        }
    }

    let status = child.wait().map_err(|e| format!("等待进程失败: {}", e))?;
    let code = status.code().unwrap_or(-1);

    // ===== Emit analysis summary =====
    let _ = app.emit("decrypt-log", LogPayload {
        line: "[信息] ────── 输出分析 ──────".to_string(),
    });
    if file_count > 0 {
        let _ = app.emit("decrypt-log", LogPayload {
            line: format!("[信息] 检测到 {} 个文件解密操作", file_count),
        });
    }
    if warning_count > 0 {
        let _ = app.emit("decrypt-log", LogPayload {
            line: format!("[信息] 警告数量: {}", warning_count),
        });
    }
    if !error_lines.is_empty() {
        let _ = app.emit("decrypt-log", LogPayload {
            line: format!("[信息] 输出中检测到 {} 条错误信息:", error_lines.len()),
        });
        for el in error_lines.iter().take(5) {
            let _ = app.emit("decrypt-log", LogPayload { line: format!("[stderr]   {}", el) });
        }
    }
    if !stderr_lines.is_empty() && code != 0 {
        let _ = app.emit("decrypt-log", LogPayload {
            line: format!("[信息] 进程 stderr 输出 {} 行 (已显示在上方日志中)", stderr_lines.len()),
        });
    }
    if code == 0 && error_lines.is_empty() && stderr_lines.is_empty() {
        let _ = app.emit("decrypt-log", LogPayload {
            line: "[信息] 分析结果: 未发现异常，进程正常退出".to_string(),
        });
    }

    // Exit code is the primary success indicator;
    // only override if exit=0 but explicit error output was captured
    if code == 0 && (!error_lines.is_empty() || !stderr_lines.is_empty()) {
        Ok(1)
    } else {
        Ok(code)
    }
}

fn emit_status(app: &AppHandle, result: Result<i32, String>) {
    let (success, code) = match result {
        Ok(code) => {
            if code == 0 {
                let _ = app.emit("decrypt-log", LogPayload { line: "[完成] 解密成功！".to_string() });
            } else {
                let _ = app.emit("decrypt-log", LogPayload { line: format!("[失败] 退出码: {}", code) });
            }
            (code == 0, code)
        }
        Err(e) => {
            let _ = app.emit("decrypt-log", LogPayload { line: format!("[错误] {}", e) });
            (false, -1)
        }
    };
    let _ = app.emit("decrypt-status", StatusPayload { success, code });
}

/// Open a URL in the default browser (no console window)
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", "start", "", &url]);
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        cmd.spawn().map_err(|e| format!("打开链接失败: {}", e))?;
    }
    Ok(())
}

/// Save text content to a file
#[tauri::command]
fn save_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| format!("写入文件失败: {}", e))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            decrypt,
            decrypt_files,
            find_syncthing,
            get_syncthing_version,
            update_syncthing,
            cancel_update,
            open_url,
            save_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
