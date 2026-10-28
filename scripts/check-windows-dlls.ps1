# Fails if a program needs the Visual C++ runtime DLLs (ticket 333):
# VCRUNTIME140.dll and MSVCP140.dll come with the VC++ Redistributable,
# which a fresh Windows install may not have. .cargo/config.toml links the
# C runtime statically so they aren't needed. CI's runners have the
# Redistributable, so a program that needs it still starts there; this
# reads what each program imports instead, with dumpbin.
#
#   scripts/check-windows-dlls.ps1 target\release\rite.exe target\release\riteui.exe

param([Parameter(Mandatory, ValueFromRemainingArguments)][string[]]$Programs)

$ErrorActionPreference = 'Stop'
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$dumpbin = & $vswhere -latest -products * -find 'VC\Tools\MSVC\**\dumpbin.exe' | Select-Object -First 1
if (-not $dumpbin) { throw "dumpbin not found (Visual Studio's C++ tools)" }

$failures = 0
foreach ($program in $Programs) {
    $out = & $dumpbin /nologo /dependents $program
    if ($LASTEXITCODE -ne 0) { throw "dumpbin failed on ${program}: $out" }
    # The DLL names are the indented lines ending in .dll.
    $dlls = @($out | Where-Object { $_ -match '^\s+\S+\.dll\s*$' } | ForEach-Object { $_.Trim() })
    $runtime = @($dlls | Where-Object { $_ -match '^(vcruntime|msvcp)' })
    Write-Host "${program}: $($dlls -join ', ')"
    if ($runtime) {
        Write-Host "  FAIL  needs $($runtime -join ', ') (the VC++ Redistributable)" -ForegroundColor Red
        $failures++
    }
}
if ($failures) { exit 1 }
