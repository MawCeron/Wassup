package main

import (
	"encoding/binary"
	"io"
)

// mp4Info reads a video's length and picture size from its MP4 boxes,
// without decoding it: the duration from moov/mvhd, the size from the
// first track header with one (the video track; audio tracks have 0x0).
func mp4Info(r io.ReaderAt, size int64) (seconds, width, height uint32) {
	walkBoxes(r, 0, size, func(typ string, start, end int64) bool {
		switch typ {
		case "moov", "trak":
			return true // Containers: look inside.
		case "mvhd":
			var h [32]byte
			if n, _ := r.ReadAt(h[:], start); n < 24 {
				return false
			}
			var timescale, duration uint64
			if h[0] == 1 { // Version 1: 64-bit times.
				timescale = uint64(binary.BigEndian.Uint32(h[20:]))
				duration = binary.BigEndian.Uint64(h[24:])
			} else {
				timescale = uint64(binary.BigEndian.Uint32(h[12:]))
				duration = uint64(binary.BigEndian.Uint32(h[16:]))
			}
			if timescale > 0 {
				seconds = uint32((duration + timescale/2) / timescale)
			}
		case "tkhd":
			if width != 0 {
				return false
			}
			var version [1]byte
			r.ReadAt(version[:], start)
			at := start + 76 // Width and height, 16.16 fixed point, after the matrix.
			if version[0] == 1 {
				at = start + 88
			}
			var wh [8]byte
			if n, _ := r.ReadAt(wh[:], at); n == 8 && at+8 <= end {
				width, height = binary.BigEndian.Uint32(wh[:])>>16, binary.BigEndian.Uint32(wh[4:])>>16
			}
		}
		return false
	})
	return seconds, width, height
}

// walkBoxes calls visit with each box between from and to; visit returns
// true to descend into the box's content.
func walkBoxes(r io.ReaderAt, from, to int64, visit func(typ string, start, end int64) bool) {
	for from+8 <= to {
		var h [16]byte
		if n, _ := r.ReadAt(h[:8], from); n < 8 {
			return
		}
		size, header := int64(binary.BigEndian.Uint32(h[:])), int64(8)
		switch size {
		case 0: // Up to the end of the file.
			size = to - from
		case 1: // A 64-bit size follows the type.
			if n, _ := r.ReadAt(h[8:], from+8); n < 8 {
				return
			}
			size, header = int64(binary.BigEndian.Uint64(h[8:])), 16
		}
		if size < header || from+size > to {
			return
		}
		if visit(string(h[4:8]), from+header, from+size) {
			walkBoxes(r, from+header, from+size, visit)
		}
		from += size
	}
}
