<?php

declare(strict_types=1);

/** Absolute path to the repository's tests/fixtures directory. */
function fixtures_path(string $relative = ''): string
{
    return dirname(__DIR__, 2) . '/tests/fixtures' . ($relative === '' ? '' : '/' . ltrim($relative, '/'));
}

/**
 * The committed xlsx, csv and tsv fixtures, sorted. No GLOB_BRACE: musl's glob()
 * (Alpine) doesn't support it, so brace patterns there match nothing.
 *
 * @return list<string> absolute paths
 */
function fixture_files(): array
{
    $files = [];
    foreach (['edge', 'laravel-excel'] as $dir) {
        foreach (['xlsx', 'csv', 'tsv'] as $ext) {
            $files = array_merge($files, glob(fixtures_path("$dir/*.$ext")) ?: []);
        }
    }
    sort($files);

    return $files;
}

/**
 * Every committed spreadsheet fixture that OpenSpout can read (xlsx, csv, tsv).
 *
 * @return array<string, array{string}> label => [absolute path]
 */
function readable_fixtures(): array
{
    $out = [];
    foreach (fixture_files() as $file) {
        $out[basename(dirname($file)) . '/' . basename($file)] = [$file];
    }

    return $out;
}
