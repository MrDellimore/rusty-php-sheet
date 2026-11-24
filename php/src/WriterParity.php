<?php

declare(strict_types=1);

namespace RustySheet\Dev;

use DateTimeInterface;
use OpenSpout\Common\Entity\Cell;
use OpenSpout\Common\Entity\Hyperlink;
use OpenSpout\Common\Entity\Row;
use OpenSpout\Common\Entity\Style\Border;
use OpenSpout\Common\Entity\Style\BorderName;
use OpenSpout\Common\Entity\Style\BorderPart;
use OpenSpout\Common\Entity\Style\BorderStyle;
use OpenSpout\Common\Entity\Style\BorderWidth;
use OpenSpout\Common\Entity\Style\CellAlignment;
use OpenSpout\Common\Entity\Style\CellVerticalAlignment;
use OpenSpout\Common\Entity\Style\Style;
use OpenSpout\Writer\AbstractWriterMultiSheets;
use OpenSpout\Writer\AutoFilter;
use OpenSpout\Writer\CSV\Options as CsvOptions;
use OpenSpout\Writer\CSV\Writer as CsvWriter;
use OpenSpout\Writer\WriterInterface;
use OpenSpout\Writer\XLSX\Entity\SheetView;
use OpenSpout\Writer\XLSX\Options as XlsxOptions;
use OpenSpout\Writer\XLSX\Writer as XlsxWriter;
use RustySheet\Writer;

/**
 * Writes the same scenario through OpenSpout and through the extension.
 *
 * A scenario is a list of operations:
 *   ['sheet', ?name]                       addSheet
 *   ['width', float, list<int> columns]    column widths (1-based)
 *   ['freeze', rows, columns]              frozen panes
 *   ['grid', bool]                         gridlines
 *   ['row', values, ?rowStyle, [col => style], ?height]
 *   ['merge', 'A1:C1']                     merged cells on the current sheet
 *   ['autofilter', 'A1:C10']               autofilter on the current sheet (before rows)
 * Values are plain PHP values, or ['cell' => value, 'style' => name] for a cell with
 * its own style, ['text' => string] for text that is never a formula, or
 * ['link' => text, 'target' => url or #place or sheet://place, 'tooltip' => ?, 'style' => ?].
 * Styles are referenced by name; one name is one style object (OpenSpout keys
 * styles by object identity).
 *
 * The OpenSpout side follows laravel-sheet-stream's OpenSpoutWriter::addRow
 * (commit cdd7beb): column style beats row style; dates get the column style if it
 * has a format, otherwise one shared date or date-time style.
 */
final class WriterParity
{
    /**
     * @param array<string, array<string, mixed>> $styles name => style spec
     */
    public static function writeOpenSpout(string $path, array $ops, array $styles, array $csv = []): void
    {
        self::ignoringNanWarning(fn () => self::writeOpenSpoutUnguarded($path, $ops, $styles, $csv));
    }

    /**
     * PHP 8.5 warns "unexpected NAN value was coerced to string" when a NAN becomes
     * text: in OpenSpout's writers, and in the extension's CSV writer, which uses
     * PHP's own strval() for floats. The scenarios write NAN on purpose, so only that
     * message is ignored; anything else still surfaces.
     */
    private static function ignoringNanWarning(callable $write): void
    {
        set_error_handler(static fn (int $no, string $msg): bool => str_starts_with($msg, 'unexpected NAN value'), E_WARNING);
        try {
            $write();
        } finally {
            restore_error_handler();
        }
    }

