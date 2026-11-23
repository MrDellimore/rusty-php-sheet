<?php

declare(strict_types=1);

use PhpOffice\PhpSpreadsheet\IOFactory;
use RustySheet\Writer;
use RustySheet\WriterException;

/*
 * Phase 7: borders, merged cells, autofilters and hyperlinks. XML parity with
 * OpenSpout is in WriterParityTest; here: argument checks, CSV behaviour, and how
 * PhpSpreadsheet (what litportal uses today) reads the files back, which is the
 * reference for what goes beyond OpenSpout (`hair` borders, `sheet://` links).
 */

function layout_tmp(string $ext): string
{
    return sys_get_temp_dir().'/'.uniqid('rs_layout_', true).'.'.$ext;
}

describe('borders', function () {
    it('reads back in PhpSpreadsheet, hair included', function () {
        $path = layout_tmp('xlsx');
        $w = Writer::create($path);
        $hair = $w->addStyle(['border' => ['all' => ['style' => 'hair', 'color' => 'A6A6A6']]]);
        $mixed = $w->addStyle(['border' => [
            'top' => ['style' => 'double'],
            'bottom' => ['style' => 'dashed', 'width' => 'medium', 'color' => 'FF0000'],
            'left' => ['style' => 'dotted', 'width' => 'thin'],
            'right' => ['style' => 'solid', 'width' => 'thick'],
        ]]);
        $w->addRow([Writer::cell('hair', $hair), Writer::cell('mixed', $mixed)]);
        $w->close();

        $sheet = IOFactory::load($path)->getActiveSheet();
        $hairBorders = $sheet->getStyle('A1')->getBorders();
        foreach (['getLeft', 'getRight', 'getTop', 'getBottom'] as $side) {
            expect($hairBorders->{$side}()->getBorderStyle())->toBe('hair')
                ->and($hairBorders->{$side}()->getColor()->getRGB())->toBe('A6A6A6');
        }
        $mixedBorders = $sheet->getStyle('B1')->getBorders();
        expect($mixedBorders->getTop()->getBorderStyle())->toBe('double')
            ->and($mixedBorders->getBottom()->getBorderStyle())->toBe('mediumDashed')
            ->and($mixedBorders->getBottom()->getColor()->getRGB())->toBe('FF0000')
            ->and($mixedBorders->getLeft()->getBorderStyle())->toBe('dotted')
            ->and($mixedBorders->getRight()->getBorderStyle())->toBe('thick');
        unlink($path);
    });

    it('rejects bad border specs', function (array $border, string $message) {
        expect(fn () => Writer::create(layout_tmp('xlsx'))->addStyle(['border' => $border]))
            ->toThrow(ValueError::class, $message);
    })->with([
        'unknown side' => [['middle' => []], 'unknown side'],
        'part not an array' => [['top' => 'thin'], 'expected an array'],
        'unknown style' => [['top' => ['style' => 'wavy']], "'wavy' is not one of"],
        'unknown width' => [['top' => ['width' => 'huge']], "'huge' is not one of"],
        'unknown key' => [['top' => ['weight' => 'thin']], 'unknown border key'],
        'value not a string' => [['top' => ['color' => 0xFF0000]], 'expected a string'],
    ]);

    it('rejects a border that is not an array', function () {
        expect(fn () => Writer::create(layout_tmp('xlsx'))->addStyle(['border' => 'all']))
            ->toThrow(ValueError::class, 'expected an array');
    });
});

