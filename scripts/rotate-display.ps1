<#
  rotate-display.ps1 —— 旋转 Windows 显示器方向，并按需做一次「复位」

  ── 用途 ────────────────────────────────────────────────────────────────
    经 user32.dll 调用 GDI（EnumDisplaySettings / ChangeDisplaySettingsEx）改
    指定显示器的 dmDisplayOrientation（0/90/180/270），再按 -Reset 做一次复位。
    场景：DP 外接屏切竖屏；或方向对了但鼠标指针映射错乱时复位一遍。
    CCD（Connecting and Configuring Displays）部分用于「真·拔插」复位与设备枚举。

  ── 参数 ────────────────────────────────────────────────────────────────
    -Orientation  0 | 90 | 180 | 270（角度）；别名 landscape(=0) / portrait(=270)
                  默认 0（横屏）
    -Device       GDI 设备名，默认 \\.\DISPLAY5
    -Reset        none | ccdcycle | topology，默认 ccdcycle
    -DownSeconds  仅 ccdcycle 用：摘除时长（秒），默认 2
    -ListDevices  只列出所有活动显示器（GDI 名 / 监视器名 / 分辨率 / 方向 / 位置）后退出
    -WhatIf       只打印将要做的改动，不调用任何 ChangeDisplaySettingsEx / SetDisplayConfig
    -? | -Help    打印用法

  ── 实测结论（真机上踩坑换来的，改脚本前先读完）────────────────────────
    1. 只设 DM_DISPLAYORIENTATION 会返回 DISP_CHANGE_BADMODE(-2)。
       必须同时把 dmPelsWidth/dmPelsHeight 换成与目标方向自洽的长短边，并置
       dmFields |= DM_DISPLAYORIENTATION|DM_PELSWIDTH|DM_PELSHEIGHT。实测验证过。
    2. DEVMODE.dmSize 必须 = 156（x64 ANSI）。
    3. EnumDisplaySettings 不回填 dmDeviceName（读出来是垃圾值），别信它去判断设备
       类型；判断物理屏/虚拟屏要用 CCD 的 outputTechnology。
    4. SetDisplayConfig(0,NULL,0,NULL, SDC_TOPOLOGY_EXTEND|SDC_APPLY) 是「假成功」：
       返回 0，但拓扑本来就没变化时它什么都不做，不能用来复位输入映射。
       别把它当默认复位手段（本脚本只在 ccdcycle 挂回失败时拿它兜底）。
    5. 真正有效的复位是 ccdcycle（真·拔插）：
         QueryDisplayConfig 取全量 path/mode
           → 把目标 path 删掉提交（mode 数组必须同步压缩并重映射 modeInfoIdx，
             SDC_USE_SUPPLIED_DISPLAY_CONFIG 不接受悬空 mode 引用）
           → 停 N 秒
           → 用原始数组原样挂回。
       只影响目标屏，旋转/分辨率/位置全保留。这是「物理关掉再开显示器」的软件等价物。
    6. 提交前先跑一遍 SDC_VALIDATE 空校验；挂回失败时用
       SDC_TOPOLOGY_EXTEND|SDC_APPLY 兜底。
    7. SDC_USE_SUPPLIED_DISPLAY_CONFIG|SDC_APPLY|SDC_ALLOW_CHANGES 是拔插的合法组合；
       拓扑标志不能与 SDC_SAVE_TO_DATABASE / SDC_ALLOW_CHANGES 同用
       （返回 87 = ERROR_INVALID_PARAMETER）。
    8. 🔴 绝对不要用 SC_MONITORPOWER（WM_SYSCOMMAND 0xF170）做「关屏」：会让显示器
       进待机并可能连带把整机弄睡眠。实测踩过。

  ── 兼容性 / 坑 ─────────────────────────────────────────────────────────
    · PowerShell 7（项目目标运行时）与 Windows PowerShell 5.1 都实测可跑
    · ⚠ 本文件必须存为 UTF-8 **with BOM**：5.1 与 Parser.ParseFile 在没有 BOM 时按
      ANSI 读，中文会被解成乱码并直接语法报错（实测）。编辑后确认 BOM 还在
    · 无外部依赖，只 Add-Type 内联 C#（C# 5 语法，兼容 5.1 的 CodeDom 编译器）
    · ⚠ Write-Host 不能写成 `Write-Host "..." -f $a,$b`：那会被当成 -ForegroundColor
      参数而报错，必须写成 `Write-Host ("..." -f $a,$b)`（踩过）
    · ⚠ 续行时二元运算符必须留在**上一行行尾**：`('a'` 换行 `+ 'b')` 这种行首 `+`
      是语法错误 —— 未闭合的 `(` 并不会让下一行行首的运算符生效（实测）
    · ⚠ 不要用 Format-Table 输出这里的表：stdout 被重定向（计划任务 / WSL 管道）时
      它按宿主宽度静默砍掉尾部列（实测 120 列下丢列），所以 -ListDevices 的表由脚本
      自己按显示宽度渲染
    · -WhatIf 是本脚本自己的 switch（脚本未用 CmdletBinding，不是通用参数），
      它不执行任何更改；-ListDevices / -WhatIf 都可安全地在正在使用的机器上跑
    · 脚本只按 -Device 指定的那一条输出动手；ccdcycle 也只摘除目标 path

  ── 从 WSL 调用（重要）──────────────────────────────────────────────────
    pwsh 是 Windows 进程，不认 /home/... 路径，必须传 UNC：
      PW='/mnt/c/Program Files/PowerShell/7/pwsh.exe'
      "$PW" -NoProfile -ExecutionPolicy Bypass -File \
        "$(wslpath -w ~/WorkSpace/display-rotate/scripts/rotate-display.ps1)" -ListDevices
    ⚠ UNC 路径会被当成「远程」脚本：执行策略为 RemoteSigned（本机默认）时会直接拒跑，
      所以调用侧要带 -ExecutionPolicy Bypass（进程级，不改系统设置）。

  ── 用法 ────────────────────────────────────────────────────────────────
    pwsh -File rotate-display.ps1 -ListDevices
    pwsh -File rotate-display.ps1 -Orientation portrait -WhatIf      # 只看计划，不改
    pwsh -File rotate-display.ps1 -Orientation portrait              # 竖屏 + ccdcycle 复位
    pwsh -File rotate-display.ps1 -Orientation 0 -Reset none         # 回横屏，不复位
    pwsh -File rotate-display.ps1 -Device '\\.\DISPLAY5' -Reset topology
