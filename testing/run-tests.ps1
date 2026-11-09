$ErrorActionPreference = "Stop"

$RepoDir = Split-Path -Parent $PSScriptRoot
Set-Location $RepoDir

$Bin = if ($env:LETHE_BIN) {
    $env:LETHE_BIN
} else {
    Join-Path $RepoDir "target\release\lethe.exe"
}
$BlockSize = if ($env:LETHE_TEST_BLOCK_SIZE) { $env:LETHE_TEST_BLOCK_SIZE } else { "1m" }
$ExpectedBytes = if ($env:LETHE_TEST_DISK_BYTES) { [int64]$env:LETHE_TEST_DISK_BYTES } else { 1GB }

function Assert-Administrator {
    $principal = New-Object Security.Principal.WindowsPrincipal(
        [Security.Principal.WindowsIdentity]::GetCurrent())
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "Run this script from an elevated (Administrator) shell."
    }
}

function Find-TestDisk {
    $minimum = [int64]($ExpectedBytes * 0.8)
    $maximum = [int64]($ExpectedBytes * 1.2)

    $disks = @(Get-Disk | Where-Object {
        -not $_.IsBoot -and
        -not $_.IsSystem -and
        -not $_.IsReadOnly -and
        $_.Size -ge $minimum -and
        $_.Size -le $maximum
    })

    if ($disks.Count -eq 0) {
        throw "No detached ~$ExpectedBytes-byte disk found."
    }
    if ($disks.Count -gt 1) {
        throw "Multiple candidate disks found. Set LETHE_TEST_DEVICE explicitly."
    }

    $disk = $disks[0]
    if ($disk.IsOffline) {
        Set-Disk -Number $disk.Number -IsOffline $false
        $disk = Get-Disk -Number $disk.Number
    }
    return $disk
}

function Open-RawDisk {
    param([string]$Path)
    return [System.IO.File]::Open(
        $Path,
        [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::ReadWrite,
        [System.IO.FileShare]::ReadWrite)
}

function Get-SampleOffsets {
    param([int64]$Size, [int]$Sample)

    if ($Size -le $Sample) {
        return @([int64]0)
    }

    $alignment = 4096
    $last = [int64]([Math]::Floor(($Size - $Sample) / $alignment) * $alignment)
    $middle = [int64]([Math]::Floor((($Size - $Sample) / 2) / $alignment) * $alignment)
    return @([int64]0, $middle, $last) | Select-Object -Unique
}

function Test-ZeroRegion {
    param(
        [System.IO.FileStream]$Stream,
        [int64]$Offset,
        [int]$Length
    )

    $buffer = New-Object byte[] $Length
    $Stream.Position = $Offset

    $read = 0
    while ($read -lt $Length) {
        $n = $Stream.Read($buffer, $read, $Length - $read)
        if ($n -le 0) {
            throw "Unexpected end of device while reading $Length bytes at offset $Offset."
        }
        $read += $n
    }

    $zeros = New-Object byte[] $Length
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $actual = [BitConverter]::ToString($sha.ComputeHash($buffer))
        $expected = [BitConverter]::ToString($sha.ComputeHash($zeros))
        return $actual -eq $expected
    } finally {
        $sha.Dispose()
    }
}

function Assert-ZeroDisk {
    param([string]$Path, [int64]$Size)

    $stream = Open-RawDisk $Path
    try {
        $sample = [int][Math]::Min(4MB, $Size)
        foreach ($offset in Get-SampleOffsets -Size $Size -Sample $sample) {
            if (-not (Test-ZeroRegion -Stream $stream -Offset $offset -Length $sample)) {
                throw "Non-zero data found at offset $offset."
            }
        }
    } finally {
        $stream.Dispose()
    }
}

function Assert-NotZeroDisk {
    param([string]$Path, [int64]$Size)

    $stream = Open-RawDisk $Path
    try {
        $sample = [int][Math]::Min(4MB, $Size)
        foreach ($offset in Get-SampleOffsets -Size $Size -Sample $sample) {
            if (-not (Test-ZeroRegion -Stream $stream -Offset $offset -Length $sample)) {
                return
            }
        }
        throw "The device is still all zeroes after the random wipe."
    } finally {
        $stream.Dispose()
    }
}

