<?php

declare(strict_types=1);

use RustySheet\Dev\TinyXlsx;
use RustySheet\Reader;
use RustySheet\ReaderException;
use RustySheet\UnsupportedFormatException;
use RustySheet\Writer;
use RustySheet\WriterException;

/*
 * Option values and failure modes that the feature tests don't reach: every option
 * value spelled out, every type error, and how each kind of broken file surfaces.
 */

function err_tmp(string $ext): string
{
    return sys_get_temp_dir().'/'.uniqid('rs_err_', true).'.'.$ext;
}

function tiny_xlsx(string $rows, ?string $styles = null, ?string $encrypt = null): string
{
    return TinyXlsx::write(err_tmp('xlsx'), $rows, $styles, $encrypt);
}

describe('reader options', function () {
    it('accepts every documented value', function (array $options) {
        $path = tiny_xlsx('<row r="1"><c><v>1</v></c></row>');
        expect(Reader::open($path, $options)->nextRows(0))->toBe([1 => [1]]);
        unlink($path);
    })->with([
        'dates serial' => [['dates' => 'serial']],
        'on_error throw' => [['on_error' => 'throw']],
        'on_error skip_row' => [['on_error' => 'skip_row']],
        'formulas_without_cached_value null' => [['formulas_without_cached_value' => 'null']],
        'formulas_without_cached_value throw' => [['formulas_without_cached_value' => 'throw']],
        'formulas_without_cached_value formula' => [['formulas_without_cached_value' => 'formula']],
        "empty_cells ''" => [['empty_cells' => '']],
        'empty_cells null' => [['empty_cells' => null]],
        'null format and timezone' => [['format' => null, 'timezone' => null]],
        'null csv' => [['csv' => null]],
    ]);

    it('accepts CSV escape characters', function (string $escape, array $expected) {
        $path = err_tmp('csv');
        file_put_contents($path, "\"a\\\"b\",c\n");
        expect(Reader::open($path, ['csv' => ['escape' => $escape]])->nextRows(0))->toBe([1 => $expected]);
        unlink($path);
    })->with([
        'none' => ['', ['a\\b"', 'c']],
        'backslash' => ['\\', ['a\\"b', 'c']],
    ]);

    it('rejects options of the wrong type or name', function (array $options, string $message) {
        expect(fn () => Reader::open(err_tmp('xlsx'), $options))->toThrow(ValueError::class, $message);
    })->with([
        'int key' => [[0 => 'serial'], 'option keys must be strings'],
        'non-UTF-8 key' => [["bad\xFF" => 1], 'unknown option'],
        'format not a string' => [['format' => 1], 'expected a string'],
        'csv not an array' => [['csv' => 'x'], 'expected an array'],
        'csv escape not a string' => [['csv' => ['escape' => 1]], 'expected a string'],
        'csv int key' => [['csv' => [0 => ',']], 'unknown option'],
        'on_error unknown' => [['on_error' => 'ignore'], "expected 'throw' or 'skip_row'"],
        'formulas_without_cached_value unknown' => [['formulas_without_cached_value' => 'zero'], "expected 'null', 'throw' or 'formula'"],
        'formulas_without_cached_value not a string' => [['formulas_without_cached_value' => null], 'expected a string'],
    ]);
});

it('reads and writes each format named explicitly, whatever the extension', function (array $options, string $delimiter) {
    $path = err_tmp('out');
    $w = Writer::create($path, $options);
    $w->addRow(['a', 'b']);
    $w->close();
    if ($options['format'] !== 'xlsx') {
        expect(file_get_contents($path))->toBe("\xEF\xBB\xBFa{$delimiter}b\n");
    }
    expect(Reader::open($path, $options)->nextRows(0))->toBe([1 => ['a', 'b']]);
    unlink($path);
})->with([
    'xlsx' => [['format' => 'xlsx'], ''],
    'csv' => [['format' => 'csv'], ','],
    'tsv' => [['format' => 'tsv'], "\t"],
]);

