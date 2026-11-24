<?php

declare(strict_types=1);

use RustySheet\Reader;
use RustySheet\ReaderException;
use RustySheet\UnsupportedFormatException;

describe('opening', function () {
    it('lists sheet names in workbook order without reading rows', function () {
        $reader = Reader::open(fixtures_path('edge/multi-sheet.xlsx'));

        expect($reader->sheetNames())->toBe(['Zeta', 'Alpha & Ω', 'Empty', 'Last']);
    });

    it('rejects formats it does not read, so callers can fall back', function (string $file) {
        expect(fn () => Reader::open(fixtures_path($file)))
            ->toThrow(UnsupportedFormatException::class);
    })->with([
        'xls' => 'laravel-excel/import-external-reference.xls',
    ]);

    it('honours an explicit format over the extension', function () {
        expect(fn () => Reader::open(fixtures_path('edge/dates.xlsx'), ['format' => 'ods']))
            ->toThrow(UnsupportedFormatException::class);
    });

    it('throws ReaderException for a missing file', function () {
        expect(fn () => Reader::open('/nonexistent/file.xlsx'))->toThrow(ReaderException::class);
    });

    it('throws ReaderException for an .xlsx that is not a zip', function () {
        $tmp = tempnam(sys_get_temp_dir(), 'rs') . '.xlsx';
        file_put_contents($tmp, "id,name\n1,a\n");
        try {
            expect(fn () => Reader::open($tmp))->toThrow(ReaderException::class, 'zip');
        } finally {
            unlink($tmp);
        }
    });

    it('puts the extension exceptions under one base class', function () {
        expect(is_subclass_of(UnsupportedFormatException::class, RustySheet\Exception::class))->toBeTrue()
            ->and(is_subclass_of(ReaderException::class, RustySheet\Exception::class))->toBeTrue()
            ->and(is_subclass_of(RustySheet\Exception::class, Exception::class))->toBeTrue();
    });

    it('cannot be constructed directly', function () {
        expect(fn () => new Reader())->toThrow(Exception::class, 'cannot instantiate');
    });
});

describe('options', function () {
    it('rejects invalid options with ValueError', function (array $options, string $message) {
        expect(fn () => Reader::open(fixtures_path('edge/dates.xlsx'), $options))
            ->toThrow(ValueError::class, $message);
    })->with([
        'unknown key' => [['datez' => 'serial'], 'unknown option'],
        'bad dates' => [['dates' => 'carbon'], "expected 'serial', 'datetime' or 'formatted'"],
        'timezone with formatted' => [['dates' => 'formatted', 'timezone' => 'UTC'], "only applies with dates => 'datetime'"],
        'wrong type' => [['preserve_empty_rows' => 'yes'], 'expected a bool'],
        'bad on_error' => [['on_error' => 'ignore'], "expected 'throw' or 'skip_row'"],
        'unknown timezone' => [['dates' => 'datetime', 'timezone' => 'Mars/Base'], 'unknown timezone'],
        'timezone without datetime' => [['timezone' => 'UTC'], "only applies with dates => 'datetime'"],
        'list instead of map' => [['serial'], 'option keys must be strings'],
        'csv not an array' => [['csv' => ';'], 'expected an array'],
        'csv unknown key' => [['csv' => ['separator' => ';']], 'unknown option'],
        'csv multi-char delimiter' => [['csv' => ['delimiter' => '::']], 'single character'],
    ]);
});

describe('nextRows', function () {
    it('keys rows by source row number and resumes across batches', function () {
        $reader = Reader::open(fixtures_path('edge/layout.xlsx'));

        expect(array_keys($reader->nextRows(0, 2)))->toBe([1, 2])
            ->and(array_keys($reader->nextRows(0, 2)))->toBe([6, 7])
            ->and(array_keys($reader->nextRows(0, 2)))->toBe([10, 11])
            ->and($reader->nextRows(0, 2))->toBe([])
            ->and($reader->nextRows(0))->toBe([]); // stays finished
    });

    it('selects sheets by name and allows reading them interleaved', function () {
        $reader = Reader::open(fixtures_path('edge/layout.xlsx'));

        $layout = $reader->nextRows('Layout', 1);
        $strings = $reader->nextRows('Strings', 1);
        $layoutNext = $reader->nextRows(0, 1);

        expect($layout)->toBe([1 => ['a', '', 'c', '', '', 'f']])
            ->and($strings)->toBe([1 => ['plain', 'plain']])
            ->and(array_keys($layoutNext))->toBe([2]);
    });

    it('returns [] for an empty sheet', function () {
        expect(Reader::open(fixtures_path('edge/multi-sheet.xlsx'))->nextRows('Empty'))->toBe([]);
    });

    it('rejects unknown sheets and bad arguments', function (mixed $sheet, int $max, string $class) {
        $reader = Reader::open(fixtures_path('edge/multi-sheet.xlsx'));

        expect(fn () => $reader->nextRows($sheet, $max))->toThrow($class);
    })->with([
        'index too high' => [9, 10, ReaderException::class],
        'negative index' => [-1, 10, ReaderException::class],
        'unknown name' => ['Nope', 10, ReaderException::class],
        'float sheet' => [1.5, 10, ValueError::class],
        'zero max' => [0, 0, ValueError::class],
    ]);

    it('converts values with PHP types', function () {
        $rows = Reader::open(fixtures_path('edge/cell-types.xlsx'))->nextRows(0);
        $b = array_map(fn ($row) => array_key_exists(1, $row) ? $row[1] : '<none>', array_values($rows));

        expect($b)->toBe([
            'hello shared', 'hello inline', 1500, 0.075, -42, 0.0015, 1000, 1.0E+20, 3, 7,
            true, false, null, null, 'plain str', 45823.572916666664, '<none>', '', null, '',
        ]);
    });
});

