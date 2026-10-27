package main

import (
	"bytes"
	"encoding/binary"
	"testing"
)

func oggPage(granule uint64) []byte {
	page := []byte("OggS\x00\x04")
	page = binary.LittleEndian.AppendUint64(page, granule)
	return append(page, make([]byte, 30)...)
}

func TestOggOpusSeconds(t *testing.T) {
	file := append(append(oggPage(0), bytes.Repeat([]byte{7}, 100_000)...), oggPage(48_000*7+20_000)...)
	if got := oggOpusSeconds(bytes.NewReader(file), int64(len(file))); got != 7 {
		t.Fatalf("got %d s; want 7", got)
	}
	if got := oggOpusSeconds(bytes.NewReader([]byte("nope")), 4); got != 0 {
		t.Fatalf("not ogg gave %d", got)
	}
}
