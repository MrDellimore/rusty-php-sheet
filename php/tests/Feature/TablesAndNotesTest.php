<?php

declare(strict_types=1);

use PhpOffice\PhpSpreadsheet\IOFactory;
use PhpOffice\PhpSpreadsheet\Spreadsheet;
use PhpOffice\PhpSpreadsheet\Worksheet\Table;
use PhpOffice\PhpSpreadsheet\Worksheet\Table\TableStyle;
use PhpOffice\PhpSpreadsheet\Writer\Xlsx as XlsxWriter;
use RustySheet\Writer;
use RustySheet\WriterException;

/*
 * Phase 9. Validation and comment XML parity with OpenSpout is in WriterParityTest;
 * here: tables (beyond OpenSpout: PhpSpreadsheet is the reference, since litportal's
 * StylesReportWorksheets::frameTable() makes them), read-back, and argument errors.
 */

function t9_tmp(string $ext = 'xlsx'): string
{
    return sys_get_temp_dir().'/'.uniqid('rs_t9_', true).'.'.$ext;
}

function t9_part(string $path, string $name): string
{
    $zip = new ZipArchive();
    $zip->open($path);
    $xml = $zip->getFromName($name);
    $zip->close();

    return (string) $xml;
}

/** litportal's frameTable(): TableStyleNone, no stripes, over header + data. */
const T9_ROWS = [['Claimant', 'Amount', 'Status'], ['Ann', 12.5, 'Open'], ['Bob', 7, 'Closed'], ['Cy', 0, 'Open']];

function phpspreadsheet_table(string $path, string $range, string $name, string $theme, bool $rowStripes = false): void
{
    $book = new Spreadsheet();
    $sheet = $book->getActiveSheet();
    $sheet->fromArray(T9_ROWS);
    $table = new Table($range, $name);
    $table->setStyle((new TableStyle($theme))->setShowRowStripes($rowStripes)->setShowColumnStripes(false));
    $sheet->addTable($table);
    (new XlsxWriter($book))->save($path);
}

