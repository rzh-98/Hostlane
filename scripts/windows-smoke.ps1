$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$work = Join-Path $env:RUNNER_TEMP ('Hostlane Test ' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $work | Out-Null
$env:HOSTLANE_DATA_DIR = Join-Path $work 'data'
$env:HOSTLANE_SSH_CONFIG = Join-Path $work 'ssh_config'
$env:HOSTLANE_NOTIFY = '0'
Set-Content -Encoding ascii $env:HOSTLANE_SSH_CONFIG "Host windows-smoke`n  HostName localhost"
$bin = (Resolve-Path target/debug/hostlane.exe).Path
$registered = $false
try {
    $pipe = (& $bin daemon socket-path).Trim()
    if (!$pipe.StartsWith('\\.\pipe\hostlane-')) { throw 'Not a local per-user pipe' }
    & $bin daemon start
    $status = & $bin --json status | ConvertFrom-Json
    if ($status.data_dir -ne $env:HOSTLANE_DATA_DIR) { throw 'Wrong data directory' }
    $hostsResult = & $bin --json hosts | ConvertFrom-Json
    if ($hostsResult.hosts[0].id -ne 'windows-smoke') { throw 'Unexpected SSH configuration' }
    $frame = '{"jsonrpc":"2.0","id":4,"method":"daemon.status"}'
    $reply = $frame | & $bin daemon exchange | ConvertFrom-Json
    if ($reply.result.pid -ne $status.pid) { throw 'Desktop pipe bridge failed' }
    $xml = [xml]((& $bin daemon install --print) -join "`n")
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $runner = (Resolve-Path target/debug/hostlane-background.exe).Path
    if ($xml.Task.Actions.Exec.Command -ne $runner) { throw 'Login task must not launch the console CLI' }
    $image = [IO.File]::ReadAllBytes($runner)
    $pe = [BitConverter]::ToInt32($image, 0x3c)
    if ([BitConverter]::ToUInt16($image, $pe + 24 + 68) -ne 2) { throw 'Login task host is not a GUI-subsystem executable' }
    if ($xml.Task.Principals.Principal.UserId -ne $sid -or $xml.Task.Principals.Principal.RunLevel -ne 'LeastPrivilege') { throw 'Incorrect task principal' }
    $registered = $true
    & $bin daemon install
    $service = & $bin --json daemon service | ConvertFrom-Json
    if (!$service.installed -or $service.manager -ne 'scheduler') { throw 'Task was not installed' }
    $scheduled = & $bin --json status | ConvertFrom-Json
    if ((Get-Process -Id $scheduled.pid).Path -ne $runner) { throw 'Daemon is not running in the console-free host' }
    if ($scheduled.data_dir -ne $env:HOSTLANE_DATA_DIR) { throw 'Scheduled daemon lost its data directory' }
    & $bin daemon uninstall
    $registered = $false
    & $bin daemon start
    & $bin daemon stop
} finally {
    $PSNativeCommandUseErrorActionPreference = $false
    if ($registered) { & $bin daemon uninstall }
    & $bin daemon stop 2>$null | Out-Null
    $global:LASTEXITCODE = 0
}
