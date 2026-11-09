$ErrorActionPreference = "Stop"

function Update-Path {
    $machine = [Environment]::GetEnvironmentVariable("Path", "Machine")
    $user = [Environment]::GetEnvironmentVariable("Path", "User")
    $env:Path = "$machine;$user;$env:Path"
}

function Add-MachinePath {
    param([string]$Directory)

    $machine = [Environment]::GetEnvironmentVariable("Path", "Machine")
    if (($machine -split ";") -notcontains $Directory) {
        [Environment]::SetEnvironmentVariable(
            "Path", "$Directory;$machine", "Machine")
    }
    if (($env:Path -split ";") -notcontains $Directory) {
        $env:Path = "$Directory;$env:Path"
    }
}

function Ensure-Chocolatey {
    if (Get-Command choco.exe -ErrorAction SilentlyContinue) {
        return
    }

    $known = "C:\ProgramData\chocolatey\bin"
    if (Test-Path (Join-Path $known "choco.exe")) {
        Add-MachinePath $known
        return
    }

    Set-ExecutionPolicy Bypass -Scope Process -Force
    [Net.ServicePointManager]::SecurityProtocol = `
        [Net.ServicePointManager]::SecurityProtocol -bor 3072
    Invoke-Expression ((New-Object Net.WebClient).DownloadString(
        "https://community.chocolatey.org/install.ps1"))
    Update-Path
}

function Ensure-Rust {
    if (Get-Command cargo.exe -ErrorAction SilentlyContinue) {
        Update-Path
        return
    }

    $hostTriple = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") {
        "aarch64-pc-windows-msvc"
    } else {
        "x86_64-pc-windows-gnu"
    }

    if ($hostTriple -eq "x86_64-pc-windows-gnu") {
        Ensure-Chocolatey
        choco install -y mingw --no-progress

        $gccCandidates = @(Get-ChildItem "C:\ProgramData\chocolatey\lib\mingw" `
            -Recurse -Filter gcc.exe -ErrorAction SilentlyContinue)
        $gcc = $gccCandidates |
            Where-Object { $_.FullName -match "mingw64" } |
            Select-Object -First 1
        if (-not $gcc) {
            $gcc = $gccCandidates | Select-Object -First 1
        }
        if (-not $gcc) {
            throw "Unable to locate gcc.exe after installing mingw."
        }
        Add-MachinePath $gcc.DirectoryName
        Add-MachinePath "C:\ProgramData\chocolatey\bin"
    } else {
        # The MSVC toolchain needs the Visual C++ build tools and Windows SDK.
        if (-not (Get-Command link.exe -ErrorAction SilentlyContinue)) {
            Ensure-Chocolatey
            choco install -y visualstudio2022buildtools --no-progress `
                --execution-timeout=7200 `
                --package-parameters="--quiet --wait --norestart --nocache --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
            Update-Path
        }

        if (-not (Get-Command link.exe -ErrorAction SilentlyContinue)) {
            $link = Get-ChildItem "C:\Program Files (x86)\Microsoft Visual Studio" `
                -Recurse -Filter link.exe -ErrorAction SilentlyContinue |
                Where-Object { $_.FullName -match "Hostx64\\arm64" } |
                Select-Object -First 1
            if ($link) {
                Add-MachinePath $link.DirectoryName
            }
        }
    }

    $rustup = Join-Path $env:TEMP "rustup-init.exe"
    $url = "https://static.rust-lang.org/rustup/dist/$hostTriple/rustup-init.exe"
    Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $rustup
    & $rustup -y --profile minimal --default-host $hostTriple
    if ($LASTEXITCODE -ne 0) {
        throw "rustup-init failed with exit code $LASTEXITCODE"
    }

    $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
    Update-Path
}

Ensure-Rust

$archive = Join-Path $env:USERPROFILE "lethe-src.tar.gz"
$destination = Join-Path $env:USERPROFILE "lethe"

if (-not (Test-Path $archive)) {
    $found = Get-ChildItem "C:\Users" -Recurse -Depth 1 `
        -Filter "lethe-src.tar.gz" -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($found) {
        $archive = $found.FullName
        $destination = Join-Path $found.Directory.FullName "lethe"
    }
}

New-Item -ItemType Directory -Force -Path $destination | Out-Null
tar -xzf $archive -C $destination
if ($LASTEXITCODE -ne 0) {
    throw "Unable to extract $archive"
}

# Marks the guest as a disposable Lethe test VM. The test harness refuses to
# auto-select a disk without this marker.
Set-Content -Path (Join-Path $env:ProgramData "lethe-test-vm") -Value "1" -Encoding ASCII

Write-Host "Lethe test VM is ready."
