<?php

declare(strict_types=1);

use PhpOffice\PhpSpreadsheet\Cell\Coordinate;
use PhpOffice\PhpSpreadsheet\IOFactory;
use PhpOffice\PhpSpreadsheet\Spreadsheet;
use PhpOffice\PhpSpreadsheet\Style\NumberFormat;
use RustySheet\Writer;
use RustySheet\WriterException;

/*
 * Phase 8: auto-sized columns must get the widths PhpSpreadsheet 5.11 gives the same
 * cells (approximate mode, what litportal uses), with the workbook's default font
 * Calibri 12 on both sides (the extension's Normal style, OpenSpout's fallback font).
 */

/**
 * A holdback-like sheet: header row with an autofilter, then data in every format
 * litportal auto-sizes. Each cell: [value, style spec] (style null = unstyled).
 */
function autosize_rows(): array
{
    $header = ['font' => 'Aptos Narrow', 'bold' => true, 'size' => 11];
    $data = ['font' => 'Aptos Narrow', 'size' => 11];
    $money = $data + ['format' => NumberFormat::FORMAT_CURRENCY_USD];
    $percent = $data + ['format' => NumberFormat::FORMAT_PERCENTAGE];
    $date = $data + ['format' => 'm/d/yyyy'];
    $count = $data + ['format' => '#,##0'];
    $twoDp = $data + ['format' => '0.00'];

    $rows = [[
        ['Claimant', $header], ['Claim #', $header], ['Settlement Amount', $header], ['% of Settlement', $header],
        ['Funded', $header], ['Liens', $header], ['Rate', $header], ['Notes', $header], ['OK', $header],
        ['Centred', $header], ['Angled', $header], ['Plain', null],
    ]];
    $names = ['Ann Lee', 'Bartholomew Featherstonehaugh', 'Cy', '山田 太郎', 'Zoë Ångström'];
    foreach ($names as $i => $name) {
        $rows[] = [
            [$name, $data],
            [sprintf('%07d', 31 * ($i + 1) ** 5), $data],
            [[1234.5, -98765.432, 0.0, 1e6, 12.345][$i], $money],
            [[0.256, 1.0, 0.0049, 0.5, 12.3456][$i], $percent],
            [45823 + $i * 400 + 0.25, $date],
            [[7, 12345, 0, 1234567, -42][$i], $count],
            [[1 / 3, 2.5, -0.004, 100, 99.995][$i], $twoDp],
            [["one line", "two\nlines here", "日本語のメモ", '', "x\nmuch longer second line"][$i], $data],
            [$i % 2 === 0, $data],
            [str_repeat('c', 3 + $i * 4), $data + ['align' => 'center']],
            [str_repeat('r', 5 + $i * 3), $data + ['rotation' => 45]],
            [[42, 3.14159, 'plain text', 1e20, -7][$i], null],
        ];
    }

    return $rows;
}

/** Widths PhpSpreadsheet calculates for the rows. */
function phpspreadsheet_widths(array $rows, string $filter, array $merges): array
{
    $book = new Spreadsheet();
    $book->getDefaultStyle()->getFont()->setName('Calibri')->setSize(12);
    $sheet = $book->getActiveSheet();
    foreach ($rows as $r => $cells) {
        foreach ($cells as $c => [$value, $style]) {
            $ref = Coordinate::stringFromColumnIndex($c + 1).($r + 1);
            $sheet->setCellValue($ref, $value);
            if ($style !== null) {
                $s = $sheet->getStyle($ref);
                // Extension styles start from OpenSpout's default font, Arial 11.
                $s->getFont()->setName($style['font'] ?? 'Arial')->setSize($style['size'] ?? 11)->setBold($style['bold'] ?? false);
                if (isset($style['format'])) {
                    $s->getNumberFormat()->setFormatCode($style['format']);
                }
                if (isset($style['align'])) {
                    $s->getAlignment()->setHorizontal($style['align']);
                }
                if (isset($style['rotation'])) {
                    $s->getAlignment()->setTextRotation($style['rotation']);
                }
            }
        }
    }
    $sheet->setAutoFilter($filter);
    foreach ($merges as $m) {
        $sheet->mergeCells($m);
    }
    $last = count($rows[0]);
    for ($c = 1; $c <= $last; $c++) {
        $sheet->getColumnDimension(Coordinate::stringFromColumnIndex($c))->setAutoSize(true);
    }
    $sheet->calculateColumnWidths();
    $widths = [];
    for ($c = 1; $c <= $last; $c++) {
        $widths[Coordinate::stringFromColumnIndex($c)] = $sheet->getColumnDimension(Coordinate::stringFromColumnIndex($c))->getWidth();
    }

    return $widths;
}

