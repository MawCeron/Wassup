package main

import (
	"bytes"
	"encoding/binary"
	"io"
)

// oggOpusSeconds is the length of an Ogg Opus file, from the granule
// position of its last page: Opus always counts in 48 kHz samples.
func oggOpusSeconds(r io.ReaderAt, size int64) uint32 {
	start := max(0, size-64<<10) // The last page is well within this.
	tail := make([]byte, size-start)
	if n, _ := r.ReadAt(tail, start); int64(n) < size-start {
		return 0
	}
	at := bytes.LastIndex(tail, []byte("OggS"))
	if at < 0 || at+14 > len(tail) {
		return 0
	}
	granule := binary.LittleEndian.Uint64(tail[at+6:])
	return uint32((granule + 24_000) / 48_000)
}
