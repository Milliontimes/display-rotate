//! 显示器方向切换 (GDI `ChangeDisplaySettingsEx`) + 真·CCD 拔插复位。
//!
//! 两套旋转枚举值**不要混用** (实测踩过):
//! - GDI `dmDisplayOrientation`: `0/1/2/3` = 0°/90°/180°/270°
//! - CCD `path.targetInfo.rotation`: `1/2/3/4` = IDENTITY/ROT90/ROT180/ROT270
//!
//! 本项目只把 GDI 那套写进 `dmDisplayOrientation`; CCD 那套只在拔插复位里原样搬运,
//! 不做换算。
//!
//! 硬约束 (全是实测结论, 不要"简化"):
//! - **只设 `dmDisplayOrientation` 会返回 `DISP_CHANGE_BADMODE`(-2)**: 必须同时把
//!   `dmPelsWidth`/`dmPelsHeight` 换成与目标方向自洽的长短边, 并在 `dmFields` 里
//!   同时置 `DM_DISPLAYORIENTATION|DM_PELSWIDTH|DM_PELSHEIGHT`。宽高与方向是一起
//!   校验的, 这是硬约束不是建议。
//! - `SDC_TOPOLOGY_*` **不能**当复位手段: 拓扑没变时它返回 0 但什么都不做 (假成功)。
//! - 拓扑标志上**不能**叠 `SDC_SAVE_TO_DATABASE`/`SDC_ALLOW_CHANGES`: 返回 87
//!   (`ERROR_INVALID_PARAMETER`)。
//! - 🔴 **绝不**用 `SC_MONITORPOWER`(`WM_SYSCOMMAND` 0xF170) 关屏: 会让显示器进
//!   待机, 实测可能把整机弄睡眠。
//!
//! 非 Windows 平台只保留类型 + 返回错误, 让 main.rs 在 Linux 上也能编译。

use std::time::Duration;

/// 目标显示器朝向。GDI 侧 `0/1/2/3` = 0°/90°/180°/270°。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Orientation {
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

impl Orientation {
    /// 菜单顺序 (0/90/180/270)
    pub const ALL: [Orientation; 4] = [
        Orientation::Deg0,
        Orientation::Deg90,
        Orientation::Deg180,
        Orientation::Deg270,
    ];

    /// 横屏 = 0° (与脚本 `-Orientation landscape` 同一约定)
    pub const LANDSCAPE: Orientation = Orientation::Deg0;
    /// 竖屏 = 270° (与脚本 `-Orientation portrait` 同一约定)
    pub const PORTRAIT: Orientation = Orientation::Deg270;

    pub fn degrees(self) -> u32 {
        match self {
            Orientation::Deg0 => 0,
            Orientation::Deg90 => 90,
            Orientation::Deg180 => 180,
            Orientation::Deg270 => 270,
        }
    }

    pub fn from_degrees(deg: u32) -> Option<Orientation> {
        match deg {
            0 => Some(Orientation::Deg0),
            90 => Some(Orientation::Deg90),
            180 => Some(Orientation::Deg180),
            270 => Some(Orientation::Deg270),
            _ => None,
        }
    }

    /// GDI `DEVMODE.dmDisplayOrientation` 取值
    pub fn devmode_value(self) -> u32 {
        match self {
            Orientation::Deg0 => 0,
            Orientation::Deg90 => 1,
            Orientation::Deg180 => 2,
            Orientation::Deg270 => 3,
        }
    }

    /// 从 GDI `DEVMODE.dmDisplayOrientation` 反解 (非法值返回 None)
    pub fn from_devmode(v: u32) -> Option<Orientation> {
        match v {
            0 => Some(Orientation::Deg0),
            1 => Some(Orientation::Deg90),
            2 => Some(Orientation::Deg180),
            3 => Some(Orientation::Deg270),
            _ => None,
        }
    }

    /// 竖屏 (90°/270°): 决定"长短边"怎么摆。
    pub fn is_portrait(self) -> bool {
        matches!(self, Orientation::Deg90 | Orientation::Deg270)
    }

    /// 双击托盘图标的 toggle: 横屏 0° ↔ 竖屏 270°。
    /// 当前是 0° → 270°, 其它 (90°/180°/270°) → 0°。
    pub fn toggle_landscape(self) -> Orientation {
        if self == Orientation::LANDSCAPE {
            Orientation::PORTRAIT
        } else {
            Orientation::LANDSCAPE
        }
    }

