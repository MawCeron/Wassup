package main

import (
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
)

func TestSaveAvatar(t *testing.T) {
	a := &account{avatarDir: t.TempDir()}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/big" {
			w.Write([]byte(strings.Repeat("x", avatarMaxBytes+1)))
			return
		}
		w.Write([]byte("jpeg"))
	}))
	defer srv.Close()

	dest := a.avatarPath("123@g.us", "42")
	if err := a.saveAvatar(srv.URL+"/ok", dest); err != nil {
		t.Fatal(err)
	}
	if b, _ := os.ReadFile(dest); string(b) != "jpeg" {
		t.Fatalf("saved %q", b)
	}
	if err := a.saveAvatar(srv.URL+"/big", a.avatarPath("123@g.us", "43")); err == nil {
		t.Fatal("accepted an oversized picture")
	}
	entries, _ := os.ReadDir(a.avatarDir)
	if len(entries) != 1 {
		t.Fatalf("leftover files after a failed download: %d entries", len(entries))
	}
	if a.avatarPath("123@g.us", "") != "" {
		t.Fatal("a chat without a picture got a path")
	}
}