#>
param(
  [ValidateSet('0', '90', '180', '270', 'landscape', 'portrait')]
  [string]$Orientation = '0',

  [string]$Device = '\\.\DISPLAY5',

  [ValidateSet('none', 'ccdcycle', 'topology')]
  [string]$Reset = 'ccdcycle',

  [ValidateRange(0, 300)]
  [int]$DownSeconds = 2,

  [switch]$ListDevices,

  [switch]$WhatIf,

  [Alias('?')]
  [switch]$Help
)
$ErrorActionPreference = 'Stop'

# ══════════════════════════════════════════════════════════════════════════
# 内联 C#：GDI / CCD 的 P/Invoke
# （DEVMODE 布局、CCD 拔插逻辑与常量均与真机验证过的原脚本一致）
# ══════════════════════════════════════════════════════════════════════════
if (-not ('RD' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public class RD {
  // ── GDI ────────────────────────────────────────────────────────────────
  // x64 ANSI DEVMODE：Marshal.SizeOf == 156，dmSize 必须填 156
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Ansi)]
  public struct DEVMODE {
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmDeviceName;
    public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra;
    public int dmFields, dmPositionX, dmPositionY, dmDisplayOrientation, dmDisplayFixedOutput;
    public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmFormName;
    public short dmLogPixels;
    public int dmBitsPerPel, dmPelsWidth, dmPelsHeight, dmDisplayFlags, dmDisplayFrequency;
    public int dmICMMethod, dmICMIntent, dmMediaType, dmDitherType, dmReserved1, dmReserved2, dmPanningWidth, dmPanningHeight;
  }

  // ── CCD ────────────────────────────────────────────────────────────────
  [StructLayout(LayoutKind.Sequential)] public struct LUID { public uint LowPart; public int HighPart; }
  [StructLayout(LayoutKind.Sequential)] public struct RATIONAL { public uint Numerator; public uint Denominator; }
  [StructLayout(LayoutKind.Sequential)] public struct PATH_SOURCE_INFO { public LUID adapterId; public uint id; public uint modeInfoIdx; public uint statusFlags; }
  [StructLayout(LayoutKind.Sequential)] public struct PATH_TARGET_INFO {
    public LUID adapterId; public uint id; public uint modeInfoIdx;
    public uint outputTechnology; public uint rotation; public uint scaling;
    public RATIONAL refreshRate; public uint scanLineOrdering; public int targetAvailable; public uint statusFlags; }
  [StructLayout(LayoutKind.Sequential)] public struct PATH_INFO { public PATH_SOURCE_INFO sourceInfo; public PATH_TARGET_INFO targetInfo; public uint flags; }
  [StructLayout(LayoutKind.Sequential)] public struct REGION2D { public uint cx; public uint cy; }
  [StructLayout(LayoutKind.Sequential)] public struct POINTL { public int x; public int y; }
  [StructLayout(LayoutKind.Sequential)] public struct VIDEO_SIGNAL_INFO {
    public ulong pixelRate; public RATIONAL hSyncFreq; public RATIONAL vSyncFreq;
    public REGION2D activeSize; public REGION2D totalSize; public uint videoStandard; public uint scanLineOrdering; }
  [StructLayout(LayoutKind.Sequential)] public struct TARGET_MODE { public VIDEO_SIGNAL_INFO vsi; }
  [StructLayout(LayoutKind.Sequential)] public struct MODE_INFO { public uint infoType; public uint id; public LUID adapterId; public TARGET_MODE targetMode; }

  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct DEVICE_INFO_HEADER { public uint type; public uint size; public LUID adapterId; public uint id; }
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct SOURCE_DEVICE_NAME {
    public DEVICE_INFO_HEADER header; [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string viewGdiDeviceName; }
  // DISPLAYCONFIG_TARGET_DEVICE_NAME（header.type = 2 = GET_TARGET_NAME）
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct TARGET_DEVICE_NAME {
    public DEVICE_INFO_HEADER header;
    public uint flags;
    public uint outputTechnology;
    public ushort edidManufactureId;
    public ushort edidProductCodeId;
    public uint connectorInstance;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=64)] public string monitorFriendlyDeviceName;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=128)] public string monitorDevicePath; }

  [DllImport("user32.dll")] public static extern int DisplayConfigGetDeviceInfo(ref SOURCE_DEVICE_NAME r);
  [DllImport("user32.dll")] public static extern int DisplayConfigGetDeviceInfo(ref TARGET_DEVICE_NAME r);
  [DllImport("user32.dll", CharSet=CharSet.Ansi)] public static extern bool EnumDisplaySettings(string d, int m, ref DEVMODE dm);
  [DllImport("user32.dll", CharSet=CharSet.Ansi)] public static extern int ChangeDisplaySettingsEx(string d, ref DEVMODE dm, IntPtr h, int f, IntPtr p);
  [DllImport("user32.dll")] public static extern int GetDisplayConfigBufferSizes(uint f, out uint np, out uint nm);
  [DllImport("user32.dll")] public static extern int QueryDisplayConfig(uint f, ref uint np, [Out] PATH_INFO[] p, ref uint nm, [Out] MODE_INFO[] m, IntPtr t);
  [DllImport("user32.dll")] public static extern int SetDisplayConfig(uint np, IntPtr p, uint nm, IntPtr m, uint f);
  [DllImport("user32.dll")] public static extern int SetDisplayConfig(uint np, [In] PATH_INFO[] p, uint nm, [In] MODE_INFO[] m, uint f);

  public const int DM_ORIENT = 0x80, DM_W = 0x80000, DM_H = 0x100000;
  public const int CDS_UPDATEREGISTRY = 0x1;
  public const uint QDC_ONLY_ACTIVE = 0x2;
  public const uint TOPO_INTERNAL = 0x1, TOPO_CLONE = 0x2, TOPO_EXTEND = 0x4, TOPO_EXTERNAL = 0x8;
  public const uint SDC_USE_SUPPLIED = 0x20, SDC_VALIDATE = 0x40, SDC_APPLY = 0x80, SDC_SAVE = 0x200;

  // 拓扑复位：注意只带 SDC_APPLY，不能加 SDC_SAVE_TO_DATABASE / SDC_ALLOW_CHANGES（会返回 87）
  public static int ApplyTopology(uint topo) { return SetDisplayConfig(0, IntPtr.Zero, 0, IntPtr.Zero, topo | SDC_APPLY); }
  public static int ValidateTopology(uint topo) { return SetDisplayConfig(0, IntPtr.Zero, 0, IntPtr.Zero, topo | SDC_VALIDATE); }

  const uint C_QDC_ONLY_ACTIVE = 0x2, C_SDC_USE_SUPPLIED = 0x20, C_SDC_VALIDATE = 0x40,
             C_SDC_APPLY = 0x80, C_SDC_SAVE = 0x200, C_SDC_ALLOW_CHANGES = 0x400, C_SDC_TOPO_EXTEND = 0x4;
  const uint C_INVALID = 0xFFFFFFFF;
  static string GdiOf(ref PATH_INFO pi) {
    var s = new SOURCE_DEVICE_NAME(); s.header.type = 1; s.header.size = (uint)Marshal.SizeOf(typeof(SOURCE_DEVICE_NAME));
    s.header.adapterId = pi.sourceInfo.adapterId; s.header.id = pi.sourceInfo.id;
    DisplayConfigGetDeviceInfo(ref s); return s.viewGdiDeviceName;
  }
  // 监视器名走 CCD，不用 DEVMODE.dmDeviceName（后者 EnumDisplaySettings 不回填）
  static string MonitorOf(ref PATH_INFO pi) {
    var t = new TARGET_DEVICE_NAME(); t.header.type = 2; t.header.size = (uint)Marshal.SizeOf(typeof(TARGET_DEVICE_NAME));
    t.header.adapterId = pi.targetInfo.adapterId; t.header.id = pi.targetInfo.id;
    int rc = DisplayConfigGetDeviceInfo(ref t);
    if (rc != 0) return "<查询失败 rc=" + rc + ">";
    return t.monitorFriendlyDeviceName;
  }

  // 真·拔插：把目标 path 摘掉再挂回
  public static string Cycle(string gdiName, int downMs) {
    var sb = new StringBuilder(); uint np=0, nm=0;
    int rc = GetDisplayConfigBufferSizes(C_QDC_ONLY_ACTIVE, out np, out nm);
    if (rc != 0) return "  bufsizes rc=" + rc;
    var origP = new PATH_INFO[np]; var origM = new MODE_INFO[nm];
    rc = QueryDisplayConfig(C_QDC_ONLY_ACTIVE, ref np, origP, ref nm, origM, IntPtr.Zero);
    if (rc != 0) return "  query rc=" + rc;
    int idx = -1;
    for (int i = 0; i < np; i++) { if (GdiOf(ref origP[i]) == gdiName) { idx = i; break; } }
    if (idx < 0) return "  '" + gdiName + "' 不在活动 path 里";
    var keep = new List<PATH_INFO>();
    for (int i = 0; i < np; i++) if (i != idx) keep.Add(origP[i]);
    // mode 数组必须同步压缩 + 重映射 modeInfoIdx：SDC_USE_SUPPLIED 不接受悬空 mode 引用
    var used = new List<uint>();
    foreach (var kp in keep) {
      if (kp.sourceInfo.modeInfoIdx != C_INVALID && !used.Contains(kp.sourceInfo.modeInfoIdx)) used.Add(kp.sourceInfo.modeInfoIdx);
      if (kp.targetInfo.modeInfoIdx != C_INVALID && !used.Contains(kp.targetInfo.modeInfoIdx)) used.Add(kp.targetInfo.modeInfoIdx);
    }
    used.Sort();
    var newM = new MODE_INFO[used.Count];
    for (int j = 0; j < used.Count; j++) newM[j] = origM[used[j]];
    var keptArr = keep.ToArray();
    for (int j = 0; j < keptArr.Length; j++) {
      if (keptArr[j].sourceInfo.modeInfoIdx != C_INVALID) keptArr[j].sourceInfo.modeInfoIdx = (uint)used.IndexOf(keptArr[j].sourceInfo.modeInfoIdx);
      if (keptArr[j].targetInfo.modeInfoIdx != C_INVALID) keptArr[j].targetInfo.modeInfoIdx = (uint)used.IndexOf(keptArr[j].targetInfo.modeInfoIdx);
    }
    sb.AppendLine(string.Format("  path index {0}/{1}, 摘除后 paths->{2} modes->{3}", idx, np, keptArr.Length, newM.Length));
    // 先空跑校验，再真正提交
    rc = SetDisplayConfig((uint)keptArr.Length, keptArr, (uint)newM.Length, newM, C_SDC_USE_SUPPLIED | C_SDC_VALIDATE | C_SDC_ALLOW_CHANGES);
    sb.AppendLine("  SDC_VALIDATE(detached) rc=" + rc);
    if (rc != 0) return sb.ToString() + "  => FAIL(validate)";
    rc = SetDisplayConfig((uint)keptArr.Length, keptArr, (uint)newM.Length, newM, C_SDC_USE_SUPPLIED | C_SDC_APPLY | C_SDC_ALLOW_CHANGES);
    sb.AppendLine("  输出已摘除 rc=" + rc + "  (该显示器此刻无信号)");
    if (rc != 0) return sb.ToString() + "  => FAIL(detach)";
    System.Threading.Thread.Sleep(downMs);
    // 用原始 path/mode 原样挂回：旋转/分辨率/位置全部保留
    rc = SetDisplayConfig(np, origP, nm, origM, C_SDC_USE_SUPPLIED | C_SDC_APPLY | C_SDC_SAVE | C_SDC_ALLOW_CHANGES);
    sb.AppendLine("  已挂回 rc=" + rc);
    if (rc != 0) {
      int rc2 = SetDisplayConfig(0, IntPtr.Zero, 0, IntPtr.Zero, C_SDC_TOPO_EXTEND | C_SDC_APPLY);
      sb.AppendLine("  !! 恢复失败，兜底 TOPOLOGY_EXTEND|APPLY rc=" + rc2);
      return sb.ToString() + (rc2 == 0 ? "  => RECOVERED" : "  => FAIL(restore!)");
    }
    return sb.ToString() + "  => OK";
  }

  // 旋转：读完整 DEVMODE → 同时设方向与自洽的长短边 → CDS_UPDATEREGISTRY
  public static int SetMode(string dev, int orient, int w, int h) {
    var d = new DEVMODE(); d.dmSize = 156;
    if (!EnumDisplaySettings(dev, -1, ref d)) return -999;
    d.dmDisplayOrientation = orient; d.dmPelsWidth = w; d.dmPelsHeight = h;
    d.dmFields |= DM_ORIENT | DM_W | DM_H;
    return ChangeDisplaySettingsEx(dev, ref d, IntPtr.Zero, CDS_UPDATEREGISTRY, IntPtr.Zero);
  }
  public static string Snap(string dev) {
    var d = new DEVMODE(); d.dmSize = 156;
    if (!EnumDisplaySettings(dev, -1, ref d)) return dev + " : <fail>";
    return string.Format("{0} : {1}x{2}@{3} orient={4} pos=({5},{6})", dev, d.dmPelsWidth, d.dmPelsHeight, d.dmDisplayFrequency, d.dmDisplayOrientation, d.dmPositionX, d.dmPositionY);
  }
  public static string Paths() {
    var sb = new StringBuilder(); uint np=0, nm=0;
    if (GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE, out np, out nm) != 0) return "  <bufsizes fail>";
    var p = new PATH_INFO[np]; var m = new MODE_INFO[nm];
    if (QueryDisplayConfig(QDC_ONLY_ACTIVE, ref np, p, ref nm, m, IntPtr.Zero) != 0) return "  <query fail>";
    string[] rot = {"","IDENTITY","ROT90","ROT180","ROT270"};
    for (int i=0;i<np;i++) {
      double rn = (double)p[i].targetInfo.refreshRate.Numerator;
      double rd = (double)p[i].targetInfo.refreshRate.Denominator;
      double hz = (rd > 0.0) ? (rn / rd) : 0.0;
      sb.AppendFormat("  path{0}: gdi={1} rotation={2} refresh={3:0.##}Hz tech=0x{4:X}\r\n", i, GdiOf(ref p[i]), (p[i].targetInfo.rotation<5?rot[p[i].targetInfo.rotation]:"?"+p[i].targetInfo.rotation), hz, p[i].targetInfo.outputTechnology);
    }
    return sb.ToString();
  }

  // ── 设备枚举（-ListDevices 用）────────────────────────────────────────
  public class DispPath {
    public int Index;
    public string GdiName = "";
    public string MonitorName = "";
    public uint OutputTech;
    public string TechName = "";
    public bool IsVirtual;
    public uint CcdRotation;
    public string CcdRotationName = "";
    public double CcdRefreshHz;
    public bool ModeOk;
    public int Width, Height, Orient, PosX, PosY, DevmodeFreq;
  }

  // outputTechnology 是判断物理/虚拟屏的唯一可信依据（不是 DEVMODE.dmDeviceName）
  public static string TechName(uint t) {
    switch (t) {
      case 0: return "HD15(VGA)";
      case 1: return "SVIDEO";
      case 2: return "COMPOSITE";
      case 3: return "COMPONENT";
      case 4: return "DVI";
      case 5: return "HDMI";
      case 6: return "LVDS";
      // 注意: DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY 枚举里 7 是【跳过】的,
      // D_JPN 从 8 开始。下面按 wingdi.h 原值逐一对应, 不要顺移。
      case 8: return "D_JPN";
      case 9: return "SDI";
      case 10: return "DISPLAYPORT_EXT";
      case 11: return "DISPLAYPORT_EMB";
      case 12: return "UDI_EXT";
      case 13: return "UDI_EMB";
      case 14: return "SDTV_DONGLE";
      case 15: return "MIRACAST";
      case 16: return "INDIRECT_WIRED";
      case 17: return "INDIRECT_VIRTUAL";
      case 18: return "DP_USB_TUNNEL";
      case 0x80000000: return "INTERNAL";
      case 0xFFFFFFFF: return "OTHER";
    }
    return "UNKNOWN(0x" + t.ToString("X") + ")";
  }
  public static string RotationName(uint r) {
    switch (r) {
      case 0: return "UNSPECIFIED";
      case 1: return "0°(IDENTITY)";
      case 2: return "90°(ROT90)";
      case 3: return "180°(ROT180)";
      case 4: return "270°(ROT270)";
    }
    return "?" + r;
  }
  public static string DispChangeName(int rc) {
    switch (rc) {
      case 0: return "DISP_CHANGE_SUCCESSFUL";
      case 1: return "DISP_CHANGE_RESTART";
      case -1: return "DISP_CHANGE_FAILED";
      case -2: return "DISP_CHANGE_BADMODE";
      case -3: return "DISP_CHANGE_NOTUPDATED";
      case -4: return "DISP_CHANGE_BADFLAGS";
      case -5: return "DISP_CHANGE_BADPARAM";
      case -6: return "DISP_CHANGE_BADDUALVIEW";
    }
    return "rc=" + rc;
  }

  public static DispPath[] List(out string error) {
    error = null;
    uint np = 0, nm = 0;
    int rc = GetDisplayConfigBufferSizes(C_QDC_ONLY_ACTIVE, out np, out nm);
    if (rc != 0) { error = "GetDisplayConfigBufferSizes rc=" + rc; return new DispPath[0]; }
    var p = new PATH_INFO[np]; var m = new MODE_INFO[nm];
    rc = QueryDisplayConfig(C_QDC_ONLY_ACTIVE, ref np, p, ref nm, m, IntPtr.Zero);
    if (rc != 0) { error = "QueryDisplayConfig rc=" + rc; return new DispPath[0]; }
    var list = new List<DispPath>();
    for (int i = 0; i < np; i++) {
      var d = new DispPath();
      d.Index = i;
      d.GdiName = GdiOf(ref p[i]);
      d.MonitorName = MonitorOf(ref p[i]);
      d.OutputTech = p[i].targetInfo.outputTechnology;
      d.TechName = TechName(d.OutputTech);
      d.IsVirtual = (d.OutputTech == 15 || d.OutputTech == 16 || d.OutputTech == 17);
      d.CcdRotation = p[i].targetInfo.rotation;
      d.CcdRotationName = RotationName(d.CcdRotation);
      double num = (double)p[i].targetInfo.refreshRate.Numerator;
      double den = (double)p[i].targetInfo.refreshRate.Denominator;
      d.CcdRefreshHz = (den > 0.0) ? (num / den) : 0.0;
      var dm = new DEVMODE(); dm.dmSize = 156;
      if (EnumDisplaySettings(d.GdiName, -1, ref dm)) {
        d.ModeOk = true;
        d.Width = dm.dmPelsWidth; d.Height = dm.dmPelsHeight;
        d.Orient = dm.dmDisplayOrientation;
        d.PosX = dm.dmPositionX; d.PosY = dm.dmPositionY;
        d.DevmodeFreq = dm.dmDisplayFrequency;
      }
      list.Add(d);
    }
    return list.ToArray();
  }

  // 结构体尺寸自检（必须在 x64 上得到 156 / 84 / 420）
  public static int SizeOfDevmode() { return Marshal.SizeOf(typeof(DEVMODE)); }
  public static int SizeOfSourceName() { return Marshal.SizeOf(typeof(SOURCE_DEVICE_NAME)); }
  public static int SizeOfTargetName() { return Marshal.SizeOf(typeof(TARGET_DEVICE_NAME)); }
}
'@
}

