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

func TestExtension(t *testing.T) {
	for name, want := range map[string]string{
		"Report.PDF":            ".pdf",
		"archive.tar.gz":        ".gz",
		"x.reallylongextension": "",
		"evil.sh;rm -rf":        "",
	} {
		if got := extension(name, ""); got != want {
			t.Errorf("extension(%q) = %q, want %q", name, got, want)
		}
	}
	// Without a usable name, the type decides.
	if got := extension("no-name", "application/pdf"); got != ".pdf" {
		t.Errorf("by type: %q", got)
	}
}

func TestSendMediaNeedsConnection(t *testing.T) {
	accounts, err := openAccounts(t.Context(), t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer closeAccounts(accounts)
	if _, _, err := accounts[0].sendMedia(t.Context(), "x@s.whatsapp.net", "/etc/hostname", "", false); err == nil {
		t.Fatal("sent from an unlinked account")
	}
}

func TestRemoveLeftovers(t *testing.T) {
	dir := t.TempDir()
	for _, name := range []string{".download-123", ".upload-9", "abc.jpg"} {
		os.WriteFile(filepath.Join(dir, name), nil, 0o600)
	}
	removeLeftovers(dir)
	entries, _ := os.ReadDir(dir)
	if len(entries) != 1 || entries[0].Name() != "abc.jpg" {
		t.Fatalf("left %v; want only abc.jpg", entries)
	}
	if got := (&account{}).cacheLimit(); got != defaultMediaCacheMB<<20 {
		t.Fatalf("default cache limit %d", got)
	}
	var a account
	a.mediaCacheMB.Store(50)
	if got := a.cacheLimit(); got != 50<<20 {
		t.Fatalf("configured cache limit %d", got)
	}
}