function rusty_widths(array $rows, string $filter, array $merges, bool|array $autoSize = true): array
{
    $path = sys_get_temp_dir().'/'.uniqid('rs_autosize_', true).'.xlsx';
    $w = Writer::create($path, ['temp_dir' => sys_get_temp_dir()]);
    $w->autoSizeColumns($autoSize);
    $w->setAutoFilter($filter);
    foreach ($merges as $m) {
        $w->mergeCells($m);
    }
    $styles = [];
    foreach ($rows as $cells) {
        $w->addRow(array_map(function (array $cell) use ($w, &$styles) {
            [$value, $style] = $cell;
            if ($style === null) {
                return $value;
            }
            $key = json_encode($style);
            $styles[$key] ??= $w->addStyle($style);

            return Writer::cell($value, $styles[$key]);
        }, $cells));
    }
    $w->close();

    $sheet = IOFactory::load($path)->getActiveSheet();
    $widths = [];
    for ($c = 1; $c <= count($rows[0]); $c++) {
        $letter = Coordinate::stringFromColumnIndex($c);
        $widths[$letter] = $sheet->getColumnDimension($letter)->getWidth();
    }
    unlink($path);

    return $widths;
}

it('sizes columns exactly as PhpSpreadsheet does', function () {
    $rows = autosize_rows();
    // Merged cells don't count, except the first cell of a one-column merge.
    $merges = ['H3:I3', 'L4:L5'];
    expect(rusty_widths($rows, 'A1:K1', $merges))->toBe(phpspreadsheet_widths($rows, 'A1:K1', $merges));
});

it('renders rarer number formats as PhpSpreadsheet does before measuring', function (string $format) {
    $values = [0, 1, -1, 0.5, -1234.5678, 98765.4321, 1e-7, 3e15, 12, 100];
    $rows = [[['Values', null]]];
    foreach ($values as $v) {
        $rows[] = [[$v, ['font' => 'Calibri', 'size' => 12, 'format' => $format]]];
    }
    expect(rusty_widths($rows, 'A1:A1', []))->toBe(phpspreadsheet_widths($rows, 'A1:A1', []));
})->with([
    'escaped literals' => ['\\-#,##0.0\\ \\k'],
    'fill and padding' => ['_(* #,##0.00_);_(* \\(#,##0.00\\);_(* "-"??_);_(@_)'],
    'condition and colour' => ['[>=100][Red]#,##0;[Blue]0.00'],
    'optional decimals' => ['0.0#'],
    'thousands only' => ['#,###'],
    'quoted text around' => ['"approx. "0.0" units"'],
    'only text' => ['"n/a"'],
    'fraction' => ['# ?/?'],
    'text placeholder' => ['@'],
    'scientific, negative exponent' => ['0.0E-00'],
    'three sections' => ['0.00;-0.00;"zero"'],
    'literal between digits' => ['000-00-0000'],
    'locale currency' => ['[$$-409]#,##0.00'],
    'vulgar fraction' => ['?/?'],
    'fixed sixteenths' => ['# ??/16'],
    'fraction, whole part with zero' => ['0 ?/?'],
    'fraction, blank whole part' => ['? ?/?'],
    'fraction, zero numerator shown' => ['# 0/0'],
    'blank decimals' => ['0.0??'],
    'text with literal' => ['"Total: "@'],
]);

it('renders dates as PhpSpreadsheet does before measuring', function (string $format) {
    // Real dates: PhpSpreadsheet has conventions of its own for 0, negatives and
    // 1900-01-01..02-28; numbers too big to be dates are measured as numbers.
    $values = [61, 367.25, 45823, 45823.999, 2958465, 3e15, 100000.5];
    $rows = [[['Dates', null]]];
    foreach ($values as $v) {
        $rows[] = [[$v, ['font' => 'Calibri', 'size' => 12, 'format' => $format]]];
    }
    expect(rusty_widths($rows, 'A1:A1', []))->toBe(phpspreadsheet_widths($rows, 'A1:A1', []));
})->with([
    'litportal' => ['m/d/yyyy'],
    'coloured' => ['[Red]m/d/yyyy'],
    'conditional' => ['[>=1]yyyy-mm-dd;"none"'],
    'with time' => ['yyyy-mm-dd hh:mm:ss'],
    'month name' => ['d mmmm yyyy'],
    'elapsed hours' => ['[h]:mm'],
]);

