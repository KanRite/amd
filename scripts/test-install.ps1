# Installs, updates, rolls back and uninstalls a real Windows install
# (ticket 282), the way a user's machine would see it. For CI's Windows job.
#
#   scripts/test-install.ps1 [-Bin target\debug]
#
# install.ps1 installs a zip of the amd.exe and amdui.exe in -Bin. Then the
# installed amd.exe updates itself to a pretend 9.9.9 (two tiny programs
# that only print their version, built with rustc), which replaces amd.exe
# while it is the program running, rolls back, and the uninstaller removes
# it all. It writes to this user's PATH, Start menu and Settings > Apps, so
# it is for a throwaway CI runner, not your own machine.

param([string]$Bin = "target\debug")

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$work = Join-Path ([IO.Path]::GetTempPath()) "amd-test-install-$PID"
$dir = Join-Path $work 'KanRite\bin'
New-Item -ItemType Directory -Force -Path $work | Out-Null
$failures = 0

function Check([string]$name, [scriptblock]$test) {
    # Windows PowerShell 5.1 turns a program's stderr, redirected, into
    # errors, which 'Stop' would throw on.
    $ErrorActionPreference = 'Continue'
    $ok = $false
    try { $ok = [bool](& $test) } catch { Write-Host "    $($_.Exception.Message)" }
    if ($ok) { Write-Host "  ok    $name" } else { Write-Host "  FAIL  $name" -ForegroundColor Red; $script:failures++ }
}

function Zip([string]$from, [string]$to) {
    Compress-Archive -Path (Join-Path $from 'amd.exe'), (Join-Path $from 'amdui.exe') -DestinationPath $to -Force
}

Write-Host 'install.ps1:'
$built = Join-Path $root $Bin
$zip = Join-Path $work 'amd.zip'
Zip $built $zip
& (Join-Path $root 'install.ps1') -Dir $dir -Zip $zip
$amd = Join-Path $dir 'amd.exe'
$uninstaller = Join-Path $dir 'amd-uninstall.ps1'
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\KanRite'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'KanRite.lnk'
function UserPath { (Get-Item 'HKCU:\Environment').GetValue('Path', '', 'DoNotExpandEnvironmentNames') -split ';' }

Check 'amd.exe and amdui.exe are installed and run' {
    (& $amd --version) -match '^amd ' -and (& (Join-Path $dir 'amdui.exe') --version) -match '^amdui '
}
Check 'the uninstaller marks it a release install' {
    (Get-Content -LiteralPath $uninstaller) -contains '# agile-md install: release'
}
Check 'the folder is on the user PATH' { (UserPath) -contains $dir }
Check 'there is a Start menu shortcut to amdui' {
    (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut).TargetPath -eq (Join-Path $dir 'amdui.exe')
}
Check 'Settings > Apps lists it, uninstalling with amd-uninstall.ps1' {
    (Get-ItemProperty $key).UninstallString -like "*$uninstaller*"
}

Write-Host 'amd update:'
# A pretend 9.9.9, zipped as the release workflow does, and the release list
# naming its checksum.
$fake = Join-Path $work 'fake'
New-Item -ItemType Directory -Force -Path $fake | Out-Null
foreach ($name in 'amd', 'amdui') {
    Set-Content -LiteralPath (Join-Path $fake "$name.rs") -Value "fn main() { println!(`"$name 9.9.9`"); }"
    & rustc -O -o (Join-Path $fake "$name.exe") (Join-Path $fake "$name.rs")
    if ($LASTEXITCODE -ne 0) { throw "rustc couldn't build the pretend $name" }
}
$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
$target = if ($arch -eq 'Arm64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
$release = Join-Path $work "amd-$target.zip"
Zip $fake $release
$sha = (Get-FileHash -LiteralPath $release -Algorithm SHA256).Hash.ToLower()
$url = 'file://' + ($release -replace '\\', '/')
$list = Join-Path $work 'releases.json'
Set-Content -LiteralPath $list -Value ('[{"tag_name":"v9.9.9","html_url":"https://example.test/9.9.9","draft":false,"prerelease":false,"assets":[{"name":"amd-' + $target + '.zip","browser_download_url":"' + $url + '","digest":"sha256:' + $sha + '"}]}]')
$env:AMD_UPDATE_RELEASES = 'file://' + ($list -replace '\\', '/')

Check 'amd update replaces the running amd.exe and amdui.exe, keeping the ones before' {
    $out = & $amd update 2>&1 | Out-String
    Write-Host $out.Trim()
    (& $amd --version) -eq 'amd 9.9.9' -and (& (Join-Path $dir 'amdui.exe') --version) -eq 'amdui 9.9.9' -and
        (Test-Path -LiteralPath "$amd.previous")
}
Check 'amd update --rollback puts the previous amd.exe back' {
    # Run from the kept copy, as a user stuck on a broken 9.9.9 would; under
    # a .exe name, which Windows wants to start a program.
    $kept = Join-Path $dir 'amd-kept.exe'
    Copy-Item -LiteralPath "$amd.previous" -Destination $kept
    $out = & $kept update --rollback 2>&1 | Out-String
    Write-Host $out.Trim()
    Remove-Item -LiteralPath $kept
    (& $amd --version) -match '^amd ' -and (& $amd --version) -ne 'amd 9.9.9' -and
        -not (Test-Path -LiteralPath "$amd.previous")
}
Remove-Item Env:\AMD_UPDATE_RELEASES

Write-Host 'install.ps1 again, over the install:'
& (Join-Path $root 'install.ps1') -Dir $dir -Zip $zip
Check 'the PATH has the folder once' { @((UserPath) | Where-Object { $_ -eq $dir }).Count -eq 1 }

Write-Host 'amd-uninstall.ps1:'
& $uninstaller -Yes
Check 'the programs, uninstaller and folder are gone' { -not (Test-Path -LiteralPath $dir) }
Check 'the Start menu shortcut is gone' { -not (Test-Path -LiteralPath $shortcut) }
Check 'Settings > Apps no longer lists it' { -not (Test-Path $key) }
Check 'the folder is off the user PATH' { -not ((UserPath) -contains $dir) }

Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
if ($failures -gt 0) {
    Write-Host "$failures failed" -ForegroundColor Red
    exit 1
}
Write-Host 'all passed'
