# display-rotate

> **[English](README.md) | [中文](README.zh-CN.md)**

### 用 deepseek harness 写的自用小工具。建议把仓库链接丢给 agent 来部署。好用记得点个 star！

把 **Windows 显示器**旋转到 0° / 90° / 180° / 270° —— 两种用法：一个可被计划任务（或任何其它程序）调用的 **PowerShell 脚本**，或一个**静默托盘程序**（Rust 单 exe），右键菜单切方向、双击在横屏 ↔ 竖屏之间 toggle。

在 Windows 上转屏看起来很简单——一次 `ChangeDisplaySettingsEx` 调用而已——但实际不是。只设 `dmDisplayOrientation` 会失败；厂商控制面板没有任何可脚本化的接口；而且旋转之后绝对定位输入设备可能还停在旧方向。本项目把这三条硬约束的可用解法打包成你真正会用的两种形态：可脚本化的 CLI，和托盘图标。

## 它做什么

| 形态 | 是什么 | 什么时候用 |
|---|---|---|
| `scripts/rotate-display.ps1` | PowerShell 命令行脚本——每次调用切一个方向，失败以退出码体现 | 要给计划任务、热键工具、启动器或其它程序调用 |
| 托盘程序（`display-rotate.exe`） | 单 crate Rust 二进制，静默无窗口，常驻通知区 | 想手动切方向，一天切几次 |

两者底层是同一套操作（GDI `ChangeDisplaySettingsEx` 与 CCD `SetDisplayConfig`），行为一致，只是前端的壳不同。

## 为什么需要它

Windows 显示器旋转有三条反直觉的硬约束，它们就是这个项目存在的全部理由——其余都只是接线。

**1. 只设 `dmDisplayOrientation` 会失败。** 只翻这一个字段，`ChangeDisplaySettingsEx` 返回 `DISP_CHANGE_BADMODE`。必须**同时**把 `dmPelsWidth` / `dmPelsHeight` 换成与目标方向自洽的长短边，并在同一次调用里于 `dmFields` 置上 `DM_DISPLAYORIENTATION | DM_PELSWIDTH | DM_PELSHEIGHT`。这是实测的硬约束，不是可选建议——宽高与方向是被一起校验的。

**2. NVIDIA 控制面板没有接口。** 它是个封闭 GUI（`nvcplui.exe`），没有 CLI / COM / 脚本接口。而"显示器方向"这件事本来也不归 NVIDIA 管：它走 Windows 通用显示驱动模型（GDI 的 `ChangeDisplaySettingsEx` / CCD 的 `SetDisplayConfig`），所有显卡厂商通用。所以"给 NVIDIA 面板写脚本"是问错了问题——该对话的是 Windows。

**3. 旋转完之后，绝对定位输入的坐标映射可能还停在旧方向。** 触摸、笔、以及某些注入式指针设备会继续按旧方向工作，直到显示器重新初始化。物理上"把显示器关掉再开"能修好，软件等价物是 **CCD 层面的真·拔插**：

```
QueryDisplayConfig            → 取全量 path/mode 数组
  → 把目标那条 path 从配置里删掉
    （mode 数组要同步压缩，并重映射 modeInfoIdx）
  → SetDisplayConfig 提交     → 该屏丢信号
  → 等 N 秒                    （-DownSeconds）
  → 用原始配置挂回
```

> ⚠️ 反例，不能当复位手段：`SetDisplayConfig(0, NULL, 0, NULL, SDC_TOPOLOGY_EXTEND | SDC_APPLY)` —— 就是 Win+P「扩展这些显示器」那个控件背后的调用，实现体是 `C:\Windows\System32\DisplaySwitch.exe` —— **在拓扑没变化时返回 0 但什么都不做**，是假成功，无法用它强制重新初始化。所以 `-Reset topology` 只作实验保留，默认走 CCD 拔插。

## 命令行脚本 `scripts/rotate-display.ps1`

需要 Windows 上的 PowerShell —— **推荐 `pwsh.exe`（PowerShell 7）**。Windows PowerShell 5.1 也能完整跑通同一个脚本（两者都实测过）。