# ══════════════════════════════════════════════════════════════════════════
# 帮助
# ══════════════════════════════════════════════════════════════════════════
if ($Help) {
  Write-Host 'rotate-display.ps1 —— 旋转显示器方向并按需复位'
  Write-Host ''
  Write-Host '  -Orientation <0|90|180|270|landscape|portrait>  目标方向，默认 0（横屏）'
  Write-Host '                                                  landscape=0, portrait=270'
  Write-Host '  -Device <GDI 设备名>                            默认 \\.\DISPLAY5'
  Write-Host '  -Reset <none|ccdcycle|topology>                 默认 ccdcycle（真·拔插复位）'
  Write-Host '  -DownSeconds <秒>                               ccdcycle 摘除时长，默认 2'
  Write-Host '  -ListDevices                                    只列出活动显示器后退出（只读）'
  Write-Host '  -WhatIf                                         只打印计划，不做任何更改（只读）'
  Write-Host '  -? | -Help                                      本帮助'
  Write-Host ''
  Write-Host '示例：'
  Write-Host '  pwsh -File rotate-display.ps1 -ListDevices'
  Write-Host '  pwsh -File rotate-display.ps1 -Orientation portrait -WhatIf'
  Write-Host '  pwsh -File rotate-display.ps1 -Orientation portrait'
  Write-Host '  pwsh -File rotate-display.ps1 -Orientation 0 -Reset none'
  exit 0
}

