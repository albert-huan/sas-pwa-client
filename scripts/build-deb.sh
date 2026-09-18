#!/usr/bin/env bash
# 在 WSL / 原生 Ubuntu 上构建 Linux .deb 包（Tauri v2）。
#
# 用法（在仓库根目录或任意位置执行）：
#   bash scripts/build-deb.sh
#
# 说明：
# - 仅产出 deb（用 --bundles deb 覆盖 tauri.conf.json 里 Windows 专用的 msi/nsis target）。
# - 不要试图在原生 Windows 上跑本脚本——deb 打包需要 Linux 的 dpkg-deb 与 webkit2gtk。
# - 依赖（apt/node/rust）只在缺失时安装；已装则跳过，可重复执行。
# - 跨盘编译慢：若从 /mnt/<盘符>/ 跑，建议先把仓库 cp 到 ~/ 再构建。

set -euo pipefail

# 仓库根目录（脚本位于 <root>/scripts/）
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# 需要 root 时用 sudo，否则空
SUDO=""
if [ "$(id -u)" -ne 0 ]; then SUDO="sudo"; fi

echo "==> 仓库目录: $REPO_ROOT"

# ① 系统依赖（Tauri v2 on Linux）
echo "==> 检查/安装系统依赖..."
if ! dpkg -s libwebkit2gtk-4.1-dev >/dev/null 2>&1; then
  $SUDO apt-get update
  $SUDO apt-get install -y curl build-essential patchelf \
    libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev \
    libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev
else
  echo "    系统依赖已满足，跳过。"
fi

# ② Node.js 22（缺失时通过 NodeSource 安装）
if ! command -v node >/dev/null 2>&1 || [ "$(node -v 2>/dev/null | cut -d. -f1 | tr -d v)" -lt 22 ]; then
  echo "==> 安装 Node.js 22..."
  curl -fsSL https://deb.nodesource.com/setup_22.x | $SUDO -E bash -
  $SUDO apt-get install -y nodejs
else
  echo "==> Node $(node -v) 已满足，跳过。"
fi

# ③ Rust（缺失时通过 rustup 安装）
if ! command -v cargo >/dev/null 2>&1; then
  echo "==> 安装 Rust..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
else
  echo "==> Rust 已安装，跳过。"
fi

# 确保 cargo 在当前 shell 可用（若上一步刚装）
if ! command -v cargo >/dev/null 2>&1; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

# ④ 安装前端依赖并构建 deb
echo "==> npm install..."
npm install

echo "==> 构建 deb（--bundles deb）..."
npm run tauri build -- --bundles deb

# ⑤ 输出产物路径
DEB="$(ls -1 src-tauri/target/release/bundle/deb/*.deb 2>/dev/null | head -n1 || true)"
if [ -n "$DEB" ]; then
  echo ""
  echo "==> 构建完成！deb 产物："
  echo "    $DEB"
  echo "    安装：sudo dpkg -i \"$DEB\""
else
  echo "!! 未找到 deb 产物，请检查上面的构建日志。" >&2
  exit 1
fi