    /// i18n 字典 key (菜单文案)
    pub fn menu_key(self) -> &'static str {
        match self {
            Orientation::Deg0 => "menu.rot0",
            Orientation::Deg90 => "menu.rot90",
            Orientation::Deg180 => "menu.rot180",
            Orientation::Deg270 => "menu.rot270",
        }
    }
}

/// CCD 拔插复位的结果 (调用方据此记日志)。
#[derive(Debug)]
pub enum CycleOutcome {
    /// 摘除 → 等待 → 用原始配置挂回, 全程成功
    Restored,
    /// 摘除成功, 但用原始配置挂回失败 → 走了 `SDC_TOPOLOGY_EXTEND` 兜底
    Fallback {
        /// 挂回失败的原因 (含返回码)
        restore_error: String,
        /// 兜底也失败时的原因; 兜底成功则为 None
        fallback_error: Option<String>,
    },
}

/// 非 Windows 平台统一的失败说辞 (Win32/CCD 没有跨平台等价物)。
#[cfg(not(target_os = "windows"))]
pub const WINDOWS_ONLY: &str = "仅 Windows 支持 (Win32/CCD)";

/// 读回 `device` 的**当前实际**朝向 (不是"上次请求的")。
pub fn current_orientation(device: &str) -> Result<Orientation, String> {
    #[cfg(target_os = "windows")]
    {
        imp::current_orientation(device)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = device;
        Err(format!("{WINDOWS_ONLY}: 读不到当前朝向"))
    }
}

/// 把 `device` 切到 `target` (GDI 路径, `CDS_UPDATEREGISTRY` 落盘)。
/// 返回 `Err` 时消息里带 `ChangeDisplaySettingsExW` 的原始返回码。
pub fn set_orientation(device: &str, target: Orientation) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        imp::set_orientation(device, target)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (device, target);
        Err(format!("{WINDOWS_ONLY}: 无法切换方向"))
    }
}

/// 真·CCD 拔插复位: 把 `device` 从活动配置里摘掉 → 停 `down` → 原样挂回。
/// 这是"把显示器关掉再开"的软件等价物 (绝对值定位输入设备要它才会重新读方向)。
pub fn cycle_display(device: &str, down: Duration) -> Result<CycleOutcome, String> {
    #[cfg(target_os = "windows")]
    {
        imp::cycle_display(device, down)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (device, down);
        Err(format!("{WINDOWS_ONLY}: 无法做 CCD 拔插"))
    }
}

/// 出错时的用户可见反馈: 托盘程序是 `windows_subsystem = "windows"`, 没有控制台,
/// 只打日志等于什么都没发生 ⇒ 再弹一个 `MessageBoxW`。
pub fn report_box(title: &str, msg: &str) {
    eprintln!("[error] {title}: {msg}");
    #[cfg(target_os = "windows")]
    imp::message_box(title, msg);
}

// ===== 平台分隔线: Windows 以下为完整 Win32 实现, 非 Windows 是纯 stub =====

#[cfg(target_os = "windows")]
mod imp {
    use super::{CycleOutcome, Orientation};

    use std::time::Duration;

    use windows_sys::Win32::Devices::Display::{
        DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig, SetDisplayConfig,
        DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_MODE_INFO,
        DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
        SDC_ALLOW_CHANGES, SDC_APPLY, SDC_SAVE_TO_DATABASE, SDC_TOPOLOGY_EXTEND,
        SDC_USE_SUPPLIED_DISPLAY_CONFIG, SDC_VALIDATE,
    };
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Gdi::{
        ChangeDisplaySettingsExW, EnumDisplaySettingsW, CDS_UPDATEREGISTRY, DEVMODEW,
        DISP_CHANGE_SUCCESSFUL, DM_DISPLAYORIENTATION, DM_PELSHEIGHT, DM_PELSWIDTH,
        ENUM_CURRENT_SETTINGS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    };

    /// `DISPLAYCONFIG_PATH_MODE_IDX_INVALID`: windows-sys 0.52 **没有**导出这个常量
    /// (它把 `modeInfoIdx` 定义成 union, 但没给哨兵值), 自己定一个。
    const PATH_MODE_IDX_INVALID: u32 = 0xFFFF_FFFF;