describe('dates', function () {
    it('yields serials by default, with true values for time-only cells', function () {
        $rows = Reader::open(fixtures_path('edge/dates.xlsx'))->nextRows(0);

        // Row 3: built-in format 14 (m/d/yyyy). sheet-stream today gives -0.25 for 0.75.
        expect(array_slice($rows[3], 1))->toBe([45823, 45823.5, 0.75, 1, 59, 60, 61, 0, 45823 + 86399 / 86400]);
    });

    it('yields DateTimeImmutable and DateInterval in datetime mode', function () {
        $rows = Reader::open(fixtures_path('edge/dates.xlsx'), ['dates' => 'datetime'])->nextRows(0);

        expect($rows[3][1])->toBeInstanceOf(DateTimeImmutable::class)
            ->and($rows[3][1]->format('Y-m-d H:i:s'))->toBe('2025-06-15 00:00:00')
            ->and($rows[13][3])->toBeInstanceOf(DateInterval::class)
            ->and($rows[13][3]->h)->toBe(18);
    });

    it('applies the timezone in datetime mode', function () {
        $rows = Reader::open(fixtures_path('edge/dates.xlsx'), [
            'dates' => 'datetime', 'timezone' => 'America/New_York',
        ])->nextRows(0);

        expect($rows[3][1]->getTimezone()->getName())->toBe('America/New_York');
    });

    it('normalises 1904 workbooks to 1900-system serials', function () {
        $rows = Reader::open(fixtures_path('edge/dates-1904.xlsx'))->nextRows(0);

        expect($rows[1][1])->toBe(1462)       // 1904-01-01
            ->and($rows[3][1])->toBe(45823)   // 2025-06-15
            ->and($rows[1][3])->toBe(0);      // General: untouched
    });
});

describe('csv', function () {
    it('reads CSV as one sheet named "", keyed by line number, without trimming', function () {
        $reader = Reader::open(fixtures_path('edge/quirks.csv'));

        expect($reader->sheetNames())->toBe([''])
            ->and($reader->nextRows('', 3))->toBe([
                2 => ['id', 'name', 'note'],
                3 => ['1', 'plain', 'quoted, with comma'],
                4 => ['2', 'say "hi"', "multi\nline"],
            ]);
    });

    it('defaults to a tab delimiter for .tsv', function () {
        expect(Reader::open(fixtures_path('edge/tabs.tsv'))->nextRows(0)[2])->toBe(['1', "tab\there", 'x']);
    });

    it('takes delimiter, enclosure and escape options', function () {
        $path = tempnam(sys_get_temp_dir(), 'rs').'.csv';
        file_put_contents($path, "a|'b|c'|'it\\'s'\n");
        try {
            $row = Reader::open($path, ['csv' => ['delimiter' => '|', 'enclosure' => "'", 'escape' => '\\']])->nextRows(0)[1];
            // Like PHP's fgetcsv, the escape character is kept in the value.
            expect($row)->toBe(['a', 'b|c', "it\\'s"]);
        } finally {
            unlink($path);
        }
    });

    it('declines encodings other than UTF-8, so callers fall back to OpenSpout', function () {
        expect(fn () => Reader::open(fixtures_path('edge/quirks.csv'), ['csv' => ['encoding' => 'ISO-8859-1']]))
            ->toThrow(UnsupportedFormatException::class, 'only UTF-8');
        expect(Reader::open(fixtures_path('edge/quirks.csv'), ['csv' => ['encoding' => 'utf-8']])->sheetNames())->toBe(['']);
    });

    it('passes invalid UTF-8 through as raw bytes', function () {
        expect(Reader::open(fixtures_path('edge/latin1-bytes.csv'))->nextRows(0)[2][0])->toBe("caf\xE9");
    });
});

