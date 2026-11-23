<?php

declare(strict_types=1);

/*
 * Performance gate (`make bench-check`): read and write the 400k-row synthetic file
 * and compare with php/bench/baseline.json. Fails on > 10% slower or > 20% more
 * peak memory. Each scenario runs RUNS times; the best time and lowest RSS count.
 *
 * Usage: php php/bench/check.php <extension.so|dylib> <large.xlsx> [--update]
 * Env: BENCH_RUNS (default 3).
 */

[, $ext, $xlsx] = $argv;
$update = in_array('--update', $argv, true);
$runs = (int) (getenv('BENCH_RUNS') ?: 3);
$baselinePath = __DIR__.'/baseline.json';
$tmp = sys_get_temp_dir();
$php = PHP_BINARY;
const SCENARIOS = ['read-xlsx', 'read-csv', 'write-xlsx', 'write-csv'];
const MAX_SLOWDOWN = 1.10;
const MAX_MEMORY_GROWTH = 1.20;

if (! is_file($xlsx)) {
    fwrite(STDERR, "missing {$xlsx}: run `make large-fixture`\n");
    exit(1);
}
preg_match('/large-(\d+)\.xlsx$/', $xlsx, $m);
$rows = (int) ($m[1] ?? 400000);

// The CSV input is derived from the XLSX with the extension itself.
$csv = preg_replace('/\.xlsx$/', '.csv', $xlsx);
if (! is_file($csv)) {
    echo "creating {$csv}\n";
    passthru(sprintf('%s -d extension=%s -r %s', escapeshellarg($php), escapeshellarg($ext), escapeshellarg(
        '$r = RustySheet\Reader::open($argv[1]); $w = RustySheet\Writer::create($argv[2], ["csv" => ["bom" => false]]);'
        .' while (($b = $r->nextRows(0, 5000)) !== []) { foreach ($b as $row) { $w->addRow(array_map(fn ($v) => $v instanceof DateTimeInterface ? $v->format("Y-m-d") : $v, $row)); } } $w->close();'
    )).' '.escapeshellarg($xlsx).' '.escapeshellarg($csv), $code);
    if ($code !== 0) {
        exit(1);
    }
}

$results = [];
foreach (SCENARIOS as $s) {
    $best = null;
    for ($i = 0; $i < $runs; $i++) {
        $out = shell_exec(sprintf('%s -d extension=%s %s %s %s %d %s', escapeshellarg($php), escapeshellarg($ext),
            escapeshellarg(__DIR__.'/scenario.php'), $s, escapeshellarg($xlsx), $rows, escapeshellarg($tmp)));
        $r = json_decode((string) $out, true);
        if (! is_array($r)) {
            fwrite(STDERR, "scenario {$s} failed: {$out}\n");
            exit(1);
        }
        $best = $best === null ? $r : [
            'rows' => $r['rows'],
            'seconds' => min($best['seconds'], $r['seconds']),
            'rss_mb' => min($best['rss_mb'], $r['rss_mb']),
        ];
    }
    $best['rows_per_s'] = (int) round($best['rows'] / $best['seconds']);
    $results[$s] = $best;
}

$baseline = is_file($baselinePath) ? json_decode(file_get_contents($baselinePath), true) : [];
$failures = [];
printf("%-11s %10s %9s %9s   %s\n", 'scenario', 'rows/s', 'seconds', 'RSS MB', 'vs baseline');
foreach ($results as $s => $r) {
    $b = $baseline['scenarios'][$s] ?? null;
    $note = 'no baseline';
    if ($b !== null) {
        $time = $r['seconds'] / $b['seconds'];
        $mem = $r['rss_mb'] / $b['rss_mb'];
        $note = sprintf('time %+.1f%%, memory %+.1f%%', ($time - 1) * 100, ($mem - 1) * 100);
        if ($time > MAX_SLOWDOWN) {
            $failures[] = sprintf('%s is %.1f%% slower than the baseline', $s, ($time - 1) * 100);
        }
        if ($mem > MAX_MEMORY_GROWTH) {
            $failures[] = sprintf('%s uses %.1f%% more memory than the baseline', $s, ($mem - 1) * 100);
        }
    }
    printf("%-11s %10s %9.2f %9.1f   %s\n", $s, number_format($r['rows_per_s']), $r['seconds'], $r['rss_mb'], $note);
}

if ($update) {
    file_put_contents($baselinePath, json_encode([
        'machine' => php_uname('s').' '.php_uname('m').', PHP '.PHP_VERSION,
        'rows' => $rows,
        'scenarios' => $results,
    ], JSON_PRETTY_PRINT)."\n");
    echo "baseline updated: php/bench/baseline.json\n";
    exit(0);
}
if ($failures !== []) {
    echo "\nBenchmark check FAILED:\n  - ".implode("\n  - ", $failures)."\n";
    exit(1);
}
echo "Benchmark check passed.\n";