### 参数

| 参数 | 取值 | 默认 |
|---|---|---|
| `-Orientation` | `0 \| 90 \| 180 \| 270`；别名 `landscape`（= 0）、`portrait`（= 270） | `0`（横屏） |
| `-Device` | GDI 设备名 | `\\.\DISPLAY5` |
| `-Reset` | `none \| ccdcycle \| topology` | `ccdcycle` |
| `-DownSeconds` | CCD 循环中显示器摘除的时长（秒） | `2` |
| `-ListDevices` | 列出所有活动显示器（`\\.\DISPLAYn` + 当前朝向）后退出 | — |
| `-WhatIf` | 只打印将要做什么，不执行 | — |

`-Reset` 取值含义：

- `none` —— 只做方向切换，不重新初始化。最快，但绝对定位输入可能停在旧方向。
- `ccdcycle`（**默认**）—— 上面讲的 CCD 真·拔插，唯一可靠的软件重初始化。
- `topology` —— `SDC_TOPOLOGY_EXTEND` 那条调用。**已知假成功**（见上），仅供实验。

### 示例

```powershell
# 注意：务必带 -ExecutionPolicy Bypass。脚本在 UNC 路径（\\wsl.localhost\...）上时，
# PowerShell 视其为远程文件，默认 RemoteSigned 策略会在执行前直接拒绝。
# 该参数只作用于本次进程，不改系统设置。

# 列出活动显示器及其当前朝向
pwsh -ExecutionPolicy Bypass -File scripts/rotate-display.ps1 -ListDevices

# 把 DISPLAY5 转成竖屏，走默认的 CCD 重初始化
pwsh -ExecutionPolicy Bypass -File scripts/rotate-display.ps1 -Orientation portrait

# 指定显示器与显式角度、不重初始化、只 dry run
pwsh -ExecutionPolicy Bypass -File scripts/rotate-display.ps1 -Orientation 90 -Device '\\.\DISPLAY3' -Reset none -WhatIf

# 用更长的摘除时间做一次完整重初始化
pwsh -ExecutionPolicy Bypass -File scripts/rotate-display.ps1 -Orientation 270 -DownSeconds 4
```

## 托盘程序

单 crate 单二进制，裸 `windows-sys`，交叉编译成 Windows exe。无窗口静默常驻通知区。

右键菜单：

- **四个方向单选项** —— 0° / 90° / 180° / 270° 四个单选项；勾选哪一项反映的是显示器**当前实际朝向**，不是上次请求的方向
- **重新初始化显示(拔插)** —— 不改方向，只跑一次 CCD 拔插（对应硬约束 3）
- **退出** —— 退出

**双击图标** = 横屏 ↔ 竖屏 toggle（`landscape` = 0°、`portrait` = 270°，与脚本的别名一致）。

### 一次性 CLI 动作

同一个 exe 也能非交互运行 —— 不起托盘、做完即退出。适合计划任务，也适合不想动托盘时的快速测试：

| 动作 | 作用 |
|---|---|
| `--set <0\|90\|180\|270>` | 切到指定方向 |
| `--toggle` | 横屏 0° ↔ 竖屏 270° |
| `--cycle` | 只做 CCD 拔插重初始化（不改方向） |
| `--status` | 打印显示器当前实际朝向 |

| 选项 | 含义 | 默认 |
|---|---|---|
| `--device <\\.\DISPLAYn>` | 目标显示器 | `\\.\DISPLAY5` |
| `--down-seconds <n>` | CCD 拔插时摘除的秒数（下限 1 s） | `2` |
| `--log-dir <path>` | 事件日志目录 | `%LOCALAPPDATA%\display-rotate` |
| `--max-log-mb <n>` | 事件日志滚动总上限 | `5` |
| `--version`、`--help` / `-h` | 版本 / 用法 | — |

```powershell
.\display-rotate.exe --status
.\display-rotate.exe --set 270
.\display-rotate.exe --cycle --down-seconds 3
```