it('leaves room for a table header\'s dropdowns as PhpSpreadsheet does', function () {
    $rows = [['Claimant', 'Amount'], ['Ann', 12.5], ['Bob', 7]];
    // PhpSpreadsheet: a table with a header row and filter buttons.
    $book = new Spreadsheet();
    $book->getDefaultStyle()->getFont()->setName('Calibri')->setSize(12);
    $sheet = $book->getActiveSheet();
    $sheet->fromArray($rows);
    $sheet->addTable(new PhpOffice\PhpSpreadsheet\Worksheet\Table('A1:B3', 'T'));
    foreach (['A', 'B'] as $c) {
        $sheet->getColumnDimension($c)->setAutoSize(true);
    }
    $sheet->calculateColumnWidths();
    $expected = ['A' => $sheet->getColumnDimension('A')->getWidth(), 'B' => $sheet->getColumnDimension('B')->getWidth()];

    $path = sys_get_temp_dir().'/'.uniqid('rs_autosize_', true).'.xlsx';
    $w = Writer::create($path);
    $w->autoSizeColumns();
    $w->addTable('A1:B3', 'T');
    foreach ($rows as $r) {
        $w->addRow($r);
    }
    $w->close();
    $read = IOFactory::load($path)->getActiveSheet();
    expect(['A' => $read->getColumnDimension('A')->getWidth(), 'B' => $read->getColumnDimension('B')->getWidth()])->toBe($expected);
    unlink($path);
});

it('measures stacked text as PhpSpreadsheet does', function () {
    $rows = [[['a long header', ['rotation' => 255]], ['x', null]], [['also long text', ['rotation' => 255]], [true, null]]];
    expect(rusty_widths($rows, 'A1:B1', []))->toBe(phpspreadsheet_widths($rows, 'A1:B1', []));
});

it('sizes only the columns asked for, and explicit widths win', function () {
    $path = sys_get_temp_dir().'/'.uniqid('rs_autosize_', true).'.xlsx';
    $w = Writer::create($path);
    $w->autoSizeColumns([3, 2]); // any order
    $w->setColumnWidth(40, [3]);
    $w->addRow(['a fairly long value', 'another long value here', 'x']);
    $w->close();
    $sheet = IOFactory::load($path)->getActiveSheet();
    expect($sheet->getColumnDimension('A')->getWidth())->toBe(-1.0) // default
        ->and($sheet->getColumnDimension('B')->getWidth())->toBeGreaterThan(20)
        ->and($sheet->getColumnDimension('C')->getWidth())->toBe(40.0);
    unlink($path);
});

it('rejects bad arguments and late calls', function () {
    $w = Writer::create(sys_get_temp_dir().'/'.uniqid('rs_autosize_', true).'.xlsx');
    expect(fn () => $w->autoSizeColumns([0]))->toThrow(ValueError::class, '1-based')
        ->and(fn () => $w->autoSizeColumns(['A']))->toThrow(ValueError::class, '1-based')
        ->and(fn () => $w->autoSizeColumns(false))->toThrow(ValueError::class, 'expected true')
        ->and(fn () => $w->autoSizeColumns([16385]))->toThrow(ValueError::class, 'out of range');
    $w->addRow(['x']);
    expect(fn () => $w->autoSizeColumns())->toThrow(WriterException::class, 'first row');
});

it('validates temp_dir and ignores auto-size for CSV', function () {
    expect(fn () => Writer::create(sys_get_temp_dir().'/x.xlsx', ['temp_dir' => '/nonexistent/dir']))
        ->toThrow(ValueError::class, 'not a directory');
    $path = sys_get_temp_dir().'/'.uniqid('rs_autosize_', true).'.csv';
    $w = Writer::create($path, ['csv' => ['bom' => false]]);
    $w->autoSizeColumns();
    $w->addRow(['a', 'b']);
    $w->close();
    expect(file_get_contents($path))->toBe("a,b\n");
    unlink($path);
});
