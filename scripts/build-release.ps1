# 生产构建脚本（Windows / Tauri v2）。
# 用法：在仓库根目录或任意位置执行
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-release.ps1
#
# 说明：
# - 必须带 --features custom-protocol，否则 Tauri 按 dev 模式加载设置页会报 ERR_CONNECTION_REFUSED。
# - CARGO_BUILD_JOBS=4 降低并发，规避安全软件瞬时拦截 link.exe（os error 5）导致的偶发失败。
# - 内置看门狗：长编译时若 rustc 线程被挂起（WaitReason=Suspended），自动 ResumeThread 恢复；
#   同时每 5 秒采样 build-rel.log，避免命令无输出被误判为卡死。

$ErrorActionPreference = 'Continue'
$env:CARGO_BUILD_JOBS = '4'

$repoRoot = (Get-Item $PSScriptRoot).Parent.FullName
$buildDir = Join-Path $repoRoot "src-tauri"
$log = Join-Path $repoRoot "build-rel.log"
Set-Location $buildDir

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Rz {
  [DllImport("kernel32.dll")] public static extern IntPtr OpenThread(int af, bool inh, int tid);
  [DllImport("kernel32.dll")] public static extern int ResumeThread(IntPtr h);
}
"@

$p = Start-Process -FilePath "cmd.exe" -ArgumentList "/c","cargo build --release --features custom-protocol > ..\build-rel.log 2>&1" -PassThru -NoNewWindow -WorkingDirectory $buildDir

$last = ""
while (-not $p.HasExited) {
  Start-Sleep -Seconds 5
  Get-Process -Name rustc -ErrorAction SilentlyContinue | ForEach-Object {
    $_.Threads | Where-Object { $_.ThreadState -eq 'Wait' -and $_.WaitReason -eq 'Suspended' } | ForEach-Object {
      $h = [Rz]::OpenThread(0x0002, $false, $_.Id)
      if ($h -ne [IntPtr]::Zero) { [Rz]::ResumeThread($h) | Out-Null }
    }
  }
  if (Test-Path $log) {
    $tail = (Get-Content $log -Tail 1) -join ""
    if ($tail -ne $last) { Write-Host "[log] $tail"; $last = $tail }
  }
  Write-Host "[watchdog] building... rustc procs: $((Get-Process -Name rustc -ErrorAction SilentlyContinue).Count)"
}
Write-Host "BUILD EXIT CODE: $($p.ExitCode)"