describe('text starting with "="', function () {
    it('is kept with calculated_formulas (OpenSpout replaces it with null)', function () {
        $path = tempnam(sys_get_temp_dir(), 'rs').'.xlsx';
        $writer = new OpenSpout\Writer\XLSX\Writer;
        $writer->openToFile($path);
        $writer->addRow(new OpenSpout\Common\Entity\Row([new OpenSpout\Common\Entity\Cell\StringCell('=== Totals ===', null)]));
        $writer->close();
        try {
            expect(Reader::open($path, ['calculated_formulas' => true])->nextRows(0))->toBe([1 => ['=== Totals ===']])
                ->and(RustySheet\Dev\SheetStreamReference::read($path, ['calculated_formulas' => true])['Sheet1'])->toBe([[null]]);
        } finally {
            unlink($path);
        }
    });
});

describe('formatted dates', function () {
    it('renders dates through their number format, like OpenSpout SHOULD_FORMAT_DATES', function () {
        $rows = Reader::open(fixtures_path('edge/date-formats.xlsx'), ['dates' => 'formatted'])->nextRows(0);
        $byCode = array_column(array_map(fn ($r) => [$r[0], $r[1]], $rows), 1, 0);

        expect($byCode['m/d/yyyy'])->toBe('6/15/2025')
            ->and($byCode['dddd, mmmm d, yyyy'])->toBe('Sunday, June 15, 2025')
            ->and($byCode['h:mm AM/PM'])->toBe('12:00 PM')
            ->and($byCode['"Day "d" of "mmmm'])->toBe('Day 15 of June')
            ->and($byCode['yyyy"年"m"月"d"日"'])->toBe('2025年6月15日')
            ->and($byCode['[h]:mm:ss'])->toBe('1099764:00:00');
    });

    it('leaves numbers that are not dates untouched', function () {
        $rows = Reader::open(fixtures_path('edge/dates.xlsx'), ['dates' => 'formatted'])->nextRows(0);

        expect($rows[2][1])->toBe(45823)          // General
            ->and($rows[3][1])->toBe('6/15/2025'); // built-in 14
    });
});

describe('formulas', function () {
    it('returns formula text by default and expands shared formulas', function () {
        $rows = Reader::open(fixtures_path('edge/formulas.xlsx'))->nextRows(0);

        expect(array_column($rows, 3))->toBe([
            '=B1+C1', '="a"&"b"', '=B3>C3', '=B4/C4', '=B5*2', '=B6*2', '=B7*2',
            '=B8*3', '=B9', '=B10/C10', '=SUM(B11:C11)', '=B12*C12/1',
        ]);
    });

    it('returns cached results with calculated_formulas', function () {
        $rows = Reader::open(fixtures_path('edge/formulas.xlsx'), ['calculated_formulas' => true])->nextRows(0);

        expect(array_column($rows, 3))->toBe([30, 'ab', false, null, 10, 12, 14, null, 45823, 3.3333333333333335, 9, 6]);
    });

    describe('formulas_without_cached_value', function () {
        // Row 4 cached an error (#DIV/0!); row 8 was never calculated.
        it('gives null by default and with null', function (array $extra) {
            $rows = Reader::open(fixtures_path('edge/formulas.xlsx'), ['calculated_formulas' => true] + $extra)->nextRows(0);

            expect($rows[4][3])->toBeNull()->and($rows[8][3])->toBeNull();
        })->with(['default' => [[]], 'null' => [['formulas_without_cached_value' => 'null']]]);

        it('gives the formula text with formula, and still null for a cached error', function () {
            $rows = Reader::open(fixtures_path('edge/formulas.xlsx'), [
                'calculated_formulas' => true,
                'formulas_without_cached_value' => 'formula',
            ])->nextRows(0);

            expect(array_column($rows, 3))->toBe([30, 'ab', false, null, 10, 12, 14, '=B8*3', 45823, 3.3333333333333335, 9, 6]);
        });

        it('throws naming the cell with throw, but not for a cached error', function () {
            $reader = Reader::open(fixtures_path('edge/formulas.xlsx'), [
                'calculated_formulas' => true,
                'formulas_without_cached_value' => 'throw',
            ]);

            expect($reader->nextRows(0, 7))->toHaveCount(7)
                ->and(fn () => $reader->nextRows(0))->toThrow(ReaderException::class, 'formulas.xlsx: Formulas!D8 has a formula with no cached value (=B8*3)')
                // The sheet is finished: a retry doesn't skip past the bad row.
                ->and($reader->nextRows(0))->toBe([]);
        });

        it('is ignored without calculated_formulas', function () {
            $rows = Reader::open(fixtures_path('edge/formulas.xlsx'), ['formulas_without_cached_value' => 'throw'])->nextRows(0);

            expect($rows[8][3])->toBe('=B8*3');
        });
    });
});

it('closes', function () {
    $reader = Reader::open(fixtures_path('edge/dates.xlsx'));
    $reader->close();

    expect(fn () => $reader->sheetNames())->toThrow(ReaderException::class, 'closed')
        ->and(fn () => $reader->nextRows(0))->toThrow(ReaderException::class, 'closed');
});
