//! display-rotate — Windows 显示器方向托盘程序 (无窗口常驻 exe)。
//!
//! 两种用法, 底层同一个操作 (GDI `ChangeDisplaySettingsEx` + CCD `SetDisplayConfig`):
//! - **常驻托盘**: 右键菜单四个方向单选项 (勾选反映**系统实际朝向**, 不是"上次请求的")
//!   + 重新初始化显示(拔插) + 退出; 双击图标 = 横屏 0° ↔ 竖屏 270° toggle。
//! - **一次性命令行**: `--set 90` / `--toggle` / `--cycle` / `--status`, 执行完即退出
//!   (给排障和脚本兜底用; Windows 上要跑脚本请用 `scripts/rotate-display.ps1`)。
//!
//! 配置优先序: 内置默认值 < exe 同目录(或当前目录)的 config.toml < 命令行参数。
//!
//! ```text
//! 启动 → 读回系统实际朝向 (写进与托盘共享的 AtomicU32) → 起托盘线程
//!      → 阻塞等 TrayCommand
//!         ├─ SetOrientation(o)  → ChangeDisplaySettingsExW → 读回刷新勾选
//!         ├─ ToggleOrientation  → 以**实际朝向**为基准 0°↔270° → 同上
//!         ├─ CycleDisplay       → CCD 摘除 → 停 down_seconds → 原样挂回
//!         └─ Quit               → 干净退出
//! ```
//!
//! 硬约束与"别这么干"清单见 `rotate.rs` 模块注释 (全是实测结论)。
//! 单实例: 托盘只允许一份 (`<TEMP>\display-rotate.lock`), 命令行一次性动作不抢锁。

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
// zig 链接器不实现 rustc 传入的 auto-image-base 选项, 忽略该 linker 消息
#![allow(linker_messages)]
// 非 Windows 构建里托盘整块 + 全部 Win32/CCD 代码都是 cfg 掉的 stub 路径
// (spawn_tray 直接返回 None), 于是它们在 Linux 上"从未被使用" —— 这是设计使然,
// 不是真的死代码。Windows 目标 (唯一交付物) 是零 warning 的。
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

mod i18n;
mod rotate;
mod tray;

use i18n::t as tr;
use rotate::Orientation;
use tray::{TrayCommand, TrayHandle, ORIENTATION_UNKNOWN};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// 托盘名 / MessageBoxW 标题
const APP_TITLE: &str = "display-rotate";

/// 内置默认目标显示器 (config.toml 的 `[display] device` 覆盖)
const DEFAULT_DEVICE: &str = "\\\\.\\DISPLAY5";
/// CCD 拔插时显示器摘除的默认时长 (秒)
const DEFAULT_DOWN_SECONDS: u64 = 2;
/// 事件日志总占用上限默认值 (MB)
const DEFAULT_MAX_LOG_MB: u64 = 5;

/// 事件日志目录名 (Windows: %LOCALAPPDATA%\<name>)
const LOG_DIR_NAME: &str = "display-rotate";

#[cfg(target_os = "linux")]
const POWERSHELL: &str = "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe";
#[cfg(target_os = "windows")]
const POWERSHELL: &str = "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe";

/// 事件日志目录默认路径: %LOCALAPPDATA%\display-rotate
fn default_log_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        std::env::temp_dir().join(LOG_DIR_NAME)
    }
    #[cfg(target_os = "windows")]
    {
        env::var("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir())
            .join(LOG_DIR_NAME)
    }
}

/// 单实例锁路径 (托盘只允许一份)
fn lock_path() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        PathBuf::from("/tmp/display-rotate.lock")
    }
    #[cfg(target_os = "windows")]
    {
        std::env::temp_dir().join("display-rotate.lock")
    }
}

struct Config {
    /// GDI 设备名, 如 `\\.\DISPLAY5`
    device: String,
    /// CCD 拔插时摘除时长
    down: Duration,
    /// 事件日志目录
    log_dir: PathBuf,
    /// 事件日志滚动总上限
    max_log_mb: u64,
}

