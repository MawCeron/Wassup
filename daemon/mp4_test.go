package main

import (
	"bytes"
	"encoding/binary"
	"testing"
)

func box(typ string, content ...[]byte) []byte {
	body := bytes.Join(content, nil)
	out := binary.BigEndian.AppendUint32(nil, uint32(8+len(body)))
	return append(append(out, typ...), body...)
}

func tkhd(width, height uint32) []byte {
	b := make([]byte, 84)
	binary.BigEndian.PutUint32(b[76:], width<<16)
	binary.BigEndian.PutUint32(b[80:], height<<16)
	return box("tkhd", b)
}

func TestMP4Info(t *testing.T) {
	mvhd := make([]byte, 100)
	binary.BigEndian.PutUint32(mvhd[12:], 1000) // Timescale.
	binary.BigEndian.PutUint32(mvhd[16:], 4500) // 4.5 s.
	// A 64-bit sized mdat first, as phones write for long videos.
	mdat := append(binary.BigEndian.AppendUint32(nil, 1), "mdat"...)
	mdat = binary.BigEndian.AppendUint64(mdat, 16+4)
	mdat = append(mdat, 0, 0, 0, 0)
	file := bytes.Join([][]byte{
		box("ftyp", []byte("mp42\x00\x00\x00\x00mp42isom")),
		mdat,
		box("moov", box("mvhd", mvhd), box("trak", tkhd(0, 0)), box("trak", tkhd(640, 360))),
	}, nil)
	seconds, width, height := mp4Info(bytes.NewReader(file), int64(len(file)))
	if seconds != 5 || width != 640 || height != 360 {
		t.Fatalf("got %ds %dx%d; want 5s 640x360", seconds, width, height)
	}
	if s, w, h := mp4Info(bytes.NewReader([]byte("not a video")), 11); s != 0 || w != 0 || h != 0 {
		t.Fatalf("garbage gave %d %d %d", s, w, h)
	}
}
