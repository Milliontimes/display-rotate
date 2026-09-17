# display-rotate —— 项目说明与待办

> 建档 2026-09-17。源码仓库在 WSL `~/WorkSpace/display-rotate`，**部署产物在 Windows 侧 `C:\AI\display-rotate\`**。
> 项目本身的介绍看 `README.md` / `README.zh-CN.md`。本文件只放「跨会话要记住的机器事实」和**未完成待办**。

## 一句话

把 Windows 显示器转到 0°/90°/180°/270°：一个可被计划任务调用的 `scripts/rotate-display.ps1`，加一个 Rust 单 exe 托盘程序（右键切方向、双击横竖屏 toggle）。

## 构建与部署链

- **形态**：托盘程序是 Rust 单 crate 单二进制，裸 `windows-sys`（无第三方 crate），交叉编译成 Windows exe（`x86_64-pc-windows-gnu` + zig 链接器）。脚本形态无需构建。
- **前置**：`rustup target add x86_64-pc-windows-gnu`；zig 在 `~/apps/zig-x86_64-linux-0.14.1`（同机 llama-watch 沿用此路径，`build-win.sh` 里 `ZIG` 变量可覆盖）。
- **产物**：`target/x86_64-pc-windows-gnu/release/display-rotate.exe`。
- **构建命令**：`bash build-win.sh`。
- **脚本运行环境**：Windows 上的 PowerShell。推荐 `pwsh.exe`（7.x），**Windows PowerShell 5.1 实测也能完整跑通**（同一份脚本，parse + `-ListDevices` 输出逐字相同），语法按 5.1 写以留余量。
- ⚠️ **从 WSL 调用必须加 `-ExecutionPolicy Bypass`**：UNC 路径上的 `.ps1` 被当作远程文件，默认 `RemoteSigned` 会在执行前抛 `SecurityError`。
- **运行时配置**：exe 同目录的 `config.toml`（**不随仓库提交**，`.gitignore` 只放行 `examples/config.example.toml`）
  - `[display] device = '\\.\DISPLAY5'`
  - `[display] down_seconds = 2`（可选）
- **脚本侧**：不读 `config.toml`，靠 `-Device '\\.\DISPLAYn'` 指定目标屏。
- **git**：分支 `main`，**当前还没有任何 commit，也没有配置 remote**（见 T2）。
- 文档冻结规格（参数表、菜单项、三条硬约束）在 `README.md` / `README.zh-CN.md`，是**唯一事实源**；改实现前先对齐文档。

## 已知坑

- **只设 `dmDisplayOrientation` 会返回 `DISP_CHANGE_BADMODE`。** 必须同时改 `dmPelsWidth`/`dmPelsHeight` 为与方向自洽的长短边，并置 `DM_DISPLAYORIENTATION|DM_PELSWIDTH|DM_PELSHEIGHT`。
- **`EnumDisplaySettings` 不回填 `dmDeviceName`**（读出来是垃圾值）。判断物理屏/虚拟屏要看 CCD 的 `outputTechnology`：`0xA`=DisplayPort external、`0x80000000`=Internal、`0x10/0x11`=IndirectWired/Virtual。
- **`DEVMODE.dmSize` 必须与你实际封送的结构体匹配**：`DEVMODEA`(ANSI) = **156**，`DEVMODEW`(Unicode) = **220**（差 64 = `dmDeviceName`/`dmFormName` 各由 `CHAR[32]` 变宽为 `WCHAR[32]`）。脚本走 Ansi/156，Rust 托盘走 W/220。**给错尺寸不报错**，只会让 `dmPelsWidth`/`dmPelsHeight` 不被回填 —— 正是旋转硬约束依赖的那两个字段。（2026-09-17 修正：原文只写了 156，对 W API 是错的。）
- **CCD `rotation`（`1/2/3/4`）与 GDI `dmDisplayOrientation`（`0/1/2/3`）是两套不同的枚举值**，别混用。
- **拓扑标志不能与 `SDC_SAVE_TO_DATABASE` / `SDC_ALLOW_CHANGES` 同用**，返回 87（`ERROR_INVALID_PARAMETER`）。
- **`SDC_TOPOLOGY_EXTEND|SDC_APPLY`（Win+P 扩展，实现在 `C:\Windows\System32\DisplaySwitch.exe`）在拓扑没变时返回 0 但什么都不做**，是假成功，不能当复位手段 ⇒ 默认复位走 CCD 拔插（`-Reset ccdcycle`）。
- 🔴 **严禁 `SC_MONITORPOWER`（`WM_SYSCOMMAND` 0xF170）**：会让显示器待机并可能连带把整机弄睡眠，实测踩过。
- 🔴 **交叉编译 rustc≥1.97 + `windows-sys 0.52` 必须改 zig-cc 包装脚本**（2026-09-17 实测）：rustc 会把 `-lwindows.0.52.0` 放在 `-Wl,-Bdynamic` **之后**，而 `windows-targets` 只提供静态归档 `libwindows.0.52.0.a` ⇒ zig/lld-link 去找不存在的 `windows.0.52.0.dll`，报
  `error: unable to find dynamic system library 'windows.0.52.0' using strategy 'no_fallback'`。
  修法：包装脚本里**剔除 `-Wl,-Bdynamic`**。试过 GNU 精确写法 `-l:libwindows.0.52.0.a`，**不行** —— COFF 的 lld-link 不认 `-l:`（报 `could not open ':libwindows...'`）。
  另外 zig 默认缓存 `~/.cache/zig` 在只读 HOME 下报 `unable to create compilation: ReadOnlyFileSystem`，需把 `ZIG_LOCAL_CACHE_DIR`/`ZIG_GLOBAL_CACHE_DIR` 指向仓内。
  ⚠️ **同机的 `llama-watch` 现在就是这个状态、编不过**（`~/.local/bin/zig-cc` 是没打这两个补丁的旧版），本项目 `build-win.sh` 里已是修好的版本。
- 🔴 **`\\wsl.localhost\` 会缓存可执行文件**（2026-09-18 实测，踩了很久）：在同一个 UNC 路径上重建 exe 后，从 Windows 侧按原路径执行**可能仍跑旧字节**——表现为新加的 `eprintln!` 死活不打印、行为与旧版完全一致。**验收/排障时必须每次把 exe 复制成唯一文件名再执行**，否则你会对着已经修好的代码怀疑人生。
- 🔴 **`set_orientation` 必须真正写回 `dmDisplayOrientation`**（2026-09-18 修，本项目的头号 bug）：只置 `dmFields` 掩码（`DM_DISPLAYORIENTATION|DM_PELSWIDTH|DM_PELSHEIGHT`）而不写值，API 收到的仍是**旧方向 + 新宽高**，于是
  - 宽高需要变化时（0↔90/270）自相矛盾 ⇒ `DISP_CHANGE_BADMODE(-2)`
  - 宽高恰好不变时（0↔180、或已在竖屏时切 90/270）整条请求退化成 **no-op：返回 0 却什么都没做**（曾出现"→ 90° 已提交"但 `--status` 仍报 270°）
  ⇒ 排查这类问题的正确姿势是**在提交前把待提交字段全量打出来**（本仓库已保留该行 `[rotate] submit ...`）：弹窗里那组数字是本地算出来的，不代表 API 收到了什么。
- **NVIDIA 控制面板无接口**：`nvcplui.exe` 是封闭 GUI，无 CLI/COM；显示器方向本来走 Windows 通用显示驱动模型（GDI/CCD），与显卡厂商无关。
- **在 WSL 里调宿主机 API**：`pwsh` 是 Windows 进程，**不认 `/home/...` 路径**，传脚本要用 UNC（`\\wsl.localhost\Ubuntu-26.04\...`）。
- **`Write-Host "..." -f $a,$b` 会被解析成 `-ForegroundColor` 而报错**，必须写成 `Write-Host ("..." -f $a,$b)`。
- ⚠️ **托盘图标尺寸不是固定的 16px，随所在显示器的 DPI 变**（2026-09-17 实测，推翻了我最初的假设）：
  ```
  GetDpiForSystem = 168 (175%)     SM_CXSMICON = 28   SM_CXICON = 56
  \\.\DISPLAY1  175% dpi=168  → 托盘小图标 28px
  \\.\DISPLAY5  100% dpi=96   → 托盘小图标 16px
  ```
  ⇒ `.ico` 必须带**多尺寸帧**，否则 Windows 只能拿现有帧硬缩。`assets/display-rotate.ico` 现为 9 帧：
  **原生 16/32/48/256**（从系统 DLL 提取）+ **20/24/28/40/64**（从 256 LANCZOS 重采样），共 15133 字节。
- 🔴 **`llama-watch/src/tray.rs` 的 `create_icon_from_ico_bytes` 只取 ICO 目录第 0 个条目**（`&bytes[6..22]`），多尺寸 ico 在它眼里只有一个尺寸。照抄时必须改成遍历 `ICONDIR.count` 个条目、按 `SM_CXSMICON`/`SM_CYSMICON` 选最匹配帧，再把 `cxDesired`/`cyDesired` 传给 `CreateIconFromResourceEx`。
- ⚠️ **图标 `imageres.dll#324` 是白色线稿**（屏幕轮廓+旋转箭头）：深色面板下清晰（用户实际环境就是深色），**浅色任务栏下会完全隐形**。这是用户明示保留的选择。要换就换候选（候选集在 `tmp/icon-candidates/ico/`，实测明暗底都清楚的是 `display_dll_0.ico`），换完记得重做多尺寸帧再 `bash build-win.sh`：
  ```bash
  # 候选 ico 是单帧来源, 需按上面 9 帧规则重新装帧后再替换
  cp tmp/icon-candidates/ico/display_dll_0.ico display-rotate/assets/display-rotate.ico
  ```