/// Windows 子进程不带控制台窗口 (CREATE_NO_WINDOW 0x08000000)。
#[cfg(target_os = "windows")]
fn no_window(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000)
}
#[cfg(not(target_os = "windows"))]
fn no_window(cmd: &mut Command) -> &mut Command {
    cmd
}

// ---------------------------------------------------------------- 本地时间

#[cfg(target_os = "windows")]
static TZ_OFFSET: OnceLock<i64> = OnceLock::new();

#[cfg(target_os = "windows")]
fn tz_offset() -> i64 {
    *TZ_OFFSET.get_or_init(|| {
        let out = no_window(&mut Command::new(POWERSHELL))
            .args(["-NoProfile", "-Command", "Write-Output ([Math]::Round(([DateTimeOffset]::Now).Offset.TotalSeconds))"])
            .output();
        if let Ok(o) = out {
            if let Ok(secs) = String::from_utf8_lossy(&o.stdout).trim().parse::<i64>() {
                return secs;
            }
        }
        0
    })
}

/// 纯 Rust 本地时间格式化 (Hinnant civil_from_days 算法)
#[cfg(target_os = "windows")]
fn format_local(unix_secs: i64, offset_secs: i64) -> String {
    let local = unix_secs + offset_secs;
    let days = local.div_euclid(86_400);
    let sod = local.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (sod / 3600, (sod % 3600) / 60, sod % 60);
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
}

#[cfg(target_os = "windows")]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 本地时间字符串 (事件日志 [时间] 前缀)
fn now_str() -> String {
    #[cfg(target_os = "linux")]
    {
        Command::new("/bin/date")
            .args(["+%F %T"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    }
    #[cfg(target_os = "windows")]
    {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        format_local(secs, tz_offset())
    }
}

// ---------------------------------------------------------------- 事件日志
// %LOCALAPPDATA%\display-rotate\display-rotate.log; 单文件超 2MB 滚动
// (.N→...→.1→主), 段数 N = ceil(总上限/2MB), 总占用 ≤ --max-log-mb。
// 托盘程序是 windows_subsystem = "windows", 没有控制台 ⇒ 这个文件就是唯一的日志落点。

static LOG: OnceLock<(PathBuf, u64)> = OnceLock::new(); // (主日志路径, 滚动段数-1)

/// 滚动段数-1: 单文件 2MB, 段数 = ceil(总上限/2MB) 且至少 1 段 ⇒ n_rot = 段数-1
fn log_rot_count(max_log_mb: u64) -> u64 {
    ((max_log_mb + 1) / 2).max(1) - 1
}

fn init_log(cfg: &Config) {
    let _ = fs::create_dir_all(&cfg.log_dir);
    let main = cfg.log_dir.join("display-rotate.log");
    let n_rot = log_rot_count(cfg.max_log_mb);
    let _ = LOG.set((main, n_rot));
}

/// 事件日志: 写一行 (带 [本地时间] 前缀由调用方保证), 超 2MB 先滚动
fn emit(s: &str) {
    println!("{s}");
    let Some((main, n_rot)) = LOG.get() else { return };
    if let Ok(md) = fs::metadata(main) {
        if md.len() > SEGMENT_BYTES {
            rotate_logs(main, *n_rot);
        }
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(main) {
        let _ = writeln!(f, "{s}");
    }
}

const SEGMENT_BYTES: u64 = 2 * 1024 * 1024;

/// 滚动: 删最旧段 → .N-1→.N ... → .1→.2 → 主→.1; n_rot=0 时直接清空
fn rotate_logs(main: &Path, n_rot: u64) {
    let dir = main
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let base = main
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "display-rotate.log".into());
    if n_rot == 0 {
        let _ = fs::write(main, "");
        return;
    }
    let _ = fs::remove_file(dir.join(format!("{base}.{n_rot}")));
    for i in (1..n_rot).rev() {
        let _ = fs::rename(
            dir.join(format!("{base}.{i}")),
            dir.join(format!("{base}.{}", i + 1)),
        );
    }
    let _ = fs::rename(main, dir.join(format!("{base}.1")));
}

// ---------------------------------------------------------------- 单实例锁

/// 单实例锁获取结果: 打开失败与"已被占用"区分
enum LockStatus {
    Acquired(File),
    Busy,
    OpenFailed(String),
}

/// File::try_lock (Windows: LockFileEx / Linux: flock), 进程退出/崩溃时内核自动释放
fn acquire_lock() -> LockStatus {
    let path = lock_path();
    let f = match fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) => return LockStatus::OpenFailed(format!("{}: {e}", path.display())),
    };
    match f.try_lock() {
        Ok(()) => LockStatus::Acquired(f),
        Err(_) => LockStatus::Busy,
    }
}

