<?php

declare(strict_types=1);

/** Absolute path to the repository's tests/fixtures directory. */
function fixtures_path(string $relative = ''): string
{
    return dirname(__DIR__, 2) . '/tests/fixtures' . ($relative === '' ? '' : '/' . ltrim($relative, '/'));
}

/**
 * Every committed spreadsheet fixture that OpenSpout can read (xlsx, csv, tsv).
 *
 * @return array<string, array{string}> label => [absolute path]
 */
function readable_fixtures(): array
{
    $files = array_merge(
        glob(fixtures_path('edge/*.{xlsx,csv,tsv}'), GLOB_BRACE),
        glob(fixtures_path('laravel-excel/*.{xlsx,csv,tsv}'), GLOB_BRACE),
    );
    sort($files);

    $out = [];
    foreach ($files as $file) {
        $out[basename(dirname($file)) . '/' . basename($file)] = [$file];
    }

    return $out;
}
