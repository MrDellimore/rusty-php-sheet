<?php

declare(strict_types=1);

// One benchmark scenario in its own process, so peak RSS (Rust allocations
// included) belongs to it alone. Prints JSON: {"rows", "seconds", "rss_mb"}.
// Usage: php -d extension=… php/bench/scenario.php <read-xlsx|read-csv|write-xlsx|write-xlsx-autosize|write-csv> <input.xlsx> <rows> <tmpdir>

use RustySheet\Reader;
use RustySheet\Writer;

[, $name, $xlsx, $rows, $tmp] = $argv;
$rows = (int) $rows;
$csv = preg_replace('/\.xlsx$/', '.csv', $xlsx);

$readAll = function (string $path): int {
    $r = Reader::open($path);
    $n = 0;
    foreach ($r->sheetNames() as $i => $_) {
        while (($batch = $r->nextRows($i, 1000)) !== []) {
            $n += count($batch);
        }
    }

    return $n;
};

$date = new DateTimeImmutable('2025-06-15 14:30:00');
$write = function (string $path, bool $autoSize = false) use ($rows, $date): int {
    $w = Writer::create($path);
    if ($autoSize) {
        $w->autoSizeColumns();
    }
    for ($i = 0; $i < $rows; $i++) {
        $row = [];
        for ($c = 0; $c < 50; $c++) {
            $row[] = match ($c % 6) {
                0 => $i,
                1 => 'name '.($i % 997),
                2 => $i / 7,
                3 => $date,
                4 => ($i % 2) === 0,
                5 => ($i % 5) === 0 ? null : 'note '.$i,
            };
        }
        $w->addRow($row);
    }
    $w->close();

    return $rows;
};

$start = hrtime(true);
$count = match ($name) {
    'read-xlsx' => $readAll($xlsx),
    'read-csv' => $readAll($csv),
    'write-xlsx' => $write("{$tmp}/bench-write.xlsx"),
    'write-xlsx-autosize' => $write("{$tmp}/bench-write.xlsx", true),
    'write-csv' => $write("{$tmp}/bench-write.csv"),
};
$seconds = (hrtime(true) - $start) / 1e9;
@unlink("{$tmp}/bench-write.xlsx");
@unlink("{$tmp}/bench-write.csv");

// ru_maxrss is bytes on macOS, kilobytes on Linux.
$maxrss = getrusage()['ru_maxrss'];
$mb = PHP_OS_FAMILY === 'Darwin' ? $maxrss / 1048576 : $maxrss / 1024;
echo json_encode(['rows' => $count, 'seconds' => round($seconds, 3), 'rss_mb' => round($mb, 1)]), "\n";