describe('tables', function () {
    it('writes the table part PhpSpreadsheet writes', function (string $theme, bool $rowStripes) {
        [$expected, $actual] = [t9_tmp(), t9_tmp()];
        phpspreadsheet_table($expected, 'A1:C4', 'Claims', $theme, $rowStripes);
        $w = Writer::create($actual);
        $w->addTable('A1:C4', 'Claims', ['style' => $theme, 'rowStripes' => $rowStripes]);
        foreach (T9_ROWS as $row) {
            $w->addRow($row);
        }
        $w->close();
        expect(t9_part($actual, 'xl/tables/table1.xml'))->toBe(t9_part($expected, 'xl/tables/table1.xml'));

        $read = IOFactory::load($actual)->getActiveSheet()->getTableByName('Claims');
        expect($read->getRange())->toBe('A1:C4')
            ->and($read->getStyle()->getTheme())->toBe($theme)
            ->and($read->getStyle()->getShowRowStripes())->toBe($rowStripes)
            ->and($read->getShowHeaderRow())->toBeTrue();
        unlink($expected);
        unlink($actual);
    })->with([
        'litportal frameTable' => [TableStyle::TABLE_STYLE_NONE, false],
        'default theme with stripes' => [TableStyle::TABLE_STYLE_MEDIUM2, true],
    ]);

    it('names columns from the header row, uniquely', function () {
        $path = t9_tmp();
        $w = Writer::create($path);
        $w->addSheet('Report');
        $w->addRow(['Title above the table']);
        $w->addTable('B2:H4', 'Totals', ['filter' => false, 'firstColumn' => true, 'lastColumn' => true, 'columnStripes' => true]);
        $w->addRow([null, 'Name', 2025, null, 'name', 1.5, '=1+1', true]);
        $w->addRow([null, 'a', 1, 2, 3, 4]);
        $w->addRow([null, 'b', 5, 6, 7, 8]);
        $w->addSheet('Second');
        $w->addTable('A1:B3', 'NoHeader', ['headerRow' => false]);
        $w->addRow([1, 2]);
        $w->close();

        $xml = t9_part($path, 'xl/tables/table1.xml');
        expect($xml)->toContain('<tableColumn id="1" name="Name"/><tableColumn id="2" name="2025"/><tableColumn id="3" name="Column3"/><tableColumn id="4" name="name2"/><tableColumn id="5" name="1.5"/><tableColumn id="6" name="=1+1"/><tableColumn id="7" name="TRUE"/>')
            ->and($xml)->not->toContain('<autoFilter')
            ->and($xml)->toContain('showFirstColumn="1" showLastColumn="1" showRowStripes="0" showColumnStripes="1"')
            ->and(t9_part($path, 'xl/tables/table2.xml'))->toContain('headerRowCount="0"')
            ->and(t9_part($path, '[Content_Types].xml'))->toContain('PartName="/xl/tables/table2.xml"');
        $book = IOFactory::load($path);
        expect($book->getSheetByName('Report')->getTableByName('Totals')->getRange())->toBe('B2:H4')
            ->and($book->getSheetByName('Second')->getTableByName('NoHeader')->getShowHeaderRow())->toBeFalse();
        unlink($path);
    });

    it('rejects what Excel would have to repair', function (callable $call, string $message) {
        $w = Writer::create(t9_tmp());
        $w->addTable('A1:C5', 'First');
        expect(fn () => $call($w))->toThrow(ValueError::class, $message);
    })->with([
        'duplicate name' => [fn (Writer $w) => $w->addTable('E1:F3', 'FIRST'), 'already exists'],
        'overlap' => [fn (Writer $w) => $w->addTable('C5:D9', 'Second'), 'overlaps the table First'],
        'merge inside' => [fn (Writer $w) => $w->mergeCells('A2:B2'), 'tables can\'t contain merged cells'],
        'autofilter over it' => [fn (Writer $w) => $w->setAutoFilter('A1:C1'), 'has its own filter'],
        'header only' => [fn (Writer $w) => $w->addTable('E1:F1', 'Lonely'), 'no row below it'],
        'cell reference' => [fn (Writer $w) => $w->addTable('E1:F3', 'AB12'), 'cell reference'],
        'spaces' => [fn (Writer $w) => $w->addTable('E1:F3', 'My table'), 'only letters, digits'],
        'leading digit' => [fn (Writer $w) => $w->addTable('E1:F3', '2025'), 'must start with'],
        'bad option' => [fn (Writer $w) => $w->addTable('E1:F3', 'T', ['stripes' => true]), 'unknown table option'],
        'option type' => [fn (Writer $w) => $w->addTable('E1:F3', 'T', ['rowStripes' => 'yes']), 'expected a bool'],
        'style type' => [fn (Writer $w) => $w->addTable('E1:F3', 'T', ['style' => 1]), 'expected a string'],
    ]);

    it('rejects a table over a merge or the autofilter declared first', function () {
        $w = Writer::create(t9_tmp());
        $w->mergeCells('A1:B1');
        $w->setAutoFilter('D1:E1');
        expect(fn () => $w->addTable('A1:B3', 'M'))->toThrow(ValueError::class, 'overlaps the merged range A1:B1')
            ->and(fn () => $w->addTable('D1:E3', 'F'))->toThrow(ValueError::class, 'overlaps the autofilter D1:E1');
    });

    it('must be added before its header row is written', function () {
        $w = Writer::create(t9_tmp());
        $w->addRow(['header']);
        expect(fn () => $w->addTable('A1:A3', 'Late'))->toThrow(WriterException::class, 'after its first row');
    });

    it('is ignored for CSV', function () {
        $path = t9_tmp('csv');
        $w = Writer::create($path, ['csv' => ['bom' => false]]);
        $w->addTable('A1:B2', 'T');
        $w->addValidation('A1:A2', ['type' => 'list', 'values' => ['a']]);
        $w->addRow([Writer::cell('a', null, 'a note'), Writer::text('b', null, 'another')]);
        $w->close();
        expect(file_get_contents($path))->toBe("a,b\n");
        unlink($path);
    });
});

