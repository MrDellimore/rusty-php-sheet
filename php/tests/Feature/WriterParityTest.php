<?php

declare(strict_types=1);

use RustySheet\Dev\RustyRead;
use RustySheet\Dev\WriterParity;

/*
 * The extension's writer must produce what OpenSpout produces. XLSX: the workbook,
 * styles and sheet XML must match after normalising the deliberate differences
 * (no <dimension>, full-precision numbers); then both files must read back the same.
 * CSV: byte-identical.
 */

const WRITER_STYLES = [
    'bold' => ['bold' => true, 'size' => 14],
    'money' => ['format' => '#,##0.00', 'color' => '002060'],
    'datetime' => ['format' => 'yyyy-mm-dd hh:mm:ss'],
    'yellow' => ['background' => 'FFFF00'],
    'yellow2' => ['background' => 'FFFF00'], // same content, separate object
    'everything' => [
        'bold' => true, 'italic' => true, 'underline' => true, 'strikethrough' => true,
        'size' => 9, 'color' => 'FF0000', 'font' => 'Courier New', 'background' => '00FF00',
        'format' => '0.00%', 'align' => 'center', 'valign' => 'top', 'wrap' => true,
        'rotation' => 45, 'shrink' => true,
    ],
    'nowrap' => ['wrap' => false, 'align' => 'right'],
    'date-col' => ['format' => 'dd/mm/yyyy'],
    'plain-col' => ['italic' => true],
    'builtin' => ['format' => 'mm-dd-yy'],
    'quote-format' => ['format' => '"Q"0 & <x>'],
];

function writer_scenarios(): array
{
    $sink = [
        ['sheet', 'Data'],
        ['freeze', 1, 0],
        ['grid', false],
        ['width', 20.0, [1]],
        ['width', 12.5, [2, 3, 5]],
        ['row', ['name', 'amount', 'when'], 'bold'],
        ['row', ['Alice', ['cell' => 1 / 3, 'style' => 'money'], ['cell' => new DateTimeImmutable('2025-06-15 14:30:00'), 'style' => 'datetime']]],
        ['row', [true, false, null, '', '=A1&"x"', 1500, 0.1 + 0.2, "<&>\x01"]],
        ['row', []],
        ['row', ['tall'], null, [], 30.0],
        ['row', [['cell' => 'yellow', 'style' => 'yellow']]],
        ['sheet', 'Second'],
        ['row', ['x']],
    ];

    $types = [
        ['row', [0, -1, PHP_INT_MAX, PHP_INT_MIN, 0.5, -0.0, 1e20, 1e-7, 123456789.123456789, 2.0, NAN, INF]],
        ['row', ['plain', 'café 🚀', '_x0041_ escaped', "ctl\x02\x1Fchars", "tab\tnl\ncr\r", 'quote " and \' and <tag>', str_repeat('é', 32770)]],
        ['row', ['=SUM(A1:B1)', '=', ['text' => '=not a formula'], ['text' => ''], '  spaces  ']],
        ['row', [new DateTimeImmutable('2025-06-15'), new DateTime('2025-12-25 23:59:59'), new DateTimeImmutable('1900-02-28'), new DateTimeImmutable('1900-03-01 06:00'), new DateTimeImmutable('2025-06-15 12:00:00.75')]],
        ['row', [new DateInterval('PT6H'), new DateInterval('P1DT2H30M'), (function () { $i = new DateInterval('PT1H'); $i->invert = 1; return $i; })(), new DateInterval('P1Y2M')]],
        // Review regressions: invalid UTF-8 in a formula, and in multiline text whose
        // wrap style must still be registered (shifting later style ids).
        ['row', ["=\"caf\xE9\"", ['cell' => "a\nb\xFF", 'style' => 'bold'], ['cell' => 'after', 'style' => 'money']]],
        ['row', ['_x005F_x0041_ literal', "_x0002_x0001_\x01"]],
    ];

    $styles = [
        ['row', ['a', 'b', 'c', 'd'], 'everything'],
        ['row', ['x', 'y'], 'nowrap'],
        ['row', [['cell' => 'one', 'style' => 'yellow'], ['cell' => 'two', 'style' => 'yellow2'], ['cell' => 3, 'style' => 'builtin'], ['cell' => 4, 'style' => 'quote-format']]],
        // Column style beats row style; empty cells with a fill or format are written.
        ['row', ['r', 'c', null, '', 'r'], 'bold', [1 => 'money', 2 => 'yellow', 3 => 'builtin']],
        // Dates: a formatted column style is used, otherwise the shared date styles.
        ['row', [new DateTimeImmutable('2025-01-02'), new DateTimeImmutable('2025-01-02 03:04:05'), new DateTimeImmutable('2025-01-02')], null, [0 => 'date-col', 2 => 'plain-col']],
        // A row of styled empty cells is still an empty row: skipped, number used up.
        ['row', [null, ''], 'yellow'],
        ['row', ['after empty']],
    ];

    $sheets = [
        ['sheet', null],
        ['row', ['default name']],
        ['sheet', 'A & B "quoted" <x>'],
        ['width', 8.0, [3, 1, 2, 7]],
        ['freeze', 2, 3],
        ['row', ['second']],
        ['sheet', null],
        ['grid', true],
        ['sheet', 'Empty at end'],
    ];

    $mt = mt_srand(42);
    $big = [['row', ['id', 'name', 'amount', 'flag', 'when', 'note'], 'bold']];
    for ($i = 0; $i < 3000; $i++) {
        $big[] = ['row', [
            $i,
            'name '.mt_rand(1, 500),
            mt_rand(0, 1_000_000) / 100,
            (bool) mt_rand(0, 1),
            mt_rand(0, 3) === 0 ? null : new DateTimeImmutable('@'.mt_rand(0, 2_000_000_000)),
            mt_rand(0, 4) === 0 ? '' : 'note '.mt_rand(),
        ], null, [2 => 'money']];
    }

    return [
        'kitchen sink' => $sink,
        'value types' => $types,
        'styles and precedence' => $styles,
        'sheets and setup' => $sheets,
        '3000 mixed rows' => $big,
    ];
}

