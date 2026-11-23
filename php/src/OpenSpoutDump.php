<?php

declare(strict_types=1);

namespace RustySheet\Dev;

use DateInterval;
use DateTimeInterface;
use OpenSpout\Reader\CSV\Reader as CsvReader;
use OpenSpout\Reader\ReaderInterface;
use OpenSpout\Reader\XLSX\Options as XlsxOptions;
use OpenSpout\Reader\XLSX\Reader as XlsxReader;

/**
 * Reads a spreadsheet with OpenSpout and returns raw rows exactly as
 * `Row::toArray()` produces them. This is the reference side of the parity tests.
 *
 * sheet-stream's own normalisation (trailing '' trim, CellNormalizer) is applied in
 * Phase 2, when there is a rusty_sheet side to compare against.
 */
final class OpenSpoutDump
{
    /**
     * @return array<string, list<list<mixed>>> sheet name => rows
     */
    public static function read(string $path, bool $preserveEmptyRows = false): array
    {
        $reader = self::readerFor($path, $preserveEmptyRows);
        $reader->open($path);

        try {
            $out = [];
            foreach ($reader->getSheetIterator() as $sheet) {
                $rows = [];
                foreach ($sheet->getRowIterator() as $row) {
                    $rows[] = $row->toArray();
                }
                $out[$sheet->getName()] = $rows;
            }

            return $out;
        } finally {
            $reader->close();
        }
    }

    /**
     * Make a value JSON-friendly while keeping its PHP type visible,
     * e.g. int(3) vs float(3.0), or '' vs null.
     */
    public static function describe(mixed $value): string
    {
        return match (true) {
            $value === null => 'null',
            is_bool($value) => 'bool(' . ($value ? 'true' : 'false') . ')',
            is_int($value) => "int($value)",
            is_float($value) => 'float(' . var_export($value, true) . ')',
            is_string($value) => 'string(' . json_encode($value, JSON_UNESCAPED_UNICODE) . ')',
            $value instanceof DateTimeInterface => 'datetime(' . $value->format('Y-m-d H:i:s.u e') . ')',
            $value instanceof DateInterval => 'interval(' . $value->format('%R %y-%m-%d %h:%i:%s.%f') . ')',
            default => get_debug_type($value),
        };
    }

    private static function readerFor(string $path, bool $preserveEmptyRows): ReaderInterface
    {
        return match (strtolower(pathinfo($path, PATHINFO_EXTENSION))) {
            'xlsx' => new XlsxReader(new XlsxOptions(SHOULD_PRESERVE_EMPTY_ROWS: $preserveEmptyRows)),
            'csv', 'tsv' => new CsvReader(),
            default => throw new \InvalidArgumentException("Unsupported fixture type: $path"),
        };
    }
}
