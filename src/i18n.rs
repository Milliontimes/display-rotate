//! 菜单/报错文案的轻量 i18n: 启动时检测一次系统 UI 语言并缓存,
//! 按稳定 key + 占位符取文案 (zh / en 两份), 回退英文。
//!
//! 借鉴 Motrix 的思路 —— 文案与逻辑分离、稳定 key、`{name}` 占位符插值 ——
//! 但针对单文件 Rust 二进制收窄为一张最小字典, 不做 vue-i18n 那套重型框架。
//!
//! 这里只管**给用户看的**文案: 托盘菜单项 + MessageBoxW 报错。
//! 事件日志走 `main::emit`, 沿用 llama-watch 的中文单语风格。

use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// 目标语言。仅区分中文与非中文 (跟随 Windows UI 语言, 自动选择)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Zh,
    En,
}

/// 一条文案的两语字典项。
#[derive(Clone, Copy)]
struct Msg {
    zh: &'static str,
    en: &'static str,
}

/// 翻译字典: 稳定 key → (中文, 英文)。
/// 菜单四个方向的文案按冻结规格: 第一项带「方向：」分组前缀, 后三项只有角度。
/// 占位符用 `{name}`。
const DICT: &[(&str, Msg)] = &[
    (
        "menu.rot0",
        Msg {
            zh: "方向：横屏 0°",
            en: "Orientation: Landscape 0°",
        },
    ),
    (
        "menu.rot90",
        Msg {
            zh: "90°",
            en: "90°",
        },
    ),
    (
        "menu.rot180",
        Msg {
            zh: "180°",
            en: "180°",
        },
    ),
    (
        "menu.rot270",
        Msg {
            zh: "竖屏 270°",
            en: "Portrait 270°",
        },
    ),
    (
        "menu.cycle",
        Msg {
            zh: "重新初始化显示(拔插)",
            en: "Reinitialize display (unplug/replug)",
        },
    ),
    (
        "menu.quit",
        Msg {
            zh: "退出 (结束 display-rotate)",
            en: "Quit (exit display-rotate)",
        },
    ),
    (
        "msg.rotate_failed",
        Msg {
            zh: "切换 {device} 到 {deg}° 失败：{err}",
            en: "Failed to rotate {device} to {deg}°: {err}",
        },
    ),
    (
        "msg.cycle_failed",
        Msg {
            zh: "重新初始化 {device} 失败：{err}",
            en: "Failed to reinitialize {device}: {err}",
        },
    ),
    (
        "msg.cycle_restore_failed",
        Msg {
            zh: "重新初始化 {device}：已摘除, 但挂回失败（{err}）；已尝试 SDC_TOPOLOGY_EXTEND 兜底",
            en: "Reinitialize {device}: detached, but re-attach failed ({err}); tried the SDC_TOPOLOGY_EXTEND fallback",
        },
    ),
    (
        "msg.status_unknown",
        Msg {
            zh: "读不到 {device} 的当前朝向：{err}",
            en: "Cannot read the current orientation of {device}: {err}",
        },
    ),
];

/// 通过 PowerShell 读系统 UI 语言并缓存; 获取失败回退 `Lang::En`。
/// 无窗口 (CREATE_NO_WINDOW, 不阻塞, 一次性)。
pub fn init_lang() -> Lang {
    *LANG.get_or_init(|| {
        // 必须用 no_window(): 否则启动时 PowerShell 会弹一个 console 窗口闪一下。
        let mut cmd = Command::new(super::POWERSHELL);
        super::no_window(&mut cmd);
        let out = cmd
            .args([
                "-NoProfile",
                "-NoLogo",
                "-Command",
                "[System.Globalization.CultureInfo]::CurrentUICulture.Name",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        match out {
            Ok(o) if o.status.success() => {
                let s = String::from_utf8_lossy(&o.stdout);
                if s.trim().to_ascii_lowercase().starts_with("zh") {
                    Lang::Zh
                } else {
                    Lang::En
                }
            }
            _ => Lang::En,
        }
    })
}

/// 取指定 key 在当前语言下的文案; 未知 key 原样返回 key 本身。
pub fn t(lang: Lang, key: &str, args: &[(&str, String)]) -> String {
    let found = DICT.iter().find(|(k, _)| *k == key).map(|(_, m)| *m);
    let template = match found {
        // 未知 key: 回退英文原文, 退而求其次直接返回 key
        None => return key.to_string(),
        Some(m) => match lang {
            Lang::Zh => m.zh,
            Lang::En => m.en,
        },
    };
    let mut out = template.to_string();
    for (name, val) in args {
        out = out.replace(&format!("{{{name}}}"), val);
    }
    out
}

/// 当前系统语言 (启动时检测并缓存)。
pub fn lang() -> Lang {
    init_lang()
}

static LANG: OnceLock<Lang> = OnceLock::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_zh_en_select() {
        let zh = t(
            Lang::Zh,
            "msg.rotate_failed",
            &[
                ("device", "\\\\.\\DISPLAY5".to_string()),
                ("deg", "90".to_string()),
                ("err", "ChangeDisplaySettingsExW 返回 -2".to_string()),
            ],
        );
        let en = t(
            Lang::En,
            "msg.rotate_failed",
            &[
                ("device", "\\\\.\\DISPLAY5".to_string()),
                ("deg", "90".to_string()),
                ("err", "ChangeDisplaySettingsExW returned -2".to_string()),
            ],
        );
        assert_eq!(
            zh,
            "切换 \\\\.\\DISPLAY5 到 90° 失败：ChangeDisplaySettingsExW 返回 -2"
        );
        assert_eq!(
            en,
            "Failed to rotate \\\\.\\DISPLAY5 to 90°: ChangeDisplaySettingsExW returned -2"
        );
    }

    #[test]
    fn t_unknown_key_falls_back() {
        assert_eq!(t(Lang::Zh, "no.such.key", &[]), "no.such.key");
        assert_eq!(t(Lang::En, "no.such.key", &[]), "no.such.key");
    }

    #[test]
    fn menu_labels_match_frozen_spec() {
        assert_eq!(t(Lang::Zh, "menu.rot0", &[]), "方向：横屏 0°");
        assert_eq!(t(Lang::Zh, "menu.rot90", &[]), "90°");
        assert_eq!(t(Lang::Zh, "menu.rot180", &[]), "180°");
        assert_eq!(t(Lang::Zh, "menu.rot270", &[]), "竖屏 270°");
        assert_eq!(t(Lang::Zh, "menu.cycle", &[]), "重新初始化显示(拔插)");
        assert_eq!(t(Lang::Zh, "menu.quit", &[]), "退出 (结束 display-rotate)");
    }

    #[test]
    fn placeholder_without_args_is_left_alone() {
        // 少传占位符时不 panic, 原样留着 (便于排障时一眼看出漏了哪个)
        assert_eq!(t(Lang::En, "menu.cycle", &[]), "Reinitialize display (unplug/replug)");
        assert_eq!(
            t(Lang::Zh, "msg.cycle_failed", &[("device", "X".to_string())]),
            "重新初始化 X 失败：{err}"
        );
    }
}