describe('merged cells', function () {
    it('reads back in PhpSpreadsheet', function () {
        $path = layout_tmp('xlsx');
        $w = Writer::create($path);
        $w->addSheet('First'); // names the initial sheet
        $w->mergeCells('A1:D1');
        $w->addRow(['Title']);
        $w->mergeCells('$b$3:$a$2'); // corners in any order, absolute refs accepted
        $w->addSheet('Next');
        $w->mergeCells('C5:E9');
        $w->close();

        $book = IOFactory::load($path);
        expect(array_values($book->getSheet(0)->getMergeCells()))->toBe(['A1:D1', 'A2:B3'])
            ->and(array_values($book->getSheet(1)->getMergeCells()))->toBe(['C5:E9']);
        unlink($path);
    });

    it('rejects ranges that Excel would have to repair', function (string $first, string $second, string $message) {
        $w = Writer::create(layout_tmp('xlsx'));
        $w->mergeCells($first);
        expect(fn () => $w->mergeCells($second))->toThrow(ValueError::class, $message);
    })->with([
        'overlap' => ['A1:C3', 'C3:D4', 'overlaps the merged range A1:C3'],
        'contained' => ['A1:C3', 'B2:B3', 'overlaps'],
        'single cell' => ['A1:B1', 'D4', 'at least two cells'],
        'not a range' => ['A1:B1', 'A1:B', 'is not a cell range'],
        'past XFD' => ['A1:B1', 'A1:XFE1', 'is not a cell range'],
    ]);

    it('only merges ranges of the current sheet', function () {
        $path = layout_tmp('xlsx');
        $w = Writer::create($path);
        $w->addSheet('One');
        $w->mergeCells('A1:B1');
        $w->addSheet('Two');
        $w->mergeCells('A1:B1'); // no overlap: another sheet
        $w->close();
        expect(array_values(IOFactory::load($path)->getSheet(1)->getMergeCells()))->toBe(['A1:B1']);
        unlink($path);
    });
});

describe('autofilter', function () {
    it('reads back in PhpSpreadsheet', function () {
        $path = layout_tmp('xlsx');
        $w = Writer::create($path);
        $w->setAutoFilter('A1:C4');
        foreach ([['name', 'n', 'ok'], ['a', 1, true], ['b', 2, false], ['c', 3, true]] as $row) {
            $w->addRow($row);
        }
        $w->close();
        expect(IOFactory::load($path)->getActiveSheet()->getAutoFilter()->getRange())->toBe('A1:C4');
        unlink($path);
    });

    it('must be set before the first row, and be a range', function () {
        $w = Writer::create(layout_tmp('xlsx'));
        expect(fn () => $w->setAutoFilter('1:4'))->toThrow(ValueError::class, 'is not a cell range');
        $w->addRow(['x']);
        expect(fn () => $w->setAutoFilter('A1:B2'))->toThrow(WriterException::class, 'first row');
    });
});

describe('hyperlinks', function () {
    it('reads back in PhpSpreadsheet, with sheet:// for places in the workbook', function () {
        $path = layout_tmp('xlsx');
        $w = Writer::create($path);
        $w->addSheet('Summary');
        $w->addRow([
            Writer::link('Detail', "#'Detail sheet'!A1"),
            Writer::link('Detail B2', "sheet://'Detail sheet'!B2", null, 'Jump'),
            Writer::link('Site', 'https://example.com/?a=1&b=2', null, 'Opens a browser'),
        ]);
        $w->addSheet('Detail sheet');
        $w->addRow(['here']);
        $w->close();

        $sheet = IOFactory::load($path)->getSheetByName('Summary');
        expect((string) $sheet->getCell('A1')->getValue())->toBe('Detail')
            ->and($sheet->getCell('A1')->getHyperlink()->getUrl())->toBe("sheet://'Detail sheet'!A1")
            ->and($sheet->getCell('B1')->getHyperlink()->getUrl())->toBe("sheet://'Detail sheet'!B2")
            ->and($sheet->getCell('B1')->getHyperlink()->getTooltip())->toBe('Jump')
            ->and($sheet->getCell('C1')->getHyperlink()->getUrl())->toBe('https://example.com/?a=1&b=2')
            ->and($sheet->getCell('C1')->getHyperlink()->getTooltip())->toBe('Opens a browser');
        unlink($path);
    });

    it('writes only the text to CSV', function () {
        $path = layout_tmp('csv');
        $w = Writer::create($path, ['csv' => ['bom' => false]]);
        $w->mergeCells('A1:B1'); // ignored for CSV, like other sheet setup
        $w->setAutoFilter('A1:B1');
        $w->addRow([Writer::link('Site', 'https://example.com'), Writer::link('=text', '#A1')]);
        $w->close();
        expect(file_get_contents($path))->toBe("Site,=text\n");
        unlink($path);
    });

    it('needs a target', function (string $target) {
        expect(fn () => Writer::link('x', $target))->toThrow(ValueError::class, 'needs a target');
    })->with(['', '#', 'sheet://']);
});