## 待办

### T1 — 对齐实现与冻结规格

- **背景**：`src/`、`scripts/`、`build-win.sh` 由其它 agent 并行编写；`README` / `examples/config.example.toml` 已按用户给定的冻结规格写好，是唯一事实源。
- **状态：已完成（2026-09-17）**。逐项核对结果：脚本参数名/取值/默认值全部与规格一致（`-Orientation 0|90|180|270`+`landscape`/`portrait` 别名、默认 `0`/`\\.\DISPLAY5`/`ccdcycle`/`2`、`-ListDevices`/`-WhatIf`/`-?`）；托盘菜单 = 四个方向单选项 + 重新初始化(拔插) + 退出，双击 = 0°↔270° toggle；`config.toml` 只认 `[display] device` / `down_seconds`；产物路径一致。
- **过程中修掉的 3 处实现/文档错误**：① 脚本 `outputTechnology` 映射整体错位一格（官方枚举 7 是跳过的）→ 按 `wingdi.h` 重写；② `dmSize` 只写了 156（对 `DEVMODEW` 错）→ 改为 A/156、W/220；③ README Deploy 的 `/mnt/c/Tools/` 与 T4 结论不一致 → 统一为 `/mnt/c/AI/display-rotate/`。

### T2 — 建 remote 并首次提交

