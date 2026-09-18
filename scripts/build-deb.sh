#!/usr/bin/env bash
# 构建 Linux .deb 包（Tauri v2）。两种模式：
#
#  ① 默认（推荐）：在 Docker 里用 **Ubuntu 22.04** 构建（scripts/Dockerfile.deb）。
#     为什么必须用老发行版：deb 里的二进制链接的是构建机的 glibc。在 24.04 / 26.04 上
#     构建出的程序要求 GLIBC_2.39 / 2.43，拿到 Ubuntu 22.04（glibc 2.35）上会被动态链接器
#     直接拒绝加载 —— GUI 双击表现为「毫无反应」，终端运行才看到 `GLIBC_2.39' not found。
#     按最老的受支持发行版构建，产物才能同时跑在 22.04 / 24.04 / 26.04 上。
#
#  ② SAS_BUILD_IN_DOCKER=0：在 WSL / 原生 Linux 本机直接构建。快，但产物只能跑在
#     「不低于本机版本」的系统上（本机是 26.04 就只能在 26.04+ 用）。
#
# 用法（在 WSL 里执行，Windows 侧无法直接出 deb）：
#   bash scripts/build-deb.sh
#   SAS_BUILD_IN_DOCKER=0 bash scripts/build-deb.sh
#   SAS_BUILD_HOME=~/my-build bash scripts/build-deb.sh
#
# 其他踩坑记录：
# - 不要试图在原生 Windows 上跑本脚本：deb 打包需要 Linux 的 dpkg-deb 与 webkit2gtk。
# - WSL 默认把 Windows 的 PATH（/mnt/c/...）带进 Linux 环境，且仓库在 /mnt/d 上的
#   node_modules 是 Windows 版（缺 Linux 原生 esbuild / tauri 二进制），必须用纯 Linux 工具链。
#   因此脚本把源码同步到 WSL 本地目录 $BUILD_HOME（保留其中的 node_modules / target 缓存），
#   既不污染 Windows 侧 node_modules，又能正确产出 Linux deb。
# - 依赖（apt / node / rust）只在缺失时安装，脚本可重复执行。
# - 构建完成后把 .deb 拷回仓库根目录，方便在 Windows 侧取用。

set -euo pipefail

# 仓库根目录（脚本位于 <root>/scripts/）
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# 是否在 Docker 里构建；基础镜像可用 SAS_BUILD_IMAGE 覆盖（务必是较老的 LTS）
SAS_IN_DOCKER="${SAS_BUILD_IN_DOCKER:-1}"
BASE_IMAGE="${SAS_BUILD_IMAGE:-ubuntu:22.04}"
IMAGE_TAG="sas-deb-builder:${BASE_IMAGE##*:}"

# WSL 本地的构建目录（独立于 /mnt/d，避免 Windows 工具链干扰；node_modules/target 在此缓存）
# 两种模式用不同目录：本机原生构建的 target/ 与容器构建的不能混用（glibc / 工具链不同）。
if [ "$SAS_IN_DOCKER" = "1" ]; then
  DEFAULT_HOME="$HOME/.sas-build-2204"
else
  DEFAULT_HOME="$HOME/.sas-build"
fi
BUILD_HOME="${SAS_BUILD_HOME:-$DEFAULT_HOME}"

# 需要 root 时用 sudo，否则空
SUDO=""
if [ "$(id -u)" -ne 0 ]; then SUDO="sudo"; fi

# 提前把 cargo 纳入 PATH（若已用 rustup 装过），否则后面检测不到 cargo 会重复下载 rustup
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

echo "==> 仓库目录: $REPO_ROOT"
echo "==> 构建目录: $BUILD_HOME"
if [ "$SAS_IN_DOCKER" = "1" ]; then
  echo "==> 构建模式: Docker（$BASE_IMAGE，产物兼容 22.04+）"
else
  echo "==> 构建模式: 本机原生（产物只兼容本机及更高版本的发行版）"
fi

# ① 同步源码到构建目录（覆盖式，但不动 BUILD_HOME 下的 node_modules / target 缓存）
echo "==> 同步源码到构建目录..."
mkdir -p "$BUILD_HOME"
tar cf - \
    --exclude=node_modules --exclude=target --exclude=dist --exclude=.git \
    --exclude='*.exe' --exclude='*.deb' --exclude='build-*.log' --exclude='.codebuddy' \
    -C "$REPO_ROOT" . \
  | tar xf - -C "$BUILD_HOME"

