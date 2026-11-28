$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$key=[Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
try {
  $p=[string]$key.GetValue('Path','','DoNotExpandEnvironmentNames'); $dir=$env:HOSTLANE_PATH_ENTRY
  if ($dir -and -not (($p -split ';' | ForEach-Object { $_.TrimEnd('\') }) -contains $dir.TrimEnd('\'))) {
    $kind=if($key.GetValueNames() -contains 'Path'){$key.GetValueKind('Path')}else{'ExpandString'}
    $p=($p.TrimEnd(';')+';'+$dir).TrimStart(';'); $key.SetValue('Path',$p,$kind)
  }; [Console]::Write([Environment]::ExpandEnvironmentVariables($p))
} finally { $key.Close() }
