<?php

declare(strict_types=1);

use RustySheet\Reader;
use RustySheet\UnsupportedFormatException;
use RustySheet\Writer;
use RustySheet\WriterException;

function writer_tmp(string $ext): string
{
    return sys_get_temp_dir().'/'.uniqid('rs_w_', true).'.'.$ext;
}

it('round-trips through the reader', function () {
    $path = writer_tmp('xlsx');
    $w = Writer::create($path);
    $w->addSheet('People');
    $w->addRow(['name', 'age', 'joined']);
    $w->addRow(['Ann', 41, new DateTimeImmutable('2024-02-29')]);
    $w->addRow(['=not a formula?', Writer::text('=kept as text'), true]);
    $w->close();

    $r = Reader::open($path);
    expect($r->sheetNames())->toBe(['People'])
        ->and($r->nextRows(0))->toBe([
            1 => ['name', 'age', 'joined'],
            2 => ['Ann', 41, 45351],
            3 => ['=not a formula?', '=kept as text', true],
        ]);
    unlink($path);
});

it('rejects invalid sheet names like OpenSpout', function (string $name, string $message) {
    $w = Writer::create(writer_tmp('xlsx'));
    $w->addSheet('Taken');
    expect(fn () => $w->addSheet($name))->toThrow(WriterException::class, $message);
})->with([
    'duplicate' => ['Taken', 'It should be unique'],
    'too long' => [str_repeat('x', 32), 'exceed 31 characters'],
    'bad char' => ['a/b', 'should not contain'],
    'quote' => ["'quoted'", 'single quote'],
    'reserved' => ['history', 'reserved name'],
    'blank' => ['', 'not be blank'],
]);

it('requires sheet setup before the first row', function () {
    $w = Writer::create(writer_tmp('xlsx'));
    $w->addRow(['x']);
    expect(fn () => $w->setColumnWidth(10, [1]))->toThrow(WriterException::class, 'first row')
        ->and(fn () => $w->freezePanes(1))->toThrow(WriterException::class, 'first row')
        ->and(fn () => $w->showGridLines(false))->toThrow(WriterException::class, 'first row');
});

it('rejects unwritable values', function (mixed $value) {
    $w = Writer::create(writer_tmp('xlsx'));
    expect(fn () => $w->addRow([$value]))->toThrow(WriterException::class);
})->with([
    'array' => [['rich', 'text']],
    'object' => [new stdClass()],
]);

it('validates options and styles', function (callable $call) {
    expect($call)->toThrow(ValueError::class);
})->with([
    'unknown option' => [fn () => Writer::create(writer_tmp('xlsx'), ['dates' => 'x'])],
    'bad csv delimiter' => [fn () => Writer::create(writer_tmp('csv'), ['csv' => ['delimiter' => '||']])],
    'unknown style key' => [fn () => Writer::create(writer_tmp('xlsx'))->addStyle(['colour' => 'FF0000'])],
    'wrong style type' => [fn () => Writer::create(writer_tmp('xlsx'))->addStyle(['bold' => 'yes'])],
    'column 0' => [fn () => Writer::create(writer_tmp('xlsx'))->setColumnWidth(10, [0])],
]);

it('declines formats it cannot write', function () {
    expect(fn () => Writer::create(writer_tmp('ods')))->toThrow(UnsupportedFormatException::class);
});

it('writes one CSV sheet only, and TSV with tabs by default', function () {
    $csv = Writer::create(writer_tmp('csv'));
    $csv->addSheet('only');
    expect(fn () => $csv->addSheet('second'))->toThrow(UnsupportedFormatException::class);

    $path = writer_tmp('tsv');
    $tsv = Writer::create($path, ['csv' => ['bom' => false]]);
    $tsv->addRow(['a', 'b c']);
    $tsv->close();
    expect(file_get_contents($path))->toBe("a\t\"b c\"\n");
    unlink($path);
});

it('throws after close', function () {
    $w = Writer::create(writer_tmp('xlsx'));
    $w->close();
    $w->close(); // closing twice is fine
    expect(fn () => $w->addRow(['x']))->toThrow(WriterException::class, 'closed');
});

describe('review regressions', function () {
    it('rejects alignments OpenSpout does not have', function () {
        $w = Writer::create(writer_tmp('xlsx'));
        expect(fn () => $w->addStyle(['align' => 'middle']))->toThrow(ValueError::class, 'not one of')
            ->and(fn () => $w->addStyle(['valign' => 'x"y']))->toThrow(ValueError::class, 'not one of')
            ->and($w->addStyle(['align' => 'justify', 'valign' => 'distributed']))->toBeInt();
    });

    it('keeps styles.xml valid for odd font names and colours', function () {
        $path = writer_tmp('xlsx');
        $w = Writer::create($path);
        $odd = $w->addStyle(['font' => 'A"B<&>', 'color' => '"bad', 'background' => '<x>']);
        $w->addRow([Writer::cell('x', $odd)]);
        $w->close();
        $zip = new ZipArchive();
        $zip->open($path);
        expect(simplexml_load_string($zip->getFromName('xl/styles.xml')))->not->toBeFalse()
            ->and(Reader::open($path)->nextRows(0))->toBe([1 => ['x']]);
        $zip->close();
        unlink($path);
    });

    it('refuses a default sheet name that is already taken', function () {
        $w = Writer::create(writer_tmp('xlsx'));
        $w->addSheet('Sheet2');
        expect(fn () => $w->addSheet())->toThrow(WriterException::class, 'It should be unique');
    });

    it('fails on an unknown style id before writing anything', function () {
        $path = writer_tmp('xlsx');
        $w = Writer::create($path);
        expect(fn () => $w->addRow(['a'], 999))->toThrow(WriterException::class, 'unknown style id')
            ->and(fn () => $w->addRow([new DateTimeImmutable()], null, [0 => 42]))->toThrow(WriterException::class)
            ->and(fn () => $w->addRow([Writer::cell('a', 7)]))->toThrow(WriterException::class);
        $w->setColumnWidth(12, [1]); // the sheet hasn't started
        $w->addRow(['b']);
        $w->close();
        expect(Reader::open($path)->nextRows(0))->toBe([1 => ['b']]);
        unlink($path);
    });

    it('finishes the file when the writer is dropped without close()', function () {
        $path = writer_tmp('xlsx');
        (function () use ($path) {
            $w = Writer::create($path);
            $w->addSheet('Kept');
            $w->addRow(['x', 1]);
        })();
        gc_collect_cycles();
        $r = Reader::open($path);
        expect($r->sheetNames())->toBe(['Kept'])->and($r->nextRows(0))->toBe([1 => ['x', 1]]);
        unlink($path);
    });

    it('rejects sizes beyond Excel limits instead of crashing', function () {
        $w = Writer::create(writer_tmp('xlsx'));
        expect(fn () => $w->freezePanes(0, 500000))->toThrow(WriterException::class, 'freeze')
            ->and(fn () => $w->freezePanes(4294967295))->toThrow(WriterException::class)
            ->and(fn () => $w->addRow(array_fill(0, 16385, 1)))->toThrow(WriterException::class, '16384');
    });
});
