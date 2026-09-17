//! Windows 托盘图标 + 右键菜单
//!
//! 菜单命令通过 mpsc Sender 推送到主循环 (托盘线程自己不碰显示器):
//! - SetOrientation: 切到某个方向 (四个单选项)
//! - ToggleOrientation: 双击图标 = 横屏 0° ↔ 竖屏 270°
//! - CycleDisplay: 重新初始化显示 (CCD 拔插)
//! - Quit: 干净退出
//!
//! 菜单里四个方向项的勾选**反映系统实际朝向**, 不反映"上次请求的": 主循环每次
//! 动作后都把 `rotate::current_orientation` 的结果写进共享的 `AtomicU32`
//! (`ORIENTATION_UNKNOWN` = 读不到), 菜单是在弹出的那一刻现构的, 所以读到的
//! 永远是最新值。
//!
//! 非 Windows 平台: 仅保留 TrayCommand/TrayHandle/spawn_tray stub 类型
//! (让 main.rs 的 `run(cfg, shared, cmd_rx: Option<&Receiver<TrayCommand>>)` 在 Linux 上也能编译)。

use std::sync::atomic::AtomicU32;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
#[cfg(target_os = "windows")]
use std::thread::{self, JoinHandle};
#[cfg(target_os = "windows")]
use std::time::Duration;

use crate::rotate::Orientation;

/// 共享朝向的哨兵值: "读不到当前朝向" (菜单里就一个都不勾)。
pub const ORIENTATION_UNKNOWN: u32 = u32::MAX;

const ICON_BYTES: &[u8] = include_bytes!("../assets/display-rotate.ico");

const WINDOW_CLASS: &str = "DisplayRotateTrayClass";
const TRAY_HWND_TITLE: &str = "DisplayRotateTray";
const APP_TIP: &str = "display-rotate";

const ID_ROT0: usize = 1001;
const ID_ROT90: usize = 1002;
const ID_ROT180: usize = 1003;
const ID_ROT270: usize = 1004;
const ID_CYCLE: usize = 1005;
const ID_QUIT: usize = 1006;

const WM_TRAYICON: u32 = 0x8000 + 1;
const HWND_MESSAGE_VAL: isize = -3;
const WM_QUIT_VAL: u32 = 0x0012;
const WM_COMMAND_VAL: u32 = 0x0111;

// ===== ICO 多尺寸帧解析 (平台无关, 所以能在 Linux 上单测) =====

/// ICO 文件头 ICONDIR = 6 字节 (reserved u16, type u16, count u16)
const ICONDIR_SIZE: usize = 6;
/// 一个 ICONDIRENTRY = 16 字节
const ICONDIRENTRY_SIZE: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IconFrame {
    /// 边长 (宽高取大者; 目录里 0 表示 256 —— 一个字节放不下 256)
    edge: u32,
    /// 图像数据长度 (dwBytesInRes)
    size: usize,
    /// 图像数据偏移 (dwImageOffset)
    offset: usize,
}

/// 遍历 ICO 目录里的 **全部** N 帧 (N = ICONDIR.count, 偏移 4, u16 LE)。
///
/// 注意: llama-watch 的实现只读第 0 个条目 (`&bytes[6..22]`), 多尺寸 .ico 在它眼里
/// 只有一个尺寸 —— 本项目的图标是 9 帧的, 必须整目录遍历。
fn parse_icon_frames(bytes: &[u8]) -> Result<Vec<IconFrame>, String> {
    if bytes.len() < ICONDIR_SIZE {
        return Err(format!("ICO 字节太短: {} 字节", bytes.len()));
    }
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if count == 0 {
        return Err("ICO 目录里一帧都没有 (count=0)".into());
    }
    let mut frames = Vec::with_capacity(count);
    for i in 0..count {
        let e = ICONDIR_SIZE + i * ICONDIRENTRY_SIZE;
        let Some(entry) = bytes.get(e..e + ICONDIRENTRY_SIZE) else {
            return Err(format!(
                "ICO 目录第 {i} 项越界 (count={count}, file_len={})",
                bytes.len()
            ));
        };
        let w = if entry[0] == 0 { 256 } else { entry[0] as u32 };
        let h = if entry[1] == 0 { 256 } else { entry[1] as u32 };
        let size = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
        let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
        if offset
            .checked_add(size)
            .map(|n| n > bytes.len())
            .unwrap_or(true)
        {
            return Err(format!(
                "ICO 第 {i} 帧数据越界: offset={offset} size={size} file_len={}",
                bytes.len()
            ));
        }
        frames.push(IconFrame {
            edge: w.max(h),
            size,
            offset,
        });
    }
    Ok(frames)
}

