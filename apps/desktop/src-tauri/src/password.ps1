$ErrorActionPreference='Stop'; Add-Type -AssemblyName PresentationFramework; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$w=New-Object Windows.Window; $w.Title='Hostlane · SSH'; $w.SizeToContent='WidthAndHeight'; $w.WindowStartupLocation='CenterScreen'; $w.Topmost=$true
$panel=New-Object Windows.Controls.StackPanel; $panel.Margin=18; $panel.Width=360; $w.Content=$panel
$label=New-Object Windows.Controls.TextBlock; $label.Text=$env:HOSTLANE_DIALOG_LABEL; $label.TextWrapping='Wrap'; [void]$panel.Children.Add($label)
$box=New-Object Windows.Controls.PasswordBox; [void]$panel.Children.Add($box)
$ok=New-Object Windows.Controls.Button; $ok.Content='保存'; $ok.IsDefault=$true; $ok.Add_Click({$w.DialogResult=$true}); [void]$panel.Children.Add($ok)
$cancel=New-Object Windows.Controls.Button; $cancel.Content='取消'; $cancel.IsCancel=$true; [void]$panel.Children.Add($cancel); $w.Add_ContentRendered({[void]$box.Focus()})
if ($w.ShowDialog() -eq $true) {[Console]::WriteLine($box.Password)}; $box.Clear()