    // 布局断言: 这些结构体的内存是直接交给 user32 的, 布局错了就是内存踩踏。
    // (编译期就能在交叉编译时验出来, 比运行时崩了再查便宜。)
    //
    // DEVMODEW = 220 而不是 156: **156 是 ANSI 版 DEVMODEA 的大小**,
    // 因为 A 版的 dmDeviceName/dmFormName 各是 32 字节, W 版各是 32 个 WCHAR = 64 字节。
    // 这里用的是 `EnumDisplaySettingsW`/`ChangeDisplaySettingsExW` + `DEVMODEW`,
    // 所以 dmSize 必须填 220 (= size_of::<DEVMODEW>()), 填 156 会让 API 少回填
    // dmPelsWidth/dmPelsHeight(偏移 172/176) 之外的东西, 直接烂掉。
    const _: () = assert!(std::mem::size_of::<DEVMODEW>() == 220);
    const _: () = assert!(std::mem::size_of::<DISPLAYCONFIG_PATH_INFO>() == 72);
    const _: () = assert!(std::mem::size_of::<DISPLAYCONFIG_MODE_INFO>() == 64);
    const _: () = assert!(std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() == 84);

    /// `\\.\DISPLAYn` → NUL 结尾的 UTF-16
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 取当前模式 (ENUM_CURRENT_SETTINGS) 的完整 DEVMODEW。
    fn current_devmode(device: &str) -> Result<DEVMODEW, String> {
        let dev = wide(device);
        let mut dm = unsafe { std::mem::zeroed::<DEVMODEW>() };
        dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        if unsafe { EnumDisplaySettingsW(dev.as_ptr(), ENUM_CURRENT_SETTINGS, &mut dm) } == 0 {
            return Err(format!(
                "EnumDisplaySettingsW 失败: {device} 不存在或当前不可访问"
            ));
        }
        Ok(dm)
    }

    pub(super) fn current_orientation(device: &str) -> Result<Orientation, String> {
        let dm = current_devmode(device)?;
        let raw = unsafe { dm.Anonymous1.Anonymous2.dmDisplayOrientation };
        Orientation::from_devmode(raw)
            .ok_or_else(|| format!("{device} 的 dmDisplayOrientation={raw} 不是 0/1/2/3"))
    }

    pub(super) fn set_orientation(device: &str, target: Orientation) -> Result<(), String> {
        let dev = wide(device);
        let mut dm = current_devmode(device)?;
        if unsafe { dm.Anonymous1.Anonymous2.dmDisplayOrientation } == target.devmode_value() {
            // 已经在目标朝向上: 不重复提交, 免得白闪一下
            return Ok(());
        }
        // 硬约束: 只改 dmDisplayOrientation 会 DISP_CHANGE_BADMODE(-2)。
        // 宽高必须与目标方向自洽 —— 用当前像素数取出长短边再按方向摆回去。
        let (w, h) = (dm.dmPelsWidth, dm.dmPelsHeight);
        let (long_edge, short_edge) = (w.max(h), w.min(h));
        let (nw, nh) = if target.is_portrait() {
            (short_edge, long_edge)
        } else {
            (long_edge, short_edge)
        };
        dm.dmPelsWidth = nw;
        dm.dmPelsHeight = nh;
        dm.dmFields |= DM_DISPLAYORIENTATION | DM_PELSWIDTH | DM_PELSHEIGHT;

        let code = unsafe {
            ChangeDisplaySettingsExW(
                dev.as_ptr(),
                &dm,
                0 as HWND,
                CDS_UPDATEREGISTRY,
                std::ptr::null(),
            )
        };
        if code != DISP_CHANGE_SUCCESSFUL {
            // 码值同时进 stderr 与调用方的日志/弹窗 (托盘程序没控制台)
            eprintln!(
                "[rotate] ChangeDisplaySettingsExW({device}, {}°) 返回 {code} (原 {w}x{h} → {nw}x{nh})",
                target.degrees()
            );
            return Err(format!(
                "ChangeDisplaySettingsExW 返回 {code} ({}° {w}x{h} → {nw}x{nh})",
                target.degrees()
            ));
        }
        Ok(())
    }

    /// 一次 CCD 活动配置快照 (path/mode 两个数组是配套的, 一起拿走)。
    struct Ccd {
        paths: Vec<DISPLAYCONFIG_PATH_INFO>,
        modes: Vec<DISPLAYCONFIG_MODE_INFO>,
    }

