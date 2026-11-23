<?php

declare(strict_types=1);

/*
 * The public API must be fully tested (implementation plan, Phase 6):
 *   - php/rusty_sheet.stub.php declares exactly what the loaded extension has;
 *   - every public method is called by some test;
 *   - every option key and every enumerated option value in the stub's array
 *     shapes appears in some test.
 * Adding a method or option without a test fails here.
 */

const STUB = __DIR__.'/../../rusty_sheet.stub.php';

/** All test sources except this file. */
function api_test_sources(): string
{
    $out = '';
    foreach (glob(__DIR__.'/*Test.php') as $file) {
        if (basename($file) !== basename(__FILE__)) {
            $out .= file_get_contents($file);
        }
    }

    return $out.file_get_contents(__DIR__.'/../../src/WriterParity.php');
}

/** @return array<string, list<string>> class => public methods, from the extension. */
function extension_api(): array
{
    $api = [];
    foreach ((new ReflectionExtension('rusty_sheet'))->getClasses() as $class) {
        // ext-php-rs gives every class a __construct that refuses `new` (tested in
        // ReaderTest and ErrorPathsTest); it isn't part of the stub.
        $methods = array_filter(
            $class->getMethods(ReflectionMethod::IS_PUBLIC),
            fn (ReflectionMethod $m) => $m->getDeclaringClass()->getName() === $class->getName()
                && $m->getName() !== '__construct',
        );
        $api[$class->getName()] = array_values(array_map(fn ($m) => $m->getName(), $methods));
        sort($api[$class->getName()]);
    }
    ksort($api);

    return $api;
}

/** @return array<string, list<string>> class => public methods, from the stub. */
function stub_api(): array
{
    $src = file_get_contents(STUB);
    // `class X {}` on one line, or a body that ends with `}` at the start of a line.
    preg_match_all('/^(?:final |abstract )?class (\w+)[^{\n]*\s*\{(\}|.*?^\})/ms', $src, $classes, PREG_SET_ORDER);
    $api = [];
    foreach ($classes as [, $name, $body]) {
        preg_match_all('/public (?:static )?function (\w+)\(/', $body, $m);
        $methods = $m[1];
        sort($methods);
        $api['RustySheet\\'.$name] = $methods;
    }
    ksort($api);

    return $api;
}

it('has a stub that matches the extension', function () {
    expect(stub_api())->toBe(extension_api());
    $functions = array_keys((new ReflectionExtension('rusty_sheet'))->getFunctions());
    foreach ($functions as $f) {
        expect(file_get_contents(STUB))->toContain("function {$f}(");
    }
});

it('calls every public method in some test', function () {
    $sources = api_test_sources();
    $missing = [];
    foreach (extension_api() as $class => $methods) {
        foreach ($methods as $method) {
            if (! preg_match('/(->|::)'.preg_quote($method, '/').'\(/', $sources)) {
                $missing[] = "{$class}::{$method}";
            }
        }
    }
    expect($missing)->toBe([]);
});

it('uses every documented option key and value in some test', function () {
    $sources = api_test_sources();
    $stub = file_get_contents(STUB);
    // Keys in the array shapes: `name?: type`. Values: quoted literals in the types.
    preg_match_all('/@param array\{(.*?)\}\s*\$\w+/s', $stub, $shapes);
    $missing = [];
    foreach ($shapes[1] as $shape) {
        preg_match_all('/(\w+)\?:/', $shape, $keys);
        foreach (array_unique($keys[1]) as $key) {
            if (! str_contains($sources, "'{$key}' =>")) {
                $missing[] = "key '{$key}'";
            }
        }
        preg_match_all("/'([\\w-]+)'(?=\\s*[|,}\\/])/", preg_replace('#//.*#', '', $shape), $values);
        foreach (array_unique($values[1]) as $value) {
            if (! preg_match("/=>\\s*'".preg_quote($value, '/')."'/", $sources)) {
                $missing[] = "value '{$value}'";
            }
        }
    }
    expect($missing)->toBe([]);
});
