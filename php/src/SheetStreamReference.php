<?php

declare(strict_types=1);

namespace RustySheet\Dev;

use DateInterval;
use DateTimeImmutable;
use DateTimeInterface;
use DateTimeZone;
use OpenSpout\Common\Entity\Cell\FormulaCell;
use OpenSpout\Reader\CSV\Options as CsvOptions;
use OpenSpout\Reader\CSV\Reader as CsvReader;
use OpenSpout\Reader\ReaderInterface;
use OpenSpout\Reader\XLSX\Options;
use OpenSpout\Reader\XLSX\Reader;

/**
 * The parity reference: what laravel-sheet-stream yields today for an XLSX file.
 *
 * A port of sheet-stream's OpenSpout read path, as of commit cdd7beb:
 * `OpenSpoutSheetReader::rows()` + `CellNormalizer`. It applies the same steps:
 * Row::toArray() (or computed formula values) → trim trailing '' → timezone →
 * dates to serials.
 *
 * One intentional difference: `dateToSerial()` here does NOT subtract the extra day
 * for dates before 1900-03-01. That subtraction is a sheet-stream bug: OpenSpout never
 * added Excel's fictitious 1900-02-29, so time-only values come out a day early
 * (18:00 → -0.25). rusty_sheet returns the true serial (0.75).
 */
final class SheetStreamReference
{
    /**
     * @param array{dates?: 'serial'|'datetime', timezone?: ?string, format_dates?: bool,
     *              calculated_formulas?: bool, preserve_empty_rows?: bool,
     *              csv?: array{delimiter?: string, enclosure?: string, escape?: string}} $options
     *
     * `format_dates` is sheet-stream's WithFormatData (OpenSpout SHOULD_FORMAT_DATES).
     * @return array<string, list<list<mixed>>> sheet name => rows
     */
    public static function read(string $path, array $options = []): array
    {
        $dates = $options['dates'] ?? 'serial';
        $tz = isset($options['timezone']) ? new DateTimeZone($options['timezone']) : null;
        $calculated = $options['calculated_formulas'] ?? false;

        $isCsv = in_array(strtolower(pathinfo($path, PATHINFO_EXTENSION)), ['csv', 'tsv'], true);
        $reader = $isCsv ? self::csvReader($options) : new Reader(new Options(
            SHOULD_FORMAT_DATES: $options['format_dates'] ?? false,
            SHOULD_PRESERVE_EMPTY_ROWS: $options['preserve_empty_rows'] ?? false,
        ));
        $reader->open($path);

        try {
            $out = [];
            foreach ($reader->getSheetIterator() as $sheet) {
                $rows = [];
                foreach ($sheet->getRowIterator() as $row) {
                    $cells = $calculated
                        ? array_map(
                            fn ($c) => $c instanceof FormulaCell ? $c->getComputedValue() : $c->getValue(),
                            $row->cells,
                        )
                        : $row->toArray();

                    // sheet-stream trims dimension padding from XLSX rows only.
                    if (! $isCsv) {
                        $cells = self::trimTrailingEmpty($cells);
                    }

                    if ($tz !== null) {
                        $cells = array_map(
                            fn ($c) => $c instanceof DateTimeImmutable ? $c->setTimezone($tz) : $c,
                            $cells,
                        );
                    }
                    if ($dates === 'serial') {
                        $cells = array_map(self::toSerial(...), $cells);
                    }
                    $rows[] = $cells;
                }
                $out[$sheet->getName()] = $rows;
            }

            return $out;
        } finally {
            $reader->close();
        }
    }

    private static function csvReader(array $options): ReaderInterface
    {
        $csv = $options['csv'] ?? [];

        return new CsvReader(new CsvOptions(
            SHOULD_PRESERVE_EMPTY_ROWS: $options['preserve_empty_rows'] ?? false,
            FIELD_DELIMITER: $csv['delimiter'] ?? ',',
            FIELD_ENCLOSURE: $csv['enclosure'] ?? '"',
            FIELD_ESCAPE: $csv['escape'] ?? '',
        ));
    }

    /** @param list<mixed> $cells */
    private static function trimTrailingEmpty(array $cells): array
    {
        while ($cells !== [] && end($cells) === '') {
            array_pop($cells);
        }

        return $cells;
    }

    private static function toSerial(mixed $cell): mixed
    {
        if ($cell instanceof DateTimeInterface) {
            $days = (int) floor(($cell->getTimestamp() + $cell->getOffset()) / 86400) + 25569;
            // sheet-stream subtracts one here for $days < 61 (see class docblock).
            $seconds = ((int) $cell->format('G')) * 3600
                + ((int) $cell->format('i')) * 60
                + (int) $cell->format('s')
                + ((int) $cell->format('u')) / 1_000_000;

            return $seconds == 0 ? $days : $days + $seconds / 86400;
        }
        if ($cell instanceof DateInterval) {
            $serial = ($cell->d * 86400 + $cell->h * 3600 + $cell->i * 60 + $cell->s + $cell->f) / 86400;
            if ($cell->invert === 1) {
                $serial = -$serial;
            }

            return $serial == floor($serial) ? (int) $serial : $serial;
        }

        return $cell;
    }
}
