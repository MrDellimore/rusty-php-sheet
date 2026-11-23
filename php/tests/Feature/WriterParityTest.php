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
    'centered' => ['align' => 'center'],
    'date-col' => ['format' => 'dd/mm/yyyy'],
    'plain-col' => ['italic' => true],
    'builtin' => ['format' => 'mm-dd-yy'],
    'quote-format' => ['format' => '"Q"0 & <x>'],
    // Phase 7: borders (OpenSpout styles and widths; 'all' is left, right, top, bottom).
    'banner' => ['bold' => true, 'size' => 16, 'background' => '1F4E78', 'color' => 'FFFFFF', 'align' => 'center'],
    'header' => ['bold' => true, 'background' => 'D9E1F2', 'border' => ['all' => ['style' => 'solid', 'width' => 'thin', 'color' => '808080']]],
    'grid' => ['border' => ['all' => ['style' => 'solid', 'width' => 'thin', 'color' => 'BFBFBF']]],
    'grid-same' => ['border' => ['all' => ['style' => 'solid', 'width' => 'thin', 'color' => 'BFBFBF']]],
    'grid-reordered' => ['border' => ['top' => ['width' => 'thin', 'color' => 'BFBFBF'], 'left' => ['width' => 'thin', 'color' => 'BFBFBF'], 'right' => ['width' => 'thin', 'color' => 'BFBFBF'], 'bottom' => ['width' => 'thin', 'color' => 'BFBFBF']]],
    'bottom-thick' => ['border' => ['bottom' => ['width' => 'thick']], 'format' => '#,##0.00'],
    'dashed-medium' => ['border' => ['top' => ['style' => 'dashed'], 'bottom' => ['style' => 'dashed', 'width' => 'thin']]],
    'dotted-double' => ['border' => ['left' => ['style' => 'dotted', 'width' => 'thick'], 'right' => ['style' => 'double', 'color' => 'FF0000']]],
    'none-border' => ['border' => ['top' => ['style' => 'none']]],
    'empty-border' => ['border' => []],
    'link' => ['color' => '0563C1', 'underline' => true],
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
        // Multiline text: the style's wrap variant is registered once and reused; a
        // style that sets wrap itself (even to false) is used as is.
        ['row', ["one\ntwo", "three\nfour"], 'bold'],
        ['row', ["five\nsix"], 'bold'],
        ['row', ["no\nwrap"], 'nowrap'],
        ['row', ['centred, wrap unset'], 'centered'],
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
        ['sheet', 'Columns only'],
        ['freeze', 0, 2],
        ['width', 9.0, []],
        ['row', ['frozen columns']],
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

    // Phase 7: a report sheet like litportal's, and a summary that links to the others.
    $banner = [
        ['sheet', 'Report'],
        ['autofilter', 'A3:D7'],
        ['freeze', 3, 0],
        ['width', 18.0, [1, 2, 3, 4]],
        ['merge', 'A1:D1'],
        ['row', ['Claimant report', null, null, null], 'banner', [], 28.0],
        ['row', []],
        ['row', ['Name', 'Amount', 'Status', 'Note'], 'header'],
        ['row', ['Ann', 1250.5, 'Open', ''], 'grid', [1 => 'bottom-thick']],
        ['row', ['Bob', 99.99, 'Closed', "two\nlines"], 'grid-same', [1 => 'bottom-thick']],
        ['row', ['Cy', 0, 'Open', null], 'grid-reordered'],
        ['row', [null, null, null, null], 'grid'], // empty cells with a border are written
        ['merge', 'C8:D9'],
        ['merge', 'a10:$B$11'],
        ['row', ['styles', ['cell' => 'dashed', 'style' => 'dashed-medium'], ['cell' => 'dotted', 'style' => 'dotted-double'], ['cell' => 'none', 'style' => 'none-border'], ['cell' => 'empty', 'style' => 'empty-border']]],
        ['sheet', 'Second & "quoted"'],
        ['autofilter', 'B2:B2'],
        ['row', ['x']],
    ];

    $links = [
        ['sheet', 'Summary'],
        ['row', [
            ['link' => 'Go to Detail', 'target' => "#'Detail sheet'!A1", 'style' => 'link'],
            ['link' => 'Detail via sheet://', 'target' => "sheet://'Detail sheet'!B2"],
            ['link' => 'Site', 'target' => 'https://example.com/a?b=1&c=<2>', 'tooltip' => 'Open "example"'],
            ['link' => 'Mail', 'target' => 'mailto:claims@example.com'],
            ['link' => '=not a formula', 'target' => '#Other!A1', 'tooltip' => 'tip'],
        ]],
        ['row', ['no link', ['link' => 'Second site', 'target' => 'https://example.org/']]],
        ['sheet', 'Detail sheet'],
        ['row', [['link' => 'Back', 'target' => '#Summary!A1']]],
        ['sheet', 'Other'],
        ['merge', 'A1:B1'],
    ];

    return [
        'kitchen sink' => $sink,
        'value types' => $types,
        'styles and precedence' => $styles,
        'sheets and setup' => $sheets,
        '3000 mixed rows' => $big,
        'banner report' => $banner,
        'links between sheets' => $links,
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
