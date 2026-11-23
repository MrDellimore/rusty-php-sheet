<?php

declare(strict_types=1);

// Stream every row of a workbook through the rusty_sheet extension.
// Usage: php -d extension=target/release/librusty_sheet.dylib php/bench/read-rusty.php <file.xlsx> [dates-mode] [batch]

use RustySheet\Reader;

$path = $argv[1] ?? exit("usage: php php/bench/read-rusty.php <file.xlsx> [serial|datetime] [batch]\n");
$dates = $argv[2] ?? 'serial';
$batch = (int) ($argv[3] ?? 1000);

$start = hrtime(true);
$reader = Reader::open($path, ['dates' => $dates]);
$rows = 0;
$cells = 0;
foreach ($reader->sheetNames() as $i => $_) {
    while (($batchRows = $reader->nextRows($i, $batch)) !== []) {
        foreach ($batchRows as $row) {
            $cells += count($row);
            $rows++;
        }
    }
}
$reader->close();
$secs = (hrtime(true) - $start) / 1e9;

printf(
    "rusty_sheet  %s  (dates=%s, batch=%d)\n  rows: %d  cells: %d  time: %.2fs  rows/s: %.0f  PHP peak memory: %.1f MB\n",
    basename($path), $dates, $batch, $rows, $cells, $secs, $rows / $secs, memory_get_peak_usage(true) / 1048576,
);