# ② 构建 deb
if [ "$SAS_IN_DOCKER" = "1" ]; then
  if ! command -v docker >/dev/null 2>&1; then
    echo "!! 未找到 docker。可改用 SAS_BUILD_IN_DOCKER=0 走本机原生构建（产物不兼容老发行版）。" >&2
    exit 1
  fi

  echo "==> 准备构建镜像 $IMAGE_TAG（首次约几分钟；之后走缓存秒过）..."
  docker build -t "$IMAGE_TAG" -f "$REPO_ROOT/scripts/Dockerfile.deb" "$REPO_ROOT/scripts"

  echo "==> 容器内构建 deb（首次编译依赖较久，输出实时可见）..."
  # 用 --config 内联覆盖 bundle.targets 为 deb：绕开 tauri.conf.json 里 Windows 专用的
  # msi/nsis，也避免某些 CLI 版本对 --bundles 取值的校验问题。
  docker run --rm -v "$BUILD_HOME:/work" -w /work "$IMAGE_TAG" bash -lc '
    set -e
    echo "node=$(node -v)  cargo=$(cargo -V)"
    npm install
    npm run tauri build -- --config "{\"bundle\":{\"targets\":[\"deb\"]}}"
    echo "==> 产物二进制的最高 GLIBC 符号需求（应不高于目标系统）："
    objdump -T src-tauri/target/release/sas-pwa-client 2>/dev/null \
      | grep -oE "GLIBC_[0-9]+\.[0-9]+" | sort -uV | tail -n 1
  '
else
  # ---- 本机原生构建（WSL / Linux）----
  echo "==> 检查/安装系统依赖..."
  if ! dpkg -s libwebkit2gtk-4.1-dev >/dev/null 2>&1; then
    $SUDO apt-get update
    $SUDO apt-get install -y curl build-essential pkg-config file patchelf \
      libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev \
      libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev
  else
    echo "    系统依赖已满足，跳过。"
  fi

  if ! command -v node >/dev/null 2>&1 || [ "$(node -v 2>/dev/null | cut -d. -f1 | tr -d v)" -lt 22 ]; then
    echo "==> 安装 Node.js 22..."
    if [ -n "$SUDO" ]; then
      curl -fsSL https://deb.nodesource.com/setup_22.x | $SUDO -E bash -
    else
      curl -fsSL https://deb.nodesource.com/setup_22.x | bash -
    fi
    $SUDO apt-get install -y nodejs
  else
    echo "==> Node $(node -v) 已满足，跳过。"
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    echo "==> 安装 Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
  else
    echo "==> Rust 已安装，跳过。"
  fi

  # 强制使用【纯 Linux】工具链（剔除任何 /mnt/* 的 Windows PATH）
  cd "$BUILD_HOME"
  export PATH="/usr/local/bin:/usr/bin:/bin:/usr/local/sbin:/usr/sbin:/sbin"
  # shellcheck disable=SC1091
  [ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
  echo "==> 工具链检查：node=$(command -v node)  cargo=$(command -v cargo)"

  echo "==> npm install（Linux）..."
  npm install
  npm run tauri build -- --config '{"bundle":{"targets":["deb"]}}'
fi

# ③ 收集产物并拷回仓库根目录，方便 Windows 侧取用
# 注意：bundle/deb/ 里会残留旧版本的包，而 `ls | head -n1` 是按名字排序（0.2.4 排在 0.2.5 前面），
# 会拷回旧产物。这里先按 tauri.conf.json 里的版本号精确取，取不到再退回「按修改时间最新」。
DEB_DIR="$BUILD_HOME/src-tauri/target/release/bundle/deb"
PKG_VER="$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$REPO_ROOT/src-tauri/tauri.conf.json" | head -n1)"
DEB=""
if [ -n "$PKG_VER" ] && [ -f "$DEB_DIR/SAS PWA Client_${PKG_VER}_amd64.deb" ]; then
  DEB="$DEB_DIR/SAS PWA Client_${PKG_VER}_amd64.deb"
else
  DEB="$(ls -1t "$DEB_DIR"/*.deb 2>/dev/null | head -n1 || true)"
fi
if [ -n "$DEB" ]; then
  cp -f "$DEB" "$REPO_ROOT/"
  echo ""
  echo "==> 构建完成！deb 产物："
  echo "    WSL 内: $DEB"
  echo "    Windows: $REPO_ROOT/$(basename "$DEB")"
  echo "    安装（Linux）：sudo apt install \"./$(basename "$DEB")\""
  echo "    提示：装之前可核对目标的 glibc 是否 ≥ 产物需求（deb 内二进制的 GLIBC 符号）。"
else
  echo "!! 未找到 deb 产物，请检查上面的构建日志。" >&2
  exit 1
fi
