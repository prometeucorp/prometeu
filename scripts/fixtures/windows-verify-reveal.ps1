param([Parameter(Mandatory=$true)][string]$Directory, [string]$Selected = '')
$ErrorActionPreference = 'Stop'
trap { [Console]::Error.WriteLine($_.ToString()); exit 1 }
$shell = New-Object -ComObject Shell.Application
$deadline = [DateTime]::UtcNow.AddSeconds(15)
while ([DateTime]::UtcNow -lt $deadline) {
    foreach ($window in $shell.Windows()) {
        try {
            if ($window.Document.Folder.Self.Path -ne $Directory) { continue }
            if ($Selected) {
                $items = @($window.Document.SelectedItems() | ForEach-Object { $_.Path })
                if ($items -notcontains (Join-Path $Directory $Selected)) { continue }
            }
            # Only the acceptance run's unique temporary directory is eligible for cleanup.
            $window.Quit()
            [Console]::WriteLine('Verified native Explorer location and selection')
            exit 0
        } catch { continue }
    }
    Start-Sleep -Milliseconds 100
}
throw 'Explorer did not show the requested directory and selection'
