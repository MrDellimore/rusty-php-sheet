<?php

declare(strict_types=1);

it('is loaded', function () {
    expect(extension_loaded('rusty_sheet'))->toBeTrue(
        'rusty_sheet is not loaded. Run tests through `make test-php`, which passes -d extension=...'
    );
});

it('reports the crate version', function () {
    expect(rusty_sheet_version())->toMatch('/^\d+\.\d+\.\d+/');
});