    private static function writeOpenSpoutUnguarded(string $path, array $ops, array $styles, array $csv): void
    {
        // .tsv is CSV with a tab, as Writer::create() treats it.
        $isCsv = str_ends_with($path, '.csv') || str_ends_with($path, '.tsv');
        if (str_ends_with($path, '.tsv')) {
            $csv += ['delimiter' => "\t"];
        }
        if ($isCsv) {
            $writer = new CsvWriter(new CsvOptions(...self::openSpoutCsvOptions($csv)));
        } else {
            // OpenSpout takes merges up front, by sheet index.
            $options = new XlsxOptions();
            $sheetIndex = 0;
            foreach ($ops as $i => $op) {
                if ($op[0] === 'sheet' && $i > array_search('sheet', array_column($ops, 0), true)) {
                    $sheetIndex++;
                }
                if ($op[0] === 'merge') {
                    [$c1, $r1, $c2, $r2] = self::bounds($op[1]);
                    $options->mergeCells($c1, $r1, $c2, $r2, $sheetIndex);
                }
            }
            $writer = new XlsxWriter($options);
        }
        $writer->openToFile($path);

        $objects = array_map(self::openSpoutStyle(...), $styles);
        $dateStyle = $dateTimeStyle = null;
        $first = true;
        $view = null;

        foreach ($ops as $op) {
            switch ($op[0]) {
                case 'sheet':
                    if ($first) {
                        $first = false;
                        if ($op[1] !== null && $writer instanceof AbstractWriterMultiSheets) {
                            $writer->getCurrentSheet()->setName($op[1]);
                        }
                    } else {
                        $sheet = $writer->addNewSheetAndMakeItCurrent();
                        if ($op[1] !== null) {
                            $sheet->setName($op[1]);
                        }
                    }
                    $view = null;
                    break;
                case 'width':
                    if ($writer instanceof AbstractWriterMultiSheets) {
                        $writer->getCurrentSheet()->setColumnWidth($op[1], ...$op[2]);
                    }
                    break;
                case 'freeze':
                case 'grid':
                    if ($writer instanceof AbstractWriterMultiSheets) {
                        $view ??= new SheetView();
                        $view = $op[0] === 'grid'
                            ? $view->withShowGridLines($op[1])
                            : $view->withFreezeRow($op[1] + 1)->withFreezeColumn(self::letters($op[2]));
                        $writer->getCurrentSheet()->setSheetView($view);
                    }
                    break;
                case 'autofilter':
                    if ($writer instanceof AbstractWriterMultiSheets) {
                        [$c1, $r1, $c2, $r2] = self::bounds($op[1]);
                        $writer->getCurrentSheet()->setAutoFilter(new AutoFilter($c1, $r1, $c2, $r2));
                    }
                    break;
                case 'row':
                    [, $values, $rowStyle, $colStyles, $height] = $op + [2 => null, 3 => [], 4 => null];
                    $style = $rowStyle !== null ? $objects[$rowStyle] : null;
                    $cells = [];
                    foreach (array_values($values) as $i => $value) {
                        $colStyle = isset($colStyles[$i]) ? $objects[$colStyles[$i]] : $style;
                        if (is_array($value) && array_key_exists('cell', $value)) {
                            $own = isset($value['style']) ? $objects[$value['style']] : null;
                            $cells[] = Cell::fromValue($value['cell'], $own);
                        } elseif (is_array($value) && array_key_exists('text', $value)) {
                            $own = isset($value['style']) ? $objects[$value['style']] : null;
                            $cells[] = new Cell\StringCell($value['text'], $own);
                        } elseif (is_array($value) && array_key_exists('link', $value)) {
                            $own = isset($value['style']) ? $objects[$value['style']] : null;
                            // PhpSpreadsheet's sheet:// form is a place in the workbook: '#…'.
                            $link = Hyperlink::toUrl(preg_replace('#^sheet://#', '#', $value['target']));
                            if (isset($value['tooltip'])) {
                                $link = $link->withTooltip($value['tooltip']);
                            }
                            $cells[] = $isCsv ? new Cell\StringCell($value['link'], $own) : new Cell\StringCell($value['link'], $own, null, $link);
                        } elseif ($value instanceof DateTimeInterface) {
                            if ($colStyle instanceof Style && $colStyle->format !== null) {
                                $ds = $colStyle;
                            } elseif ($value->format('H:i:s') !== '00:00:00') {
                                $ds = $dateTimeStyle ??= (new Style())->withFormat('yyyy-mm-dd hh:mm:ss');
                            } else {
                                $ds = $dateStyle ??= (new Style())->withFormat('yyyy-mm-dd');
                            }
                            $cells[] = new Cell\DateTimeCell($value, $ds);
                        } else {
                            $cells[] = Cell::fromValue($value, $colStyle);
                        }
                    }
                    $writer->addRow(new Row($cells, $height ?? 0));
                    break;
            }
        }
        $writer->close();
    }

    /** $autoSize: size every sheet's columns, so its rows take the buffered path. */
    public static function writeRusty(string $path, array $ops, array $styles, array $csv = [], bool $autoSize = false): void
    {
        self::ignoringNanWarning(fn () => self::writeRustyUnguarded($path, $ops, $styles, $csv, $autoSize));
    }

