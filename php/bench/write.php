<?php

declare(strict_types=1);

// Write N rows × 50 mixed columns with OpenSpout or the extension.
// Usage: php [-d extension=…] php/bench/write.php <openspout|rusty> <out.xlsx|out.csv> [rows]

require __DIR__.'/../vendor/autoload.php';

use OpenSpout\Common\Entity\Cell;
use OpenSpout\Common\Entity\Row;
use OpenSpout\Common\Entity\Style\Style;

[$_, $driver, $path] = $argv;
$rows = (int) ($argv[3] ?? 400_000);
$csv = str_ends_with($path, '.csv');

$date = new DateTimeImmutable('2025-06-15 14:30:00');
$row = static function (int $i) use ($date): array {
    $out = [];
    for ($c = 0; $c < 50; $c++) {
        $out[] = match ($c % 6) {
            0 => $i,
            1 => 'name '.($i % 997),
            2 => $i / 7,
            3 => $date,
            4 => ($i % 2) === 0,
            5 => ($i % 5) === 0 ? null : 'note '.$i,
        };
    }

    return $out;
};

$start = hrtime(true);
if ($driver === 'openspout') {
    $w = $csv ? new OpenSpout\Writer\CSV\Writer() : new OpenSpout\Writer\XLSX\Writer();
    $w->openToFile($path);
    $dateStyle = (new Style())->withFormat('yyyy-mm-dd hh:mm:ss');
    for ($i = 0; $i < $rows; $i++) {
        $w->addRow(new Row(array_map(
            fn ($v) => $v instanceof DateTimeInterface ? new Cell\DateTimeCell($v, $dateStyle) : Cell::fromValue($v),
            $row($i),
        )));
    }
    $w->close();
} else {
    $w = RustySheet\Writer::create($path);
    for ($i = 0; $i < $rows; $i++) {
        $w->addRow($row($i));
    }
    $w->close();
}
$secs = (hrtime(true) - $start) / 1e9;
printf("%-9s %s  rows: %d  time: %.2fs  rows/s: %s  size: %.1f MB  PHP peak: %.1f MB\n",
    $driver, basename($path), $rows, $secs, number_format($rows / $secs), filesize($path) / 1048576, memory_get_peak_usage(true) / 1048576);
