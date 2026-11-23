<?php

declare(strict_types=1);

use RustySheet\Reader;

/*
 * The CSV parser is a port of PHP's fgetcsv() as it behaves with a UTF-8 LC_CTYPE
 * (see trim_line_end in fgetcsv.rs), so PHP itself is the reference:
 * thousands of random small inputs built from the characters that drive its state
 * machine (enclosures, escapes, delimiters, line breaks, spaces) must parse
 * identically. A fixed seed keeps failures reproducible.
 */

/** OpenSpout's CSV row loop: fgetcsv() records, blank lines ([null]) skipped. */
function fgetcsv_rows(string $path, string $escape): array
{
    $fh = fopen($path, 'r');
    $rows = [];
    while (($row = fgetcsv($fh, null, ',', '"', $escape)) !== false) {
        if ($row !== [null]) {
            $rows[] = $row;
        }
    }
    fclose($fh);

    return $rows;
}

it('parses random input exactly like fgetcsv()', function (string $escape) {
    $previous = setlocale(LC_CTYPE, '0');
    if (setlocale(LC_CTYPE, 'C.UTF-8', 'en_US.UTF-8', 'UTF-8') === false) {
        $this->markTestSkipped('no UTF-8 locale available');
    }
    mt_srand(20261123);
    // Invalid UTF-8 (\xFF, and the halves of a split 'é') matters too: PHP steps
    // over it with mblen() when looking for the line ending.
    $alphabet = ['a', 'b', '"', '"', ',', "\n", "\r\n", "\r", ' ', "\t", '\\', "\0", 'é', "\xFF", "\xC3", "\xA9"];
    $path = sys_get_temp_dir().'/'.uniqid('rs_diff_', true).'.csv';
    try {
        for ($i = 0; $i < 3000; $i++) {
            $input = '';
            for ($n = mt_rand(1, 32); $n > 0; $n--) {
                $input .= $alphabet[mt_rand(0, count($alphabet) - 1)];
            }
            file_put_contents($path, $input);
            $actual = array_values(Reader::open($path, ['csv' => ['escape' => $escape]])->nextRows(0, 1000));
            expect($actual)->toBe(fgetcsv_rows($path, $escape), 'input (hex): '.bin2hex($input));
        }
    } finally {
        @unlink($path);
        setlocale(LC_CTYPE, $previous);
    }
})->with(['no escape' => [''], 'backslash escape' => ['\\'], 'NUL escape' => ["\0"]]);