// ---------------------------------------------------------------- config.toml
// 手写解析 (与 llama-watch 同一套): 只为两个键引 toml/serde 不值当。
// 字符串值推荐用单引号(TOML 字面字符串): `device = '\\.\DISPLAY5'` 反斜杠原样保留,
// 双引号里 `\D`/`\d` 是非法/意外转义, 会把设备名解析坏。

/// 去 TOML 字符串值的引号:
/// - `'...'` 字面字符串: 去掉首尾单引号, 内容原样(反斜杠不转义) —— 设备名/路径首选。
/// - `"..."` 基础字符串: 去掉首尾双引号并解码 `\\`/`\"` 转义。
/// - 无引号(数值/裸词): 原样。
fn toml_value(raw: &str) -> String {
    let v = raw.trim();
    if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        return v[1..v.len() - 1].to_string();
    }
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        let inner = &v[1..v.len() - 1];
        // 基础字符串转义: \\ → \ , \" → " ; 其余原样(不做 \t 等解码,
        // 因为那是把双引号路径变 TAB 的元凶)。
        let mut out = String::with_capacity(inner.len());
        let mut it = inner.chars();
        while let Some(c) = it.next() {
            if c == '\\' {
                match it.next() {
                    Some('\\') => out.push('\\'),
                    Some('"') => out.push('"'),
                    Some('\'') => out.push('\''),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    v.to_string()
}

/// 把 config.toml 内容应用到 cfg(逐行幂等)。未知键/坏值忽略。
/// 只认冻结规格里的两个键: `device` / `down_seconds` (节头 `[display]` 被跳过)。
fn apply_toml(cfg: &mut Config, text: &str) {
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('[') {
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim();
        let val = toml_value(&line[eq + 1..]);
        match key {
            "device" => {
                if !val.is_empty() {
                    cfg.device = val;
                }
            }
            "down_seconds" => {
                // 下限 1s: 0 秒的"拔插"等于没拔
                if let Ok(v) = val.parse::<u64>() {
                    cfg.down = Duration::from_secs(v.max(1));
                }
            }
            _ => {}
        }
    }
}

/// 找 config.toml: exe 同目录优先, 其次当前目录。
fn find_config_files() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("config.toml");
            if p.is_file() {
                v.push(p);
            }
        }
    }
    if let Ok(cwd) = env::current_dir() {
        let p = cwd.join("config.toml");
        if p.is_file() && !v.iter().any(|x| *x == p) {
            v.push(p);
        }
    }
    v
}

/// 加载 exe 同目录(优先)与当前目录的 config.toml, 返回实际加载的文件列表。
fn load_config_file(cfg: &mut Config) -> Vec<PathBuf> {
    let files = find_config_files();
    for p in &files {
        if let Ok(text) = fs::read_to_string(p) {
            apply_toml(cfg, &text);
        }
    }
    files
}

// ---------------------------------------------------------------- 动作

/// 命令行一次性动作 (不带动作 = 走托盘)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    None,
    Set(Orientation),
    Toggle,
    Cycle,
    Status,
}

/// 从系统读回实际朝向, 写进共享状态供托盘菜单勾选; 读不到记 ORIENTATION_UNKNOWN。
/// 返回 None 时已经把错误记进日志并弹过 MessageBox 了。
fn refresh_orientation(cfg: &Config, shared: &AtomicU32) -> Option<Orientation> {
    match rotate::current_orientation(&cfg.device) {
        Ok(o) => {
            shared.store(o.degrees(), Ordering::Relaxed);
            Some(o)
        }
        Err(e) => {
            shared.store(ORIENTATION_UNKNOWN, Ordering::Relaxed);
            let msg = tr(
                i18n::lang(),
                "msg.status_unknown",
                &[("device", cfg.device.clone()), ("err", e)],
            );
            emit(&format!("[{0}] {msg}", now_str()));
            rotate::report_box(APP_TITLE, &msg);
            None
        }
    }
}

