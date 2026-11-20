param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [Parameter(Mandatory = $true)][string]$Version
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-MsiRow($Database, [string]$Query, [int]$ColumnCount) {
    $view = $Database.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $Database, @($Query))
    $record = $null
    try {
        [void]$view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null)
        $record = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)
        if ($null -eq $record) {
            throw "Missing MSI row for $Query"
        }
        foreach ($index in 1..$ColumnCount) {
            $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, @($index))
        }
    } finally {
        if ($null -ne $record) {
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($record)
        }
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($view)
    }
}

function Assert-X64Executable([string]$Path) {
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($Path))
    try {
        # Read the PE signature and machine field at the offset in the DOS header.
        if ($reader.ReadUInt16() -ne 0x5A4D) {
            throw "Missing DOS header in $Path"
        }
        $reader.BaseStream.Position = 0x3C
        $reader.BaseStream.Position = $reader.ReadUInt32()
        if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) {
            throw "Expected an x64 Windows executable at $Path"
        }
    } finally {
        $reader.Dispose()
    }
}

$Installer = (Get-Item -LiteralPath $Installer).FullName
$windowsInstaller = New-Object -ComObject WindowsInstaller.Installer
$database = $null
try {
    $database = $windowsInstaller.GetType().InvokeMember('OpenDatabase', 'InvokeMethod', $null, $windowsInstaller, @($Installer, 0))

    $pathName, $pathValue, $pathComponent = Get-MsiRow $database `
        'SELECT `Name`, `Value`, `Component_` FROM `Environment` WHERE `Environment` = ''StrudelandCliPath''' 3
    # The MSI prefixes mean set on install, remove on uninstall, and system scope.
    if ($pathName -ne '=-*PATH' -or $pathValue -cne '[~];[StrudelandCliDir]' -or $pathComponent -cne 'StrudelandCli') {
        throw 'Expected the MSI to append the CLI directory to the system PATH and remove that entry on uninstall'
    }

    $directory, $keyFile = Get-MsiRow $database `
        'SELECT `Directory_`, `KeyPath` FROM `Component` WHERE `Component` = ''StrudelandCli''' 2
    $directoryParent, $directoryName = Get-MsiRow $database `
        'SELECT `Directory_Parent`, `DefaultDir` FROM `Directory` WHERE `Directory` = ''StrudelandCliDir''' 2
    $cliComponent, $cliFile = Get-MsiRow $database `
        'SELECT `Component_`, `FileName` FROM `File` WHERE `File` = ''StrudelandCliExe''' 2
    if ($directory -cne 'StrudelandCliDir' -or $keyFile -cne 'StrudelandCliExe' -or
        $directoryParent -cne 'ProgramFiles64Folder' -or $directoryName.Split('|')[-1] -cne 'Strudeland CLI' -or
        $cliComponent -cne 'StrudelandCli' -or $cliFile.Split('|')[-1] -cne 'strudeland.exe') {
        throw 'Expected the CLI component at Program Files\Strudeland CLI\strudeland.exe'
    }

    # Tauri's stock MSI template names the GUI File table row Path.
    $guiFile = Get-MsiRow $database 'SELECT `FileName` FROM `File` WHERE `File` = ''Path''' 1
    $guiName = $guiFile.Split('|')[-1]
} finally {
    if ($null -ne $database) {
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($database)
    }
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($windowsInstaller)
}

# An administrative image extracts the payload without installing the app or modifying PATH.
$extract = Join-Path ([IO.Path]::GetTempPath()) ('strudeland-msi-' + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $extract)
try {
    $log = Join-Path $extract 'msiexec.log'
    $arguments = '/a "{0}" /qn /norestart TARGETDIR="{1}" /L*V "{2}"' -f $Installer, $extract, $log
    $process = Start-Process -FilePath msiexec.exe -ArgumentList $arguments -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        if (Test-Path -LiteralPath $log) {
            Get-Content -LiteralPath $log -Tail 100
        }
        throw "MSI extraction exited with code $($process.ExitCode)"
    }

    $cli = @(Get-ChildItem -LiteralPath $extract -Recurse -File -Filter 'strudeland.exe' |
        Where-Object { $_.Directory.Name -ceq 'Strudeland CLI' })
    $gui = @(Get-ChildItem -LiteralPath $extract -Recurse -File -Filter $guiName |
        Where-Object { $_.Directory.Name -eq 'strudeland' })
    if ($cli.Count -ne 1 -or $gui.Count -ne 1) {
        throw 'Expected separate CLI and GUI executables in the MSI payload'
    }
    Assert-X64Executable $cli[0].FullName
    Assert-X64Executable $gui[0].FullName
    if ((Get-FileHash -LiteralPath $cli[0].FullName).Hash -eq (Get-FileHash -LiteralPath $gui[0].FullName).Hash) {
        throw 'The MSI packages identical content as the CLI and GUI'
    }

    $actualVersion = & $cli[0].FullName --version
    if ($LASTEXITCODE -ne 0 -or ($actualVersion -join "`n") -cne "strudeland $Version") {
        throw "Expected strudeland $Version from the packaged CLI, got $actualVersion"
    }
    $docs = & $cli[0].FullName docs --list
    if ($LASTEXITCODE -ne 0 -or $docs -cnotcontains 'fast') {
        throw 'The packaged CLI failed the docs --list smoke test'
    }
    Write-Output "Verified the x64 strudeland $Version CLI in the MSI"
} finally {
    Remove-Item -LiteralPath $extract -Recurse -Force
}