    private static function writeRustyUnguarded(string $path, array $ops, array $styles, array $csv, bool $autoSize): void
    {
        $writer = Writer::create($path, $csv === [] ? [] : ['csv' => $csv]);
        if ($autoSize) {
            $writer->autoSizeColumns();
        }
        $ids = [];
        $id = function (?string $name) use (&$ids, $styles, $writer): ?int {
            if ($name === null) {
                return null;
            }

            return $ids[$name] ??= $writer->addStyle($styles[$name]);
        };

        foreach ($ops as $op) {
            switch ($op[0]) {
                case 'sheet':
                    $writer->addSheet($op[1]);
                    if ($autoSize) {
                        $writer->autoSizeColumns(); // a new sheet (the first call only names the first)
                    }
                    break;
                case 'width':
                    $writer->setColumnWidth($op[1], $op[2]);
                    break;
                case 'freeze':
                    $writer->freezePanes($op[1], $op[2]);
                    break;
                case 'grid':
                    $writer->showGridLines($op[1]);
                    break;
                case 'merge':
                    $writer->mergeCells($op[1]);
                    break;
                case 'autofilter':
                    $writer->setAutoFilter($op[1]);
                    break;
                case 'row':
                    [, $values, $rowStyle, $colStyles, $height] = $op + [2 => null, 3 => [], 4 => null];
                    $mapped = array_map(function ($value) use ($id) {
                        if (is_array($value) && array_key_exists('cell', $value)) {
                            return Writer::cell($value['cell'], $id($value['style'] ?? null));
                        }
                        if (is_array($value) && array_key_exists('text', $value)) {
                            return Writer::text($value['text'], $id($value['style'] ?? null));
                        }
                        if (is_array($value) && array_key_exists('link', $value)) {
                            return Writer::link($value['link'], $value['target'], $id($value['style'] ?? null), $value['tooltip'] ?? null);
                        }

                        return $value;
                    }, array_values($values));
                    $writer->addRow($mapped, $id($rowStyle), array_map($id, $colStyles), $height);
                    break;
            }
        }
        $writer->close();
    }

    /**
     * Each cell's resolved style (font, fill, number format, alignment) instead of
     * its style id, so a mismatch reports what differs rather than a number.
     *
     * @param array<string, string> $parts from xlsxParts()
     * @return array<string, string>
     */
    public static function resolveStyles(array $parts): array
    {
        $styles = new \SimpleXMLElement($parts['xl/styles.xml']);
        $ns = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main';
        $styles->registerXPathNamespace('m', $ns);
        $formats = [];
        foreach ($styles->xpath('//m:numFmts/m:numFmt') as $f) {
            $formats[(string) $f['numFmtId']] = (string) $f['formatCode'];
        }
        $fonts = array_map(fn ($f) => $f->asXML(), $styles->xpath('//m:fonts/m:font'));
        $fills = array_map(fn ($f) => $f->asXML(), $styles->xpath('//m:fills/m:fill'));
        $borders = array_map(fn ($f) => $f->asXML(), $styles->xpath('//m:borders/m:border'));
        $resolved = [];
        foreach ($styles->xpath('//m:cellXfs/m:xf') as $i => $xf) {
            $id = (string) $xf['numFmtId'];
            $resolved[$i] = md5(implode('|', [
                $formats[$id] ?? 'builtin:'.$id,
                $fonts[(int) $xf['fontId']],
                $fills[(int) $xf['fillId']],
                $borders[(int) $xf['borderId']],
                (string) $xf['applyFont'],
                (string) $xf['applyBorder'],
                $xf->alignment ? $xf->alignment->asXML() : '',
            ]));
        }
        foreach ($parts as $name => $xml) {
            if (str_starts_with($name, 'xl/worksheets/')) {
                $parts[$name] = preg_replace_callback('# s="(\d+)"#', fn ($m) => ' s="'.$resolved[(int) $m[1]].'"', $xml);
            }
        }
        unset($parts['xl/styles.xml']);

        return $parts;
    }

