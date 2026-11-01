param(
    [switch]$Extended,
    [ValidateRange(0, 3600)]
    [int]$SustainedSeconds = 0
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    cargo build --release --jobs 1 -p fetchx-queue --features phase28-benchmark --example phase28_bench
    if ($LASTEXITCODE -ne 0) {
        throw "Phase 28 benchmark build failed with exit code $LASTEXITCODE"
    }

    $runner = Join-Path $repoRoot "target\release\examples\phase28_bench.exe"
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $runner
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $arguments = [System.Collections.Generic.List[string]]::new()
    if ($Extended) {
        $arguments.Add("--extended")
    }
    if ($SustainedSeconds -gt 0) {
        $arguments.Add("--sustained-seconds=$SustainedSeconds")
    }
    $startInfo.Arguments = $arguments -join " "
    $stageFile = Join-Path $env:TEMP ("fetchx-phase28-stage-{0}.txt" -f [Guid]::NewGuid())
    $env:PHASE28_STAGE_FILE = $stageFile
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    [void]$process.Start()
    $stdoutReader = $process.StandardOutput
    $stderrTask = $process.StandardError.ReadToEndAsync()

    $samples = [System.Collections.Generic.List[object]]::new()
    $logicalProcessors = [Environment]::ProcessorCount
    $stage = "process_startup"
    $lastPhysicalDiskSample = [DateTime]::MinValue
    $previousSample = $null
    $diskCounterErrors = 0
    $processIoErrors = 0
    $pendingLine = $stdoutReader.ReadLineAsync()
    while (-not $process.HasExited -or -not $pendingLine.IsCompleted) {
        if (Test-Path -LiteralPath $stageFile) {
            $reportedStage = Get-Content -LiteralPath $stageFile -Raw
            if (-not [string]::IsNullOrWhiteSpace($reportedStage)) {
                $stage = $reportedStage.Trim()
            }
        }
        if ($pendingLine.IsCompleted) {
            $line = $pendingLine.GetAwaiter().GetResult()
            if ($null -ne $line) {
                Write-Output $line
                if ($line -match '^STAGE name=(\S+)') {
                    $stage = $Matches[1]
                }
                $pendingLine = $stdoutReader.ReadLineAsync()
            }
        }
        $sample = Get-Process -Id $process.Id -ErrorAction SilentlyContinue
        if ($null -ne $sample) {
            $diskRead = $null
            $diskWrite = $null
            $processRead = $null
            $processWrite = $null
            try {
                $io = Get-CimInstance Win32_Process -Filter "ProcessId = $($process.Id)" -ErrorAction Stop
                $processRead = [double]$io.ReadTransferCount
                $processWrite = [double]$io.WriteTransferCount
            }
            catch {
                $processRead = $null
                $processWrite = $null
                $processIoErrors++
            }
            if (((Get-Date) - $lastPhysicalDiskSample).TotalSeconds -ge 1) {
                try {
                    $counter = Get-Counter '\PhysicalDisk(_Total)\Disk Read Bytes/sec','\PhysicalDisk(_Total)\Disk Write Bytes/sec' -ErrorAction Stop
                    $diskRead = ($counter.CounterSamples | Where-Object { $_.Path -match 'disk read bytes/sec$' } | Select-Object -First 1).CookedValue
                    $diskWrite = ($counter.CounterSamples | Where-Object { $_.Path -match 'disk write bytes/sec$' } | Select-Object -First 1).CookedValue
                }
                catch {
                    $diskRead = $null
                    $diskWrite = $null
                    $diskCounterErrors++
                }
                $lastPhysicalDiskSample = Get-Date
            }
            $now = Get-Date
            $cpuPercent = 0.0
            if ($null -ne $previousSample) {
                $interval = [Math]::Max(0.001, ($now - $previousSample.Time).TotalSeconds)
                $cpuPercent = 100.0 * ($sample.TotalProcessorTime.TotalSeconds - $previousSample.CpuSeconds) / ($interval * $logicalProcessors)
            }
            $currentSample = [pscustomobject]@{
                Time = $now
                Stage = $stage
                PrivateBytes = $sample.PrivateMemorySize64
                WorkingSetBytes = $sample.WorkingSet64
                CpuSeconds = $sample.TotalProcessorTime.TotalSeconds
                CpuPercent = $cpuPercent
                Handles = $sample.HandleCount
                Threads = $sample.Threads.Count
                DiskReadBytesPerSecond = $diskRead
                DiskWriteBytesPerSecond = $diskWrite
                ProcessReadBytes = $processRead
                ProcessWriteBytes = $processWrite
            }
            $samples.Add($currentSample)
            $previousSample = $currentSample
        }
        Start-Sleep -Milliseconds 250
        $process.Refresh()
    }
    $process.WaitForExit()
    $process.Refresh()
    $exitCode = $process.ExitCode

    $stderr = $stderrTask.GetAwaiter().GetResult()
    if ($stderr) {
        Write-Output $stderr
    }

    if ($samples.Count -gt 0) {
        $baseline = $samples[0]
        $peakPrivate = ($samples | Measure-Object -Property PrivateBytes -Maximum).Maximum
        $peakWorking = ($samples | Measure-Object -Property WorkingSetBytes -Maximum).Maximum
        $post = $samples[$samples.Count - 1]
        $elapsedSeconds = [Math]::Max(0.001, ($post.Time - $baseline.Time).TotalSeconds)
        $cpuSeconds = $post.CpuSeconds - $baseline.CpuSeconds
        $averageCpuPercent = 100.0 * $cpuSeconds / ($elapsedSeconds * $logicalProcessors)
        $diskReadSamples = @($samples | Where-Object { $null -ne $_.DiskReadBytesPerSecond } | Select-Object -ExpandProperty DiskReadBytesPerSecond)
        $diskWriteSamples = @($samples | Where-Object { $null -ne $_.DiskWriteBytesPerSecond } | Select-Object -ExpandProperty DiskWriteBytesPerSecond)
        $diskReadAverage = if ($diskReadSamples.Count) { ($diskReadSamples | Measure-Object -Average).Average } else { 0 }
        $diskWriteAverage = if ($diskWriteSamples.Count) { ($diskWriteSamples | Measure-Object -Average).Average } else { 0 }
        $diskReadPeak = if ($diskReadSamples.Count) { ($diskReadSamples | Measure-Object -Maximum).Maximum } else { 0 }
        $diskWritePeak = if ($diskWriteSamples.Count) { ($diskWriteSamples | Measure-Object -Maximum).Maximum } else { 0 }
        $processIoSamples = @($samples | Where-Object { $null -ne $_.ProcessReadBytes -and $null -ne $_.ProcessWriteBytes })
        $processReadBytes = if ($processIoSamples.Count -ge 2) { [Math]::Max(0, $processIoSamples[-1].ProcessReadBytes - $processIoSamples[0].ProcessReadBytes) } else { 0 }
        $processWriteBytes = if ($processIoSamples.Count -ge 2) { [Math]::Max(0, $processIoSamples[-1].ProcessWriteBytes - $processIoSamples[0].ProcessWriteBytes) } else { 0 }
        Write-Output ("PROCESS_RESOURCE baseline_private_mib={0:N2} peak_private_mib={1:N2} post_private_mib={2:N2} baseline_working_set_mib={3:N2} peak_working_set_mib={4:N2} post_working_set_mib={5:N2} avg_cpu_percent_all_cores={6:N2} peak_cpu_percent_all_cores={7:N2} peak_handles={8} peak_threads={9} samples={10} system_disk_read_avg_bytes_s={11:N0} system_disk_read_peak_bytes_s={12:N0} system_disk_write_avg_bytes_s={13:N0} system_disk_write_peak_bytes_s={14:N0} disk_counter_samples={15} disk_counter_errors={16} process_io_read_bytes={17:N0} process_io_write_bytes={18:N0} process_io_errors={19}" -f `
            ($baseline.PrivateBytes / 1MB), ($peakPrivate / 1MB), ($post.PrivateBytes / 1MB), `
            ($baseline.WorkingSetBytes / 1MB), ($peakWorking / 1MB), ($post.WorkingSetBytes / 1MB), `
            $averageCpuPercent, ($samples | Measure-Object -Property CpuPercent -Maximum).Maximum, `
            ($samples | Measure-Object -Property Handles -Maximum).Maximum, `
            ($samples | Measure-Object -Property Threads -Maximum).Maximum, $samples.Count, `
            $diskReadAverage, $diskReadPeak, $diskWriteAverage, $diskWritePeak, $diskReadSamples.Count, $diskCounterErrors, `
            $processReadBytes, $processWriteBytes, $processIoErrors)
        foreach ($group in ($samples | Group-Object -Property Stage)) {
            $stageSamples = @($group.Group)
            if ($stageSamples.Count -lt 2) {
                continue
            }
            $stageStart = $stageSamples[0]
            $stageEnd = $stageSamples[$stageSamples.Count - 1]
            $stageCpuAverage = ($stageSamples | Measure-Object -Property CpuPercent -Average).Average
            $stageCpuPeak = ($stageSamples | Measure-Object -Property CpuPercent -Maximum).Maximum
            $stagePeakMemory = ($stageSamples | Measure-Object -Property WorkingSetBytes -Maximum).Maximum / 1MB
            $stageDiskReads = @($stageSamples | Where-Object { $null -ne $_.DiskReadBytesPerSecond } | Select-Object -ExpandProperty DiskReadBytesPerSecond)
            $stageDiskWrites = @($stageSamples | Where-Object { $null -ne $_.DiskWriteBytesPerSecond } | Select-Object -ExpandProperty DiskWriteBytesPerSecond)
            $stageDiskReadAverage = if ($stageDiskReads.Count) { ($stageDiskReads | Measure-Object -Average).Average } else { 0 }
            $stageDiskWriteAverage = if ($stageDiskWrites.Count) { ($stageDiskWrites | Measure-Object -Average).Average } else { 0 }
            Write-Output ("PROCESS_STAGE name={0} samples={1} avg_cpu_percent_all_cores={2:N2} peak_cpu_percent_all_cores={3:N2} peak_working_set_mib={4:N2} system_disk_read_avg_bytes_s={5:N0} system_disk_write_avg_bytes_s={6:N0}" -f `
                $group.Name, $stageSamples.Count, $stageCpuAverage, $stageCpuPeak, $stagePeakMemory, $stageDiskReadAverage, $stageDiskWriteAverage)
        }
    }

    if ($exitCode -ne 0) {
        throw "Phase 28 benchmark runner failed with exit code $exitCode"
    }
}
finally {
    Remove-Item Env:PHASE28_STAGE_FILE -ErrorAction SilentlyContinue
    if ($stageFile -and (Test-Path -LiteralPath $stageFile)) {
        Remove-Item -LiteralPath $stageFile -Force
    }
    Pop-Location
}
