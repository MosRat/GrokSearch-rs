<# 
Orchestrate a normal release through GitHub Actions.

Usage:
  powershell -NoProfile -ExecutionPolicy Bypass -File scripts/release-via-workflows.ps1 0.1.25

Flow:
  1. Dispatch bump.yml on main with the requested version.
  2. Wait for bump.yml to commit manifests and push vX.Y.Z.
  3. Dispatch release.yml on vX.Y.Z, because tags pushed by the manually
     dispatched bump workflow do not start release.yml reliably.
  4. Wait for release.yml to finish and print the GitHub Release URL.
#>

param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidatePattern('^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$')]
    [string] $Version,

    [string] $Branch = "main",

    [int] $TagTimeoutSeconds = 180,

    [switch] $NoWatch
)

$ErrorActionPreference = "Stop"

function Invoke-GhJson {
    param(
        [Parameter(Mandatory = $true)]
        [string[]] $Arguments
    )

    $output = & gh @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "gh $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
    if ([string]::IsNullOrWhiteSpace($output)) {
        return $null
    }
    return $output | ConvertFrom-Json
}

function Invoke-Gh {
    param(
        [Parameter(Mandatory = $true)]
        [string[]] $Arguments
    )

    $output = & gh @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "gh $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
    return $output
}

function Get-RunIdFromWorkflowRunOutput {
    param([string] $Output)

    if ($Output -match '/actions/runs/(\d+)') {
        return $Matches[1]
    }
    return $null
}

function Get-NewestWorkflowRunId {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Workflow,

        [Parameter(Mandatory = $true)]
        [datetime] $StartedAfter,

        [string] $HeadBranch
    )

    $runs = Invoke-GhJson @(
        "run", "list",
        "--workflow", $Workflow,
        "--limit", "20",
        "--json", "databaseId,createdAt,event,headBranch,status,displayTitle"
    )

    $filtered = @($runs | Where-Object {
        ([datetime] $_.createdAt) -ge $StartedAfter.AddSeconds(-10) -and
        $_.event -eq "workflow_dispatch" -and
        ([string]::IsNullOrEmpty($HeadBranch) -or $_.headBranch -eq $HeadBranch)
    } | Sort-Object { [datetime] $_.createdAt } -Descending)

    if ($filtered.Count -eq 0) {
        throw "Could not find a new workflow_dispatch run for $Workflow."
    }

    return [string] $filtered[0].databaseId
}

function Find-NewestWorkflowRunId {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Workflow,

        [Parameter(Mandatory = $true)]
        [datetime] $StartedAfter,

        [Parameter(Mandatory = $true)]
        [string] $HeadBranch
    )

    $runs = Invoke-GhJson @(
        "run", "list",
        "--workflow", $Workflow,
        "--limit", "20",
        "--json", "databaseId,createdAt,headBranch,status,event"
    )

    $filtered = @($runs | Where-Object {
        ([datetime] $_.createdAt) -ge $StartedAfter.AddSeconds(-10) -and
        $_.headBranch -eq $HeadBranch
    } | Sort-Object { [datetime] $_.createdAt } -Descending)

    if ($filtered.Count -eq 0) {
        return $null
    }

    return [string] $filtered[0].databaseId
}

function Wait-ForRun {
    param(
        [Parameter(Mandatory = $true)]
        [string] $RunId
    )

    if ($NoWatch) {
        Write-Host "Run started: $RunId"
        return
    }

    Invoke-Gh @("run", "watch", $RunId, "--exit-status", "--interval", "30") | Out-Host
}

function Test-RemoteTagExists {
    param([string] $Tag)

    $output = & git ls-remote --tags origin "refs/tags/$Tag"
    if ($LASTEXITCODE -ne 0) {
        throw "git ls-remote failed with exit code $LASTEXITCODE"
    }
    return -not [string]::IsNullOrWhiteSpace($output)
}

$tag = "v$Version"

Invoke-Gh @("auth", "status") | Out-Host

if (Test-RemoteTagExists $tag) {
    throw "Tag $tag already exists on origin."
}

Write-Host "Dispatching Bump Version for $Version on $Branch..."
$bumpStartedAt = Get-Date
$bumpOutput = Invoke-Gh @(
    "workflow", "run", "bump.yml",
    "--ref", $Branch,
    "-f", "version=$Version",
    "-f", "dry_run=false"
)
$bumpOutput | Out-Host

$bumpRunId = Get-RunIdFromWorkflowRunOutput $bumpOutput
if (-not $bumpRunId) {
    Start-Sleep -Seconds 5
    $bumpRunId = Get-NewestWorkflowRunId -Workflow "bump.yml" -StartedAfter $bumpStartedAt -HeadBranch $Branch
}

Write-Host "Waiting for Bump Version run $bumpRunId..."
Wait-ForRun $bumpRunId

Write-Host "Waiting for remote tag $tag..."
$deadline = (Get-Date).AddSeconds($TagTimeoutSeconds)
while ((Get-Date) -lt $deadline) {
    if (Test-RemoteTagExists $tag) {
        break
    }
    Start-Sleep -Seconds 5
}

if (-not (Test-RemoteTagExists $tag)) {
    throw "Timed out waiting for origin tag $tag."
}

$releaseStartedAt = Get-Date
$releaseRunId = Find-NewestWorkflowRunId -Workflow "release.yml" -StartedAfter $bumpStartedAt -HeadBranch $tag

if ($releaseRunId) {
    Write-Host "Release workflow already started for ${tag}: $releaseRunId"
} else {
    Write-Host "Dispatching Release for $tag..."
    $releaseOutput = Invoke-Gh @(
        "workflow", "run", "release.yml",
        "--ref", $tag
    )
    $releaseOutput | Out-Host

    $releaseRunId = Get-RunIdFromWorkflowRunOutput $releaseOutput
    if (-not $releaseRunId) {
        Start-Sleep -Seconds 5
        $releaseRunId = Get-NewestWorkflowRunId -Workflow "release.yml" -StartedAfter $releaseStartedAt -HeadBranch $tag
    }
}

Write-Host "Waiting for Release run $releaseRunId..."
Wait-ForRun $releaseRunId

if (-not $NoWatch) {
    $release = Invoke-GhJson @("release", "view", $tag, "--json", "url,tagName,publishedAt")
    Write-Host "Release complete: $($release.url)"
}