/// 切到目标朝向; 失败 = 日志 + stderr + MessageBoxW (托盘程序没控制台)。
fn apply_orientation(cfg: &Config, target: Orientation) -> bool {
    match rotate::set_orientation(&cfg.device, target) {
        Ok(()) => {
            emit(&format!(
                "[{0}] {1} → {2}° 已提交",
                now_str(),
                cfg.device,
                target.degrees()
            ));
            true
        }
        Err(e) => {
            let msg = tr(
                i18n::lang(),
                "msg.rotate_failed",
                &[
                    ("device", cfg.device.clone()),
                    ("deg", target.degrees().to_string()),
                    ("err", e),
                ],
            );
            emit(&format!("[{0}] {msg}", now_str()));
            rotate::report_box(APP_TITLE, &msg);
            false
        }
    }
}

/// CCD 拔插重初始化 (真·"把显示器关掉再开"; 不改方向)。
fn cycle_display_action(cfg: &Config) -> bool {
    emit(&format!(
        "[{0}] {1} 开始 CCD 拔插重初始化 (摘除 {2}s 后原样挂回)",
        now_str(),
        cfg.device,
        cfg.down.as_secs()
    ));
    match rotate::cycle_display(&cfg.device, cfg.down) {
        Ok(rotate::CycleOutcome::Restored) => {
            emit(&format!(
                "[{0}] {1} 已用原始配置挂回, 重初始化完成",
                now_str(),
                cfg.device
            ));
            true
        }
        Ok(rotate::CycleOutcome::Fallback {
            restore_error,
            fallback_error,
        }) => {
            // 摘除成功但挂回失败: 必须明说, 否则用户只看到屏黑了一下就再没回来
            let msg = tr(
                i18n::lang(),
                "msg.cycle_restore_failed",
                &[("device", cfg.device.clone()), ("err", restore_error)],
            );
            match fallback_error {
                Some(e) => emit(&format!("[{0}] {msg}；{e}", now_str())),
                None => emit(&format!("[{0}] {msg}", now_str())),
            }
            rotate::report_box(APP_TITLE, &msg);
            false
        }
        Err(e) => {
            let msg = tr(
                i18n::lang(),
                "msg.cycle_failed",
                &[("device", cfg.device.clone()), ("err", e)],
            );
            emit(&format!("[{0}] {msg}", now_str()));
            rotate::report_box(APP_TITLE, &msg);
            false
        }
    }
}

/// 托盘主循环: 阻塞等命令 (本程序没有网络/定时轮询,
/// 不需要 llama-watch 那种 20ms tick 的 accept 循环)。
fn run(cfg: &Config, shared: Arc<AtomicU32>, cmd_rx: Option<&Receiver<TrayCommand>>) {
    let Some(rx) = cmd_rx else {
        emit("无托盘可用 (非 Windows 构建或托盘初始化失败), 退出");
        return;
    };
    while let Ok(cmd) = rx.recv() {
        match cmd {
            TrayCommand::Quit => {
                emit("[tray] 收到退出命令, 准备干净退出");
                return;
            }
            TrayCommand::SetOrientation(target) => {
                emit(&format!("[tray] 切方向: {}°", target.degrees()));
                apply_orientation(cfg, target);
                refresh_orientation(cfg, &shared);
            }
            TrayCommand::ToggleOrientation => {
                // 以**系统实际朝向**为基准做 toggle, 不是以"上次请求的"
                if let Some(cur) = refresh_orientation(cfg, &shared) {
                    let target = cur.toggle_landscape();
                    emit(&format!(
                        "[tray] 双击图标: {}° → {}°",
                        cur.degrees(),
                        target.degrees()
                    ));
                    apply_orientation(cfg, target);
                    refresh_orientation(cfg, &shared);
                }
            }
            TrayCommand::CycleDisplay => {
                cycle_display_action(cfg);
                refresh_orientation(cfg, &shared);
            }
        }
    }
    emit("[tray] 托盘命令通道已关闭, 退出");
}

