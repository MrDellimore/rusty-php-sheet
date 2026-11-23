<?php

/**
 * IDE stubs for the rusty_sheet extension, for PhpStorm autocompletion and static analysis.
 * PHP never loads this file.
 *
 * Status:
 *   - rusty_sheet_version(), RustySheet\Reader (XLSX, CSV, TSV) and the exceptions: implemented.
 *   - RustySheet\Writer and RustySheet\Cell (XLSX, CSV, TSV): implemented.
 *
 * Keep this file in sync with crates/rusty_sheet/src as features land.
 */

namespace {
    /** Version of the loaded extension, e.g. "0.1.0". */
    function rusty_sheet_version(): string {}
}

namespace RustySheet {

/**
 * Base class of everything the extension throws. The extension's exceptions can be
 * caught but not constructed from PHP (`new` throws).
 */
class Exception extends \Exception {}

/**
 * Thrown for what this extension doesn't handle (xls, ods, CSV in encodings other than UTF-8).
 * Callers should catch it and fall back to OpenSpout.
 */
final class UnsupportedFormatException extends Exception {}

/** Unreadable or corrupt files, unknown sheets, use after close(). */
final class ReaderException extends Exception {}

/** Invalid sheet names, setup after the first row, unwritable values, use after close(). */
final class WriterException extends Exception {}

/**
 * Streaming spreadsheet reader. Not constructible with `new`; use Reader::open().
 *
 * Values match what laravel-sheet-stream yields through OpenSpout today, except for
 * deliberate fixes listed in docs/parity-notes.md.
 */
final class Reader
{
    /**
     * @param array{
     *   format?: 'xlsx'|'csv'|'tsv',             // default: from the file extension
     *   dates?: 'serial'|'datetime'|'formatted', // default 'serial'
     *   timezone?: ?string,                      // only with dates => 'datetime'
     *   calculated_formulas?: bool,              // default false → "=FORMULA" strings
     *   preserve_empty_rows?: bool,              // default false
     *   on_error?: 'throw'|'skip_row',           // recoverable row errors; default 'throw'
     *   csv?: array{
     *     delimiter?: string,                    // one byte; default ',' (tab for .tsv)
     *     enclosure?: string,                    // one byte; default '"'
     *     escape?: string,                       // '' (default) or one byte, as PHP fgetcsv()
     *     encoding?: string,                     // only UTF-8; others throw UnsupportedFormatException
     *   },
     * } $options
     *
     * CSV/TSV is parsed exactly like PHP's fgetcsv() (OpenSpout's CSV reader): one sheet
     * named '', every value a string, rows not trimmed, blank lines skipped unless
     * preserve_empty_rows (then [''] ), a UTF-8 BOM skipped, invalid UTF-8 passed through.
     *
     * Dates: 'serial' gives Excel serials (int for whole days, float otherwise; durations
     * as day fractions). 'datetime' gives DateTimeImmutable (PHP's default timezone, or
     * `timezone`) and DateInterval for elapsed-time formats like [h]:mm:ss. 'formatted' gives
     * the display string for the cell's number format, exactly as OpenSpout 5.15's
     * SHOULD_FORMAT_DATES renders it (sheet-stream's WithFormatData); other numbers stay numbers.
     *
     * @throws UnsupportedFormatException Format not handled: fall back to OpenSpout.
     * @throws ReaderException            File missing, not an XLSX, or corrupt.
     * @throws \ValueError                Invalid option.
     */
    public static function open(string $path, array $options = []): self {}

    /** @return list<string> Sheet names in workbook order, read without touching row data. */
    public function sheetNames(): array {}

    /**
     * Read the next batch of rows from a sheet; [] means the sheet is finished.
     *
     * Each row is a 0-indexed list of string|int|float|bool|null|\DateTimeImmutable,
     * trimmed after the last non-empty cell. Gaps inside a row are ''.
     *
     * @param int|string $sheet 0-based index or sheet name
     * @return array<int, list<mixed>> 1-based source row number (CSV: line number) => row
     *
     * Row errors (a cell past column XFD, a row past 1,048,576) throw unless
     * `on_error => 'skip_row'`; malformed XML or zip data always throws. After an
     * exception the sheet is finished: the next call returns [].
     *
     * @throws ReaderException Unknown sheet, corrupt data, or the reader is closed.
     * @throws \ValueError     $max < 1, or $sheet is neither int nor string.
     */
    public function nextRows(int|string $sheet, int $max = 1000): array {}