function tmp_path(string $ext): string
{
    return sys_get_temp_dir().'/'.uniqid('rs_writer_', true).'.'.$ext;
}

it('writes XLSX like OpenSpout', function (string $name) {
    $ops = writer_scenarios()[$name];
    [$os, $rs] = [tmp_path('xlsx'), tmp_path('xlsx')];
    try {
        WriterParity::writeOpenSpout($os, $ops, WRITER_STYLES);
        WriterParity::writeRusty($rs, $ops, WRITER_STYLES);

        $expected = WriterParity::xlsxParts($os);
        $actual = WriterParity::xlsxParts($rs);
        expect(array_keys($actual))->toBe(array_keys($expected));

        // styles.xml must be byte-identical; cells are additionally compared by their
        // resolved style (font, fill, format, alignment) for clearer failures.
        expect($actual['xl/styles.xml'])->toBe($expected['xl/styles.xml']);
        $expected = WriterParity::resolveStyles($expected);
        $actual = WriterParity::resolveStyles($actual);
        foreach ($expected as $part => $xml) {
            expect($actual[$part])->toBe($xml, $part);
        }

        // And both read back identically (numbers at OpenSpout's 14 digits).
        $round = fn (array $sheets) => array_map(fn ($rows) => array_map(fn ($row) => array_map(
            fn ($v) => is_float($v) ? sprintf('%.14G', $v) : $v, $row), $rows), $sheets);
        expect($round(RustyRead::read($rs)))->toBe($round(RustyRead::read($os)));
    } finally {
        @unlink($os);
        @unlink($rs);
    }
})->with(array_keys(writer_scenarios()));

it('writes CSV byte-for-byte like OpenSpout', function (array $csv, string $ext = 'csv') {
    $ops = array_values(array_filter(
        array_merge(writer_scenarios()['value types'], writer_scenarios()['3000 mixed rows']),
        fn ($op) => $op[0] === 'row',
    ));
    [$os, $rs] = [tmp_path($ext), tmp_path($ext)];
    try {
        WriterParity::writeOpenSpout($os, $ops, WRITER_STYLES, $csv);
        WriterParity::writeRusty($rs, $ops, WRITER_STYLES, $csv);

        expect(file_get_contents($rs))->toBe(file_get_contents($os));
    } finally {
        @unlink($os);
        @unlink($rs);
    }
})->with([
    'defaults' => [[]],
    'semicolon, single quote, no BOM' => [['delimiter' => ';', 'enclosure' => "'", 'bom' => false]],
    'tab, CRLF' => [['delimiter' => "\t", 'eol' => "\r\n"]],
    '.tsv path (tab by default)' => [[], 'tsv'],
]);
