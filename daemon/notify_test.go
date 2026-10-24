package main

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/MawCeron/Wassup/daemon/ipc"
)

func TestNotificationText(t *testing.T) {
	group := &ipc.Chat{Name: "Family", IsGroup: true}
	msg := &ipc.Message{SenderName: "Ana", Text: "<b>see</b> & go"}

	title, body := notificationText(group, msg, false)
	if title != "Family" || body != "Ana: &lt;b&gt;see&lt;/b&gt; &amp; go" {
		t.Fatalf("got %q / %q", title, body)
	}
	photo := &ipc.Message{Text: "look", Media: &ipc.Media{Kind: ipc.MediaKind_MEDIA_KIND_IMAGE}}
	if _, body := notificationText(&ipc.Chat{Name: "Ana"}, photo, false); body != "Photo: look" {
		t.Fatalf("photo body: %q", body)
	}
	voice := &ipc.Message{Media: &ipc.Media{Kind: ipc.MediaKind_MEDIA_KIND_AUDIO, VoiceNote: true}}
	if _, body := notificationText(&ipc.Chat{Name: "Ana"}, voice, false); body != "Voice message" {
		t.Fatalf("voice note body: %q", body)
	}
	if _, body := notificationText(group, msg, true); body != "New message" {
		t.Fatalf("hidden content leaked: %q", body)
	}
}

func TestLoadConfig(t *testing.T) {
	dir := t.TempDir()
	if c, err := loadConfig(dir); err != nil || c.HideNotificationContent {
		t.Fatalf("missing file: %+v, %v", c, err)
	}
	write := func(s string) {
		if err := os.WriteFile(filepath.Join(dir, "config.json"), []byte(s), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	write(`{"hide_notification_content": true}`)
	if c, err := loadConfig(dir); err != nil || !c.HideNotificationContent {
		t.Fatalf("valid file: %+v, %v", c, err)
	}
	write(`{"media_cache_mb": 200}`)
	if c, err := loadConfig(dir); err != nil || c.MediaCacheMB != 200 {
		t.Fatalf("cache size: %+v, %v", c, err)
	}
	write(`{"media_cache_mb": -1}`)
	if _, err := loadConfig(dir); err == nil {
		t.Fatal("accepted a negative cache size")
	}
	write(`{"hide_notification_contents": true}`)
	if _, err := loadConfig(dir); err == nil {
		t.Fatal("a misspelled option was silently ignored")
	}
}
