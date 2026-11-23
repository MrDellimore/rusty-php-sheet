<?php

declare(strict_types=1);

// Print what OpenSpout returns for a spreadsheet, with PHP types shown.
// Usage: php php/bin/dump-openspout.php <file> [--preserve-empty-rows]

require __DIR__ . '/../vendor/autoload.php';

use RustySheet\Dev\OpenSpoutDump;

$path = $argv[1] ?? null;
if ($path === null || !is_file($path)) {
    fwrite(STDERR, "usage: php php/bin/dump-openspout.php <file> [--preserve-empty-rows]\n");
    exit(1);
}

foreach (OpenSpoutDump::read($path, in_array('--preserve-empty-rows', $argv, true)) as $sheet => $rows) {
    echo "== $sheet (" . count($rows) . " rows)\n";
    foreach ($rows as $i => $row) {
        echo str_pad((string) ($i + 1), 4, ' ', STR_PAD_LEFT), ': ',
            implode(', ', array_map(OpenSpoutDump::describe(...), $row)), "\n";
    }
}