一次性动作的退出码：`0` 成功、`1` 动作失败、`2` 参数错误。
生效优先级：内置默认值 < `config.toml` < 命令行参数。

> 这组一次性动作是 exe **附带的便利功能，不属于脚本那份冻结的参数契约**。凡是脚本化 / 计划任务，优先用 `scripts/rotate-display.ps1` —— 它有明确的退出码、支持 `-WhatIf`，也没有下面这个 console 注意事项。
>
> **Console 注意事项**：exe 链接为 Windows **GUI 子系统**，不会自己创建控制台。**从终端调用（或重定向输出）时** `--status` / `--help` 正常打印；**从资源管理器双击**时它无处可写，此时可靠信道是日志文件 `%LOCALAPPDATA%\display-rotate\display-rotate.log`。


## 配置 `config.toml`

托盘程序从 **exe 同目录**读 `config.toml`。仓库里的示例见 `examples/config.example.toml`。

```toml
[display]
device = '\\.\DISPLAY5'   # GDI 设备名（实践中必填；内置默认即 DISPLAY5）
# down_seconds = 2         # CCD 重初始化的摘除时长（秒）
```

> **路径值请用单引号** —— TOML 字面字符串，反斜杠原样保留。**不要**用双引号：双引号是 TOML 基础字符串，其中的 `\D` / `\d` 可能非法转义或语义不同，设备名会被解析坏。

命令行脚本不读这个文件，改用 `-Device` 传参。两个入口各配一份 `\\.\DISPLAYn` 是刻意的——多显示器时没有任何可靠办法猜出你指的是哪一块。

## 构建

```bash
# Windows 原生 exe（zig 交叉编译，裸 windows-sys，无第三方 crate）
bash build-win.sh
# 产物: target/x86_64-pc-windows-gnu/release/display-rotate.exe
```