- **背景**：`git branch` 显示 `main` 尚无任何提交；`git remote -v` 为空；全部文件仍未被跟踪。
- **要做**：确认远端地址后 `git remote add` + 首次 commit/push（凭据见 skill `dsh-git`）。远端目标 = `https://github.com/Milliontimes/display-rotate.git`（`~/apps/gh/gh` 已登录 `Milliontimes`，scopes 含 `repo`）。
- **注意**：`.gitignore` 忽略 `/config.toml`、`/target`、`/.zig-cache`、`*.exe`、`*.pdb`，只放行 `examples/config.example.toml`。**`Cargo.lock` 故意入库**（二进制 crate，锁死 `windows-sys 0.52.0`）。

### T3 — 交叉编译验证（编译部分已完成，运行部分未做）

- **编译：已完成（2026-09-17）**。`rm -rf target/x86_64-pc-windows-gnu && bash build-win.sh` 全新编译通过，产出
  `target/x86_64-pc-windows-gnu/release/display-rotate.exe`（**436736 B**，sha256 `495e9275b06e07f81187a6ed422b2fd5802a5c1573b78932559651da1a2a1e9d`）。
  独立校验：`PE32+ / x86-64 / Subsystem=2 (WINDOWS_GUI)`；导入表含 `ChangeDisplaySettingsExW`/`SetDisplayConfig`/`QueryDisplayConfig`/`Shell_NotifyIconW`/`CreateIconFromResourceEx`/`TrackPopupMenu`/`CheckMenuRadioItem`；**`SendMessageW`/`SystemParametersInfoW`/`SetThreadExecutionState`/`PostMessageW` 导入数为 0**（确认没走 `SC_MONITORPOWER`）。`cargo test` = 21/21。
- **运行（只读部分）：已做（2026-09-17）**。用 pwsh 从有 console 的父进程调 exe，跑了 `--version` / `--help` / `--status`（**只读动作，没碰 `--set`/`--toggle`/`--cycle`**）：`--version` → `display-rotate v0.1.0`；`--status` → `\\.\DISPLAY5 = 0°`，**读回的正是当前实际朝向**。⇒ zig 链接出的 PE 能加载、能跑、能正确读显示器状态。
  ⚠️ 顺带推翻一个说法：**GUI 子系统下 stdout 并非必然丢失** —— 从终端（或重定向输出）调用时 `--status`/`--help` 正常打印，只有从资源管理器双击才无处可写。README 已按这个实际情况写。
- **运行（会改屏幕的部分）：CLI 四向已实机验收（2026-09-18）**。修掉"没写回 `dmDisplayOrientation`"后，`--set 90/180/270/0` 逐个跑通，**每步都用独立的 `EnumDisplaySettingsA` 探针读回**（不依赖 app 自报）：
  `90°→orient=1 1080x1920` / `180°→orient=2 1920x1080` / `270°→orient=3 1080x1920` / `0°→orient=0 1920x1080`，全 exit=0。