describe('broken XLSX files', function () {
    $beyondXfd = '<row r="1"><c r="A1"><v>1</v></c></row><row r="2"><c r="XFE2"><v>2</v></c></row><row r="3"><c r="A3"><v>3</v></c></row>';

    it('throws for a cell beyond column XFD by default', function () use ($beyondXfd) {
        $path = tiny_xlsx($beyondXfd);
        $r = Reader::open($path);
        expect(fn () => $r->nextRows(0))->toThrow(ReaderException::class, 'cell reference `XFE2` is beyond column XFD')
            ->and($r->nextRows(0))->toBe([]); // the sheet is finished after an error
        unlink($path);
    });

    it('skips the bad row with on_error => skip_row', function () use ($beyondXfd) {
        $path = tiny_xlsx($beyondXfd);
        expect(Reader::open($path, ['on_error' => 'skip_row'])->nextRows(0))->toBe([1 => [1], 3 => [3]]);
        unlink($path);
    });

    it('always throws for malformed XML, even with skip_row', function () {
        $path = tiny_xlsx('<row r="1"><c><v>1</v></c></row><row r="2" a="></row>');
        expect(fn () => Reader::open($path, ['on_error' => 'skip_row'])->nextRows(0, 10))
            ->toThrow(ReaderException::class, 'malformed XML');
        unlink($path);
    });

    it('throws on the first read when styles.xml is corrupt', function () {
        $path = tiny_xlsx('<row r="1"><c><v>1</v></c></row>', '<styleSheet><cellXfs><xf a="1></cellXfs></styleSheet>');
        $r = Reader::open($path);
        expect($r->sheetNames())->toBe(['S'])
            ->and(fn () => $r->nextRows(0))->toThrow(ReaderException::class, 'xl/styles.xml');
        unlink($path);
    });

    it('reports an encrypted part as unsupported, so callers fall back', function () {
        $path = tiny_xlsx('<row r="1"><c><v>1</v></c></row>', null, 'secret');
        expect(fn () => Reader::open($path)->nextRows(0))
            ->toThrow(UnsupportedFormatException::class, 'encrypted');
        unlink($path);
    });
});

describe('ISO date cells (t="d")', function () {
    it('reads them like OpenSpout, and as serials in serial mode', function () {
        $path = tiny_xlsx('<row r="1"><c t="d"><v>2024-01-02</v></c><c t="d"><v>2024-01-02T06:00:00</v></c><c t="d"><v>not a date</v></c><c><v>1</v></c></row>');
        expect(Reader::open($path)->nextRows(0))->toBe([1 => [45293, 45293.25, null, 1]]);
        $dt = Reader::open($path, ['dates' => 'datetime'])->nextRows(0)[1];
        expect($dt[0])->toBeInstanceOf(DateTimeImmutable::class)
            ->and($dt[0]->format('Y-m-d H:i'))->toBe('2024-01-02 00:00')
            ->and($dt[2])->toBeNull();
        unlink($path);
    });
});

describe('writer options', function () {
    it('takes the format from the option over the extension', function () {
        $path = err_tmp('dat');
        $w = Writer::create($path, ['format' => 'CSV', 'csv' => ['bom' => false, 'eol' => "\r\n", 'enclosure' => "'"]]);
        $w->addRow(['a b', "it's"]);
        $w->close();
        expect(file_get_contents($path))->toBe("'a b','it''s'\r\n");
        unlink($path);
    });

    it('uses date_format and datetime_format for dates', function () {
        $path = err_tmp('xlsx');
        $w = Writer::create($path, ['date_format' => 'dd/mm/yyyy', 'datetime_format' => 'dd/mm/yyyy hh:mm']);
        $w->addRow([new DateTimeImmutable('2025-06-15'), new DateTimeImmutable('2025-06-15 14:30')]);
        $w->close();
        $zip = new ZipArchive();
        $zip->open($path);
        $styles = $zip->getFromName('xl/styles.xml');
        $zip->close();
        expect($styles)->toContain('formatCode="dd/mm/yyyy"')
            ->and($styles)->toContain('formatCode="dd/mm/yyyy hh:mm"')
            ->and(Reader::open($path, ['dates' => 'formatted'])->nextRows(0))->toBe([1 => ['15/06/2025', '15/06/2025 14:30']]);
        unlink($path);
    });

    it('rejects options of the wrong type or name', function (array $options, string $message) {
        expect(fn () => Writer::create(err_tmp('xlsx'), $options))->toThrow(ValueError::class, $message);
    })->with([
        'format' => [['format' => 1], 'expected a string'],
        'date_format' => [['date_format' => 1], 'expected a string'],
        'datetime_format' => [['datetime_format' => null], 'expected a string'],
        'csv not an array' => [['csv' => 'x'], 'expected an array'],
        'csv bom' => [['csv' => ['bom' => 'yes']], 'expected a bool'],
        'csv eol' => [['csv' => ['eol' => 1]], 'expected a string'],
        'csv unknown' => [['csv' => ['escape' => '\\']], 'unknown option'],
    ]);

    it('reports files that cannot be created', function (string $ext) {
        expect(fn () => Writer::create('/nonexistent-dir/'.uniqid().'.'.$ext))
            ->toThrow(WriterException::class, 'I/O error');
    })->with(['xlsx', 'csv']);
});

