<?php

declare(strict_types=1);

/*
 * Enforce the coverage rules from the implementation plan (Phase 6):
 *   - line targets per crate (rusty_sheet_core >= 95%, rusty_sheet >= 90%);
 *   - the ratchet: no crate drops below coverage/baseline.json;
 *   - every uncovered line is explained in coverage/exclusions.md (and every
 *     exclusion still matches an uncovered line, so the list stays accurate).
 *
 * Usage: php scripts/coverage-check.php <summary.json> <baseline.json> [--update-baseline]
 * The lcov file and exclusions are found next to them.
 */

[$self, $summaryPath, $baselinePath] = $argv + [null, null, null];
$update = in_array('--update-baseline', $argv, true);
$root = dirname(__DIR__);
$lcovPath = dirname($summaryPath).'/lcov.info';
$exclusionsPath = $root.'/coverage/exclusions.md';

const CRATES = [
    'rusty_sheet_core' => ['path' => 'crates/rusty_sheet_core/src/', 'lines' => 95.0],
    'rusty_sheet' => ['path' => 'crates/rusty_sheet/src/', 'lines' => 90.0],
];

function crate_of(string $file, string $root): ?string
{
    $rel = str_replace($root.'/', '', $file);
    foreach (CRATES as $name => $c) {
        if (str_starts_with($rel, $c['path'])) {
            return $name;
        }
    }

    return null;
}

// ---- totals per crate ----------------------------------------------------------
$summary = json_decode(file_get_contents($summaryPath), true, flags: JSON_THROW_ON_ERROR);
$totals = [];
foreach ($summary['data'][0]['files'] as $f) {
    $crate = crate_of($f['filename'], $root);
    if ($crate === null) {
        continue;
    }
    foreach (['lines', 'functions'] as $k) {
        $totals[$crate][$k]['covered'] = ($totals[$crate][$k]['covered'] ?? 0) + $f['summary'][$k]['covered'];
        $totals[$crate][$k]['count'] = ($totals[$crate][$k]['count'] ?? 0) + $f['summary'][$k]['count'];
    }
}
$pct = fn (array $t): float => $t['count'] === 0 ? 100.0 : round(100 * $t['covered'] / $t['count'], 2);

$failures = [];
$current = [];
echo "Coverage\n";
foreach (CRATES as $name => $c) {
    $lines = $pct($totals[$name]['lines']);
    $functions = $pct($totals[$name]['functions']);
    $current[$name] = ['lines' => $lines, 'functions' => $functions];
    printf("  %-17s lines %6.2f%% (target %.0f%%)   functions %6.2f%%\n", $name, $lines, $c['lines'], $functions);
    if ($lines < $c['lines']) {
        $failures[] = sprintf('%s: line coverage %.2f%% is below the %.0f%% target', $name, $lines, $c['lines']);
    }
}

// ---- ratchet ---------------------------------------------------------------------
$baseline = is_file($baselinePath) ? json_decode(file_get_contents($baselinePath), true, flags: JSON_THROW_ON_ERROR) : [];
foreach ($current as $name => $now) {
    foreach (['lines', 'functions'] as $k) {
        $was = $baseline[$name][$k] ?? 0.0;
        if ($now[$k] + 0.005 < $was) {
            $failures[] = sprintf('%s: %s coverage fell from %.2f%% to %.2f%% (baseline)', $name, $k, $was, $now[$k]);
        }
    }
}

// ---- every uncovered line explained ------------------------------------------------
$exclusions = [];
foreach (file($exclusionsPath) as $line) {
    // | `path` | `code` | reason |
    if (preg_match('/^\|\s*`([^`]+)`\s*\|\s*`(.+?)`\s*\|\s*(.+?)\s*\|\s*$/', $line, $m)) {
        $exclusions[] = ['path' => $m[1], 'code' => $m[2], 'reason' => $m[3], 'used' => false];
    }
}
$unexplained = [];
$explained = 0;
$file = null;
$source = [];
foreach (file($lcovPath) as $l) {
    $l = trim($l);
    if (str_starts_with($l, 'SF:')) {
        $file = substr($l, 3);
        $source = crate_of($file, $root) !== null ? file($file) : [];
    } elseif ($source !== [] && preg_match('/^DA:(\d+),0$/', $l, $m)) {
        $rel = str_replace($root.'/', '', $file);
        $text = trim($source[(int) $m[1] - 1]);
        $hit = false;
        foreach ($exclusions as &$e) {
            if ($e['path'] === $rel && str_contains($text, $e['code'])) {
                $e['used'] = $hit = true;
            }
        }
        unset($e);
        if ($hit) {
            $explained++;
        } else {
            $unexplained[] = "{$rel}:{$m[1]}  {$text}";
        }
    }
}
foreach ($unexplained as $u) {
    $failures[] = "uncovered and not in coverage/exclusions.md: {$u}";
}
foreach ($exclusions as $e) {
    if (! $e['used']) {
        $failures[] = "stale exclusion (now covered or gone; remove it): {$e['path']} `{$e['code']}`";
    }
}
printf("  %d uncovered lines explained in coverage/exclusions.md, %d unexplained\n", $explained, count($unexplained));

if ($update && $failures === []) {
    file_put_contents($baselinePath, json_encode($current, JSON_PRETTY_PRINT)."\n");
    echo "  baseline updated: coverage/baseline.json\n";
}

if ($failures !== []) {
    echo "\nCoverage check FAILED:\n  - ".implode("\n  - ", $failures)."\n";
    exit(1);
}
echo "Coverage check passed.\n";