$degToDm = @{ 0 = 0; 90 = 1; 180 = 2; 270 = 3 }   # 角度 → DMDO_*
$degList = @('0', '90', '180', '270')

# ── -ListDevices 用的定宽表格渲染（中文按双宽算，保证列对齐）────────────
function Get-DisplayWidth([string]$s) {
  if ([string]::IsNullOrEmpty($s)) { return 0 }
  $w = 0
  foreach ($ch in $s.ToCharArray()) {
    $c = [int]$ch
    if (($c -ge 0x1100 -and $c -le 0x115F) -or ($c -ge 0x2E80 -and $c -le 0xA4CF) -or
        ($c -ge 0xAC00 -and $c -le 0xD7A3) -or ($c -ge 0xF900 -and $c -le 0xFAFF) -or
        ($c -ge 0xFE30 -and $c -le 0xFE6F) -or ($c -ge 0xFF00 -and $c -le 0xFF60) -or
        ($c -ge 0xFFE0 -and $c -le 0xFFE6)) { $w += 2 } else { $w += 1 }
  }
  return $w
}
function Format-TableRow([string[]]$cells, [int[]]$widths) {
  $parts = @()
  for ($i = 0; $i -lt $cells.Count; $i++) {
    $pad = $widths[$i] - (Get-DisplayWidth $cells[$i])
    if ($pad -lt 0) { $pad = 0 }
    $parts += ($cells[$i] + (' ' * $pad))
  }
  return ($parts -join '  ')
}

