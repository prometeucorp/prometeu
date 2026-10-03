param([Parameter(Mandatory=$true)][int]$AppId, [Parameter(Mandatory=$true)][string]$Directory, [string]$Title = 'Choose the repository', [switch]$File)
$ErrorActionPreference = 'Stop'
trap { [Console]::Error.WriteLine($_.ToString()); [Console]::Error.WriteLine($_.ScriptStackTrace); exit 1 }
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class DialogMessages {
    public delegate bool ChildCallback(IntPtr window, IntPtr data);
    [DllImport("user32.dll")]
    public static extern bool EnumChildWindows(IntPtr window, ChildCallback callback, IntPtr data);
    [DllImport("user32.dll")]
    public static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetClassName(IntPtr window, System.Text.StringBuilder name, int size);
    [DllImport("user32.dll")]
    public static extern IntPtr GetDlgItem(IntPtr window, int id);
    [DllImport("user32.dll", SetLastError=true)]
    public static extern bool PostMessage(IntPtr window, uint message, IntPtr param, IntPtr data);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr param, string text, uint flags, uint timeout, out IntPtr result);
}
'@
# Scope all automation to this acceptance run's process and its real folder dialog.
$root = [System.Windows.Automation.AutomationElement]::RootElement
$process = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $AppId)
$class = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ClassNameProperty, '#32770')
$deadline = [DateTime]::UtcNow.AddSeconds(20)
$dialog = $null
while (-not $dialog -and [DateTime]::UtcNow -lt $deadline) {
    foreach ($window in $root.FindAll([System.Windows.Automation.TreeScope]::Children, $process)) {
        $candidate = $window.FindFirst([System.Windows.Automation.TreeScope]::Subtree, $class)
        if ($candidate -and $candidate.Current.Name -eq $Title) { $dialog = $candidate; break }
    }
    if (-not $dialog) { Start-Sleep -Milliseconds 100 }
}
if (-not $dialog) { throw 'Native directory dialog was not found' }
function Control([string]$id) {
    $condition = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::AutomationIdProperty, $id)
    return $dialog.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
}
$edit = $null
$button = $null
while ([DateTime]::UtcNow -lt $deadline) {
    if ($File) {
        $script:fileEdit = [IntPtr]::Zero
        [void][DialogMessages]::EnumChildWindows([IntPtr]$dialog.Current.NativeWindowHandle, {
            param($window, $data)
            $name = [System.Text.StringBuilder]::new(256)
            [void][DialogMessages]::GetClassName($window, $name, 256)
            if ([DialogMessages]::GetDlgCtrlID($window) -eq 1148 -and $name.ToString() -eq 'Edit') { $script:fileEdit = $window }
            return $true
        }, [IntPtr]::Zero)
        $edit = $script:fileEdit
    } else { $edit = Control '1152' }
    $button = [DialogMessages]::GetDlgItem([IntPtr]$dialog.Current.NativeWindowHandle, 1)
    if ($edit -and $button -ne [IntPtr]::Zero) { break }
    Start-Sleep -Milliseconds 100
}
if (-not $edit -or $button -eq [IntPtr]::Zero) { throw 'Native selection controls were not found' }
$editWindow = if ($File) { $edit } else { [IntPtr]$edit.Current.NativeWindowHandle }
$result = [IntPtr]::Zero
$ok = [DialogMessages]::SendMessageTimeout($editWindow, 0x000C, [IntPtr]::Zero, $Directory, 2, 5000, [ref]$result)
if ($ok -eq [IntPtr]::Zero) { throw 'Could not enter the directory in the native dialog' }
function Confirm-Directory {
    if ($dialog.Current.ProcessId -ne $AppId) { throw 'Unexpected native directory dialog owner' }
    if (-not [DialogMessages]::PostMessage([IntPtr]$dialog.Current.NativeWindowHandle, 0x0111, [IntPtr]1, $button)) { throw 'Could not confirm the native directory' }
}
Confirm-Directory
if ($File) { exit 0 }
# Entering an absolute folder first navigates into it. Confirm the resulting
# folder name before confirming again to choose that directory.
$leaf = [IO.Path]::GetFileName($Directory.TrimEnd('\'))
$deadline = [DateTime]::UtcNow.AddSeconds(10)
while ((Control '1152').Current.Name -ne $leaf -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
if ((Control '1152').Current.Name -ne $leaf) { throw 'Native dialog did not navigate to the selected directory' }
Confirm-Directory
