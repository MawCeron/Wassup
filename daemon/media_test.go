package main

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestEvictMediaRemovesLeastRecentlyUsed(t *testing.T) {
	root := t.TempDir()
	dir := filepath.Join(root, "account")
	if err := os.Mkdir(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	old := time.Now().Add(-time.Hour)
	for i, name := range []string{"oldest", "middle", "newest"} {
		path := filepath.Join(dir, name)
		if err := os.WriteFile(path, make([]byte, 100), 0o600); err != nil {
			t.Fatal(err)
		}
		at := old.Add(time.Duration(i) * time.Minute)
		os.Chtimes(path, at, at)
	}
	if err := evictMedia(root, 250); err != nil {
		t.Fatal(err)
	}
	for name, kept := range map[string]bool{"oldest": false, "middle": true, "newest": true} {
		_, err := os.Stat(filepath.Join(dir, name))
		if (err == nil) != kept {
			t.Errorf("%s kept=%v, want %v", name, err == nil, kept)
		}
	}
}
