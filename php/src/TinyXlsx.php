<?php

declare(strict_types=1);

namespace RustySheet\Dev;

use ZipArchive;

/**
 * Builds minimal XLSX files for tests of the reader's error paths and rare markup.
 * One sheet named "S"; the sheet XML is given as the content of <sheetData>.
 */
final class TinyXlsx
{
    private const NS = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main';

    /**
     * @param string               $rows    the XML inside <sheetData>
     * @param ?string              $styles  xl/styles.xml, or null for none
     * @param ?string              $encrypt password: encrypt the worksheet entry
     */
    public static function write(string $path, string $rows, ?string $styles = null, ?string $encrypt = null): string
    {
        $zip = new ZipArchive();
        $zip->open($path, ZipArchive::CREATE | ZipArchive::OVERWRITE);
        $zip->addFromString('[Content_Types].xml', '<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>');
        $zip->addFromString('_rels/.rels', '<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>');
        $zip->addFromString('xl/workbook.xml', '<?xml version="1.0"?><workbook xmlns="'.self::NS.'" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>');
        $zip->addFromString('xl/_rels/workbook.xml.rels', '<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>');
        if ($styles !== null) {
            $zip->addFromString('xl/styles.xml', $styles);
        }
        $zip->addFromString('xl/worksheets/sheet1.xml', '<?xml version="1.0"?><worksheet xmlns="'.self::NS.'"><sheetData>'.$rows.'</sheetData></worksheet>');
        if ($encrypt !== null) {
            $zip->setEncryptionName('xl/worksheets/sheet1.xml', ZipArchive::EM_TRAD_PKWARE, $encrypt);
        }
        $zip->close();

        return $path;
    }
}