function Fill-Random {
    param([string]$Path, [int64]$Size)

    $stream = Open-RawDisk $Path
    try {
        $chunkSize = 1MB
        $buffer = New-Object byte[] $chunkSize
        $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
        try {
            $remaining = $Size
            while ($remaining -gt 0) {
                $count = [int][Math]::Min($chunkSize, $remaining)
                $rng.GetBytes($buffer)
                $stream.Write($buffer, 0, $count)
                $remaining -= $count
            }
            $stream.Flush($true)
        } finally {
            $rng.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
}

Assert-Administrator

$disk = $null
if ($env:LETHE_TEST_DEVICE) {
    if ($env:LETHE_TEST_DEVICE -notmatch 'PhysicalDrive(\d+)') {
        throw "LETHE_TEST_DEVICE must look like \\.\PhysicalDriveN."
    }
    $disk = Get-Disk -Number ([int]$Matches[1])
} elseif (Test-Path (Join-Path $env:ProgramData "lethe-test-vm")) {
    $disk = Find-TestDisk
} else {
    throw "Refusing to guess a disk outside a Lethe test VM. Set LETHE_TEST_DEVICE=\\.\PhysicalDriveN."
}

$DeviceId = "\\.\PhysicalDrive$($disk.Number)"
$DeviceSize = [int64]$disk.Size

$cargo = Get-Command cargo.exe -ErrorAction SilentlyContinue
if ($cargo) {
    $CargoExe = $cargo.Source
} else {
    $CargoExe = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
}
if (-not (Test-Path $CargoExe)) {
    throw "cargo was not found. Provision the VM first (just vm-up <platform>)."
}

if (-not (Get-Command gcc.exe -ErrorAction SilentlyContinue)) {
    $gccCandidates = @(Get-ChildItem "C:\ProgramData\chocolatey\lib\mingw" `
        -Recurse -Filter gcc.exe -ErrorAction SilentlyContinue)
    $gcc = $gccCandidates |
        Where-Object { $_.FullName -match "mingw64" } |
        Select-Object -First 1
    if (-not $gcc) {
        $gcc = $gccCandidates | Select-Object -First 1
    }
    if ($gcc) {
        $env:Path = "$($gcc.DirectoryName);$env:Path"
    }
}

Write-Host "==> Building Lethe"
& $CargoExe build --locked --release
if ($LASTEXITCODE -ne 0) {
    throw "cargo build failed with exit code $LASTEXITCODE"
}

Write-Host "==> Device list ($DeviceId)"
$list = & $Bin list 2>&1 | Out-String
Write-Host $list
if ($LASTEXITCODE -ne 0) {
    throw "lethe list failed with exit code $LASTEXITCODE"
}
if (-not $list.Contains($DeviceId)) {
    throw "Device $DeviceId is not reported by lethe list."
}

Write-Host "==> Filling $DeviceId with non-zero data"
Fill-Random -Path $DeviceId -Size $DeviceSize

Write-Host "==> Zero wipe with full verification"
& $Bin wipe --scheme=zero --verify=all --blocksize=$BlockSize --yes $DeviceId
if ($LASTEXITCODE -ne 0) {
    throw "Zero wipe failed with exit code $LASTEXITCODE"
}

Write-Host "==> Independently checking that $DeviceId is all zeroes"
Assert-ZeroDisk -Path $DeviceId -Size $DeviceSize

Write-Host "==> Random wipe with partial verification"
& $Bin wipe --scheme=random --verify=last --coverage=10 --blocksize=$BlockSize --yes $DeviceId
if ($LASTEXITCODE -ne 0) {
    throw "Random wipe failed with exit code $LASTEXITCODE"
}

Write-Host "==> Independently checking that $DeviceId is no longer all zeroes"
Assert-NotZeroDisk -Path $DeviceId -Size $DeviceSize

Write-Host "PASS: $DeviceId was wiped and verified successfully."
