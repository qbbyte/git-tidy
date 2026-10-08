# Verify the conflict-marker parser against a draft produced by real git.
# This is the "equivalent to hand-rolled git" acceptance check from doc section 7.13.
#
# ASCII only on purpose: Windows PowerShell 5.1 reads .ps1 without a BOM as ANSI,
# so Chinese in this file would come out mojibake.
$ErrorActionPreference = 'Stop'

$root = Join-Path ([System.IO.Path]::GetTempPath()) ("git-tidy-conflict-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $root | Out-Null
$runner = Join-Path $PSScriptRoot 'check-conflict-real.mjs'
# .NET relative paths follow the process working directory, not PowerShell's location,
# so file writes here are all absolute.
$file = Join-Path $root 'a.txt'
try {
    Push-Location $root
    git init -q -b main . | Out-Null
    git config user.name t
    git config user.email t@example.com
    git config core.autocrlf false

    # Two conflicts at least 7 lines apart: closer than that and git folds them into
    # a single hunk (3 lines of context on each side), which would make the
    # "clean segments stay in place" assertion vacuous.
    [System.IO.File]::WriteAllText($file, (0..9 | ForEach-Object { "line$_" }) -join "`n" + "`n")
    git add -A; git commit -qm base

    git checkout -q -b side
    [System.IO.File]::WriteAllText($file, "CHANGED_SIDE`n" + ((1..8 | ForEach-Object { "line$_" }) -join "`n") + "`nCHANGED_SIDE_END`n")
    git add -A; git commit -qm side

    git checkout -q main
    [System.IO.File]::WriteAllText($file, "CHANGED_MAIN`n" + ((1..8 | ForEach-Object { "line$_" }) -join "`n") + "`nCHANGED_MAIN_END`n")
    git add -A; git commit -qm main

    git merge side 2>&1 | Out-Null
    if ($LASTEXITCODE -eq 0) { throw 'merge succeeded, no conflict was created' }

    $proj = $PSScriptRoot | Split-Path -Parent
    Push-Location $proj
    node --experimental-strip-types $runner $file
    if ($LASTEXITCODE -ne 0) { throw 'parser check failed' }
} finally {
    Pop-Location
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
}