describe('validation and comments read back', function () {
    it('in PhpSpreadsheet', function () {
        $path = t9_tmp();
        $w = Writer::create($path);
        $w->addValidation('B2:B9', ['type' => 'list', 'values' => ['Yes', 'No'], 'errorTitle' => 'Invalid selection', 'error' => 'Please choose one of the listed options.']);
        // litportal's MissingInfoWorkbookRenderer: a formula value, beyond OpenSpout.
        $w->addValidation('C2:C9', ['type' => 'date', 'operator' => 'greaterThanOrEqual', 'value1' => '=DATE(1900,1,1)', 'errorTitle' => 'Invalid date', 'error' => 'Please enter a valid date.']);
        $w->addRow(['Name', Writer::text('Answer', null, ['runs' => [['text' => 'Pick ', 'bold' => true], ['text' => 'yes or no']]]), 'When', Writer::link('Help', 'https://example.com/help', null, 'Opens help', 'and a note')]);
        $w->close();

        $sheet = IOFactory::load($path)->getActiveSheet();
        $list = $sheet->getDataValidation('B2');
        $date = $sheet->getDataValidation('C5');
        expect($list->getType())->toBe('list')
            ->and($list->getFormula1())->toBe('"Yes,No"')
            ->and($list->getErrorTitle())->toBe('Invalid selection')
            ->and($date->getType())->toBe('date')
            ->and($date->getOperator())->toBe('greaterThanOrEqual')
            ->and($date->getFormula1())->toBe('DATE(1900,1,1)')
            ->and($sheet->getComment('B1')->getText()->getPlainText())->toBe('Pick yes or no')
            ->and($sheet->getComment('D1')->getText()->getPlainText())->toBe('and a note')
            ->and($sheet->getCell('D1')->getHyperlink()->getUrl())->toBe('https://example.com/help');
        unlink($path);
    });

    it('rejects bad rules', function (array $rule, string $message) {
        expect(fn () => Writer::create(t9_tmp())->addValidation('A1:A9', $rule))->toThrow(ValueError::class, $message);
    })->with([
        'no type' => [[], 'rule.type'],
        'unknown type' => [['type' => 'colour', 'operator' => 'equal', 'value1' => 1], 'unknown validation type'],
        'list without values' => [['type' => 'list'], "either 'values' or 'source'"],
        'list with both' => [['type' => 'list', 'values' => ['a'], 'source' => 'A1:A3'], "either 'values' or 'source'"],
        'comma in a value' => [['type' => 'list', 'values' => ['a,b']], 'contains a comma'],
        'list too long' => [['type' => 'list', 'values' => [str_repeat('x', 256)]], 'at most 255'],
        'values not strings' => [['type' => 'list', 'values' => [1, 2]], 'list of strings'],
        'values not a list' => [['type' => 'list', 'values' => 'a'], 'list of strings'],
        'no operator' => [['type' => 'whole', 'value1' => 1], 'rule.operator'],
        'unknown operator' => [['type' => 'whole', 'operator' => 'near', 'value1' => 1], 'unknown validation operator'],
        'no value1' => [['type' => 'decimal', 'operator' => 'equal'], 'rule.value1'],
        'between needs value2' => [['type' => 'whole', 'operator' => 'between', 'value1' => 1], 'second value'],
        'value of the wrong type' => [['type' => 'whole', 'operator' => 'equal', 'value1' => [1]], 'expected an int'],
        'custom without formula' => [['type' => 'custom'], 'rule.formula'],
        'unknown error style' => [['type' => 'custom', 'formula' => 'A1>0', 'errorStyle' => 'loud'], 'unknown error style'],
        'unknown key' => [['type' => 'custom', 'formula' => 'A1>0', 'hint' => 'x'], 'unknown rule key'],
        'flag type' => [['type' => 'custom', 'formula' => 'A1>0', 'allowBlank' => 'no'], 'expected a bool'],
        'text type' => [['type' => 'custom', 'formula' => 'A1>0', 'prompt' => 5], 'expected a string'],
        'bad range' => [['type' => 'custom', 'formula' => 'A1>0', 'range' => 'x'], 'unknown rule key'],
    ]);

    it('rejects a bad range', function () {
        expect(fn () => Writer::create(t9_tmp())->addValidation('nope', ['type' => 'custom', 'formula' => 'A1>0']))
            ->toThrow(ValueError::class, 'not a cell range');
    });

    it('rejects bad comments', function (mixed $comment, string $message) {
        expect(fn () => Writer::cell('x', null, $comment))->toThrow(ValueError::class, $message);
    })->with([
        'not text or array' => [5, 'expected a string or an array'],
        'unknown key' => [['note' => 'x'], 'unknown comment key'],
        'visible type' => [['visible' => 'yes'], 'expected a bool'],
        'box size type' => [['width' => 100], 'expected a string'],
        'runs not a list' => [['runs' => 'x'], 'list of runs'],
        'run not an array' => [['runs' => ['x']], 'each run must be an array'],
        'run without text' => [['runs' => [['bold' => true]]], 'needs a text'],
        'run key' => [['runs' => [['text' => 'x', 'underline' => true]]], 'unknown run key'],
        'run size' => [['runs' => [['text' => 'x', 'size' => '10']]], 'expected an int'],
        'run flag' => [['runs' => [['text' => 'x', 'italic' => 1]]], 'expected a bool'],
        'run text' => [['runs' => [['text' => 5]]], 'expected a string'],
    ]);
});