/// 一次性动作, 返回进程退出码。
fn run_once(cfg: &Config, action: Action) -> i32 {
    let ok = match action {
        Action::Status => match rotate::current_orientation(&cfg.device) {
            Ok(o) => {
                println!("{} = {}°", cfg.device, o.degrees());
                emit(&format!("[{0}] {1} = {2}°", now_str(), cfg.device, o.degrees()));
                true
            }
            Err(e) => {
                let msg = tr(
                    i18n::lang(),
                    "msg.status_unknown",
                    &[("device", cfg.device.clone()), ("err", e)],
                );
                emit(&format!("[{0}] {msg}", now_str()));
                rotate::report_box(APP_TITLE, &msg);
                false
            }
        },
        Action::Set(o) => apply_orientation(cfg, o),
        Action::Toggle => match rotate::current_orientation(&cfg.device) {
            Ok(cur) => {
                let target = cur.toggle_landscape();
                emit(&format!(
                    "[{0}] toggle: {1}° → {2}°",
                    now_str(),
                    cur.degrees(),
                    target.degrees()
                ));
                apply_orientation(cfg, target)
            }
            Err(e) => {
                let msg = tr(
                    i18n::lang(),
                    "msg.status_unknown",
                    &[("device", cfg.device.clone()), ("err", e)],
                );
                emit(&format!("[{0}] {msg}", now_str()));
                rotate::report_box(APP_TITLE, &msg);
                false
            }
        },
        Action::Cycle => cycle_display_action(cfg),
        Action::None => true,
    };
    if ok {
        0
    } else {
        1
    }
}

// ---------------------------------------------------------------- 入口

fn usage() -> ! {
    eprintln!(
        "display-rotate v{VERSION} — Windows 显示器方向托盘程序 (无窗口常驻)\n\
         \n\
         用法: display-rotate [动作] [选项]\n\
         \n\
         不带动作时: 常驻托盘。右键 = 方向/重新初始化显示(拔插)/退出, 双击 = 横竖屏 toggle。\n\
         \n\
         动作 (一次性, 做完即退出; 失败退出码 1):\n\
           --set <0|90|180|270>   切到指定方向\n\
           --toggle               横屏 0° ↔ 竖屏 270°\n\
           --cycle                只做 CCD 拔插重初始化 (不改方向)\n\
           --status               打印当前实际朝向\n\
         \n\
         选项:\n\
           --device <\\\\.\\DISPLAYn>\n\
                                  目标显示器 (默认 {DEFAULT_DEVICE})\n\
           --down-seconds <n>     CCD 拔插时摘除的秒数 (默认 {DEFAULT_DOWN_SECONDS})\n\
           --log-dir <path>       事件日志目录\n\
           --max-log-mb <n>       事件日志滚动总上限 MB (默认 {DEFAULT_MAX_LOG_MB})\n\
           --version              打印版本\n\
           --help, -h             本帮助\n\
         \n\
         配置: exe 同目录(其次当前目录)的 config.toml, 键 [display] device / down_seconds;\n\
               优先级 内置默认值 < config.toml < 命令行参数。"
    );
    std::process::exit(0);
}

/// 取下一个参数值; 缺失则报错退出。
fn next_val(args: &[String], i: &mut usize, flag: &str) -> String {
    *i += 1;
    match args.get(*i) {
        Some(v) => v.clone(),
        None => {
            eprintln!("{flag} 缺少值");
            std::process::exit(2);
        }
    }
}

fn parse_u64(s: &str, flag: &str) -> u64 {
    match s.parse::<u64>() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("{flag} 需要非负整数, 收到 {s:?}");
            std::process::exit(2);
        }
    }
}