    /** Release the file handles. Any further call throws ReaderException. */
    public function close(): void {}
}

/**
 * Streaming spreadsheet writer producing the same files as OpenSpout 5.15's writers:
 * identical styles.xml and cell XML for XLSX, byte-identical CSV. Deliberate XLSX
 * differences: no <dimension> element, no empty comments/VML/shared-strings parts,
 * and numbers at full precision (OpenSpout keeps 14 significant digits).
 * Not constructible with `new`; use Writer::create().
 */
final class Writer
{
    /**
     * @param array{
     *   format?: 'xlsx'|'csv'|'tsv',
     *   date_format?: string,      // style for dates without time; default 'yyyy-mm-dd'
     *   datetime_format?: string,  // style for dates with time; default 'yyyy-mm-dd hh:mm:ss'
     *   csv?: array{delimiter?: string, enclosure?: string, bom?: bool, eol?: string},
     * } $options                   // CSV defaults: ',' (tab for .tsv), '"', BOM on, "\n"
     *
     * @throws UnsupportedFormatException
     * @throws WriterException
     * @throws \ValueError
     */
    public static function create(string $path, array $options = []): self {}

    /** The first call names the default sheet ("Sheet1"); later calls add a sheet (XLSX only). */
    public function addSheet(?string $name = null): void {}

    /**
     * Before the sheet's first row. Consecutive columns become one range. No-op for CSV.
     *
     * @param list<int> $columns 1-based
     */
    public function setColumnWidth(float $width, array $columns): void {}

    /** Freeze the top $rows rows and the left $columns columns. No-op for CSV. */
    public function freezePanes(int $rows, int $columns = 0): void {}

    public function showGridLines(bool $show): void {}

    /**
     * Merge a range of the current sheet, e.g. 'A1:H1', any time before the sheet ends
     * (the next addSheet() or close()). Written in call order. No-op for CSV.
     *
     * @throws \ValueError Not a range, a single cell, or overlapping an earlier merge.
     */
    public function mergeCells(string $range): void {}

    /**
     * Put an autofilter on a range of the current sheet, e.g. 'A1:F1', before the
     * sheet's first row (like OpenSpout, it changes the sheet header). No-op for CSV.
     *
     * @throws \ValueError     Not a range.
     * @throws WriterException After the sheet's first row.
     */
    public function setAutoFilter(string $range): void {}

    /**
     * Register a style; returns its id. Like OpenSpout (keyed by object identity),
     * every call is a distinct style. Defaults: Arial 11, black. Unstyled cells use
     * OpenSpout's fallback style (Calibri 12).
     *
     * Borders: one entry per side ('all' sets left, right, top and bottom). Each part
     * defaults to OpenSpout's medium solid black. 'hair' (a hairline, any width) goes
     * beyond OpenSpout, like PhpSpreadsheet's BORDER_HAIR. Styles whose parts were given
     * in a different order get separate border entries, as in OpenSpout.
     *
     * @param array{bold?: bool, italic?: bool, underline?: bool, strikethrough?: bool,
     *              size?: int, color?: string, font?: string, background?: string,
     *              format?: string, align?: string, valign?: string, wrap?: bool,
     *              rotation?: int, shrink?: bool,
     *              border?: array<'top'|'right'|'bottom'|'left'|'all', array{
     *                style?: 'solid'|'dashed'|'dotted'|'double'|'none'|'hair',
     *                width?: 'thin'|'medium'|'thick',
     *                color?: string,
     *              }>} $style  colors are 'RRGGBB'
     */
    public function addStyle(array $style): int {}

    /**
     * Values: string|int|float|bool|null|\DateTimeInterface|\DateInterval|Cell.
     * A string starting with "=" is written as a formula unless wrapped with text().
     * The style of a value is its column style, else the row style; a date uses its
     * column style only if that has a format, otherwise date_format/datetime_format.
     * A string containing "\n" gets wrap text. A row of empty values is skipped
     * (its row number is still used).
     *
     * @param array<int, int> $columnStyles 0-based column index => style id
     */
    public function addRow(array $values, ?int $rowStyle = null, array $columnStyles = [], ?float $height = null): void {}

    /** A value with its own style, used as is (no column/row style, no date style). */
    public static function cell(mixed $value, ?int $style = null): Cell {}

    /** A string that is never treated as a formula ('' is written as an empty string). */
    public static function text(string $value, ?int $style = null): Cell {}

    /**
     * Text with a hyperlink. $target is a URL, '#Sheet!A1' / "#'My sheet'!A1" for a
     * place in the workbook, or PhpSpreadsheet's "sheet://'My sheet'!A1" (the same).
     * CSV writes the text only.
     *
     * @throws \ValueError Empty target.
     */
    public static function link(string $text, string $target, ?int $style = null, ?string $tooltip = null): Cell {}

    /** Finish the file. Without it the file is incomplete. Closing twice is fine. */
    public function close(): void {}
}

/** A value with its own style, from Writer::cell() / Writer::text() / Writer::link(). */
final class Cell {}
}