- **仍未验收（托盘交互）**：托盘图标的实际观感（175%/28px 与 100%/16px）、菜单勾选是否随实际朝向刷新、双击 toggle、`重新初始化显示(拔插)` 的返回码与黑屏时长、以及**绝对定位输入是否真的跟着恢复**（这才是项目的立项目的）。原口径：托盘图标在 175%(28px)/100%(16px) 下的清晰度、四个方向单选项的勾选是否反映实际朝向、双击 0°↔270° toggle、`重新初始化显示(拔插)` 的实际返回码与显示器是否会短暂黑一下、以及**绝对定位输入是否真的跟着恢复**（这才是这个项目的立项目的）。
- **建议首次验收用脚本而非 exe**（脚本有退出码、可 `-WhatIf`，且不依赖托盘）：
  ```bash
  pwsh -ExecutionPolicy Bypass -File <repo>/scripts/rotate-display.ps1 -ListDevices
  pwsh -ExecutionPolicy Bypass -File <repo>/scripts/rotate-display.ps1 -Orientation 270     # 观察指针是否跟随
  pwsh -ExecutionPolicy Bypass -File <repo>/scripts/rotate-display.ps1 -Orientation 0
  ```
- **`.cargo/config.toml` 是 `build-win.sh` 生成的文件**，内容取决于构建机上 `~/.local/bin` 是否可写：可写 → 装到共享的 `~/.local/bin/zig-cc`（同时**修好同机 llama-watch 那条链**）；不可写（沙箱/CI）→ 退回仓内 `.zig-cache/zig-cc`。两种情况下都**必须先跑一次 `build-win.sh`**，否则 `cargo build --target x86_64-pc-windows-gnu` 会因为 linker 路径不存在而失败。

### T4 — ~~定部署位置~~（已定，2026-09-17）

- **结论**：部署目录 = **`C:\AI\display-rotate\`**（WSL 侧 `/mnt/c/AI/display-rotate/`），与同机 `llama-watch` 并列，沿用现有习惯。用户已拍板。
- **落点**：`display-rotate.exe` + 手写的 `config.toml`（`device` 指到实际那块屏）。
- **剩余**：实现落地后确认 `build-win.sh` 末尾的部署提示与 `README.md` / `README.zh-CN.md` 的 Deploy 节都写的是这个路径。

## 变更记录

| 日期 | 变更 | 说明 |
|---|---|---|
| 2026-09-17 | 建档并新增 `README.md` / `README.zh-CN.md` / `examples/config.example.toml` / 本文件 | 按 llama-watch 的文档风格；内容以用户给定的冻结规格为准 |
| 2026-09-17 | 交付 `scripts/rotate-display.ps1`（570 行）+ Rust 托盘（`src/` 4 文件 2182 行）+ `build-win.sh` | 并行 subagent 编写；已全新交叉编译通过、`cargo test` 21/21 |
| 2026-09-17 | 修 `outputTechnology` 映射错位（枚举 7 被跳过） | 脚本 `-ListDevices` 原把 `0xA` 印成 `DISPLAYPORT_EMB`；已按 `wingdi.h` 重写并复验 |
| 2026-09-17 | 修 `dmSize` 表述：A=156 / W=220 | 原文只写 156，对 `DEVMODEW` 是错的；Rust 侧走 W/220 |
| 2026-09-17 | 统一部署路径为 `/mnt/c/AI/display-rotate/` | README 两处原写 `/mnt/c/Tools/`，与 T4 结论冲突 |
| 2026-09-17 | 补 `-ExecutionPolicy Bypass` 到 README 示例 + 已知坑 | UNC 上的 `.ps1` 被当远程文件，默认策略会拒跑 |
| 2026-09-17 | `.gitignore` 放行 `Cargo.lock` | 二进制 crate 应锁版本；唯一依赖 `windows-sys 0.52.0` |
| 2026-09-17 | `.cargo/config.toml` 改指 `~/.local/bin/zig-cc` | 原值指向被 gitignore 的 `.zig-cache/`，发布后会失效；文件本身由 `build-win.sh` 生成 |
| 2026-09-17 | 补 README 的「一次性 CLI 动作」小节（EN/ZH 对称） | 按 `main.rs` 的 `usage()` 原文写：`--set/--toggle/--cycle/--status` + `--device/--down-seconds/--log-dir/--max-log-mb`、退出码 0/1/2、优先级链 |
| 2026-09-18 | 🔴 修 `set_orientation` 未写回 `dmDisplayOrientation` | 头号 bug：宽高变则 -2，宽高不变则静默 no-op；四向已实机验收 |
| 2026-09-18 | 记录 `\\wsl.localhost\` 缓存 exe 的环境坑 | 同名重建后执行仍跑旧字节，验收必须换唯一文件名 |
| 2026-09-17 | 只读运行 exe 验证通过 | `--version`/`--help`/`--status` 均正常，`--status` 读回 `\\.\DISPLAY5 = 0°` |