/// 选最匹配 `desired` 的一帧: 优先「不小于 desired 里最小的」, 没有就取最大的那帧。
///
/// 托盘小图标尺寸随显示器 DPI 变 (100% → 16px, 175% → 28px), 所以不能写死 16;
/// 选中帧还会作为 `cxDesired`/`cyDesired` 传给 `CreateIconFromResourceEx`,
/// 帧稍稍不匹配时由 Windows 缩 (但别拿 256 去缩 16 —— 线稿会糊成一团)。
fn pick_icon_frame(bytes: &[u8], desired: u32) -> Result<IconFrame, String> {
    let frames = parse_icon_frames(bytes)?;
    let mut best = frames[0];
    for f in &frames[1..] {
        let take = match (best.edge >= desired, f.edge >= desired) {
            (false, true) => true,                // 合格的胜过不合格的
            (true, false) => false,
            (true, true) => f.edge < best.edge,   // 都合格: 取更小的
            (false, false) => f.edge > best.edge, // 都不合格: 取更大的
        };
        if take {
            best = *f;
        }
    }
    Ok(best)
}

/// 菜单项 id ↔ 方向
fn menu_id(o: Orientation) -> usize {
    match o {
        Orientation::Deg0 => ID_ROT0,
        Orientation::Deg90 => ID_ROT90,
        Orientation::Deg180 => ID_ROT180,
        Orientation::Deg270 => ID_ROT270,
    }
}

fn orientation_of_menu_id(id: usize) -> Option<Orientation> {
    Orientation::ALL.into_iter().find(|o| menu_id(*o) == id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    SetOrientation(Orientation),
    ToggleOrientation,
    CycleDisplay,
    Quit,
}

pub struct TrayHandle {
    shutdown_tx: Sender<()>,
    #[cfg(target_os = "windows")]
    join: Option<JoinHandle<()>>,
    #[cfg(not(target_os = "windows"))]
    _join: std::marker::PhantomData<()>,
}

impl TrayHandle {
    pub fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        #[cfg(target_os = "windows")]
        if let Some(j) = self.join {
            let _ = j.join();
        }
    }
}

// ===== 平台分隔线: Windows 以下为完整 Win32 实现, 非 Windows 是 stub =====
#[cfg(target_os = "windows")]
mod imp {
    use super::*;

