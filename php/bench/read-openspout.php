<?php

declare(strict_types=1);

// Baseline: stream every row of a workbook through OpenSpout, as sheet-stream does.
// Usage: php php/bench/read-openspout.php <file.xlsx|file.csv>

require __DIR__ . '/../vendor/autoload.php';



$path = $argv[1] ?? exit("usage: php php/bench/read-openspout.php <file.xlsx>\n");

$start = hrtime(true);
$reader = str_ends_with(strtolower($path), '.xlsx') ? new OpenSpout\Reader\XLSX\Reader() : new OpenSpout\Reader\CSV\Reader();
$reader->open($path);
$rows = 0;
$cells = 0;
foreach ($reader->getSheetIterator() as $sheet) {
    foreach ($sheet->getRowIterator() as $row) {
        $cells += count($row->toArray());
        $rows++;
    }
}
$reader->close();
$secs = (hrtime(true) - $start) / 1e9;

printf(
    "openspout  %s\n  rows: %d  cells: %d  time: %.2fs  rows/s: %.0f  peak memory: %.1f MB\n",
    basename($path), $rows, $cells, $secs, $rows / $secs, memory_get_peak_usage(true) / 1048576,
);