依赖：`rustup target add x86_64-pc-windows-gnu` + [zig](https://ziglang.org/download/)（`build-win.sh` 里的 `ZIG` 变量指向 zig 可执行文件）。

## 部署

```bash
cp target/x86_64-pc-windows-gnu/release/display-rotate.exe /mnt/c/AI/display-rotate/display-rotate.exe
cp examples/config.example.toml /mnt/c/AI/display-rotate/config.toml   # 然后改 `device`
```

- 脚本除 `pwsh.exe` 外不需要任何东西；把 `scripts/rotate-display.ps1` 拷到任意位置按路径调用即可。
- 托盘程序要把 `config.toml` 放在 **exe 同目录**。
- **自启（可选）**：把 exe 放进「启动」文件夹（`Win+R` → `shell:startup`），或建一个登录时运行的任务计划程序条目。因为程序无窗口，任务设成「隐藏」和启动文件夹快捷方式在观感上没有区别。

## 已知坑（均为实测）

- **`EnumDisplaySettings` 不回填 `dmDeviceName`** —— 读出来是垃圾值。要判断一块屏是物理还是虚拟，得看 CCD 的 `outputTechnology`：`0xA` = DisplayPort external、`0x80000000` = Internal、`0x10` / `0x11` = IndirectWired / Virtual。
- **`DEVMODE.dmSize` 必须与你实际封送的结构体匹配**：`DEVMODEA`（ANSI）= **156**，`DEVMODEW`（Unicode）= **220** —— 差的 64 字节是 `dmDeviceName` / `dmFormName` 从 `CHAR[32]` 变宽成 `WCHAR[32]`。脚本走 `DEVMODEA`/156，Rust 托盘走 `EnumDisplaySettingsW`/`DEVMODEW`/220。**尺寸给错不会报错**，只会让 `dmPelsWidth`/`dmPelsHeight` 不被回填 —— 正是旋转硬约束依赖的那两个字段。
- **CCD 与 GDI 是两套不同的旋转枚举。** CCD 的 `rotation` 是 `1/2/3/4` = IDENTITY / ROT90 / ROT180 / ROT270，而 GDI 的 `dmDisplayOrientation` 是 `0/1/2/3` = 0° / 90° / 180° / 270°。**别把一套的值直接塞进另一套。**
- **拓扑标志不能与 `SDC_SAVE_TO_DATABASE` / `SDC_ALLOW_CHANGES` 同用** —— 返回 87（`ERROR_INVALID_PARAMETER`）。
- 🔴 **不要用 `SC_MONITORPOWER`**（`WM_SYSCOMMAND` `0xF170`）做"关屏"：它会让显示器进待机，并可能**连带把整机弄睡眠**。实测踩过，这个项目里严禁使用。
- **在 WSL 里调宿主机 API**：`pwsh` 是 Windows 进程，**不认 `/home/...` 路径**。传脚本要用 UNC —— `\\wsl.localhost\Ubuntu-26.04\...`。
- **UNC 路径必须带 `-ExecutionPolicy Bypass`**：UNC 上的 `.ps1` 被当作远程文件，默认 `RemoteSigned` 策略会在脚本执行前抛 `SecurityError`。这是「照抄 README 示例却跑不起来」的头号原因。
- **`Write-Host "..." -f $a,$b` 在 PowerShell 里是 bug** —— 它会把 `-f` 解析成 `-ForegroundColor` 而报错。必须写成 `Write-Host ("..." -f $a,$b)`。

## 边界（诚实说明）

- **只在 Windows 上有效。** 整套机制是 Win32/CCD，没有 Linux / macOS 路径。
- **目标显示器不能被独占**（全屏独占应用、部分游戏、远程桌面会话）。被独占时切换模式会失败——用户态下没有绕过的办法。
- **一次只能指定一块屏。** 多显示器时必须显式点名：脚本用 `-Device '\\.\DISPLAYn'`，托盘程序用 `config.toml` 的 `[display] device`。没有"全部旋转"模式。
- **设备名不跨重启/驱动更新/换线稳定。** 找不到屏时重新跑 `-ListDevices`，再改 `config.toml` / `-Device` 指过去。
- **本项目不含安装器，也不含自动更新。** 部署方式就是"拷贝 exe"。

## 待办

按大致优先级排列。**以下都还没实现**，欢迎提 issue 或 PR。

- **显示器枚举与选择。** 现在目标显示器必须手写：`config.toml` 的 `device`，或脚本的 `-Device`，两者都要填原始 GDI 名（如 `\\.\DISPLAY5`）。脚本已经能列出接了哪些屏（`rotate-display.ps1 -ListDevices`），但你仍得自己读表、再把名字敲回去。希望做到：
  - 托盘程序启动时枚举活动显示器（并在显示拓扑变化时刷新），**每块屏一个子菜单**，各自带方向单选项和勾选状态；
  - 脚本除了原始设备名，还能接受更友好的定位方式 —— 序号、监视器友好名，或 `primary`；
  - 选中的显示器被记住，而不是写死在配置里。
- **让托盘图标在浅色任务栏下可读。** 当前图标是白色线稿，浅色任务栏上会消失。方案二选一：出亮/暗两套按系统主题切换，或换成带颜色的图标。
- **把托盘那条路径的实机验收补完。** 一次性 CLI（`--set`）已在真机验收；托盘本身还没有 —— 图标在 28px（175% DPI）与 16px（100% DPI）下的实际观感、菜单勾选是否跟随**实际**朝向、双击 toggle、以及 `重新初始化显示(拔插)`。
- **验证这个项目存在的理由。** 绝对定位输入（触摸 / 笔 / 注入式指针）在 CCD 拔插复位后是否真的跟随旋转，目前仍未在真机确认。
- **可选：记住每块屏的朝向**，这样旋转过的显示器拔插后能回到你离开时的样子。

## 相关项目

与 [llama-watch](https://github.com/Milliontimes/llama-watch)、[idm-watch](https://github.com/Milliontimes/idm-watch) 同栈（零第三方依赖、zig 交叉编译、静默 Windows 后台工具）。

## License

MIT —— Copyright (c) 2026 Milliontimes，详见 [LICENSE](LICENSE)。