describe('writer arguments', function () {
    it('ignores sheet setup for CSV', function () {
        $path = err_tmp('csv');
        $w = Writer::create($path, ['csv' => ['bom' => false]]);
        $w->setColumnWidth(20, [1, 2]);
        $w->freezePanes(1, 1);
        $w->showGridLines(false);
        $w->addRow(['a', 1]);
        $w->close();
        expect(file_get_contents($path))->toBe("a,1\n");
        unlink($path);
    });

    it('rejects bad columnStyles', function (array $columnStyles, string $message) {
        $w = Writer::create(err_tmp('xlsx'));
        $style = $w->addStyle(['bold' => true]);
        expect(fn () => $w->addRow(['x'], null, array_map(fn ($v) => $v === 'STYLE' ? $style : $v, $columnStyles)))
            ->toThrow(ValueError::class, $message);
    })->with([
        'string key' => [['a' => 'STYLE'], 'keys must be 0-based column indexes'],
        'negative key' => [[-1 => 'STYLE'], 'keys must be >= 0'],
        'not a style id' => [[0 => 'x'], 'values must be style ids'],
    ]);

    it('rejects style values of the wrong type', function (array $style) {
        expect(fn () => Writer::create(err_tmp('xlsx'))->addStyle($style))->toThrow(ValueError::class);
    })->with([
        'size' => [['size' => '12']],
        'color' => [['color' => 0xFF0000]],
        'wrap' => [['wrap' => 1]],
    ]);

    it('accepts columnStyles keys in any order', function () {
        $path = err_tmp('xlsx');
        $w = Writer::create($path);
        $bold = $w->addStyle(['bold' => true]);
        $w->addRow(['a', 'b', 'c'], null, [2 => $bold, 0 => $bold]);
        $w->close();
        $zip = new ZipArchive();
        $zip->open($path);
        $sheet = $zip->getFromName('xl/worksheets/sheet1.xml');
        $zip->close();
        expect($sheet)->toContain('<c r="A1" s="1"')->and($sheet)->toContain('<c r="B1" s="0"')->and($sheet)->toContain('<c r="C1" s="1"');
        unlink($path);
    });

    it('rejects a negative style id', function () {
        $w = Writer::create(err_tmp('xlsx'));
        expect(fn () => $w->addRow(['x'], -1))->toThrow(ValueError::class, 'unknown style id')
            ->and(fn () => Writer::cell('x', -1))->toThrow(ValueError::class, 'unknown style id');
    });

    it('rejects text() of a non-string and unwritable values', function () {
        $w = Writer::create(err_tmp('xlsx'));
        $fh = fopen('php://memory', 'r');
        expect(fn () => Writer::text(12))->toThrow(ValueError::class, 'expected a string')
            ->and(fn () => $w->addRow([$fh]))->toThrow(WriterException::class, 'cannot write a value of this type');
        fclose($fh);
    });
});

it('throws its exceptions itself; PHP code cannot construct them', function (string $class) {
    expect(fn () => new $class('custom'))->toThrow(Exception::class, 'cannot instantiate');
})->with([RustySheet\Exception::class, UnsupportedFormatException::class, ReaderException::class, WriterException::class]);