fn main() {
    // 启动即检测系统 UI 语言并缓存 (菜单/报错文案按此中英自适应)
    let _ = i18n::init_lang();
    let args: Vec<String> = env::args().skip(1).collect();
    let mut cfg = Config {
        device: DEFAULT_DEVICE.to_string(),
        down: Duration::from_secs(DEFAULT_DOWN_SECONDS),
        log_dir: default_log_dir(),
        max_log_mb: DEFAULT_MAX_LOG_MB,
    };
    let mut action = Action::None;

    // 配置来源: 内置默认 < config.toml < 命令行参数
    let config_files = load_config_file(&mut cfg);

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--device" => cfg.device = next_val(&args, &mut i, "--device"),
            "--down-seconds" => {
                // 下限 1s: 0 秒的"拔插"等于没拔
                let s = parse_u64(&next_val(&args, &mut i, "--down-seconds"), "--down-seconds");
                cfg.down = Duration::from_secs(s.max(1));
            }
            "--log-dir" => cfg.log_dir = PathBuf::from(next_val(&args, &mut i, "--log-dir")),
            "--max-log-mb" => {
                cfg.max_log_mb = parse_u64(&next_val(&args, &mut i, "--max-log-mb"), "--max-log-mb")
            }
            "--set" => {
                let v = next_val(&args, &mut i, "--set");
                let deg = parse_u64(&v, "--set");
                // 先 try_from 再查表: 直接 `as u32` 会把 4294967296 截成 0° 而"成功"
                match u32::try_from(deg).ok().and_then(Orientation::from_degrees) {
                    Some(o) => action = Action::Set(o),
                    None => {
                        eprintln!("--set 只接受 0/90/180/270, 收到 {v:?}");
                        std::process::exit(2);
                    }
                }
            }
            "--toggle" => action = Action::Toggle,
            "--cycle" => action = Action::Cycle,
            "--status" => action = Action::Status,
            "--version" => {
                println!("display-rotate v{VERSION}");
                return;
            }
            "--help" | "-h" => usage(),
            other => {
                eprintln!("未知参数: {other}");
                eprintln!("用 --help 查看用法");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    init_log(&cfg);

    if config_files.is_empty() {
        emit("配置: 使用内置默认值 (可放 exe 同目录的 config.toml 覆盖)");
    } else {
        for p in &config_files {
            emit(&format!("配置: 已加载 {}", p.display()));
        }
        emit("配置: 命令行参数优先级最高");
    }

    // 一次性动作: 做完就退出 —— 不碰托盘, 也不抢单实例锁
    // (所以托盘在跑的时候 `--set 90` 照样能用)
    if action != Action::None {
        let code = run_once(&cfg, action);
        std::process::exit(code);
    }

    // 单实例锁: 托盘只允许一份 (否则任务栏里两个图标互相打架)
    let _lock = match acquire_lock() {
        LockStatus::Acquired(f) => Some(f),
        LockStatus::Busy => {
            emit(&format!(
                "已有 display-rotate 托盘在运行 (锁 {}), 退出",
                lock_path().display()
            ));
            return;
        }
        LockStatus::OpenFailed(msg) => {
            emit(&format!("无法打开单实例锁文件: {msg}, 退出"));
            return;
        }
    };

    let t = now_str();
    emit(&format!(
        "[{t}] display-rotate v{VERSION} 启动: device={} down={}s log={} max_log_mb={}",
        cfg.device,
        cfg.down.as_secs(),
        cfg.log_dir.join("display-rotate.log").display(),
        cfg.max_log_mb
    ));

    // 主循环与托盘线程共享"当前实际朝向": 启动先读一次, 菜单勾选与双击 toggle 都靠它
    let shared = Arc::new(AtomicU32::new(ORIENTATION_UNKNOWN));
    refresh_orientation(&cfg, &shared);

    // Windows: 启动托盘线程, 拿到 cmd_rx 让 run 主循环感知菜单命令
    // Linux / 非 Windows: 拿不到 Receiver, 只能走上面的一次性动作路径
    let _tray_handle: Option<TrayHandle>;
    let cmd_rx: Option<Receiver<TrayCommand>>;
    #[cfg(target_os = "windows")]
    {
        match tray::spawn_tray(Arc::clone(&shared)) {
            Some((h, rx)) => {
                _tray_handle = Some(h);
                cmd_rx = Some(rx);
                emit("[tray] 托盘已启动 (右键 = 方向/重新初始化显示(拔插)/退出, 双击 = 横竖屏 toggle)");
            }
            None => {
                _tray_handle = None;
                cmd_rx = None;
                eprintln!("[tray] 托盘初始化失败, 以无托盘模式运行");
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        _tray_handle = None;
        cmd_rx = None;
    }
    run(&cfg, shared, cmd_rx.as_ref());
    // 走到这里说明托盘命令通道已关闭: 顺手把托盘线程 join 掉再退出
    if let Some(h) = _tray_handle {
        h.shutdown();
    }
}

// ---------------------------------------------------------------- 单元测试

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cfg() -> Config {
        Config {
            device: DEFAULT_DEVICE.to_string(),
            down: Duration::from_secs(DEFAULT_DOWN_SECONDS),
            log_dir: std::env::temp_dir().join(LOG_DIR_NAME),
            max_log_mb: DEFAULT_MAX_LOG_MB,
        }
    }

    #[test]
    fn toml_value_quotes() {
        assert_eq!(toml_value("'\\\\.\\DISPLAY5'"), "\\\\.\\DISPLAY5");
        assert_eq!(toml_value("  'a b'  "), "a b");
        assert_eq!(toml_value("\"C:\\\\x\""), "C:\\x");
        assert_eq!(toml_value("42"), "42");
        // 单引号里反斜杠原样保留(这就是设备名必须用单引号的原因)
        assert_eq!(toml_value("'C:\\AI\\x'"), "C:\\AI\\x");
    }

    #[test]
    fn apply_toml_reads_frozen_spec_keys() {
        let mut cfg = test_cfg();
        let text = "\
# display-rotate 配置示例
[display]
device = '\\\\.\\DISPLAY3'   # 目标屏
down_seconds = 7
unknown_key = '忽略我'
";
        apply_toml(&mut cfg, text);
        assert_eq!(cfg.device, "\\\\.\\DISPLAY3");
        assert_eq!(cfg.down, Duration::from_secs(7));
    }

    #[test]
    fn apply_toml_ignores_bad_values_and_keeps_defaults() {
        let mut cfg = test_cfg();
        apply_toml(&mut cfg, "[display]\ndevice = ''\ndown_seconds = abc\n");
        assert_eq!(cfg.device, DEFAULT_DEVICE);
        assert_eq!(cfg.down, Duration::from_secs(DEFAULT_DOWN_SECONDS));
    }

    #[test]
    fn down_seconds_has_floor_of_one() {
        let mut cfg = test_cfg();
        apply_toml(&mut cfg, "down_seconds = 0\n");
        assert_eq!(cfg.down, Duration::from_secs(1));
    }

    #[test]
    fn log_rot_count_derives_from_max_log_mb() {
        // 单段 2MB: 上限 5MB ⇒ 段数 3 ⇒ 滚动段 2
        assert_eq!(log_rot_count(5), 2);
        assert_eq!(log_rot_count(1), 0);
        assert_eq!(log_rot_count(2), 0);
        assert_eq!(log_rot_count(10), 4);
    }

    #[test]
    fn rotate_logs_works_on_display_rotate_log_name() {
        let dir = std::env::temp_dir().join(format!("display-rotate-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let main = dir.join("display-rotate.log");
        fs::write(&main, "new").unwrap();
        fs::write(dir.join("display-rotate.log.1"), "old1").unwrap();
        fs::write(dir.join("display-rotate.log.2"), "old2").unwrap();

        rotate_logs(&main, 2);
        // 主 → .1, .1 → .2, 原 .2 被删; 主文件不存在(由 emit 的下一次 append 重建)
        assert!(!main.exists());
        assert_eq!(
            fs::read_to_string(dir.join("display-rotate.log.1")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(dir.join("display-rotate.log.2")).unwrap(),
            "old1"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_once_status_never_panics() {
        // 非 Windows 上 --status 必然失败, 但必须是干净的 1 而不是 panic
        #[cfg(not(target_os = "windows"))]
        assert_eq!(run_once(&test_cfg(), Action::Status), 1);
    }
}