# ══════════════════════════════════════════════════════════════════════════
# -ListDevices：枚举活动 path + 各自的 GDI 现状（只读，不改任何东西）
# ══════════════════════════════════════════════════════════════════════════
if ($ListDevices) {
  $listErr = ''
  $rows = [RD]::List([ref]$listErr)
  if ($listErr) {   # 注意：$null -ne '' 为真，不能写成 -ne ''
    Write-Host ('枚举失败: ' + $listErr)
    exit 1
  }
  Write-Host '=== 活动显示器（QueryDisplayConfig QDC_ONLY_ACTIVE_PATHS）==='
  # 自己渲染定宽表格，不用 Format-Table：后者在 stdout 被重定向（计划任务 / WSL
  # 互操作管道）时按宿主宽度静默砍掉尾部列（实测 120 列下丢列）。
  $headers = @('#', 'GDI 设备名', '监视器名', '接口', '类型', '分辨率', '方向(GDI)', '刷新', 'CCD 旋转', '位置', '目标')
  $data = New-Object System.Collections.ArrayList
  foreach ($r in $rows) {
    if ($r.ModeOk) {
      $degCur = if ($r.Orient -ge 0 -and $r.Orient -le 3) { $degList[$r.Orient] } else { '?' }
      $res    = '{0}x{1}' -f $r.Width, $r.Height
      $orient = '{0}({1}°)' -f $r.Orient, $degCur
      $freq   = '{0}' -f $r.DevmodeFreq
      $pos    = '({0},{1})' -f $r.PosX, $r.PosY
    } else {
      $res = '<EnumDisplaySettings 失败>'; $orient = '-'; $freq = '-'; $pos = '-'
    }
    $mon  = if ([string]::IsNullOrEmpty($r.MonitorName)) { '<未知>' } else { $r.MonitorName }
    $gdi  = if ([string]::IsNullOrEmpty($r.GdiName)) { '<无 GDI 名>' } else { $r.GdiName }
    $kind = if ($r.IsVirtual) { '虚拟/间接' } else { '物理' }
    $mark = if ($r.GdiName -eq $Device) { '*' } else { '' }
    [void]$data.Add([string[]]@([string]$r.Index, $gdi, $mon, $r.TechName, $kind, $res, $orient, $freq, $r.CcdRotationName, $pos, $mark))
  }
  $widths = New-Object 'int[]' $headers.Count
  for ($i = 0; $i -lt $headers.Count; $i++) { $widths[$i] = Get-DisplayWidth $headers[$i] }
  foreach ($row in $data) {
    for ($i = 0; $i -lt $row.Count; $i++) {
      $w = Get-DisplayWidth $row[$i]
      if ($w -gt $widths[$i]) { $widths[$i] = $w }
    }
  }
  Write-Host (Format-TableRow $headers $widths)
  Write-Host (Format-TableRow ([string[]]($widths | ForEach-Object { '-' * $_ })) $widths)
  foreach ($row in $data) { Write-Host (Format-TableRow $row $widths) }
  if ($data.Count -eq 0) { Write-Host '  <没有活动 path>' }
  Write-Host ''
  $szDm = [RD]::SizeOfDevmode(); $szSrc = [RD]::SizeOfSourceName(); $szTgt = [RD]::SizeOfTargetName()
  Write-Host ('（自检）Marshal.SizeOf: DEVMODE={0}（应 156） SOURCE_DEVICE_NAME={1}（应 84） TARGET_DEVICE_NAME={2}（应 420）' -f $szDm, $szSrc, $szTgt)
  Write-Host ('提示：判断物理/虚拟屏看「输出接口」（CCD outputTechnology），不要看 DEVMODE.dmDeviceName。' +
              ' MIRACAST / INDIRECT_WIRED / INDIRECT_VIRTUAL 属虚拟或间接输出。')
  exit 0
}

