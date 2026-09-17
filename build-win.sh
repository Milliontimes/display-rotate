#!/usr/bin/env bash
# 交叉编译 Windows 原生 exe (x86_64-pc-windows-gnu, zig 链接器)
# 前置: rustup target add x86_64-pc-windows-gnu
#       zig 解压到 ~/apps/zig-x86_64-linux-0.14.1 (或修改下方 ZIG 路径)
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"

ZIG="${ZIG:-$HOME/apps/zig-x86_64-linux-0.14.1/zig}"

# zig 缓存必须落在可写目录 (zig 0.14 环境变量带 _DIR 后缀; 默认 ~/.cache/zig
# 在只读 HOME/沙箱下会报 ReadOnlyFileSystem)
export ZIG_LOCAL_CACHE_DIR="${ZIG_LOCAL_CACHE_DIR:-$PWD/.zig-cache/local}"
export ZIG_GLOBAL_CACHE_DIR="${ZIG_GLOBAL_CACHE_DIR:-$PWD/.zig-cache/global}"

# 链接器包装: zig cc + 三处参数修补 (都实测过):
# 1) 剔除 rustc 传的 GNU 工具链遗留库: -lmsvcrt / -l:libpthread.a / -lpthread
#    (zig 自动链接 api-ms-win-crt 与原生线程)
# 2) 剔除 `-Wl,-Bdynamic`: rustc 把它放在 windows-targets 的 `-lwindows.0.52.0`
#    **之前**, 于是 zig 按"只找动态库"的策略去找 windows.0.52.0.dll, 报
#    "unable to find dynamic system library 'windows.0.52.0' using strategy 'no_fallback'";
#    而该 crate 提供的其实是静态归档 libwindows.0.52.0.a (就在随后的 -L 目录里)。
#    去掉这个标志即恢复静态归档查找 (mingw 那批 -lkernel32/-luser32 同样是 .a)。
#    (试过 GNU 的 `-l:libwindows.0.52.0.a` 精确写法: zig 会把它原样丢给 lld-link,
#     而 COFF 的 lld-link 不认 `-l:` 语法, 报 could not open ':lib...'。)
# 3) 把 zig 的缓存目录默认值烘进包装脚本: 只读 HOME (沙箱/CI) 下 zig 默认的
#    ~/.cache/zig 写不进去, 会报 "unable to create compilation: ReadOnlyFileSystem"。
#    烘进来以后 `cargo build --release --target x86_64-pc-windows-gnu` 单跑也能过。
# 落点优先 ~/.local/bin (与同机 llama-watch 共用同一条链): 那里的包装脚本必须
# **已经带上面这些修补** 才复用; 只读 HOME / 沙箱里写不进去时退回项目内 .zig-cache/。
WRAP="$HOME/.local/bin/zig-cc"
if ! { [ -x "$WRAP" ] && grep -qF 'Bdynamic' "$WRAP" && grep -qF "$ZIG" "$WRAP"; }; then
  if mkdir -p "$HOME/.local/bin" 2>/dev/null && [ -w "$HOME/.local/bin" ]; then
    WRAP="$HOME/.local/bin/zig-cc"
    CACHE_ROOT="$HOME/.zig-cache"
  else
    WRAP="$PWD/.zig-cache/zig-cc"
    CACHE_ROOT="$PWD/.zig-cache"
    mkdir -p "$CACHE_ROOT"
  fi
  cat > "$WRAP" <<EOF
#!/usr/bin/env bash
# zig cc 包装: 交叉编译到 x86_64-windows-gnu (由 display-rotate/build-win.sh 生成)
export ZIG_LOCAL_CACHE_DIR="\${ZIG_LOCAL_CACHE_DIR:-$CACHE_ROOT/local}"
export ZIG_GLOBAL_CACHE_DIR="\${ZIG_GLOBAL_CACHE_DIR:-$CACHE_ROOT/global}"
args=()
for a in "\$@"; do
  case "\$a" in
    -lmsvcrt|-l:libpthread.a|-lpthread|-Wl,-Bdynamic) continue ;;
  esac
  args+=("\$a")
done
exec $ZIG cc -target x86_64-windows-gnu "\${args[@]}"
EOF
  chmod +x "$WRAP"
fi

mkdir -p .cargo
printf '[target.x86_64-pc-windows-gnu]\nlinker = "%s"\n' "$WRAP" > .cargo/config.toml

cargo build --release --target x86_64-pc-windows-gnu --offline
echo
echo "产物: target/x86_64-pc-windows-gnu/release/display-rotate.exe"
echo "部署: cp target/x86_64-pc-windows-gnu/release/display-rotate.exe /mnt/c/AI/display-rotate/display-rotate.exe"
echo "      cp examples/config.example.toml /mnt/c/AI/display-rotate/config.toml   # 然后改 device"
