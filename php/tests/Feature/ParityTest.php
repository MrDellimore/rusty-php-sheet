<?php

declare(strict_types=1);

use RustySheet\Dev\OpenSpoutDump;
use RustySheet\Dev\RustyRead;
use RustySheet\Dev\SheetStreamReference;

/*
 * The extension must yield exactly what laravel-sheet-stream yields today through
 * OpenSpout, for every fixture and option combination, except for the deliberate
 * deviations listed below (see docs/parity-notes.md).
 */

/** Option combinations: [reference options, extension options, batch size]. */
function parity_modes(): array
{
    return [
        'serial' => [[], [], 1000],
        'datetime' => [['dates' => 'datetime'], ['dates' => 'datetime'], 1000],
        'datetime+tz' => [
            ['dates' => 'datetime', 'timezone' => 'America/New_York'],
            ['dates' => 'datetime', 'timezone' => 'America/New_York'],
            1000,
        ],
        'calculated' => [['calculated_formulas' => true], ['calculated_formulas' => true], 1000],
        // sheet-stream's WithFormatData: dates as display strings.
        'formatted' => [['format_dates' => true], ['dates' => 'formatted'], 1000],
        'formatted+calculated' => [
            ['format_dates' => true, 'calculated_formulas' => true],
            ['dates' => 'formatted', 'calculated_formulas' => true],
            1000,
        ],
        // Small batches exercise nextRows() resuming mid-sheet.
        'preserve-empty' => [['preserve_empty_rows' => true], ['preserve_empty_rows' => true], 7],
    ];
}

/**
 * Cells where rusty_sheet deliberately differs from OpenSpout.
 * file => [sheet, row index (0-based), column, modes it applies to, rusty value].
 */
const PARITY_DEVIATIONS = [
    // Unparseable number: OpenSpout gives 0.0 (silent corruption), rusty_sheet null.
    ['edge/cell-types.xlsx', 'Types', 18, 1, ['serial', 'datetime', 'datetime+tz', 'calculated', 'formatted', 'formatted+calculated', 'preserve-empty'], 'null'],
    // Shared-formula children: OpenSpout gives "=", rusty_sheet the real formula.
    ['edge/formulas.xlsx', 'Formulas', 5, 3, ['serial', 'datetime', 'datetime+tz', 'formatted', 'preserve-empty'], 'string("=B6*2")'],
    ['edge/formulas.xlsx', 'Formulas', 6, 3, ['serial', 'datetime', 'datetime+tz', 'formatted', 'preserve-empty'], 'string("=B7*2")'],
];

/** CSV options a fixture needs, applied to both sides. */
function csv_options_for(string $rel): array
{
    return match ($rel) {
        'edge/semicolon-bom.csv', 'laravel-excel/csv-with-other-delimiter.csv' => ['csv' => ['delimiter' => ';']],
        'edge/tabs.tsv', 'laravel-excel/import-titles.tsv' => ['csv' => ['delimiter' => "\t"]],
        default => [],
    };
}

/** @return array<string, array{string, string}> */
function parity_cases(): array
{
    $cases = [];
    foreach (glob(fixtures_path('{edge,laravel-excel}/*.{xlsx,csv,tsv}'), GLOB_BRACE) as $file) {
        foreach (array_keys(parity_modes()) as $mode) {
            $rel = basename(dirname($file)) . '/' . basename($file);
            $cases["$rel [$mode]"] = [$rel, $mode];
        }
    }
    ksort($cases);

    return $cases;
}

/** Render every value with its type, so int 3 ≠ float 3.0 and '' ≠ null. */
function render_sheets(array $sheets): array
{
    return array_map(
        fn (array $rows) => array_map(fn (array $row) => array_map(OpenSpoutDump::describe(...), $row), $rows),
        $sheets,
    );
}

it('matches sheet-stream + OpenSpout', function (string $rel, string $mode) {
    [$refOptions, $rustyOptions, $batch] = parity_modes()[$mode];
    $path = fixtures_path($rel);

    $csv = csv_options_for($rel);
    $expected = render_sheets(SheetStreamReference::read($path, $refOptions + $csv));
    $actual = render_sheets(RustyRead::read($path, $rustyOptions + $csv, $batch));

    // Deliberate deviation: OpenSpout turns any text starting with "=" into a formula
    // cell, so with calculated formulas the text is replaced by null. rusty_sheet keeps it.
    if ($refOptions['calculated_formulas'] ?? false) {
        foreach ($expected as $sheet => $rows) {
            foreach ($rows as $r => $cells) {
                foreach ($cells as $c => $value) {
                    $got = $actual[$sheet][$r][$c] ?? null;
                    if ($value === 'null' && is_string($got) && str_starts_with($got, 'string("=')) {
                        $expected[$sheet][$r][$c] = $got;
                    }
                }
            }
        }
    }

    foreach (PARITY_DEVIATIONS as [$file, $sheet, $row, $col, $modes, $rustyValue]) {
        if ($file === $rel && in_array($mode, $modes, true)) {
            expect($actual[$sheet][$row][$col])->toBe($rustyValue, 'deliberate deviation');
            $expected[$sheet][$row][$col] = $rustyValue;
        }
    }

    expect(array_keys($actual))->toBe(array_keys($expected));
    foreach ($expected as $sheet => $rows) {
        expect($actual[$sheet])->toBe($rows, "sheet '$sheet'");
    }
})->with(parity_cases());