# ══════════════════════════════════════════════════════════════════════════
# 归一化方向
# ══════════════════════════════════════════════════════════════════════════
$deg = switch ($Orientation) {
  'landscape' { 0 }
  'portrait'  { 270 }
  default     { [int]$Orientation }
}
$dmOrient = $degToDm[$deg]

# ══════════════════════════════════════════════════════════════════════════
# 读当前 DEVMODE，算目标长短边
# ══════════════════════════════════════════════════════════════════════════
Write-Host ('=== target: {0} ===' -f $Device)

$cur = New-Object RD+DEVMODE
$cur.dmSize = 156
$hasCur = [RD]::EnumDisplaySettings($Device, -1, [ref]$cur)
if (-not $hasCur) {
  Write-Host ('  !! EnumDisplaySettings 读不到 {0}：设备名不存在或不是活动输出。' -f $Device)
  Write-Host '     先跑 -ListDevices 拿当前 GDI 设备名。'
  exit 2
}
$curDeg = if ($cur.dmDisplayOrientation -ge 0 -and $cur.dmDisplayOrientation -le 3) { $degList[$cur.dmDisplayOrientation] } else { "?$($cur.dmDisplayOrientation)" }
Write-Host ('  before  ' + [RD]::Snap($Device))
Write-Host ([RD]::Paths())