    use crate::i18n::t as tr;
    use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
        NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CheckMenuRadioItem, CreateIconFromResourceEx, CreatePopupMenu,
        CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow,
        DispatchMessageW, GetCursorPos, GetSystemMetrics, GetWindowLongPtrW, HCURSOR, HICON, HMENU,
        IDI_APPLICATION, LR_DEFAULTCOLOR, LoadCursorW, MF_BYCOMMAND, MF_SEPARATOR, MF_STRING,
        MSG, PM_REMOVE, PeekMessageW, PostQuitMessage, RegisterClassExW, SetForegroundWindow,
        SetWindowLongPtrW, SM_CXSMICON, SM_CYSMICON, TPM_NONOTIFY, TPM_RETURNCMD,
        TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, GWLP_USERDATA, WNDCLASSEXW,
    };

    const WM_DESTROY_VAL: u32 = 0x0002;
    // 托盘回调事件 (NOTIFYICON_VERSION_4: LOWORD(lParam) = 事件, HIWORD = 图标 id)
    const WM_LBUTTONDBLCLK_VAL: u32 = 0x0203;
    const WM_RBUTTONUP_VAL: u32 = 0x0205;
    const WM_CONTEXTMENU_VAL: u32 = 0x007B;

    /// 用户数据 + icon + 共享朝向一起打包; 通过 unsafe impl Send 跨线程传递。
    /// `orientation` 由主循环写、由托盘线程在弹出的那一刻读。
    pub(super) struct TrayCtx {
        pub(super) tx: Sender<TrayCommand>,
        pub(super) orientation: Arc<AtomicU32>,
        pub(super) icon: HICON,
    }
    // SAFETY: 只在 tray 线程内访问 icon; Sender/Arc<AtomicU32> 都是 Send; 整体唯一所有者.
    unsafe impl Send for TrayCtx {}

    pub(super) fn create_icon_from_ico_bytes(bytes: &[u8]) -> Result<HICON, String> {
        // 托盘小图标的尺寸**随所在显示器的 DPI 变** (实测: 100% → 16px, 175% → 28px),
        // 所以先问系统要度量, 再挑最匹配的那帧, 而不是写死 16 或只取第 0 帧。
        let (cx, cy) = unsafe { (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON)) };
        let wanted = cx.max(cy).max(1) as u32;
        let frame = pick_icon_frame(bytes, wanted)?;
        let image_data = &bytes[frame.offset..frame.offset + frame.size];
        let h = unsafe {
            CreateIconFromResourceEx(
                image_data.as_ptr(),
                image_data.len() as u32,
                1,
                0x00030000,
                cx,
                cy,
                LR_DEFAULTCOLOR,
            )
        };
        if h == 0 {
            return Err(format!(
                "CreateIconFromResourceEx 失败 (选中 {}px 帧 / {} 字节, 目标 {}x{})",
                frame.edge, frame.size, cx, cy
            ));
        }
        Ok(h)
    }

    pub(super) fn run_tray_thread(ctx: TrayCtx, shutdown_rx: Receiver<()>) {
        let hicon = ctx.icon;
        unsafe {
            let hinstance: HINSTANCE = GetModuleHandleW(std::ptr::null());
            let class_w: Vec<u16> = WINDOW_CLASS
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let title_w: Vec<u16> = TRAY_HWND_TITLE
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let tip_w: Vec<u16> = APP_TIP.encode_utf16().chain(std::iter::once(0)).collect();

            let hcursor: HCURSOR = LoadCursorW(0 as HINSTANCE, IDI_APPLICATION);

            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0,
                lpfnWndProc: Some(window_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: hicon,
                hCursor: hcursor,
                hbrBackground: 0 as _,
                lpszMenuName: 0 as _,
                lpszClassName: class_w.as_ptr(),
                hIconSm: 0 as _,
            };
            let atom = RegisterClassExW(&wc);
            if atom == 0 {
                eprintln!("[tray] RegisterClassExW 失败");
                return;
            }

            let hwnd = CreateWindowExW(
                0,
                class_w.as_ptr(),
                title_w.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE_VAL as HWND,
                0 as HMENU,
                hinstance,
                std::ptr::null(),
            );
            if hwnd == 0 {
                eprintln!("[tray] CreateWindowExW 失败");
                return;
            }

            // 唯一的 Box, 生命周期 = 托盘线程; window_proc 通过 GWLP_USERDATA 取它
            let ctx_ptr = Box::into_raw(Box::new(ctx));
            let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, ctx_ptr as isize);

            let mut nic = std::mem::zeroed::<NOTIFYICONDATAW>();
            nic.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nic.hWnd = hwnd;
            nic.uID = 1;
            nic.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
            nic.uCallbackMessage = WM_TRAYICON;
            nic.hIcon = hicon;
            for (i, &c) in tip_w.iter().take(127).enumerate() {
                nic.szTip[i] = c;
            }
            if Shell_NotifyIconW(NIM_ADD, &nic) == 0 {
                eprintln!("[tray] NIM_ADD 失败");
                DestroyWindow(hwnd);
                return;
            }
            nic.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &nic);

            let mut msg = std::mem::zeroed::<MSG>();
            loop {
                if shutdown_rx.try_recv().is_ok() {
                    break;
                }
                let r = PeekMessageW(&mut msg, 0 as HWND, 0, 0, PM_REMOVE);
                if r != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                    if msg.message == WM_QUIT_VAL {
                        break;
                    }
                } else {
                    thread::sleep(Duration::from_millis(100));
                }
            }

            let _ = Shell_NotifyIconW(NIM_DELETE, &nic);
            DestroyWindow(hwnd);
            drop(Box::from_raw(ctx_ptr));
            DestroyIcon(hicon);
        }
    }

    /// 现构菜单并弹出: 勾选项从 `ctx.orientation` 现读, 所以永远是系统实际朝向。
    unsafe fn build_and_show_menu(hwnd: HWND, ctx: &TrayCtx) -> usize {
        unsafe {
            let hmenu = CreatePopupMenu();
            if hmenu == 0 {
                return 0;
            }
            let lang = crate::i18n::lang();
            let mut labels: Vec<(usize, Vec<u16>)> = Vec::new();
            for o in Orientation::ALL {
                let s: Vec<u16> = tr(lang, o.menu_key(), &[])
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                labels.push((menu_id(o), s));
            }
            for (id, s) in &labels {
                let _ = AppendMenuW(hmenu, MF_STRING, *id, s.as_ptr());
            }
            // 四个方向项是连续的一组, 用 CheckMenuRadioItem 让勾选唯一;
            // 读不到朝向 (ORIENTATION_UNKNOWN) 时一个都不勾。
            if let Some(cur) = Orientation::from_degrees(
                ctx.orientation.load(std::sync::atomic::Ordering::Relaxed),
            ) {
                let _ = CheckMenuRadioItem(
                    hmenu,
                    ID_ROT0 as u32,
                    ID_ROT270 as u32,
                    menu_id(cur) as u32,
                    MF_BYCOMMAND,
                );
            }

            let s_cycle: Vec<u16> = tr(lang, "menu.cycle", &[])
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let s_quit: Vec<u16> = tr(lang, "menu.quit", &[])
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
            let _ = AppendMenuW(hmenu, MF_STRING, ID_CYCLE, s_cycle.as_ptr());
            let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());
            let _ = AppendMenuW(hmenu, MF_STRING, ID_QUIT, s_quit.as_ptr());

            let mut pt = POINT { x: 0, y: 0 };
            let _ = GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(hwnd);

            let cmd = TrackPopupMenu(
                hmenu,
                TPM_RIGHTBUTTON | TPM_NONOTIFY | TPM_RETURNCMD,
                pt.x,
                pt.y,
                0,
                hwnd,
                std::ptr::null(),
            );
            let _ = DestroyMenu(hmenu);
            cmd as usize
        }
    }

    unsafe fn handle_menu_cmd(cmd: usize, tx: &Sender<TrayCommand>) {
        if let Some(o) = orientation_of_menu_id(cmd) {
            let _ = tx.send(TrayCommand::SetOrientation(o));
            return;
        }
        match cmd {
            ID_CYCLE => {
                let _ = tx.send(TrayCommand::CycleDisplay);
            }
            ID_QUIT => {
                let _ = tx.send(TrayCommand::Quit);
            }
            _ => {}
        }
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayCtx;
        let _ = wparam;
        match msg {
            WM_TRAYICON => {
                let ev = (lparam as u32) & 0xFFFF;
                if !ctx_ptr.is_null() {
                    let ctx = &*ctx_ptr;
                    if ev == WM_RBUTTONUP_VAL || ev == WM_CONTEXTMENU_VAL {
                        let cmd = build_and_show_menu(hwnd, ctx);
                        handle_menu_cmd(cmd, &ctx.tx);
                    } else if ev == WM_LBUTTONDBLCLK_VAL {
                        // 双击 = 横竖屏 toggle (最常用的操作), 不弹菜单
                        let _ = ctx.tx.send(TrayCommand::ToggleOrientation);
                    }
                }
                0
            }
            WM_COMMAND_VAL => {
                let menu_id = (wparam as usize) & 0xFFFF;
                if !ctx_ptr.is_null() {
                    let ctx = &*ctx_ptr;
                    handle_menu_cmd(menu_id, &ctx.tx);
                }
                0
            }
            WM_DESTROY_VAL => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// 启动托盘线程; 失败返回 None。
/// `orientation` 是主循环与托盘线程共享的"当前实际朝向"。
pub fn spawn_tray(orientation: Arc<AtomicU32>) -> Option<(TrayHandle, Receiver<TrayCommand>)> {
    #[cfg(target_os = "windows")]
    {
        let hicon = match imp::create_icon_from_ico_bytes(ICON_BYTES) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[tray] 创建 HICON 失败: {e}");
                return None;
            }
        };
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<TrayCommand>();
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();
        let ctx = imp::TrayCtx {
            tx: cmd_tx,
            orientation,
            icon: hicon,
        };
        let join = thread::Builder::new()
            .name("display-rotate-tray".into())
            .spawn(move || imp::run_tray_thread(ctx, shutdown_rx))
            .ok()?;
        Some((
            TrayHandle {
                shutdown_tx,
                join: Some(join),
            },
            cmd_rx,
        ))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = orientation;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_are_unique_and_roundtrip() {
        let ids: Vec<usize> = Orientation::ALL.into_iter().map(menu_id).collect();
        for (i, a) in ids.iter().enumerate() {
            for b in &ids[i + 1..] {
                assert_ne!(a, b, "菜单 id 撞了: {a}");
            }
        }
        for o in Orientation::ALL {
            assert_eq!(orientation_of_menu_id(menu_id(o)), Some(o));
        }
    }

    #[test]
    fn unknown_menu_id_is_none() {
        assert_eq!(orientation_of_menu_id(ID_CYCLE), None);
        assert_eq!(orientation_of_menu_id(ID_QUIT), None);
        assert_eq!(orientation_of_menu_id(0), None);
    }

    /// 直接拿仓库里真正会被 include_bytes! 进去的那个 .ico 验: 9 帧、每帧都在文件内。
    #[test]
    fn shipped_ico_has_multi_size_frames() {
        let frames = parse_icon_frames(ICON_BYTES).expect("图标目录应能解析");
        assert_eq!(frames.len(), 9, "图标应为 9 帧, 实际 {}", frames.len());
        let edges: Vec<u32> = frames.iter().map(|f| f.edge).collect();
        assert_eq!(edges, vec![16, 20, 24, 28, 32, 40, 48, 64, 256]);
        for f in &frames {
            assert!(f.size > 0, "{0}px 帧长度是 0", f.edge);
        }
    }

    /// 托盘尺寸随 DPI 变: 100% → 16px, 175% → 28px。
    #[test]
    fn pick_icon_frame_matches_tray_size() {
        let pick = |d: u32| pick_icon_frame(ICON_BYTES, d).unwrap().edge;
        assert_eq!(pick(16), 16);
        assert_eq!(pick(20), 20);
        assert_eq!(pick(28), 28);
        // 没有精确帧时取"不小于目标里最小的"
        assert_eq!(pick(17), 20);
        assert_eq!(pick(56), 64);
        // 比所有帧都大 → 取最大帧, 由 Windows 自己缩
        assert_eq!(pick(300), 256);
        // 度量取不到 (0) 时不能 panic, 退化成最小帧
        assert_eq!(pick(0), 16);
    }

    #[test]
    fn parse_icon_frames_rejects_garbage() {
        assert!(parse_icon_frames(&[]).is_err());
        assert!(parse_icon_frames(&[0, 0, 1, 0, 0, 0]).is_err()); // count=0
        // count=2 但目录项只有一项
        let mut b = vec![0u8, 0, 1, 0, 2, 0];
        b.extend_from_slice(&[16, 16, 0, 0, 1, 0, 32, 0, 8, 0, 0, 0, 22, 0, 0, 0]);
        assert!(parse_icon_frames(&b).is_err());
        // 帧数据越界 (offset 22 + size 250 > 文件长度)
        let mut c = vec![0u8, 0, 1, 0, 1, 0];
        c.extend_from_slice(&[16, 16, 0, 0, 1, 0, 32, 0, 250, 0, 0, 0, 22, 0, 0, 0]);
        assert!(parse_icon_frames(&c).is_err());
    }
}
