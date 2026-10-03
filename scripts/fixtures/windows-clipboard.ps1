param([string]$Image, [string]$File, [string]$Ready, [string]$Done)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$previous = [System.Windows.Forms.Clipboard]::GetDataObject()
$bitmap = $null
try {
    if ($Image) {
        $bitmap = [System.Drawing.Image]::FromFile($Image)
        [System.Windows.Forms.Clipboard]::SetImage($bitmap)
    } else {
        $files = [System.Collections.Specialized.StringCollection]::new()
        [void]$files.Add($File)
        [System.Windows.Forms.Clipboard]::SetFileDropList($files)
    }
    [IO.File]::WriteAllText($Ready, 'ready')
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while (-not [IO.File]::Exists($Done) -and [DateTime]::UtcNow -lt $deadline) {
        [System.Windows.Forms.Application]::DoEvents()
        Start-Sleep -Milliseconds 100
    }
    if (-not [IO.File]::Exists($Done)) { throw 'Clipboard acceptance did not finish' }
} finally {
    if ($previous) { [System.Windows.Forms.Clipboard]::SetDataObject($previous, $true) }
    else { [System.Windows.Forms.Clipboard]::Clear() }
    if ($bitmap) { $bitmap.Dispose() }
}
