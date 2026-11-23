<?php

declare(strict_types=1);

use RustySheet\Dev\OpenSpoutDump;

// Phase 0 sanity check: OpenSpout (the parity reference) can read every fixture.
// If one of the hand-built edge fixtures is malformed, this is where it shows up.

it('is readable by OpenSpout', function (string $path) {
    $sheets = OpenSpoutDump::read($path);

    expect($sheets)->toBeArray()->not->toBeEmpty();
})->with(readable_fixtures());

it('exposes edge sheets in workbook order, not part order', function () {
    $sheets = OpenSpoutDump::read(fixtures_path('edge/multi-sheet.xlsx'));

    expect(array_keys($sheets))->toBe(['Zeta', 'Alpha & Ω', 'Empty', 'Last'])
        ->and($sheets['Zeta'][0])->toBe(['sheet Zeta', 1])
        ->and($sheets['Empty'])->toBe([]);
});
