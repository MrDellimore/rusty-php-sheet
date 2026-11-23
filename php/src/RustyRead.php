<?php

declare(strict_types=1);

namespace RustySheet\Dev;

use RustySheet\Reader;

/** Read a whole workbook through the extension, the way an adapter would. */
final class RustyRead
{
    /**
     * @param array<string, mixed> $options passed to Reader::open()
     * @return array<string, list<list<mixed>>> sheet name => rows (row-number keys dropped)
     */
    public static function read(string $path, array $options = [], int $batch = 1000): array
    {
        $reader = Reader::open($path, $options);
        try {
            $out = [];
            foreach ($reader->sheetNames() as $i => $name) {
                $rows = [];
                $lastKey = 0;
                while (($batchRows = $reader->nextRows($i, $batch)) !== []) {
                    foreach ($batchRows as $number => $row) {
                        if ($number <= $lastKey) {
                            throw new \LogicException("row numbers must increase: $number after $lastKey");
                        }
                        $lastKey = $number;
                        $rows[] = $row;
                    }
                }
                $out[$name] = $rows;
            }

            return $out;
        } finally {
            $reader->close();
        }
    }
}
