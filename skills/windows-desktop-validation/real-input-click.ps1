param([string]$Exe, [string]$Out, [string[]]$AppArgs = @(), [string]$Clicks = "900,40;110,177")
# Real (injected) mouse input, guarded: a click is only sent while the app window is in
# the foreground and is the window under the cursor, so nothing lands in other apps.
# Clicks: "x,y" (physical px relative to the window) or "dx,y,dx,dy" for a drag, e.g.
# "d900,40,-80,-60" presses at 900,40 and moves by -80,-60. Captures (PrintWindow) go to $Out.
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class R {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, IntPtr e);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint f);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint f);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
"@
[R]::SetProcessDPIAware() | Out-Null
New-Item -ItemType Directory -Force $Out | Out-Null
$saved = New-Object R+POINT; [R]::GetCursorPos([ref]$saved) | Out-Null
if ($AppArgs.Count) { $p = Start-Process $Exe -ArgumentList $AppArgs -PassThru } else { $p = Start-Process $Exe -PassThru }
$h = [IntPtr]::Zero
for ($i = 0; $i -lt 50 -and $h -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 200; $p.Refresh(); $h = $p.MainWindowHandle }
Start-Sleep -Milliseconds 1500
# Alt tap lets this process move the foreground.
[R]::keybd_event(0x12, 0, 0, [IntPtr]::Zero); [R]::keybd_event(0x12, 0, 2, [IntPtr]::Zero)
[R]::SetForegroundWindow($h) | Out-Null; [R]::SetWindowPos($h, [IntPtr](-1), 0, 0, 0, 0, 3) | Out-Null; Start-Sleep -Milliseconds 400
$r = New-Object R+RECT; [R]::GetWindowRect($h, [ref]$r) | Out-Null
function Shot($name) {
  $bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
  $g = [System.Drawing.Graphics]::FromImage($bmp); $hdc = $g.GetHdc()
  [R]::PrintWindow($h, $hdc, 3) | Out-Null; $g.ReleaseHdc($hdc)
  $bmp.Save((Join-Path $Out "$name.png")); $g.Dispose(); $bmp.Dispose()
}
Shot "0-start"
$n = 1
foreach ($c in ($Clicks -split ";")) {
  if ($c.StartsWith("d")) {
    # Drag: press at x,y, move by dx,dy in steps, release; report how far the window moved.
    $v = $c.Substring(1) -split ","; $x0 = $r.L + [int]$v[0]; $y0 = $r.T + [int]$v[1]
    [R]::SetCursorPos($x0, $y0) | Out-Null; Start-Sleep -Milliseconds 200
    $pt = New-Object R+POINT; $pt.X = $x0; $pt.Y = $y0
    if ([R]::GetForegroundWindow() -ne $h -or [R]::GetAncestor([R]::WindowFromPoint($pt), 2) -ne $h) { "drag skipped"; break }
    $before = New-Object R+RECT; [R]::GetWindowRect($h, [ref]$before) | Out-Null
    [R]::mouse_event(2, 0, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 150
    for ($k = 1; $k -le 10; $k++) { [R]::SetCursorPos($x0 + [int]$v[2] * $k / 10, $y0 + [int]$v[3] * $k / 10) | Out-Null; Start-Sleep -Milliseconds 40 }
    Start-Sleep -Milliseconds 150; [R]::mouse_event(4, 0, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 600
    [R]::GetWindowRect($h, [ref]$r) | Out-Null
    "drag $n moved window by $($r.L - $before.L),$($r.T - $before.T)"
    Shot "$n-after-drag"; $n++; continue
  }
  $xy = $c -split ","; $pt = New-Object R+POINT; $pt.X = $r.L + [int]$xy[0]; $pt.Y = $r.T + [int]$xy[1]
  [R]::SetCursorPos($pt.X, $pt.Y) | Out-Null; Start-Sleep -Milliseconds 200
  $under = [R]::GetAncestor([R]::WindowFromPoint($pt), 2)
  $fg = [R]::GetForegroundWindow()
  $ht = [R]::SendMessage($h, 0x0084, [IntPtr]0, [IntPtr](($pt.Y -shl 16) -bor ($pt.X -band 0xFFFF)))
  if ($fg -ne $h -or $under -ne $h) { "click $n skipped: foreground=$fg under=$under app=$h"; break }
  [R]::mouse_event(2, 0, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 60
  [R]::mouse_event(4, 0, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 800
  Shot "$n-after-$($xy[0])-$($xy[1])"; "click $n at $c nchittest=$ht"; $n++
}
[R]::SetCursorPos($saved.X, $saved.Y) | Out-Null
Stop-Process -Id $p.Id -Force