    /**
     * The XML parts to compare, normalised for the deliberate differences:
     * no <dimension>, no comments/VML/shared-strings plumbing, and numbers compared
     * at OpenSpout's 14 significant digits.
     *
     * @return array<string, string>
     */
    public static function xlsxParts(string $path): array
    {
        $zip = new \ZipArchive();
        $zip->open($path);
        $parts = [];
        for ($i = 0; $i < $zip->numFiles; $i++) {
            $name = $zip->getNameIndex($i);
            if (! preg_match('#^xl/(workbook\.xml|styles\.xml|worksheets/sheet\d+\.xml|worksheets/_rels/sheet\d+\.xml\.rels)$#', $name)) {
                continue;
            }
            $xml = $zip->getFromIndex($i);
            if (str_ends_with($name, '.rels')) {
                // OpenSpout always relates its (empty) comments and VML parts; the
                // extension writes a .rels part only for links to URLs.
                $xml = preg_replace('#^  <Relationship Id="rId_comments[^\n]*\n#m', '', $xml);
                if (! str_contains($xml, '<Relationship ')) {
                    continue;
                }
            }
            $xml = preg_replace('#<dimension ref="[^"]*"/>|<legacyDrawing r:id="[^"]*"/>#', '', $xml);
            $xml = preg_replace_callback('#<v>([^<]*)</v>#', static function (array $m): string {
                return is_numeric($m[1]) ? '<v>'.sprintf('%.14G', (float) $m[1]).'</v>' : $m[0];
            }, $xml);
            $parts[$name] = $xml;
        }
        $zip->close();
        ksort($parts);

        return $parts;
    }

    /** 'B2:D5' → [1, 2, 3, 5]: 0-based columns, 1-based rows, as OpenSpout takes them. */
    private static function bounds(string $range): array
    {
        preg_match('/^\$?([A-Z]+)\$?(\d+):\$?([A-Z]+)\$?(\d+)$/', strtoupper($range), $m);
        $col = static function (string $letters): int {
            $n = 0;
            foreach (str_split($letters) as $ch) {
                $n = $n * 26 + ord($ch) - 64;
            }

            return $n - 1;
        };

        return [$col($m[1]), (int) $m[2], $col($m[3]), (int) $m[4]];
    }

    /** ['top' => ['style' => 'dashed', 'width' => 'thin', 'color' => 'FF0000'], 'all' => …] */
    private static function openSpoutBorder(array $spec): Border
    {
        $parts = [];
        foreach ($spec as $side => $part) {
            $sides = $side === 'all' ? ['left', 'right', 'top', 'bottom'] : [$side];
            foreach ($sides as $name) {
                $parts[] = new BorderPart(
                    BorderName::from($name),
                    $part['color'] ?? BorderPart::DEFAULT_COLOR,
                    isset($part['width']) ? BorderWidth::from($part['width']) : BorderPart::DEFAULT_WIDTH,
                    isset($part['style']) ? BorderStyle::from($part['style']) : BorderPart::DEFAULT_STYLE,
                );
            }
        }

        return new Border(...$parts);
    }

    private static function openSpoutStyle(array $spec): Style
    {
        $map = [
            'bold' => 'fontBold', 'italic' => 'fontItalic', 'underline' => 'fontUnderline',
            'strikethrough' => 'fontStrikethrough', 'size' => 'fontSize', 'color' => 'fontColor',
            'font' => 'fontName', 'background' => 'backgroundColor', 'format' => 'format',
            'wrap' => 'shouldWrapText', 'rotation' => 'textRotation', 'shrink' => 'shouldShrinkToFit',
        ];
        $args = [];
        foreach ($spec as $key => $value) {
            match ($key) {
                'align' => $args['cellAlignment'] = CellAlignment::from($value),
                'valign' => $args['cellVerticalAlignment'] = CellVerticalAlignment::from($value),
                'border' => $args['border'] = self::openSpoutBorder($value),
                default => $args[$map[$key]] = $value,
            };
        }

        return new Style(...$args);
    }

    private static function openSpoutCsvOptions(array $csv): array
    {
        $out = [];
        foreach (['delimiter' => 'FIELD_DELIMITER', 'enclosure' => 'FIELD_ENCLOSURE', 'bom' => 'SHOULD_ADD_BOM', 'eol' => 'EOL'] as $k => $v) {
            if (array_key_exists($k, $csv)) {
                $out[$v] = $csv[$k];
            }
        }

        return $out;
    }

    private static function letters(int $columns): string
    {
        $s = '';
        $n = $columns;
        do {
            $s = chr(65 + $n % 26).$s;
            $n = intdiv($n, 26) - 1;
        } while ($n >= 0);

        return $s;
    }
}