    fn query_active() -> Result<Ccd, String> {
        let (mut n_path, mut n_mode) = (0u32, 0u32);
        let rc = unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut n_path, &mut n_mode)
        };
        if rc != 0 {
            return Err(format!("GetDisplayConfigBufferSizes 返回 {rc}"));
        }
        let mut paths =
            vec![unsafe { std::mem::zeroed::<DISPLAYCONFIG_PATH_INFO>() }; n_path as usize];
        let mut modes =
            vec![unsafe { std::mem::zeroed::<DISPLAYCONFIG_MODE_INFO>() }; n_mode as usize];
        let rc = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut n_path,
                paths.as_mut_ptr(),
                &mut n_mode,
                modes.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        if rc != 0 {
            return Err(format!("QueryDisplayConfig 返回 {rc}"));
        }
        paths.truncate(n_path as usize);
        modes.truncate(n_mode as usize);
        Ok(Ccd { paths, modes })
    }

    /// 一条 path 对应的 GDI 名 (`\\.\DISPLAYn`)。
    fn path_gdi_name(path: &DISPLAYCONFIG_PATH_INFO) -> Result<String, String> {
        let mut name = unsafe { std::mem::zeroed::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() };
        name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        name.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        name.header.adapterId = path.sourceInfo.adapterId;
        name.header.id = path.sourceInfo.id;
        let rc = unsafe { DisplayConfigGetDeviceInfo(&mut name.header) };
        if rc != 0 {
            return Err(format!("DisplayConfigGetDeviceInfo 返回 {rc}"));
        }
        let len = name
            .viewGdiDeviceName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(name.viewGdiDeviceName.len());
        Ok(String::from_utf16_lossy(&name.viewGdiDeviceName[..len]))
    }

    /// 旧 modeInfoIdx → 压缩后的新下标; 该 mode 还没搬过就顺手搬进 dst。
    fn remap_mode_idx(
        old: u32,
        remap: &mut Vec<(u32, u32)>,
        src: &[DISPLAYCONFIG_MODE_INFO],
        dst: &mut Vec<DISPLAYCONFIG_MODE_INFO>,
    ) -> Result<u32, String> {
        if old == PATH_MODE_IDX_INVALID {
            return Ok(PATH_MODE_IDX_INVALID);
        }
        if let Some((_, new)) = remap.iter().find(|(o, _)| *o == old) {
            return Ok(*new);
        }
        let Some(m) = src.get(old as usize) else {
            return Err(format!(
                "path 引用了越界的 modeInfoIdx={old} (mode 数组只有 {} 项)",
                src.len()
            ));
        };
        dst.push(*m);
        let new = (dst.len() - 1) as u32;
        remap.push((old, new));
        Ok(new)
    }

    /// 从配置里摘掉 `victim`: 其余 path 原样保留; **mode 数组只保留仍被引用的条目**,
    /// 并同步重映射每条 path 的 `sourceInfo.modeInfoIdx`/`targetInfo.modeInfoIdx`
    /// (`SDC_USE_SUPPLIED_DISPLAY_CONFIG` 不接受悬空 mode 引用, 不压缩就会失败)。
    fn drop_path(
        ccd: &Ccd,
        victim: usize,
    ) -> Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>), String> {
        let mut remap: Vec<(u32, u32)> = Vec::new(); // 旧下标 → 新下标
        let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = Vec::new();
        let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = Vec::new();
        for (i, p) in ccd.paths.iter().enumerate() {
            if i == victim {
                continue;
            }
            let mut p = *p;
            // modeInfoIdx 在 windows-sys 里是 union, 这里按"裸下标"语义取用
            let s = unsafe { p.sourceInfo.Anonymous.modeInfoIdx };
            let t = unsafe { p.targetInfo.Anonymous.modeInfoIdx };
            let ns = remap_mode_idx(s, &mut remap, &ccd.modes, &mut modes)?;
            let nt = remap_mode_idx(t, &mut remap, &ccd.modes, &mut modes)?;
            p.sourceInfo.Anonymous.modeInfoIdx = ns;
            p.targetInfo.Anonymous.modeInfoIdx = nt;
            paths.push(p);
        }
        Ok((paths, modes))
    }

    pub(super) fn cycle_display(device: &str, down: Duration) -> Result<CycleOutcome, String> {
        let ccd = query_active()?;
        let victim = ccd
            .paths
            .iter()
            .position(|p| {
                path_gdi_name(p)
                    .map(|n| n.eq_ignore_ascii_case(device))
                    .unwrap_or(false)
            })
            .ok_or_else(|| format!("活动 path 里没有 {device}"))?;
        if ccd.paths.len() <= 1 {
            // 只剩一条 path 时"摘掉它"等于把所有显示关掉, 不做
            return Err(format!(
                "活动 path 只有 {device} 一条, 拒绝整体摘除 (会关掉全部显示)"
            ));
        }
        let (paths, modes) = drop_path(&ccd, victim)?;

        // 1) 空跑校验: 不带 SDC_APPLY, 不产生任何实际变化
        let rc = unsafe {
            SetDisplayConfig(
                paths.len() as u32,
                paths.as_ptr(),
                modes.len() as u32,
                modes.as_ptr(),
                SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_VALIDATE | SDC_ALLOW_CHANGES,
            )
        };
        if rc != 0 {
            return Err(format!("摘除配置校验失败 (SDC_VALIDATE 返回 {rc})"));
        }

        // 2) 提交摘除 → 目标屏丢信号
        let rc = unsafe {
            SetDisplayConfig(
                paths.len() as u32,
                paths.as_ptr(),
                modes.len() as u32,
                modes.as_ptr(),
                SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_APPLY | SDC_ALLOW_CHANGES,
            )
        };
        if rc != 0 {
            return Err(format!("摘除提交失败 (SDC_APPLY 返回 {rc})"));
        }

        // 3) 摘除时长 (这才叫"拔插")
        std::thread::sleep(down);

        // 4) 用**原始**数组原样挂回 (带 SDC_SAVE_TO_DATABASE 落盘)
        let rc = unsafe {
            SetDisplayConfig(
                ccd.paths.len() as u32,
                ccd.paths.as_ptr(),
                ccd.modes.len() as u32,
                ccd.modes.as_ptr(),
                SDC_USE_SUPPLIED_DISPLAY_CONFIG
                    | SDC_APPLY
                    | SDC_SAVE_TO_DATABASE
                    | SDC_ALLOW_CHANGES,
            )
        };
        if rc == 0 {
            return Ok(CycleOutcome::Restored);
        }

        let restore_error = format!("挂回失败 (SetDisplayConfig 返回 {rc})");
        // 5) 兜底: Win+P "扩展" 拓扑。注意它在拓扑没变时是**假成功**, 所以只当
        //    挂回真的失败了的补救, 绝不当常规复位手段。
        let frc = unsafe {
            SetDisplayConfig(
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                SDC_TOPOLOGY_EXTEND | SDC_APPLY,
            )
        };
        Ok(CycleOutcome::Fallback {
            restore_error,
            fallback_error: (frc != 0)
                .then(|| format!("SDC_TOPOLOGY_EXTEND|SDC_APPLY 兜底也失败 (返回 {frc})")),
        })
    }

    pub(super) fn message_box(title: &str, msg: &str) {
        let t = wide(title);
        let m = wide(msg);
        unsafe {
            MessageBoxW(
                0 as HWND,
                m.as_ptr(),
                t.as_ptr(),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST,
            )
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn degress_roundtrip() {
        for o in Orientation::ALL {
            assert_eq!(Orientation::from_degrees(o.degrees()), Some(o));
        }
        assert_eq!(Orientation::from_degrees(45), None);
        assert_eq!(Orientation::from_degrees(u32::MAX), None);
    }

    #[test]
    fn devmode_enum_mapping() {
        assert_eq!(Orientation::Deg0.devmode_value(), 0);
        assert_eq!(Orientation::Deg90.devmode_value(), 1);
        assert_eq!(Orientation::Deg180.devmode_value(), 2);
        assert_eq!(Orientation::Deg270.devmode_value(), 3);
        for o in Orientation::ALL {
            assert_eq!(Orientation::from_devmode(o.devmode_value()), Some(o));
        }
        // CCD 的 1/2/3/4 与 GDI 的 0/1/2/3 是两套枚举: 4 在 GDI 侧非法
        assert_eq!(Orientation::from_devmode(4), None);
    }

    #[test]
    fn portrait_flag_and_edges() {
        assert!(!Orientation::Deg0.is_portrait());
        assert!(Orientation::Deg90.is_portrait());
        assert!(!Orientation::Deg180.is_portrait());
        assert!(Orientation::Deg270.is_portrait());
    }

    #[test]
    fn toggle_is_landscape_portrait() {
        assert_eq!(Orientation::Deg0.toggle_landscape(), Orientation::Deg270);
        assert_eq!(Orientation::Deg270.toggle_landscape(), Orientation::Deg0);
        // 90/180 不是这对别名里的成员, 一律回到横屏
        assert_eq!(Orientation::Deg90.toggle_landscape(), Orientation::Deg0);
        assert_eq!(Orientation::Deg180.toggle_landscape(), Orientation::Deg0);
    }

    /// 非 Windows 构建下三个入口都必须是干净的 Err (不是 panic)。
    /// Windows 构建上这些函数是真的会动显示器的, 所以这个测试只在非 Windows 跑。
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn non_windows_paths_error_out() {
        assert!(current_orientation("\\\\.\\DISPLAY5").is_err());
        assert!(set_orientation("\\\\.\\DISPLAY5", Orientation::Deg90).is_err());
        assert!(cycle_display("\\\\.\\DISPLAY5", Duration::from_millis(1)).is_err());
    }
}