# 目标方向自洽的长短边：0/180 长边当宽，90/270 短边当宽（只设方向不换长短边会 BADMODE）
$long = [Math]::Max($cur.dmPelsWidth, $cur.dmPelsHeight)
$short = [Math]::Min($cur.dmPelsWidth, $cur.dmPelsHeight)
if ($deg -eq 90 -or $deg -eq 270) { $tgtW = $short; $tgtH = $long } else { $tgtW = $long; $tgtH = $short }

# ══════════════════════════════════════════════════════════════════════════
# -WhatIf：只打印计划，不碰任何 Set* API
# ══════════════════════════════════════════════════════════════════════════
if ($WhatIf) {
  Write-Host ('  [WhatIf] 当前: {0}x{1} orient={2}({3}°) pos=({4},{5}) @{6}Hz' -f `
    $cur.dmPelsWidth, $cur.dmPelsHeight, $cur.dmDisplayOrientation, $curDeg, $cur.dmPositionX, $cur.dmPositionY, $cur.dmDisplayFrequency)
  Write-Host ('  [WhatIf] 旋转: -Orientation {0} → {1}° (dmDisplayOrientation={2}), dmPelsWidth/Height={3}x{4}' -f `
    $Orientation, $deg, $dmOrient, $tgtW, $tgtH)
  Write-Host ('  [WhatIf] 将调 ChangeDisplaySettingsEx("{0}", DEVMODE{{dmFields |= DM_DISPLAYORIENTATION|DM_PELSWIDTH|DM_PELSHEIGHT}}, CDS_UPDATEREGISTRY, NULL)  → 未执行' -f $Device)
  switch ($Reset) {
    'ccdcycle' {
      Write-Host ('  [WhatIf] 复位 ccdcycle: QueryDisplayConfig 取全量 path/mode → 摘除目标 path（mode 数组同步压缩+重映射 modeInfoIdx）' +
                  (' → SDC_VALIDATE 空跑 → 提交摘除 → 停 {0}s → 用原始数组原样挂回  → 未执行' -f $DownSeconds))
    }
    'topology' {
      Write-Host '  [WhatIf] 复位 topology: SetDisplayConfig(0,NULL,0,NULL, TOPOLOGY_EXTEND|SDC_APPLY)（实测「假成功」，仅作兜底）  → 未执行'
    }
    'none' {
      Write-Host '  [WhatIf] 复位 none: 不复位（输入映射可能仍是旧的）'
    }
  }
  Write-Host '  => WhatIf：未做任何修改'
  Write-Host '=== done (whatif) ==='
  exit 0
}

