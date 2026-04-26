#!/usr/bin/env pwsh
#Requires -Version 7.0
<#
.SYNOPSIS
    Prepare and cut a Toku release.
.DESCRIPTION
    Automates the release process:
    1. Reads current version from workspace Cargo.toml
    2. Bumps the specified semver component (major, minor, or patch)
    3. Updates version in all workspace member Cargo.toml files
    4. Validates there are unreleased changelog entries
    5. Stamps the [Unreleased] section in CHANGELOG.md with version and date
    6. Runs cargo check to update Cargo.lock
    7. Commits, tags, and (optionally) pushes

    The release workflow (.github/workflows/release.yml) then:
    - Builds cross-platform binaries (Linux x86_64/aarch64, macOS x86_64/aarch64, Windows)
    - Creates a GitHub Release with binaries and SHA-256 checksums
    - Publishes all crates to crates.io in dependency order
.PARAMETER Bump
    Which semver component to bump: major, minor, or patch.
.PARAMETER Push
    Push the commit and tag to origin after creating them.
.PARAMETER DryRun
    Show what would happen without making changes.
.EXAMPLE
    ./scripts/release.ps1 patch
    ./scripts/release.ps1 minor -Push
    ./scripts/release.ps1 major -DryRun
#>
param(
    [Parameter(Mandatory, Position = 0)]
    [ValidateSet("major", "minor", "patch")]
    [string]$Bump,

    [switch]$Push,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$RepoRoot = git rev-parse --show-toplevel 2>$null
if (-not $RepoRoot) { Write-Error "Not in a git repository"; exit 1 }
Set-Location $RepoRoot

# --- Workspace crate members (in publish order) ---

$WorkspaceMembers = @(
    "crates/toku-core",
    "crates/toku-db",
    "crates/toku-import",
    "crates/toku-meta",
    "crates/toku-export",
    "crates/toku-cli"
)

# --- Read and bump version from workspace root ---

$cargoToml = Get-Content Cargo.toml -Raw
$currentMatch = [regex]::Match($cargoToml, '(?m)^version = "(\d+)\.(\d+)\.(\d+)"')
if (-not $currentMatch.Success) {
    Write-Error "Could not parse version from Cargo.toml"
    exit 1
}

$major = [int]$currentMatch.Groups[1].Value
$minor = [int]$currentMatch.Groups[2].Value
$patch = [int]$currentMatch.Groups[3].Value
$currentVersion = "$major.$minor.$patch"

switch ($Bump) {
    "major" { $major++; $minor = 0; $patch = 0 }
    "minor" { $minor++; $patch = 0 }
    "patch" { $patch++ }
}

$Version = "$major.$minor.$patch"
$Tag = "v$Version"
$Today = Get-Date -Format "yyyy-MM-dd"

Write-Host "`n📦 Release: $currentVersion → $Version ($Bump bump)" -ForegroundColor Cyan

# --- Preflight checks ---

Write-Host "`n🔍 Preflight checks" -ForegroundColor Cyan

# Clean working tree
$status = git status --porcelain
if ($status) {
    Write-Error "Working tree is not clean. Commit or stash changes first."
    exit 1
}
Write-Host "  ✓ Working tree clean" -ForegroundColor Green

# On main branch
$branch = git branch --show-current
if ($branch -ne "main") {
    Write-Error "Must be on 'main' branch (currently on '$branch')."
    exit 1
}
Write-Host "  ✓ On main branch" -ForegroundColor Green

# Tag doesn't already exist
$existing = git tag -l $Tag
if ($existing) {
    Write-Error "Tag '$Tag' already exists."
    exit 1
}
Write-Host "  ✓ Tag $Tag is available" -ForegroundColor Green

# Changelog has unreleased entries
$changelog = Get-Content CHANGELOG.md -Raw
if ($changelog -notmatch '## \[Unreleased\]\s*\n+### ') {
    Write-Error "No entries found under [Unreleased] in CHANGELOG.md."
    exit 1
}
Write-Host "  ✓ Changelog has unreleased entries" -ForegroundColor Green

# All workspace member Cargo.toml files exist
foreach ($member in $WorkspaceMembers) {
    $memberToml = Join-Path $member "Cargo.toml"
    if (-not (Test-Path $memberToml)) {
        Write-Error "Workspace member '$memberToml' not found."
        exit 1
    }
}
Write-Host "  ✓ All workspace members found" -ForegroundColor Green

# Tests pass
Write-Host "`n🧪 Running tests..." -ForegroundColor Cyan
$testOutput = cargo test --workspace 2>&1
$testExitCode = $LASTEXITCODE
if ($testExitCode -ne 0) {
    $testOutput | Write-Host
    Write-Error "Tests failed. Fix before releasing."
    exit 1
}
Write-Host "  ✓ All tests pass" -ForegroundColor Green

# Clippy clean
$clippyOutput = cargo clippy --workspace -- -D warnings 2>&1
$clippyExitCode = $LASTEXITCODE
if ($clippyExitCode -ne 0) {
    $clippyOutput | Write-Host
    Write-Error "Clippy warnings found. Fix before releasing."
    exit 1
}
Write-Host "  ✓ Clippy clean" -ForegroundColor Green

if ($DryRun) {
    Write-Host "`n📋 Dry run — would perform:" -ForegroundColor Yellow
    Write-Host "  1. Bump version to $Version in:"
    Write-Host "     - Cargo.toml (workspace root)"
    foreach ($member in $WorkspaceMembers) {
        Write-Host "     - $member/Cargo.toml"
    }
    Write-Host "  2. Stamp CHANGELOG.md [Unreleased] → [$Version] - $Today"
    Write-Host "  3. Update Cargo.lock"
    Write-Host "  4. Commit: 'chore: release v$Version'"
    Write-Host "  5. Tag: $Tag"
    if ($Push) { Write-Host "  6. Push to origin with tag" }
    Write-Host "  7. Release workflow builds binaries + publishes all crates to crates.io"
    exit 0
}

# --- Apply changes ---

Write-Host "`n📦 Preparing release $Tag" -ForegroundColor Cyan

# 1. Bump workspace root Cargo.toml version
$cargoToml = Get-Content Cargo.toml -Raw
$cargoToml = $cargoToml -replace '(?m)^version = "[^"]*"', "version = `"$Version`""
Set-Content Cargo.toml -Value $cargoToml -NoNewline
Write-Host "  ✓ Cargo.toml (root) → $Version" -ForegroundColor Green

# 2. Bump each workspace member Cargo.toml
foreach ($member in $WorkspaceMembers) {
    $memberToml = Join-Path $member "Cargo.toml"
    $content = Get-Content $memberToml -Raw

    # Update the crate's own version
    $content = $content -replace '(?m)^version = "[^"]*"', "version = `"$Version`""

    # Update workspace sibling dependency versions (e.g., toku-core = { version = "0.1.0", path = ... })
    foreach ($sibling in $WorkspaceMembers) {
        $siblingName = Split-Path $sibling -Leaf
        $content = $content -replace "($siblingName\s*=\s*\{\s*version\s*=\s*)`"[^`"]*`"", "`$1`"$Version`""
    }

    Set-Content $memberToml -Value $content -NoNewline
    Write-Host "  ✓ $memberToml → $Version" -ForegroundColor Green
}

# 3. Update Cargo.lock
cargo check --quiet 2>$null
Write-Host "  ✓ Cargo.lock updated" -ForegroundColor Green

# 4. Stamp CHANGELOG.md
$changelog = Get-Content CHANGELOG.md -Raw

# Add empty [Unreleased] and rename old one
$changelog = $changelog -replace '## \[Unreleased\]', "## [Unreleased]`n`n## [$Version] - $Today"

# Update comparison links at bottom (if they exist)
$prevVersionMatch = [regex]::Match($changelog, '\[(\d+\.\d+\.\d+)\]:\s*https://')
if ($prevVersionMatch.Success) {
    $prevVersion = $prevVersionMatch.Groups[1].Value
    $newLinks = "[Unreleased]: https://github.com/kafkade/toku/compare/v$Version...HEAD`n[$Version]: https://github.com/kafkade/toku/compare/v$prevVersion...v$Version"
    $changelog = $changelog -replace '\[Unreleased\]:\s*https://github.com/kafkade/toku/compare/v[\d.]+\.\.\.HEAD', $newLinks
}

Set-Content CHANGELOG.md -Value $changelog -NoNewline
Write-Host "  ✓ CHANGELOG.md stamped" -ForegroundColor Green

# 5. Collect all modified files and commit
$filesToAdd = @("Cargo.toml", "Cargo.lock", "CHANGELOG.md")
foreach ($member in $WorkspaceMembers) {
    $filesToAdd += Join-Path $member "Cargo.toml"
}
git add @filesToAdd
git commit -m "chore: release v$Version"
git tag -a $Tag -m "Release $Version"
Write-Host "  ✓ Committed and tagged $Tag" -ForegroundColor Green

# 6. Push (optional)
if ($Push) {
    Write-Host "`n🚀 Pushing to origin..." -ForegroundColor Cyan
    git push origin main --follow-tags
    Write-Host "  ✓ Pushed — release workflow will build binaries + publish to crates.io" -ForegroundColor Green
} else {
    Write-Host "`n📌 Ready to push. Run:" -ForegroundColor Yellow
    Write-Host "  git push origin main --follow-tags" -ForegroundColor White
}

Write-Host "`n✅ Release $Tag prepared successfully!`n" -ForegroundColor Green