$exitCode = 0

# ══════════════════════════════════════════════════════════════════════════
# 1) 旋转
# ══════════════════════════════════════════════════════════════════════════
$rc = [RD]::SetMode($Device, $dmOrient, $tgtW, $tgtH)
Write-Host ('  rotate  {0}x{1} orient={2}({3}°) -> rc={4} {5}' -f $tgtW, $tgtH, $dmOrient, $deg, $rc, [RD]::DispChangeName($rc))
if ($rc -ne 0) { $exitCode = 3 }
Start-Sleep -Milliseconds 900
Write-Host ('  rotated ' + [RD]::Snap($Device))

# ══════════════════════════════════════════════════════════════════════════
# 2) 复位
# ══════════════════════════════════════════════════════════════════════════
switch ($Reset) {
  'ccdcycle' {
    Write-Host ('  reset   CCD 真·拔插（摘除 {0}s 再原样挂回）…' -f $DownSeconds)
    $log = [RD]::Cycle($Device, $DownSeconds * 1000)
    Write-Host $log
    if ($log -notmatch '=> (OK|RECOVERED)') { $exitCode = 3 }
    Start-Sleep -Milliseconds 1200
    Write-Host ('  after   ' + [RD]::Snap($Device))
  }
  'topology' {
    $vrc = [RD]::ValidateTopology([RD]::TOPO_EXTEND)
    $rc2 = [RD]::ApplyTopology([RD]::TOPO_EXTEND)
    Write-Host ('  reset   SDC_VALIDATE(TOPOLOGY_EXTEND) rc={0}; SetDisplayConfig(0,NULL,0,NULL, TOPOLOGY_EXTEND|SDC_APPLY) rc={1}' -f $vrc, $rc2)
    Write-Host '  ⚠ 实测：拓扑无变化时此项是「假成功」（rc=0 但不重建输出），不能当默认复位手段。'
    if ($rc2 -ne 0) { $exitCode = 3 }
    Start-Sleep -Milliseconds 1000
    Write-Host ('  after   ' + [RD]::Snap($Device))
    Write-Host ([RD]::Paths())
  }
  'none' {
    Write-Host '  reset   none（未复位；若指针映射不对，改用 -Reset ccdcycle）'
  }
}

Write-Host '=== done ==='
exit $exitCode
